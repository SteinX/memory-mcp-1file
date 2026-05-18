/// End-to-end integration tests for learning memory tools.
///
/// These tests exercise the handler-level logic functions (the same functions
/// called by the MCP handler dispatch) rather than internal helpers, covering
/// the full lifecycle: create → get → search → promote → reject/archive →
/// supersede → migrate_legacy dry-run.
///
/// Pattern follows the existing contract test style in
/// `src/server/logic/code/search.rs` and `src/server/logic/memory.rs`.
use memory_mcp::server::logic::learning;
use memory_mcp::server::params::{
    LearningMemoryArchiveParams, LearningMemoryCreateParams, LearningMemoryGetParams,
    LearningMemoryMigrateLegacyParams, LearningMemoryPromoteParams, LearningMemoryRejectParams,
    LearningMemorySearchParams, LearningMemorySupersededParams,
};
use memory_mcp::storage::StorageBackend;
use memory_mcp::test_utils::TestContext;
use serde_json::json;

// ─── helpers ─────────────────────────────────────────────────────────────────

fn tool_json(result: &rmcp::model::CallToolResult) -> serde_json::Value {
    let val = serde_json::to_value(result).unwrap();
    let text = val["content"][0]["text"].as_str().unwrap();
    serde_json::from_str(text).unwrap()
}

fn extract_id(json: &serde_json::Value) -> String {
    json["contract"]["identity"]["stable_memory_id"]
        .as_str()
        .unwrap_or_else(|| panic!("stable_memory_id not found in contract. JSON: {}", json))
        .to_string()
}

fn create_params(content: &str, kind: &str, status: Option<&str>) -> LearningMemoryCreateParams {
    LearningMemoryCreateParams {
        content: content.to_string(),
        kind: kind.to_string(),
        status: status.map(|s| s.to_string()),
        confidence: Some(0.8),
        scope: Some("global".to_string()),
        project_id: None,
        source: Some("manual".to_string()),
        evidence: None,
        applies_to: None,
        trigger_hints: None,
        constraints: None,
    }
}

// ─── contract field presence ──────────────────────────────────────────────────

/// Verify that `contract`, `summary`, and `learning_summary` are present in
/// every create response — the core contract requirement.
#[tokio::test]
async fn learning_create_response_has_contract_summary_learning_summary() {
    let ctx = TestContext::new().await;

    let result = learning::create(
        &ctx.state,
        create_params("Always use snake_case", "user_preference", None),
    )
    .await
    .unwrap();
    let json = tool_json(&result);

    assert!(json["contract"].is_object(), "contract must be an object");
    assert!(json["summary"].is_object(), "summary must be an object");
    assert!(
        json["learning_summary"].is_object(),
        "learning_summary must be an object"
    );
    assert!(
        json["contract"]["compatibility"]["clients_must_ignore_unknown_fields"]
            .as_bool()
            .unwrap_or(false),
        "contract.compatibility.clients_must_ignore_unknown_fields must be true"
    );
}

// ─── create → get ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn learning_create_then_get_returns_same_record() {
    let ctx = TestContext::new().await;

    // Create
    let create_result = learning::create(
        &ctx.state,
        create_params("Prefer explicit error handling", "project_lesson", None),
    )
    .await
    .unwrap();
    let create_json = tool_json(&create_result);
    let id = extract_id(&create_json);

    // Get
    let get_result = learning::get(&ctx.state, LearningMemoryGetParams { id: id.clone() })
        .await
        .unwrap();
    let get_json = tool_json(&get_result);

    assert_eq!(extract_id(&get_json), id, "get must return the same id");
    assert!(
        get_json["contract"].is_object(),
        "get response must have contract"
    );
    assert!(
        get_json["summary"].is_object(),
        "get response must have summary"
    );
    assert!(
        get_json["learning_summary"].is_object(),
        "get response must have learning_summary"
    );

    // Candidate by default
    assert_eq!(
        get_json["learning_summary"]["status"].as_str().unwrap(),
        "candidate"
    );
    assert_eq!(
        get_json["learning_summary"]["included_in_default_search"]
            .as_bool()
            .unwrap(),
        false,
        "candidate must be excluded from default search"
    );
}

// ─── candidate excluded from default search ───────────────────────────────────

#[tokio::test]
async fn candidate_excluded_from_default_search() {
    let ctx = TestContext::new().await;

    // Create a candidate
    let create_result = learning::create(
        &ctx.state,
        create_params(
            "UNIQUE_CANDIDATE_CONTENT_XYZ_12345",
            "project_pattern",
            Some("candidate"),
        ),
    )
    .await
    .unwrap();
    let create_json = tool_json(&create_result);
    let id = extract_id(&create_json);
    assert!(!id.is_empty());

    // Default search (confirmed+rule only) must NOT return the candidate
    let search_result = learning::search(
        &ctx.state,
        LearningMemorySearchParams {
            query: "UNIQUE_CANDIDATE_CONTENT_XYZ_12345".to_string(),
            filter: None,
            scope: None,
            project_id: None,
            limit: Some(20),
        },
    )
    .await
    .unwrap();
    let search_json = tool_json(&search_result);
    let records = search_json["records"].as_array().unwrap();
    let found = records.iter().any(|r| extract_id(r) == id);
    assert!(
        !found,
        "candidate must NOT appear in default search results"
    );
}

// ─── promote candidate → confirmed → appears in search ───────────────────────

#[tokio::test]
async fn promote_candidate_to_confirmed_appears_in_search() {
    let ctx = TestContext::new().await;

    // 1. Create candidate
    let create_result = learning::create(
        &ctx.state,
        create_params(
            "UNIQUE_PROMOTE_CONTENT_ABC_99887",
            "user_preference",
            Some("candidate"),
        ),
    )
    .await
    .unwrap();
    let create_json = tool_json(&create_result);
    let id = extract_id(&create_json);

    // 2. Verify candidate excluded from default search
    let search_before = learning::search(
        &ctx.state,
        LearningMemorySearchParams {
            query: "UNIQUE_PROMOTE_CONTENT_ABC_99887".to_string(),
            filter: None,
            scope: None,
            project_id: None,
            limit: Some(20),
        },
    )
    .await
    .unwrap();
    let search_before_json = tool_json(&search_before);
    let before_ids: Vec<String> = search_before_json["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| extract_id(r))
        .collect();
    assert!(
        !before_ids.contains(&id),
        "candidate must not appear in search before promotion"
    );

    // 3. Promote to confirmed
    let promote_result = learning::promote(
        &ctx.state,
        LearningMemoryPromoteParams {
            id: id.clone(),
            target_status: "confirmed".to_string(),
            target_kind: None,
        },
    )
    .await
    .unwrap();
    let promote_json = tool_json(&promote_result);
    assert_eq!(
        promote_json["learning_summary"]["status"].as_str().unwrap(),
        "confirmed"
    );
    assert_eq!(
        promote_json["learning_summary"]["included_in_default_search"]
            .as_bool()
            .unwrap(),
        true,
        "confirmed must be included in default search"
    );

    // 4. Verify confirmed appears in default search
    let search_after = learning::search(
        &ctx.state,
        LearningMemorySearchParams {
            query: "UNIQUE_PROMOTE_CONTENT_ABC_99887".to_string(),
            filter: None,
            scope: None,
            project_id: None,
            limit: Some(20),
        },
    )
    .await
    .unwrap();
    let search_after_json = tool_json(&search_after);
    let after_ids: Vec<String> = search_after_json["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| extract_id(r))
        .collect();
    assert!(
        after_ids.contains(&id),
        "confirmed record must appear in default search after promotion"
    );
}

// ─── reject → excluded from default search, included in audit search ──────────

#[tokio::test]
async fn reject_excludes_from_default_search_included_in_audit() {
    let ctx = TestContext::new().await;

    // Create confirmed record
    let create_result = learning::create(
        &ctx.state,
        create_params(
            "UNIQUE_REJECT_CONTENT_DEF_77665",
            "project_pitfall",
            Some("confirmed"),
        ),
    )
    .await
    .unwrap();
    let create_json = tool_json(&create_result);
    let id = extract_id(&create_json);

    // Reject it
    let reject_result = learning::reject(
        &ctx.state,
        LearningMemoryRejectParams {
            id: id.clone(),
            reason: Some("test rejection".to_string()),
        },
    )
    .await
    .unwrap();
    let reject_json = tool_json(&reject_result);
    assert_eq!(
        reject_json["learning_summary"]["status"].as_str().unwrap(),
        "rejected"
    );
    assert_eq!(
        reject_json["learning_summary"]["included_in_default_search"]
            .as_bool()
            .unwrap(),
        false,
        "rejected must be excluded from default search"
    );

    // Default search must NOT return it
    let search_default = learning::search(
        &ctx.state,
        LearningMemorySearchParams {
            query: "UNIQUE_REJECT_CONTENT_DEF_77665".to_string(),
            filter: None,
            scope: None,
            project_id: None,
            limit: Some(20),
        },
    )
    .await
    .unwrap();
    let default_json = tool_json(&search_default);
    let default_ids: Vec<String> = default_json["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| extract_id(r))
        .collect();
    assert!(
        !default_ids.contains(&id),
        "rejected record must NOT appear in default search"
    );

    // Audit search (include_invalidated=true) MUST return it
    let search_audit = learning::search(
        &ctx.state,
        LearningMemorySearchParams {
            query: "UNIQUE_REJECT_CONTENT_DEF_77665".to_string(),
            filter: Some(json!({ "include_invalidated": true, "audit": true })),
            scope: None,
            project_id: None,
            limit: Some(20),
        },
    )
    .await
    .unwrap();
    let audit_json = tool_json(&search_audit);
    let audit_ids: Vec<String> = audit_json["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| extract_id(r))
        .collect();
    assert!(
        audit_ids.contains(&id),
        "rejected record MUST appear in audit search"
    );
}

// ─── archive → excluded from default search, included in audit search ─────────

#[tokio::test]
async fn archive_excludes_from_default_search_included_in_audit() {
    let ctx = TestContext::new().await;

    // Create confirmed record
    let create_result = learning::create(
        &ctx.state,
        create_params(
            "UNIQUE_ARCHIVE_CONTENT_GHI_55443",
            "workflow_rule",
            Some("confirmed"),
        ),
    )
    .await
    .unwrap();
    let create_json = tool_json(&create_result);
    let id = extract_id(&create_json);

    // Archive it
    let archive_result = learning::archive(
        &ctx.state,
        LearningMemoryArchiveParams {
            id: id.clone(),
            reason: Some("test archive".to_string()),
        },
    )
    .await
    .unwrap();
    let archive_json = tool_json(&archive_result);
    assert_eq!(
        archive_json["learning_summary"]["status"].as_str().unwrap(),
        "archived"
    );
    assert_eq!(
        archive_json["learning_summary"]["included_in_default_search"]
            .as_bool()
            .unwrap(),
        false,
        "archived must be excluded from default search"
    );

    // Default search must NOT return it
    let search_default = learning::search(
        &ctx.state,
        LearningMemorySearchParams {
            query: "UNIQUE_ARCHIVE_CONTENT_GHI_55443".to_string(),
            filter: None,
            scope: None,
            project_id: None,
            limit: Some(20),
        },
    )
    .await
    .unwrap();
    let default_json2 = tool_json(&search_default);
    let default_ids: Vec<String> = default_json2["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| extract_id(r))
        .collect();
    assert!(
        !default_ids.contains(&id),
        "archived record must NOT appear in default search"
    );

    // Audit search MUST return it
    let search_audit = learning::search(
        &ctx.state,
        LearningMemorySearchParams {
            query: "UNIQUE_ARCHIVE_CONTENT_GHI_55443".to_string(),
            filter: Some(json!({ "include_invalidated": true, "audit": true })),
            scope: None,
            project_id: None,
            limit: Some(20),
        },
    )
    .await
    .unwrap();
    let audit_json2 = tool_json(&search_audit);
    let audit_ids: Vec<String> = audit_json2["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| extract_id(r))
        .collect();
    assert!(
        audit_ids.contains(&id),
        "archived record MUST appear in audit search"
    );
}

// ─── supersede → lineage tracked ─────────────────────────────────────────────

#[tokio::test]
async fn supersede_tracks_lineage() {
    let ctx = TestContext::new().await;

    // Create original confirmed record
    let orig_result = learning::create(
        &ctx.state,
        create_params("Original rule v1", "workflow_rule", Some("confirmed")),
    )
    .await
    .unwrap();
    let orig_json = tool_json(&orig_result);
    let orig_id = extract_id(&orig_json);

    // Create replacement record
    let repl_result = learning::create(
        &ctx.state,
        create_params("Replacement rule v2", "workflow_rule", Some("confirmed")),
    )
    .await
    .unwrap();
    let repl_json = tool_json(&repl_result);
    let repl_id = extract_id(&repl_json);

    // Supersede original with replacement
    let supersede_result = learning::supersede(
        &ctx.state,
        LearningMemorySupersededParams {
            id: orig_id.clone(),
            replacement_id: repl_id.clone(),
            reason: Some("updated rule".to_string()),
        },
    )
    .await
    .unwrap();
    let supersede_json = tool_json(&supersede_result);

    // Status must be superseded
    assert_eq!(
        supersede_json["learning_summary"]["status"]
            .as_str()
            .unwrap(),
        "superseded"
    );
    assert_eq!(
        supersede_json["learning_summary"]["included_in_default_search"]
            .as_bool()
            .unwrap(),
        false,
        "superseded must be excluded from default search"
    );

    // Lineage: superseded_by must point to replacement
    let record = &supersede_json["record"];
    let superseded_by = record["superseded_by"].as_str().unwrap_or("");
    assert!(
        superseded_by.contains(&repl_id) || superseded_by == repl_id,
        "superseded_by must reference the replacement id, got: {}",
        superseded_by
    );

    // invalidation_reason must be "superseded"
    let inv_reason = record["invalidation_reason"].as_str().unwrap_or("");
    assert_eq!(
        inv_reason, "superseded",
        "invalidation_reason must be 'superseded'"
    );

    // contract and summary must be present
    assert!(supersede_json["contract"].is_object());
    assert!(supersede_json["summary"].is_object());
}

// ─── migration dry-run → zero writes ─────────────────────────────────────────

#[tokio::test]
async fn migration_dry_run_zero_writes() {
    let ctx = TestContext::new().await;

    // Create a legacy-style memory (no learning metadata)
    use memory_mcp::server::logic::memory::store_memory;
    use memory_mcp::server::params::StoreMemoryParams;

    let legacy_params = StoreMemoryParams {
        content: "USER — Preference: always use snake_case for variable names".to_string(),
        memory_type: Some("semantic".to_string()),
        user_id: None,
        agent_id: None,
        run_id: None,
        namespace: None,
        importance_score: None,
        metadata: None,
    };
    store_memory(&ctx.state, legacy_params).await.unwrap();

    // Count records before dry-run
    let before_count = ctx.state.storage.count_memories().await.unwrap();

    // Run migration in dry_run=true mode
    let migrate_result = learning::migrate_legacy(
        &ctx.state,
        LearningMemoryMigrateLegacyParams {
            prefix_allowlist: None,
            scope: Some("global".to_string()),
            project_id: None,
            dry_run: true,
            limit: Some(50),
            include_invalidated: None,
            invalidate_source: None,
            extract_research_lessons: None,
        },
    )
    .await
    .unwrap();
    let migrate_json = tool_json(&migrate_result);

    // dry_run flag must be reflected in response
    assert_eq!(
        migrate_json["dry_run"].as_bool().unwrap_or(false),
        true,
        "response must report dry_run=true"
    );

    // Count records after dry-run — must be unchanged
    let after_count = ctx.state.storage.count_memories().await.unwrap();

    assert_eq!(
        before_count, after_count,
        "dry_run must not write any records (before={}, after={})",
        before_count, after_count
    );

    // counts.created must be 0
    assert_eq!(
        migrate_json["counts"]["created"].as_u64().unwrap_or(1),
        0,
        "dry_run must report 0 created records"
    );

    // contract must be present
    assert!(migrate_json["contract"].is_object());
}

// ─── promote to rule ──────────────────────────────────────────────────────────

#[tokio::test]
async fn promote_candidate_to_rule_appears_in_search() {
    let ctx = TestContext::new().await;

    // Create candidate
    let create_result = learning::create(
        &ctx.state,
        create_params(
            "UNIQUE_RULE_CONTENT_JKL_33221",
            "workflow_rule",
            Some("candidate"),
        ),
    )
    .await
    .unwrap();
    let create_json3 = tool_json(&create_result);
    let id = extract_id(&create_json3);
    let promote_result = learning::promote(
        &ctx.state,
        LearningMemoryPromoteParams {
            id: id.clone(),
            target_status: "rule".to_string(),
            target_kind: None,
        },
    )
    .await
    .unwrap();
    let promote_json = tool_json(&promote_result);
    assert_eq!(
        promote_json["learning_summary"]["status"].as_str().unwrap(),
        "rule"
    );
    assert_eq!(
        promote_json["learning_summary"]["included_in_default_search"]
            .as_bool()
            .unwrap(),
        true,
        "rule must be included in default search"
    );

    // Verify appears in search
    let search_result = learning::search(
        &ctx.state,
        LearningMemorySearchParams {
            query: "UNIQUE_RULE_CONTENT_JKL_33221".to_string(),
            filter: None,
            scope: None,
            project_id: None,
            limit: Some(20),
        },
    )
    .await
    .unwrap();
    let search_json3 = tool_json(&search_result);
    let ids: Vec<String> = search_json3["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| extract_id(r))
        .collect();
    assert!(
        ids.contains(&id),
        "rule record must appear in default search"
    );
}

// ─── search response contract fields ─────────────────────────────────────────

#[tokio::test]
async fn search_response_has_contract_and_summary() {
    let ctx = TestContext::new().await;

    let search_result = learning::search(
        &ctx.state,
        LearningMemorySearchParams {
            query: "anything".to_string(),
            filter: None,
            scope: None,
            project_id: None,
            limit: Some(5),
        },
    )
    .await
    .unwrap();
    let json = tool_json(&search_result);

    assert!(
        json["contract"].is_object(),
        "search response must have contract"
    );
    assert!(
        json["summary"].is_object(),
        "search response must have summary"
    );
    assert!(
        json["records"].is_array(),
        "search response must have records array"
    );
    assert!(json["count"].is_number(), "search response must have count");
}
