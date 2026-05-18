//! Code indexing, search, and symbol logic.
//!
//! This module is the handler facade — it re-exports all public functions from
//! the sub-modules so that callers (`server/handler.rs`, tests) can use the
//! original unqualified paths without changes.

mod indexing;
mod search;
mod symbols;

use std::sync::Arc;

use serde_json::{json, Value};

use crate::config::AppState;
use crate::storage::StorageBackend;
use crate::types::{
    CodeIntelligenceDiagnostic, ContractReasonCode, IndexState, IndexStatus,
    ServingGenerationMetadata,
};

// Re-export everything so external callers see the same flat API as before.
pub use indexing::{
    cancel_index, cleanup_abandoned_index_jobs, delete_project, get_degradation_info,
    get_index_status, get_project_projection, get_project_projection_by_locator, get_project_stats,
    index_project, list_projects,
};
pub use search::{recall_code, search_code};
pub(crate) use search::{recall_code_with_context, search_code_with_context};
pub(crate) use symbols::search_symbols_with_context;
pub use symbols::{search_symbols, symbol_graph};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CodeToolContext {
    session_id: Option<String>,
}

impl CodeToolContext {
    pub(crate) fn from_session_id(session_id: Option<String>) -> Self {
        Self { session_id }
    }

    pub(crate) fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    #[cfg(test)]
    pub(crate) async fn bound_project_id(&self, state: &Arc<AppState>) -> Option<String> {
        let session_id = self.session_id()?;
        state
            .session_bindings
            .binding_status(session_id)
            .await
            .project_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProjectResolutionSource {
    Explicit,
    SessionBinding,
    CrossProject,
}

impl ProjectResolutionSource {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Explicit => "explicit",
            Self::SessionBinding => "session_binding",
            Self::CrossProject => "cross_project",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectResolution {
    pub project_id: Option<String>,
    pub source: ProjectResolutionSource,
    pub reason_code: Option<ContractReasonCode>,
    pub binding_state: Option<&'static str>,
}

impl ProjectResolution {
    pub(crate) fn project_id(&self) -> Option<&str> {
        self.project_id.as_deref()
    }

    pub(crate) fn is_stale_binding(&self) -> bool {
        self.reason_code == Some(ContractReasonCode::Stale)
            && self.binding_state == Some("stale_binding")
    }

    pub(crate) fn as_json(&self) -> Value {
        let mut value = json!({
            "source": self.source.as_str(),
            "project_id": self.project_id,
        });

        if let Some(reason_code) = &self.reason_code {
            value["reason_code"] = json!(reason_code);
        }
        if let Some(binding_state) = self.binding_state {
            value["binding_state"] = json!(binding_state);
        }

        value
    }
}

pub(crate) async fn resolve_project_for_code_tool(
    state: &Arc<AppState>,
    explicit_project_id: Option<String>,
    context: Option<&CodeToolContext>,
) -> ProjectResolution {
    if let Some(project_id) = explicit_project_id {
        return ProjectResolution {
            project_id: Some(project_id),
            source: ProjectResolutionSource::Explicit,
            reason_code: None,
            binding_state: None,
        };
    }

    let Some(context) = context else {
        return ProjectResolution {
            project_id: None,
            source: ProjectResolutionSource::CrossProject,
            reason_code: None,
            binding_state: None,
        };
    };
    let Some(session_id) = context.session_id() else {
        return ProjectResolution {
            project_id: None,
            source: ProjectResolutionSource::CrossProject,
            reason_code: None,
            binding_state: None,
        };
    };

    let binding = state.session_bindings.binding_status(session_id).await;
    let Some(project_id) = binding.project_id else {
        return ProjectResolution {
            project_id: None,
            source: ProjectResolutionSource::CrossProject,
            reason_code: None,
            binding_state: None,
        };
    };

    if missing_project_binding_diagnostic(state, Some(&project_id))
        .await
        .is_some()
    {
        return ProjectResolution {
            project_id: Some(project_id),
            source: ProjectResolutionSource::SessionBinding,
            reason_code: Some(ContractReasonCode::Stale),
            binding_state: Some("stale_binding"),
        };
    }

    ProjectResolution {
        project_id: Some(project_id),
        source: ProjectResolutionSource::SessionBinding,
        reason_code: None,
        binding_state: None,
    }
}

pub(crate) fn apply_project_resolution(response: &mut Value, resolution: &ProjectResolution) {
    response["project_resolution"] = resolution.as_json();

    if resolution.is_stale_binding() {
        response["reason_code"] = json!(ContractReasonCode::Stale);
        let partial = &mut response["summary"]["partial"];
        partial["is_partial"] = json!(true);
        let existing_reason = partial["reason_code"].as_str().unwrap_or("");
        if existing_reason != "missing" && existing_reason != "no_serving_generation" {
            partial["reason_code"] = json!(ContractReasonCode::Stale);
            partial["reason"] = json!("stale_binding");
            partial["message"] = json!(
                "Session-bound project is no longer registered or indexed; refusing cross-project fallback."
            );
        }
    }
}

pub(crate) fn completed_semantic_generation_caught_up(status: &IndexStatus) -> bool {
    status.status == IndexState::Completed
        && status.semantic_generation == status.structural_generation
}

pub(crate) async fn effective_indexing_generation_for_project(
    state: &Arc<AppState>,
    project_id: &str,
    serving_generation: Option<u64>,
    fallback_generation: Option<u64>,
    status_hint: Option<&IndexStatus>,
) -> (Option<u64>, bool) {
    let completed = match status_hint {
        Some(status) => completed_semantic_generation_caught_up(status),
        None => state
            .storage
            .get_index_status(project_id)
            .await
            .ok()
            .flatten()
            .as_ref()
            .map(completed_semantic_generation_caught_up)
            .unwrap_or(false),
    };

    if completed {
        return (None, false);
    }

    let explicit = state
        .storage
        .get_indexing_generation(project_id)
        .await
        .ok()
        .flatten();
    let abandoned_max = state
        .storage
        .list_abandoned_generations(project_id)
        .await
        .ok()
        .and_then(|generations| {
            generations
                .into_iter()
                .filter(|generation| Some(*generation) != serving_generation)
                .max()
        });
    let interrupted = explicit.is_none()
        && match (abandoned_max, serving_generation) {
            (Some(abandoned), Some(serving)) => abandoned > serving,
            _ => false,
        };

    (
        explicit.or(abandoned_max).or(fallback_generation),
        interrupted,
    )
}

pub(crate) fn completed_serving_metadata(
    mut serving: ServingGenerationMetadata,
    status: Option<&IndexStatus>,
) -> ServingGenerationMetadata {
    if status
        .map(completed_semantic_generation_caught_up)
        .unwrap_or(false)
    {
        serving.indexing = None;
    }
    serving
}

pub(crate) struct MissingProjectBindingDiagnostic {
    pub code_intelligence: serde_json::Value,
    pub project_binding: serde_json::Value,
    pub reason_code: ContractReasonCode,
    pub reason: String,
    pub message: String,
}

pub(crate) async fn missing_project_binding_diagnostic(
    state: &Arc<AppState>,
    project_id: Option<&str>,
) -> Option<MissingProjectBindingDiagnostic> {
    let project_id = project_id?;

    if state.project_registry.get(project_id).await.is_some() {
        return None;
    }

    let has_status = matches!(
        state.storage.get_index_status(project_id).await,
        Ok(Some(_))
    );
    if has_status {
        return None;
    }

    let has_index_rows = state
        .storage
        .count_chunks(project_id, None)
        .await
        .unwrap_or(0)
        > 0
        || state
            .storage
            .count_symbols(project_id, None)
            .await
            .unwrap_or(0)
            > 0
        || state
            .storage
            .count_manifest_entries(project_id)
            .await
            .unwrap_or(0)
            > 0;
    if has_index_rows {
        return None;
    }

    let message = format!(
        "Requested project_id '{project_id}' is not registered on this server and has no usable index status. Mount a server-visible project root, then register/index it via index_project (or startup PROJECT_PATH/--project-path). Client-local paths are not server-visible unless mounted."
    );

    Some(MissingProjectBindingDiagnostic {
        code_intelligence: CodeIntelligenceDiagnostic::degraded(message.clone()).as_json(),
        project_binding: json!({
            "project_id": project_id,
            "state": "missing",
            "reason_code": ContractReasonCode::Missing,
            "remediation": {
                "mount_server_visible_root": true,
                "register_or_index": "run index_project with a server-visible path (or set PROJECT_PATH/--project-path)",
                "note": "client-local absolute paths are not visible to the server unless mounted"
            }
        }),
        reason_code: ContractReasonCode::Missing,
        reason: "project_missing".to_string(),
        message,
    })
}

pub(crate) fn apply_missing_project_binding_diagnostic(
    response: &mut serde_json::Value,
    diagnostic: &MissingProjectBindingDiagnostic,
) {
    response["code_intelligence"] = diagnostic.code_intelligence.clone();
    response["project_binding"] = diagnostic.project_binding.clone();
    response["reason_code"] = json!(diagnostic.reason_code);

    let partial = &mut response["summary"]["partial"];
    partial["is_partial"] = json!(true);
    partial["reason_code"] = json!(diagnostic.reason_code);
    partial["reason"] = json!(diagnostic.reason);
    partial["message"] = json!(diagnostic.message);
}

#[cfg(test)]
mod tests {
    use super::CodeToolContext;
    use crate::server::params::{
        GetIndexStatusParams, GetProjectProjectionParams, GetProjectStatsParams,
        GetProjectionByLocatorParams, IndexProjectParams, RecallCodeParams, SearchCodeParams,
        SearchSymbolsParams, SymbolGraphParams,
    };
    use crate::storage::StorageBackend;
    use crate::test_utils::TestContext;
    use crate::types::{
        CapabilityKind, CodeChunk, CodeRelationType, CodeSymbol, ConfidenceClass, Datetime,
        IndexFileCheckpoint, IndexJobPhase, IndexState, IndexStatus, RelationClass,
        RelationProvenance, StalenessState, SymbolRelation, SymbolType,
    };
    use std::fs;

    #[tokio::test]
    async fn test_code_logic_flow() {
        let ctx = TestContext::new().await;
        let unique_id = format!("test_project_{}", uuid::Uuid::new_v4().simple());
        let project_path = ctx._temp_dir.path().join(&unique_id);
        fs::create_dir_all(&project_path).unwrap();
        fs::write(
            project_path.join("main.rs"),
            "fn main() { println!(\"Hello\"); }",
        )
        .unwrap();

        let index_params = IndexProjectParams {
            path: Some(project_path.to_string_lossy().to_string()),
            project_id: None,
            resume: None,
            job_id: None,
            resume_token: None,
            allow_full_restart_fallback: None,
            force: None,
            confirm_failed_restart: None,
            include_patterns: None,
            exclude_patterns: None,
        };

        // 1. Trigger Indexing
        let result = super::index_project(&ctx.state, index_params)
            .await
            .unwrap();
        // Should return "indexing" status immediately
        if let rmcp::model::RawContent::Text(t) = &result.content[0].raw {
            assert!(t.text.contains("indexing"));
        } else {
            panic!("Expected text content");
        }

        // 2. Wait for indexing to complete
        // Since it's a background task, we poll get_index_status
        let status_params = GetIndexStatusParams {
            project_id: unique_id.clone(),
        };

        let mut retries = 0;
        let mut last_status = String::new();
        loop {
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
            let res = super::get_index_status(&ctx.state, status_params.clone())
                .await
                .unwrap();
            if let rmcp::model::RawContent::Text(t) = &res.content[0].raw {
                last_status = t.text.clone();
                // In tests the embedding queue has no receiver so embeddings never
                // complete; accept either fully-completed or embedding_pending (AST done).
                let indexing_done = t.text.contains("\"status\":\"completed\"")
                    || t.text.contains("\"status\":\"embedding_pending\"");
                if indexing_done {
                    // Give the BM25 index a moment to be rebuilt after indexing
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    break;
                }
            }
            retries += 1;
            if retries > 100 {
                panic!("Indexing timed out. Last status: {}", last_status);
            }
        }

        // 3. Search Code
        let search_params = SearchCodeParams {
            query: "Hello".to_string(),
            project_id: Some(unique_id.clone()),
            limit: Some(5),
        };
        let search_res = super::search_code(&ctx.state, search_params).await.unwrap();

        if let rmcp::model::RawContent::Text(t) = &search_res.content[0].raw {
            assert!(
                t.text.contains("main.rs"),
                "Expected 'main.rs' in search results. Got: {}",
                &t.text[..std::cmp::min(500, t.text.len())]
            );
            assert!(t.text.contains("Hello"));
        } else {
            panic!("Expected text content");
        }
    }

    #[tokio::test]
    async fn search_code_treats_blank_project_id_as_absent() {
        let ctx = TestContext::new().await;
        let unique_id = format!("test_project_blank_{}", uuid::Uuid::new_v4().simple());
        let project_path = ctx._temp_dir.path().join(&unique_id);
        fs::create_dir_all(&project_path).unwrap();
        fs::write(
            project_path.join("main.rs"),
            "fn main() { println!(\"Hello Blank\"); }",
        )
        .unwrap();

        super::index_project(
            &ctx.state,
            IndexProjectParams {
                path: Some(project_path.to_string_lossy().to_string()),
                project_id: None,
                resume: None,
                job_id: None,
                resume_token: None,
                allow_full_restart_fallback: None,
                force: None,
                confirm_failed_restart: None,
                include_patterns: None,
                exclude_patterns: None,
            },
        )
        .await
        .unwrap();

        let status_params = GetIndexStatusParams {
            project_id: unique_id.clone(),
        };

        let mut retries = 0;
        loop {
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
            let res = super::get_index_status(&ctx.state, status_params.clone())
                .await
                .unwrap();
            if let rmcp::model::RawContent::Text(t) = &res.content[0].raw {
                let indexing_done = t.text.contains("\"status\":\"completed\"")
                    || t.text.contains("\"status\":\"embedding_pending\"");
                if indexing_done {
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    break;
                }
            }
            retries += 1;
            assert!(
                retries <= 100,
                "Indexing timed out for blank project_id test"
            );
        }

        let search_res = super::search_code(
            &ctx.state,
            SearchCodeParams {
                query: "Hello Blank".to_string(),
                project_id: Some("   ".to_string()),
                limit: Some(5),
            },
        )
        .await
        .unwrap();

        if let rmcp::model::RawContent::Text(t) = &search_res.content[0].raw {
            assert!(
                t.text.contains("Hello Blank"),
                "Expected blank project_id to fall back to global search. Got: {}",
                t.text
            );
        } else {
            panic!("Expected text content");
        }
    }

    #[tokio::test]
    async fn search_symbols_treats_blank_project_id_as_absent() {
        let ctx = TestContext::new().await;
        let unique_id = format!("test_symbols_blank_{}", uuid::Uuid::new_v4().simple());
        let project_path = ctx._temp_dir.path().join(&unique_id);
        fs::create_dir_all(&project_path).unwrap();
        fs::write(project_path.join("lib.rs"), "fn hello_symbol() {}\n").unwrap();

        super::index_project(
            &ctx.state,
            IndexProjectParams {
                path: Some(project_path.to_string_lossy().to_string()),
                project_id: None,
                resume: None,
                job_id: None,
                resume_token: None,
                allow_full_restart_fallback: None,
                force: None,
                confirm_failed_restart: None,
                include_patterns: None,
                exclude_patterns: None,
            },
        )
        .await
        .unwrap();

        let status_params = GetIndexStatusParams {
            project_id: unique_id.clone(),
        };

        let mut retries = 0;
        loop {
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
            let res = super::get_index_status(&ctx.state, status_params.clone())
                .await
                .unwrap();
            if let rmcp::model::RawContent::Text(t) = &res.content[0].raw {
                let indexing_done = t.text.contains("\"status\":\"completed\"")
                    || t.text.contains("\"status\":\"embedding_pending\"");
                if indexing_done {
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    break;
                }
            }
            retries += 1;
            assert!(
                retries <= 100,
                "Indexing timed out for blank symbol project_id test"
            );
        }

        let symbol_res = super::search_symbols(
            &ctx.state,
            SearchSymbolsParams {
                query: "hello_symbol".to_string(),
                project_id: Some("  ".to_string()),
                limit: Some(10),
                offset: Some(0),
                symbol_type: None,
                path_prefix: None,
            },
        )
        .await
        .unwrap();

        if let rmcp::model::RawContent::Text(t) = &symbol_res.content[0].raw {
            assert!(
                t.text.contains("hello_symbol"),
                "Expected blank project_id to fall back to unfiltered symbol search. Got: {}",
                t.text
            );
            assert!(
                t.text.contains("\"project_id\":null"),
                "Expected normalized filter to be null. Got: {}",
                t.text
            );
        } else {
            panic!("Expected text content");
        }
    }

    #[tokio::test]
    async fn search_code_reports_missing_project_binding_for_unregistered_project_id() {
        let ctx = TestContext::new().await;

        let search_res = super::search_code(
            &ctx.state,
            SearchCodeParams {
                query: "missing project".to_string(),
                project_id: Some("unregistered_project_binding".to_string()),
                limit: Some(5),
            },
        )
        .await
        .unwrap();

        if let rmcp::model::RawContent::Text(t) = &search_res.content[0].raw {
            let json: serde_json::Value = serde_json::from_str(&t.text).unwrap();
            assert_eq!(json["reason_code"], "missing");
            assert_eq!(json["summary"]["partial"]["reason_code"], "missing");
            assert_eq!(json["summary"]["partial"]["reason"], "project_missing");
            assert_eq!(json["project_binding"]["state"], "missing");
            assert_eq!(
                json["project_binding"]["project_id"],
                "unregistered_project_binding"
            );
            assert_eq!(json["code_intelligence"]["status"], "degraded");
            assert_eq!(json["code_intelligence"]["reason_code"], "degraded");
            assert!(json["summary"]["partial"]["message"]
                .as_str()
                .unwrap()
                .contains("server-visible"));
        } else {
            panic!("Expected text content");
        }
    }

    #[tokio::test]
    async fn search_symbols_skips_missing_project_binding_when_project_is_registered() {
        let ctx = TestContext::new().await;
        let unique_id = format!("test_registered_symbols_{}", uuid::Uuid::new_v4().simple());
        let project_path = ctx._temp_dir.path().join(&unique_id);
        fs::create_dir_all(&project_path).unwrap();
        fs::write(project_path.join("lib.rs"), "fn registered_symbol() {}\n").unwrap();

        super::index_project(
            &ctx.state,
            IndexProjectParams {
                path: Some(project_path.to_string_lossy().to_string()),
                project_id: None,
                resume: None,
                job_id: None,
                resume_token: None,
                allow_full_restart_fallback: None,
                force: None,
                confirm_failed_restart: None,
                include_patterns: None,
                exclude_patterns: None,
            },
        )
        .await
        .unwrap();

        let status_params = GetIndexStatusParams {
            project_id: unique_id.clone(),
        };

        let mut retries = 0;
        loop {
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
            let res = super::get_index_status(&ctx.state, status_params.clone())
                .await
                .unwrap();
            if let rmcp::model::RawContent::Text(t) = &res.content[0].raw {
                let indexing_done = t.text.contains("\"status\":\"completed\"")
                    || t.text.contains("\"status\":\"embedding_pending\"");
                if indexing_done {
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    break;
                }
            }
            retries += 1;
            assert!(
                retries <= 100,
                "Indexing timed out for registered project diagnostic test"
            );
        }

        let symbol_res = super::search_symbols(
            &ctx.state,
            SearchSymbolsParams {
                query: "registered_symbol".to_string(),
                project_id: Some(unique_id.clone()),
                limit: Some(10),
                offset: Some(0),
                symbol_type: None,
                path_prefix: None,
            },
        )
        .await
        .unwrap();

        if let rmcp::model::RawContent::Text(t) = &symbol_res.content[0].raw {
            let json: serde_json::Value = serde_json::from_str(&t.text).unwrap();
            assert_eq!(json["filters"]["project_id"], unique_id);
            assert!(json.get("project_binding").is_none());
            assert!(json.get("reason_code").is_none());
            assert_ne!(json["summary"]["partial"]["reason_code"], "missing");
        } else {
            panic!("Expected text content");
        }
    }

    #[tokio::test]
    async fn symbol_graph_exposes_contract_metadata() {
        let ctx = TestContext::new().await;
        let unique_id = format!("test_symbols_contract_{}", uuid::Uuid::new_v4().simple());
        let project_path = ctx._temp_dir.path().join(&unique_id);
        fs::create_dir_all(&project_path).unwrap();
        fs::write(
            project_path.join("lib.rs"),
            "fn target() {}\nfn caller() { target(); }\n",
        )
        .unwrap();

        super::index_project(
            &ctx.state,
            IndexProjectParams {
                path: Some(project_path.to_string_lossy().to_string()),
                project_id: None,
                resume: None,
                job_id: None,
                resume_token: None,
                allow_full_restart_fallback: None,
                force: None,
                confirm_failed_restart: None,
                include_patterns: None,
                exclude_patterns: None,
            },
        )
        .await
        .unwrap();

        let status_params = GetIndexStatusParams {
            project_id: unique_id.clone(),
        };

        let mut retries = 0;
        loop {
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
            let res = super::get_index_status(&ctx.state, status_params.clone())
                .await
                .unwrap();
            if let rmcp::model::RawContent::Text(t) = &res.content[0].raw {
                let indexing_done = t.text.contains("\"status\":\"completed\"")
                    || t.text.contains("\"status\":\"embedding_pending\"");
                if indexing_done {
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    break;
                }
            }
            retries += 1;
            assert!(
                retries <= 100,
                "Indexing timed out for symbol graph contract test"
            );
        }

        let mut symbol_retries = 0;
        let caller_id = loop {
            let symbols = ctx
                .state
                .storage
                .search_symbols("caller", Some(&unique_id), 10, 0, None, None, None)
                .await
                .unwrap()
                .0;
            if let Some(caller_id) = symbols
                .iter()
                .find(|symbol| symbol.name == "caller")
                .and_then(|symbol| symbol.id.as_ref())
                .map(|id| crate::types::record_key_to_string(&id.key))
            {
                break caller_id;
            }

            symbol_retries += 1;
            assert!(
                symbol_retries <= 25,
                "caller symbol readiness timed out for project {unique_id}"
            );
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
        };

        let response = super::symbol_graph(
            &ctx.state,
            SymbolGraphParams {
                action: "related".to_string(),
                symbol_id: caller_id.clone(),
                depth: Some(1),
                direction: Some("outgoing".to_string()),
            },
        )
        .await
        .unwrap();

        let value = serde_json::to_value(&response).unwrap();
        let text = value["content"][0]["text"].as_str().unwrap();
        let json: serde_json::Value = serde_json::from_str(text).unwrap();

        assert_eq!(json["contract"]["schema_version"], 1);
        assert_eq!(json["contract"]["identity"]["stable_symbol_id"], caller_id);
        assert_eq!(
            json["contract"]["compatibility"]["db_shape_is_not_public_contract"],
            true
        );
        assert_eq!(json["contract"]["identity"]["stable_node_ids"], true);
        assert_eq!(
            json["contract"]["identity"]["edge_ids_are_local_only"],
            true
        );
        assert_eq!(
            json["contract"]["identity"]["node_id_semantics"],
            "stable_project_scoped_symbol_id"
        );
        assert_eq!(
            json["contract"]["identity"]["edge_id_semantics"],
            "local_only_edge_reference"
        );
        assert_eq!(
            json["contract"]["surface_guidance"]["preferred_response_fields"][0],
            "nodes"
        );
        assert_eq!(
            json["contract"]["surface_guidance"]["legacy_compatibility_fields"][0],
            "symbols"
        );
        assert_eq!(
            json["contract"]["traversal_defaults"]["frontier_semantics"],
            "unexpanded_symbol_boundary_for_manual_follow_up"
        );
        assert_eq!(
            json["contract"]["traversal_defaults"]["frontier_items_identity_basis"],
            "stable_project_scoped_symbol_id"
        );
        assert_eq!(
            json["contract"]["traversal_defaults"]["frontier_items_are_stable_node_ids"],
            true
        );
        assert_eq!(
            json["contract"]["traversal_defaults"]["frontier_items_are_project_scoped"],
            true
        );
        assert_eq!(
            json["contract"]["traversal_defaults"]["frontier_is_cursor"],
            false
        );
        assert_eq!(json["contract"]["projection_state"], "missing");
        assert_eq!(json["nodes"][0]["kind"], "function");
        assert_eq!(json["edges"][0]["relation_type"], "calls");
    }

    #[tokio::test]
    async fn code_search_exposes_contract_metadata() {
        let ctx = TestContext::new().await;
        let unique_id = format!(
            "test_code_search_contract_{}",
            uuid::Uuid::new_v4().simple()
        );
        let project_path = ctx._temp_dir.path().join(&unique_id);
        fs::create_dir_all(&project_path).unwrap();
        fs::write(
            project_path.join("main.rs"),
            "fn main() { println!(\"contract hello\"); }\n",
        )
        .unwrap();

        super::index_project(
            &ctx.state,
            IndexProjectParams {
                path: Some(project_path.to_string_lossy().to_string()),
                project_id: None,
                resume: None,
                job_id: None,
                resume_token: None,
                allow_full_restart_fallback: None,
                force: None,
                confirm_failed_restart: None,
                include_patterns: None,
                exclude_patterns: None,
            },
        )
        .await
        .unwrap();

        let status_params = GetIndexStatusParams {
            project_id: unique_id.clone(),
        };

        let mut retries = 0;
        loop {
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
            let res = super::get_index_status(&ctx.state, status_params.clone())
                .await
                .unwrap();
            if let rmcp::model::RawContent::Text(t) = &res.content[0].raw {
                let indexing_done = t.text.contains("\"status\":\"completed\"")
                    || t.text.contains("\"status\":\"embedding_pending\"");
                if indexing_done {
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    break;
                }
            }
            retries += 1;
            assert!(
                retries <= 100,
                "Indexing timed out for code search contract test"
            );
        }

        let search_res = super::search_code(
            &ctx.state,
            SearchCodeParams {
                query: "contract hello".to_string(),
                project_id: Some(unique_id.clone()),
                limit: Some(5),
            },
        )
        .await
        .unwrap();

        let search_value = serde_json::to_value(&search_res).unwrap();
        let search_text = search_value["content"][0]["text"].as_str().unwrap();
        let search_json: serde_json::Value = serde_json::from_str(search_text).unwrap();
        assert_eq!(search_json["contract"]["schema_version"], 1);
        assert_eq!(
            search_json["contract"]["compatibility"]["db_shape_is_not_public_contract"],
            true
        );
        assert_eq!(
            search_json["contract"]["identity"]["stable_node_ids"],
            false
        );
        assert_eq!(
            search_json["contract"]["identity"]["node_ids_are_project_scoped"],
            false
        );
        assert_eq!(search_json["contract"]["identity"]["node_id_semantics"], "local_only_chunk_record_id; stable_local_locator_is_project_id_plus_file_path_plus_start_line_plus_end_line");
        assert_eq!(
            search_json["contract"]["identity"]["edge_id_semantics"],
            "local_only_result_edge_reference"
        );
        assert_eq!(
            search_json["contract"]["surface_guidance"]["preferred_response_fields"][0],
            "results[].file_path"
        );

        let recall_res = super::recall_code(
            &ctx.state,
            crate::server::params::RecallCodeParams {
                query: "contract hello".to_string(),
                project_id: Some(unique_id),
                limit: Some(5),
                mode: None,
                vector_weight: None,
                bm25_weight: None,
                ppr_weight: None,
                path_prefix: None,
                language: None,
                chunk_type: None,
            },
        )
        .await
        .unwrap();

        let recall_value = serde_json::to_value(&recall_res).unwrap();
        let recall_text = recall_value["content"][0]["text"].as_str().unwrap();
        let recall_json: serde_json::Value = serde_json::from_str(recall_text).unwrap();
        assert_eq!(recall_json["contract"]["schema_version"], 1);
        assert_eq!(
            recall_json["contract"]["compatibility"]["db_shape_is_not_public_contract"],
            true
        );
        assert_eq!(
            recall_json["contract"]["identity"]["stable_node_ids"],
            false
        );
        assert_eq!(
            recall_json["contract"]["identity"]["node_ids_are_project_scoped"],
            false
        );
        assert_eq!(recall_json["contract"]["identity"]["node_id_semantics"], "local_only_chunk_record_id; stable_local_locator_is_project_id_plus_file_path_plus_start_line_plus_end_line");
        assert_eq!(
            recall_json["contract"]["identity"]["edge_id_semantics"],
            "local_only_result_edge_reference"
        );
        assert_eq!(
            recall_json["contract"]["surface_guidance"]["forbidden_to_depend_fields"][0],
            "results[].id"
        );
        assert_eq!(
            recall_json["contract"]["surface_guidance"]["preferred_response_fields"][0],
            "results[].file_path"
        );
    }

    #[tokio::test]
    async fn no_binding_preserves_project_id_none_breadth() {
        let ctx = TestContext::new().await;
        let project_a = format!("test_resolution_a_{}", uuid::Uuid::new_v4().simple());
        let project_b = format!("test_resolution_b_{}", uuid::Uuid::new_v4().simple());

        let project_a_path = ctx._temp_dir.path().join(&project_a);
        let project_b_path = ctx._temp_dir.path().join(&project_b);
        fs::create_dir_all(&project_a_path).unwrap();
        fs::create_dir_all(&project_b_path).unwrap();

        fs::write(
            project_a_path.join("lib.rs"),
            "fn alpha_source() { println!(\"alpha only marker\"); }\n",
        )
        .unwrap();
        fs::write(
            project_b_path.join("lib.rs"),
            "fn beta_source() { println!(\"beta only marker\"); }\n",
        )
        .unwrap();

        super::index_project(
            &ctx.state,
            IndexProjectParams {
                path: Some(project_a_path.to_string_lossy().to_string()),
                project_id: None,
                resume: None,
                job_id: None,
                resume_token: None,
                allow_full_restart_fallback: None,
                force: None,
                confirm_failed_restart: None,
                include_patterns: None,
                exclude_patterns: None,
            },
        )
        .await
        .unwrap();
        super::index_project(
            &ctx.state,
            IndexProjectParams {
                path: Some(project_b_path.to_string_lossy().to_string()),
                project_id: None,
                resume: None,
                job_id: None,
                resume_token: None,
                allow_full_restart_fallback: None,
                force: None,
                confirm_failed_restart: None,
                include_patterns: None,
                exclude_patterns: None,
            },
        )
        .await
        .unwrap();

        for project_id in [&project_a, &project_b] {
            let status_params = GetIndexStatusParams {
                project_id: project_id.clone(),
            };

            let mut retries = 0;
            loop {
                tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
                let res = super::get_index_status(&ctx.state, status_params.clone())
                    .await
                    .unwrap();
                if let rmcp::model::RawContent::Text(t) = &res.content[0].raw {
                    let indexing_done = t.text.contains("\"status\":\"completed\"")
                        || t.text.contains("\"status\":\"embedding_pending\"");
                    if indexing_done {
                        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                        break;
                    }
                }
                retries += 1;
                assert!(
                    retries <= 100,
                    "Indexing timed out for code_project_resolution_sources test"
                );
            }
        }

        let explicit_search = super::search_code(
            &ctx.state,
            SearchCodeParams {
                query: "alpha only marker".to_string(),
                project_id: Some(project_a.clone()),
                limit: Some(10),
            },
        )
        .await
        .unwrap();
        if let rmcp::model::RawContent::Text(t) = &explicit_search.content[0].raw {
            let json: serde_json::Value = serde_json::from_str(&t.text).unwrap();
            assert_eq!(json["project_resolution"]["source"], "explicit");
            assert_eq!(json["project_resolution"]["project_id"], project_a);
        } else {
            panic!("Expected text content");
        }

        let explicit_recall = super::recall_code(
            &ctx.state,
            RecallCodeParams {
                query: "alpha only marker".to_string(),
                project_id: Some(project_a.clone()),
                limit: Some(10),
                mode: None,
                vector_weight: None,
                bm25_weight: None,
                ppr_weight: None,
                path_prefix: None,
                language: None,
                chunk_type: None,
            },
        )
        .await
        .unwrap();
        if let rmcp::model::RawContent::Text(t) = &explicit_recall.content[0].raw {
            let json: serde_json::Value = serde_json::from_str(&t.text).unwrap();
            assert_eq!(json["project_resolution"]["source"], "explicit");
            assert_eq!(json["project_resolution"]["project_id"], project_a);
        } else {
            panic!("Expected text content");
        }

        let explicit_symbols = super::search_symbols(
            &ctx.state,
            SearchSymbolsParams {
                query: "alpha_source".to_string(),
                project_id: Some(project_a.clone()),
                limit: Some(10),
                offset: Some(0),
                symbol_type: None,
                path_prefix: None,
            },
        )
        .await
        .unwrap();
        if let rmcp::model::RawContent::Text(t) = &explicit_symbols.content[0].raw {
            let json: serde_json::Value = serde_json::from_str(&t.text).unwrap();
            assert_eq!(json["project_resolution"]["source"], "explicit");
            assert_eq!(json["project_resolution"]["project_id"], project_a);
        } else {
            panic!("Expected text content");
        }

        let cross_project_search = super::search_code(
            &ctx.state,
            SearchCodeParams {
                query: "only marker".to_string(),
                project_id: None,
                limit: Some(20),
            },
        )
        .await
        .unwrap();
        if let rmcp::model::RawContent::Text(t) = &cross_project_search.content[0].raw {
            let json: serde_json::Value = serde_json::from_str(&t.text).unwrap();
            assert_eq!(json["project_resolution"]["source"], "cross_project");
            assert!(json["project_resolution"]["project_id"].is_null());
            assert!(
                t.text.contains("alpha only marker") && t.text.contains("beta only marker"),
                "Expected broad cross-project behavior to include both project markers. Got: {}",
                t.text
            );
        } else {
            panic!("Expected text content");
        }

        let session_id = format!("sid-resolution-{}", uuid::Uuid::new_v4().simple());
        ctx.state
            .session_bindings
            .bind(session_id.clone(), project_a.clone())
            .await;
        let session_bound_search = super::search_code_with_context(
            &ctx.state,
            SearchCodeParams {
                query: "alpha only marker".to_string(),
                project_id: None,
                limit: Some(10),
            },
            Some(CodeToolContext::from_session_id(Some(session_id))),
        )
        .await
        .unwrap();
        if let rmcp::model::RawContent::Text(t) = &session_bound_search.content[0].raw {
            let json: serde_json::Value = serde_json::from_str(&t.text).unwrap();
            assert_eq!(json["project_resolution"]["source"], "session_binding");
            assert_eq!(json["project_resolution"]["project_id"], project_a);
        } else {
            panic!("Expected text content");
        }
    }

    #[tokio::test]
    async fn code_project_resolution_stale_binding() {
        let ctx = TestContext::new().await;

        let stable_project = format!("test_stale_live_{}", uuid::Uuid::new_v4().simple());
        let stale_project = "test_stale_bound_project".to_string();

        let stable_path = ctx._temp_dir.path().join(&stable_project);
        fs::create_dir_all(&stable_path).unwrap();
        fs::write(
            stable_path.join("lib.rs"),
            "fn live_marker() { println!(\"live project marker\"); }\n",
        )
        .unwrap();

        super::index_project(
            &ctx.state,
            IndexProjectParams {
                path: Some(stable_path.to_string_lossy().to_string()),
                project_id: None,
                resume: None,
                job_id: None,
                resume_token: None,
                allow_full_restart_fallback: None,
                force: None,
                confirm_failed_restart: None,
                include_patterns: None,
                exclude_patterns: None,
            },
        )
        .await
        .unwrap();

        let status_params = GetIndexStatusParams {
            project_id: stable_project,
        };

        let mut retries = 0;
        loop {
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
            let res = super::get_index_status(&ctx.state, status_params.clone())
                .await
                .unwrap();
            if let rmcp::model::RawContent::Text(t) = &res.content[0].raw {
                let indexing_done = t.text.contains("\"status\":\"completed\"")
                    || t.text.contains("\"status\":\"embedding_pending\"");
                if indexing_done {
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    break;
                }
            }
            retries += 1;
            assert!(
                retries <= 100,
                "Indexing timed out for code_project_resolution_stale_binding test"
            );
        }

        let session_id = format!("sid-stale-{}", uuid::Uuid::new_v4().simple());
        ctx.state
            .session_bindings
            .bind(session_id.clone(), stale_project.clone())
            .await;
        let stale_search = super::search_code_with_context(
            &ctx.state,
            SearchCodeParams {
                query: "live project marker".to_string(),
                project_id: None,
                limit: Some(20),
            },
            Some(CodeToolContext::from_session_id(Some(session_id))),
        )
        .await
        .unwrap();

        if let rmcp::model::RawContent::Text(t) = &stale_search.content[0].raw {
            let json: serde_json::Value = serde_json::from_str(&t.text).unwrap();

            assert_eq!(json["project_resolution"]["source"], "session_binding");
            assert_eq!(json["project_resolution"]["project_id"], stale_project);
            assert_eq!(json["project_resolution"]["reason_code"], "stale");
            assert_eq!(json["project_resolution"]["binding_state"], "stale_binding");
            assert_eq!(json["summary"]["partial"]["reason_code"], "stale");
            assert_eq!(json["reason_code"], "stale");
            assert_eq!(json["count"], 0);
            assert!(json["results"].as_array().unwrap().is_empty());
            assert_ne!(json["project_resolution"]["source"], "cross_project");
        } else {
            panic!("Expected text content");
        }
    }

    #[tokio::test]
    async fn explicit_project_id_overrides_session_binding() {
        let ctx = TestContext::new().await;
        let project_a = format!("test_explicit_override_a_{}", uuid::Uuid::new_v4().simple());
        let project_b = format!("test_explicit_override_b_{}", uuid::Uuid::new_v4().simple());

        let project_a_path = ctx._temp_dir.path().join(&project_a);
        let project_b_path = ctx._temp_dir.path().join(&project_b);
        fs::create_dir_all(&project_a_path).unwrap();
        fs::create_dir_all(&project_b_path).unwrap();
        fs::write(
            project_a_path.join("lib.rs"),
            "fn explicit_override_target() { println!(\"explicit override alpha marker\"); }\n",
        )
        .unwrap();
        fs::write(
            project_b_path.join("lib.rs"),
            "fn session_binding_decoy() { println!(\"session binding beta marker\"); }\n",
        )
        .unwrap();

        for project_path in [&project_a_path, &project_b_path] {
            super::index_project(
                &ctx.state,
                IndexProjectParams {
                    path: Some(project_path.to_string_lossy().to_string()),
                    project_id: None,
                    resume: None,
                    job_id: None,
                    resume_token: None,
                    allow_full_restart_fallback: None,
                    force: None,
                    confirm_failed_restart: None,
                    include_patterns: None,
                    exclude_patterns: None,
                },
            )
            .await
            .unwrap();
        }

        for project_id in [&project_a, &project_b] {
            let status_params = GetIndexStatusParams {
                project_id: project_id.clone(),
            };
            let mut retries = 0;
            loop {
                tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
                let res = super::get_index_status(&ctx.state, status_params.clone())
                    .await
                    .unwrap();
                if let rmcp::model::RawContent::Text(t) = &res.content[0].raw {
                    let indexing_done = t.text.contains("\"status\":\"completed\"")
                        || t.text.contains("\"status\":\"embedding_pending\"");
                    if indexing_done {
                        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                        break;
                    }
                }
                retries += 1;
                assert!(
                    retries <= 100,
                    "Indexing timed out for explicit override test"
                );
            }
        }

        let session_id = format!("sid-override-{}", uuid::Uuid::new_v4().simple());
        ctx.state
            .session_bindings
            .bind(session_id.clone(), project_b.clone())
            .await;
        let context = Some(CodeToolContext::from_session_id(Some(session_id.clone())));

        let search_res = super::search_code_with_context(
            &ctx.state,
            SearchCodeParams {
                query: "explicit override alpha marker".to_string(),
                project_id: Some(project_a.clone()),
                limit: Some(10),
            },
            context.clone(),
        )
        .await
        .unwrap();
        if let rmcp::model::RawContent::Text(t) = &search_res.content[0].raw {
            let json: serde_json::Value = serde_json::from_str(&t.text).unwrap();
            assert_eq!(json["project_resolution"]["source"], "explicit");
            assert_eq!(json["project_resolution"]["project_id"], project_a);
            assert!(t.text.contains("explicit override alpha marker"));
            assert!(!t.text.contains("session binding beta marker"));
        } else {
            panic!("Expected text content");
        }

        let recall_res = super::recall_code_with_context(
            &ctx.state,
            RecallCodeParams {
                query: "explicit override alpha marker".to_string(),
                project_id: Some(project_a.clone()),
                limit: Some(10),
                mode: None,
                vector_weight: None,
                bm25_weight: None,
                ppr_weight: None,
                path_prefix: None,
                language: None,
                chunk_type: None,
            },
            context.clone(),
        )
        .await
        .unwrap();
        if let rmcp::model::RawContent::Text(t) = &recall_res.content[0].raw {
            let json: serde_json::Value = serde_json::from_str(&t.text).unwrap();
            assert_eq!(json["project_resolution"]["source"], "explicit");
            assert_eq!(json["project_resolution"]["project_id"], project_a);
            assert!(t.text.contains("explicit override alpha marker"));
            assert!(!t.text.contains("session binding beta marker"));
        } else {
            panic!("Expected text content");
        }

        let symbols_res = super::search_symbols_with_context(
            &ctx.state,
            SearchSymbolsParams {
                query: "explicit_override_target".to_string(),
                project_id: Some(project_a.clone()),
                limit: Some(10),
                offset: Some(0),
                symbol_type: None,
                path_prefix: None,
            },
            context,
        )
        .await
        .unwrap();
        if let rmcp::model::RawContent::Text(t) = &symbols_res.content[0].raw {
            let json: serde_json::Value = serde_json::from_str(&t.text).unwrap();
            assert_eq!(json["project_resolution"]["source"], "explicit");
            assert_eq!(json["project_resolution"]["project_id"], project_a);
            assert!(t.text.contains("explicit_override_target"));
            assert!(!t.text.contains("session_binding_decoy"));
        } else {
            panic!("Expected text content");
        }
    }

    #[tokio::test]
    async fn session_binding_scopes_code_search() {
        let ctx = TestContext::new().await;
        let project_a = format!("test_session_scope_a_{}", uuid::Uuid::new_v4().simple());
        let project_b = format!("test_session_scope_b_{}", uuid::Uuid::new_v4().simple());

        let project_a_path = ctx._temp_dir.path().join(&project_a);
        let project_b_path = ctx._temp_dir.path().join(&project_b);
        fs::create_dir_all(&project_a_path).unwrap();
        fs::create_dir_all(&project_b_path).unwrap();
        fs::write(
            project_a_path.join("lib.rs"),
            "fn bound_project_target() { println!(\"shared session marker bound\"); }\n",
        )
        .unwrap();
        fs::write(
            project_b_path.join("lib.rs"),
            "fn other_project_target() { println!(\"shared session marker other\"); }\n",
        )
        .unwrap();

        for project_path in [&project_a_path, &project_b_path] {
            super::index_project(
                &ctx.state,
                IndexProjectParams {
                    path: Some(project_path.to_string_lossy().to_string()),
                    project_id: None,
                    resume: None,
                    job_id: None,
                    resume_token: None,
                    allow_full_restart_fallback: None,
                    force: None,
                    confirm_failed_restart: None,
                    include_patterns: None,
                    exclude_patterns: None,
                },
            )
            .await
            .unwrap();
        }

        for project_id in [&project_a, &project_b] {
            let status_params = GetIndexStatusParams {
                project_id: project_id.clone(),
            };
            let mut retries = 0;
            loop {
                tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
                let res = super::get_index_status(&ctx.state, status_params.clone())
                    .await
                    .unwrap();
                if let rmcp::model::RawContent::Text(t) = &res.content[0].raw {
                    let indexing_done = t.text.contains("\"status\":\"completed\"")
                        || t.text.contains("\"status\":\"embedding_pending\"");
                    if indexing_done {
                        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                        break;
                    }
                }
                retries += 1;
                assert!(retries <= 100, "Indexing timed out for session scope test");
            }
        }

        let broad_search = super::search_code(
            &ctx.state,
            SearchCodeParams {
                query: "shared session marker".to_string(),
                project_id: None,
                limit: Some(20),
            },
        )
        .await
        .unwrap();
        if let rmcp::model::RawContent::Text(t) = &broad_search.content[0].raw {
            let json: serde_json::Value = serde_json::from_str(&t.text).unwrap();
            assert_eq!(json["project_resolution"]["source"], "cross_project");
            assert!(json["project_resolution"]["project_id"].is_null());
            assert!(t.text.contains("shared session marker bound"));
            assert!(t.text.contains("shared session marker other"));
        } else {
            panic!("Expected text content");
        }

        let session_id = format!("sid-scope-{}", uuid::Uuid::new_v4().simple());
        ctx.state
            .session_bindings
            .bind(session_id.clone(), project_a.clone())
            .await;
        let context = Some(CodeToolContext::from_session_id(Some(session_id.clone())));

        let scoped_search = super::search_code_with_context(
            &ctx.state,
            SearchCodeParams {
                query: "shared session marker".to_string(),
                project_id: None,
                limit: Some(20),
            },
            context.clone(),
        )
        .await
        .unwrap();
        if let rmcp::model::RawContent::Text(t) = &scoped_search.content[0].raw {
            let json: serde_json::Value = serde_json::from_str(&t.text).unwrap();
            assert_eq!(json["project_resolution"]["source"], "session_binding");
            assert_eq!(json["project_resolution"]["project_id"], project_a);
            assert!(t.text.contains("shared session marker bound"));
            assert!(!t.text.contains("shared session marker other"));
        } else {
            panic!("Expected text content");
        }

        let scoped_recall = super::recall_code_with_context(
            &ctx.state,
            RecallCodeParams {
                query: "shared session marker".to_string(),
                project_id: None,
                limit: Some(20),
                mode: None,
                vector_weight: None,
                bm25_weight: None,
                ppr_weight: None,
                path_prefix: None,
                language: None,
                chunk_type: None,
            },
            context.clone(),
        )
        .await
        .unwrap();
        if let rmcp::model::RawContent::Text(t) = &scoped_recall.content[0].raw {
            let json: serde_json::Value = serde_json::from_str(&t.text).unwrap();
            assert_eq!(json["project_resolution"]["source"], "session_binding");
            assert_eq!(json["project_resolution"]["project_id"], project_a);
            assert!(t.text.contains("shared session marker bound"));
            assert!(!t.text.contains("shared session marker other"));
        } else {
            panic!("Expected text content");
        }

        let scoped_symbols = super::search_symbols_with_context(
            &ctx.state,
            SearchSymbolsParams {
                query: "target".to_string(),
                project_id: None,
                limit: Some(20),
                offset: Some(0),
                symbol_type: None,
                path_prefix: None,
            },
            context,
        )
        .await
        .unwrap();
        if let rmcp::model::RawContent::Text(t) = &scoped_symbols.content[0].raw {
            let json: serde_json::Value = serde_json::from_str(&t.text).unwrap();
            assert_eq!(json["project_resolution"]["source"], "session_binding");
            assert_eq!(json["project_resolution"]["project_id"], project_a);
            assert!(t.text.contains("bound_project_target"));
            assert!(!t.text.contains("other_project_target"));
        } else {
            panic!("Expected text content");
        }
    }

    #[tokio::test]
    async fn project_info_projection_returns_export_only_projection_document() {
        let ctx = TestContext::new().await;
        let unique_id = format!("test_projection_doc_{}", uuid::Uuid::new_v4().simple());
        let project_path = ctx._temp_dir.path().join(&unique_id);
        fs::create_dir_all(&project_path).unwrap();
        fs::write(
            project_path.join("lib.rs"),
            "fn target() {}\nfn caller() { target(); }\n",
        )
        .unwrap();

        super::index_project(
            &ctx.state,
            IndexProjectParams {
                path: Some(project_path.to_string_lossy().to_string()),
                project_id: None,
                resume: None,
                job_id: None,
                resume_token: None,
                allow_full_restart_fallback: None,
                force: None,
                confirm_failed_restart: None,
                include_patterns: None,
                exclude_patterns: None,
            },
        )
        .await
        .unwrap();

        let status_params = GetIndexStatusParams {
            project_id: unique_id.clone(),
        };

        let mut retries = 0;
        loop {
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
            let res = super::get_index_status(&ctx.state, status_params.clone())
                .await
                .unwrap();
            if let rmcp::model::RawContent::Text(t) = &res.content[0].raw {
                let indexing_done = t.text.contains("\"status\":\"completed\"")
                    || t.text.contains("\"status\":\"embedding_pending\"");
                if indexing_done {
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    break;
                }
            }
            retries += 1;
            assert!(
                retries <= 100,
                "Indexing timed out for projection builder test"
            );
        }

        let mut projection_retries = 0;
        let json: serde_json::Value = loop {
            let projection_res = super::get_project_projection(
                &ctx.state,
                crate::server::params::GetProjectProjectionParams {
                    project_id: unique_id.clone(),
                    relation_scope: None,
                    sort_mode: None,
                },
            )
            .await
            .unwrap();

            let value = serde_json::to_value(&projection_res).unwrap();
            let text = value["content"][0]["text"].as_str().unwrap();
            let json: serde_json::Value = serde_json::from_str(text).unwrap();
            if json["projection"]["edges"].as_array().unwrap().len() >= 1 {
                break json;
            }

            projection_retries += 1;
            assert!(
                projection_retries <= 25,
                "Projection graph readiness timed out: {}",
                text
            );
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
        };

        assert_eq!(json["project_id"], unique_id);
        assert_eq!(json["projection"]["project_id"], json["project_id"]);
        assert_eq!(json["projection"]["request"]["relation_scope"], "all");
        assert_eq!(json["projection"]["request"]["sort_mode"], "canonical");
        assert_eq!(
            json["projection"]["shaping"]["relation_scope_applied"],
            "all"
        );
        assert_eq!(
            json["projection"]["shaping"]["sort_mode_applied"],
            "canonical"
        );
        assert_eq!(
            json["projection"]["shaping"]["node_selection_basis"],
            "relation_endpoint_induced_subgraph"
        );
        assert_eq!(
            json["projection"]["shaping"]["edge_selection_basis"],
            "all_relation_edges"
        );
        assert_eq!(
            json["projection"]["shaping"]["output_kind"],
            "induced_symbol_graph"
        );
        assert_eq!(json["projection"]["contract"]["schema_version"], 1);
        assert_eq!(
            json["projection"]["contract"]["projection"]["basis"],
            "semantic_generation"
        );
        assert_eq!(json["projection"]["summary"]["result_kind"], "graph");
        assert_eq!(json["projection"]["summary"]["partial"]["is_partial"], true);
        assert_eq!(
            json["projection"]["summary"]["partial"]["reason_code"],
            "stale"
        );
        assert_eq!(
            json["projection"]["summary"]["partial"]["reason"],
            "projection_stale"
        );
        assert_eq!(json["locator"]["lookup"]["state"], "created");
        assert_eq!(json["locator"]["lookup"]["found"], true);
        assert!(json["locator"]["lookup"]["reason_code"].is_null());
        assert_eq!(json["locator"]["lifecycle"]["same_process_only"], true);
        assert_eq!(json["locator"]["lifecycle"]["client_persistable"], false);
        assert!(json["projection"]["nodes"].as_array().unwrap().len() >= 1);
        assert!(json["projection"]["edges"].as_array().unwrap().len() >= 1);
        assert_eq!(json["projection"]["counts"]["symbols"].as_u64().unwrap(), 2);
        assert_eq!(json["projection"]["counts"]["nodes"].as_u64().unwrap(), 2);
        assert_eq!(json["projection"]["counts"]["edges"].as_u64().unwrap(), 1);
        let node_ids: Vec<String> = json["projection"]["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|node| node["id"].as_str().unwrap().to_string())
            .collect();
        let mut sorted_node_ids = node_ids.clone();
        sorted_node_ids.sort();
        assert_eq!(node_ids, sorted_node_ids);

        let edge_pairs: Vec<(String, String)> = json["projection"]["edges"]
            .as_array()
            .unwrap()
            .iter()
            .map(|edge| {
                (
                    edge["from_id"].as_str().unwrap().to_string(),
                    edge["to_id"].as_str().unwrap().to_string(),
                )
            })
            .collect();
        let mut sorted_edge_pairs = edge_pairs.clone();
        sorted_edge_pairs.sort();
        assert_eq!(edge_pairs, sorted_edge_pairs);
    }

    #[tokio::test]
    async fn project_info_projection_option_changes_only_projection_payload() {
        let ctx = TestContext::new().await;
        let unique_id = format!("test_projection_opts_{}", uuid::Uuid::new_v4().simple());
        let project_path = ctx._temp_dir.path().join(&unique_id);
        fs::create_dir_all(&project_path).unwrap();
        fs::write(
            project_path.join("lib.rs"),
            "fn target() {}\nfn caller() { target(); }\n",
        )
        .unwrap();

        super::index_project(
            &ctx.state,
            IndexProjectParams {
                path: Some(project_path.to_string_lossy().to_string()),
                project_id: None,
                resume: None,
                job_id: None,
                resume_token: None,
                allow_full_restart_fallback: None,
                force: None,
                confirm_failed_restart: None,
                include_patterns: None,
                exclude_patterns: None,
            },
        )
        .await
        .unwrap();

        let status_params = GetIndexStatusParams {
            project_id: unique_id.clone(),
        };

        let mut retries = 0;
        loop {
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
            let res = super::get_index_status(&ctx.state, status_params.clone())
                .await
                .unwrap();
            if let rmcp::model::RawContent::Text(t) = &res.content[0].raw {
                let indexing_done = t.text.contains("\"status\":\"completed\"")
                    || t.text.contains("\"status\":\"embedding_pending\"");
                if indexing_done {
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    break;
                }
            }
            retries += 1;
            assert!(
                retries <= 100,
                "Indexing timed out for projection options test"
            );
        }

        let projection_res = super::get_project_projection(
            &ctx.state,
            crate::server::params::GetProjectProjectionParams {
                project_id: unique_id.clone(),
                relation_scope: Some("none".to_string()),
                sort_mode: Some("canonical".to_string()),
            },
        )
        .await
        .unwrap();

        let value = serde_json::to_value(&projection_res).unwrap();
        let text = value["content"][0]["text"].as_str().unwrap();
        let json: serde_json::Value = serde_json::from_str(text).unwrap();

        assert_eq!(json["projection"]["request"]["relation_scope"], "none");
        assert_eq!(json["projection"]["request"]["sort_mode"], "canonical");
        assert_eq!(
            json["projection"]["shaping"]["relation_scope_applied"],
            "none"
        );
        assert_eq!(
            json["projection"]["shaping"]["node_selection_basis"],
            "empty_graph_when_no_edges_retained"
        );
        assert_eq!(
            json["projection"]["shaping"]["edge_selection_basis"],
            "no_edges_retained"
        );
        assert_eq!(json["projection"]["shaping"]["output_kind"], "empty_graph");
        assert_eq!(json["projection"]["nodes"].as_array().unwrap().len(), 0);
        assert_eq!(json["projection"]["edges"].as_array().unwrap().len(), 0);
        assert_eq!(json["projection"]["counts"]["nodes"].as_u64().unwrap(), 0);
        assert_eq!(json["projection"]["counts"]["edges"].as_u64().unwrap(), 0);
        assert_eq!(json["projection"]["contract"]["schema_version"], 1);
        assert_eq!(
            json["projection"]["contract"]["identity"]["project_id"],
            unique_id
        );
        assert_eq!(
            json["projection"]["contract"]["projection"]["basis"],
            "semantic_generation"
        );
        assert_eq!(json["projection"]["summary"]["partial"]["is_partial"], true);
        assert_eq!(
            json["projection"]["summary"]["partial"]["reason_code"],
            "stale"
        );
        assert_eq!(
            json["projection"]["summary"]["partial"]["reason"],
            "projection_stale"
        );
    }

    #[tokio::test]
    async fn project_info_projection_supports_imports_scope() {
        let ctx = TestContext::new().await;
        let unique_id = format!("test_projection_imports_{}", uuid::Uuid::new_v4().simple());
        let project_path = ctx._temp_dir.path().join(&unique_id);
        fs::create_dir_all(&project_path).unwrap();
        fs::write(
            project_path.join("lib.rs"),
            "fn target() {}\nfn caller() { use crate::target; }\n",
        )
        .unwrap();

        super::index_project(
            &ctx.state,
            IndexProjectParams {
                path: Some(project_path.to_string_lossy().to_string()),
                project_id: None,
                resume: None,
                job_id: None,
                resume_token: None,
                allow_full_restart_fallback: None,
                force: None,
                confirm_failed_restart: None,
                include_patterns: None,
                exclude_patterns: None,
            },
        )
        .await
        .unwrap();

        let status_params = GetIndexStatusParams {
            project_id: unique_id.clone(),
        };

        let mut retries = 0;
        loop {
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
            let res = super::get_index_status(&ctx.state, status_params.clone())
                .await
                .unwrap();
            if let rmcp::model::RawContent::Text(t) = &res.content[0].raw {
                let indexing_done = t.text.contains("\"status\":\"completed\"")
                    || t.text.contains("\"status\":\"embedding_pending\"");
                if indexing_done {
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    break;
                }
            }
            retries += 1;
            assert!(
                retries <= 100,
                "Indexing timed out for projection imports test"
            );
        }

        let mut projection_retries = 0;
        let json: serde_json::Value = loop {
            let projection_res = super::get_project_projection(
                &ctx.state,
                crate::server::params::GetProjectProjectionParams {
                    project_id: unique_id.clone(),
                    relation_scope: Some("imports".to_string()),
                    sort_mode: Some("canonical".to_string()),
                },
            )
            .await
            .unwrap();

            let value = serde_json::to_value(&projection_res).unwrap();
            let text = value["content"][0]["text"].as_str().unwrap();
            let json: serde_json::Value = serde_json::from_str(text).unwrap();
            if json["projection"]["edges"]
                .as_array()
                .is_some_and(|edges| edges.iter().any(|edge| edge["relation_type"] == "imports"))
            {
                break json;
            }

            projection_retries += 1;
            assert!(
                projection_retries <= 25,
                "Imports projection readiness timed out: {}",
                text
            );
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
        };

        assert_eq!(json["projection"]["request"]["relation_scope"], "imports");
        assert_eq!(json["projection"]["request"]["sort_mode"], "canonical");
        assert_eq!(
            json["projection"]["shaping"]["relation_scope_applied"],
            "imports"
        );
        assert_eq!(
            json["projection"]["shaping"]["node_selection_basis"],
            "relation_endpoint_induced_subgraph"
        );
        assert_eq!(
            json["projection"]["shaping"]["edge_selection_basis"],
            "only_import_edges"
        );
        assert_eq!(
            json["projection"]["shaping"]["output_kind"],
            "induced_symbol_graph"
        );
        let edges = json["projection"]["edges"].as_array().unwrap();
        assert!(edges.iter().all(|edge| edge["relation_type"] == "imports"));
        assert_eq!(
            json["projection"]["counts"]["edges"].as_u64().unwrap(),
            edges.len() as u64
        );
        let nodes = json["projection"]["nodes"].as_array().unwrap();
        assert_eq!(
            json["projection"]["counts"]["nodes"].as_u64().unwrap(),
            nodes.len() as u64
        );
        let edge_node_ids: std::collections::HashSet<String> = edges
            .iter()
            .flat_map(|edge| {
                [
                    edge["from_id"].as_str().unwrap().to_string(),
                    edge["to_id"].as_str().unwrap().to_string(),
                ]
            })
            .collect();
        let node_ids: std::collections::HashSet<String> = nodes
            .iter()
            .map(|node| node["id"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(node_ids, edge_node_ids);
        assert_eq!(json["projection"]["contract"]["schema_version"], 1);
        assert_eq!(
            json["projection"]["contract"]["identity"]["project_id"],
            unique_id
        );
        assert_eq!(json["projection"]["summary"]["partial"]["is_partial"], true);
        assert_eq!(
            json["projection"]["summary"]["partial"]["reason_code"],
            "stale"
        );
        assert_eq!(
            json["projection"]["summary"]["partial"]["reason"],
            "projection_stale"
        );
    }

    #[tokio::test]
    async fn project_info_projection_by_locator_reads_back_ephemeral_projection() {
        let ctx = TestContext::new().await;
        let unique_id = format!("test_projection_locator_{}", uuid::Uuid::new_v4().simple());
        let project_path = ctx._temp_dir.path().join(&unique_id);
        fs::create_dir_all(&project_path).unwrap();
        fs::write(
            project_path.join("lib.rs"),
            "fn target() {}\nfn caller() { target(); }\n",
        )
        .unwrap();

        super::index_project(
            &ctx.state,
            IndexProjectParams {
                path: Some(project_path.to_string_lossy().to_string()),
                project_id: None,
                resume: None,
                job_id: None,
                resume_token: None,
                allow_full_restart_fallback: None,
                force: None,
                confirm_failed_restart: None,
                include_patterns: None,
                exclude_patterns: None,
            },
        )
        .await
        .unwrap();

        let status_params = GetIndexStatusParams {
            project_id: unique_id.clone(),
        };

        let mut retries = 0;
        loop {
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
            let res = super::get_index_status(&ctx.state, status_params.clone())
                .await
                .unwrap();
            if let rmcp::model::RawContent::Text(t) = &res.content[0].raw {
                let indexing_done = t.text.contains("\"status\":\"completed\"")
                    || t.text.contains("\"status\":\"embedding_pending\"");
                if indexing_done {
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    break;
                }
            }
            retries += 1;
            assert!(
                retries <= 100,
                "Indexing timed out for locator projection test"
            );
        }

        let projection_res = super::get_project_projection(
            &ctx.state,
            GetProjectProjectionParams {
                project_id: unique_id.clone(),
                relation_scope: None,
                sort_mode: None,
            },
        )
        .await
        .unwrap();

        let value = serde_json::to_value(&projection_res).unwrap();
        let text = value["content"][0]["text"].as_str().unwrap();
        let json: serde_json::Value = serde_json::from_str(text).unwrap();
        let locator = json["locator"]["locator"].as_str().unwrap().to_string();

        assert_eq!(
            json["locator"]["locator_kind"],
            "ephemeral_projection_handle"
        );
        assert_eq!(json["locator"]["project_id"], unique_id);
        assert_eq!(json["locator"]["lookup"]["state"], "created");
        assert_eq!(json["locator"]["lookup"]["found"], true);
        assert_eq!(json["locator"]["lifecycle"]["same_process_only"], true);
        assert_eq!(
            json["locator"]["lifecycle"]["survives_process_restart"],
            false
        );
        assert_eq!(
            json["locator"]["lifecycle"]["survives_generation_change"],
            false
        );
        assert_eq!(
            json["projection"]["contract"]["projection"]["materialization"]["is_addressable"],
            false
        );

        let readback_res = super::get_project_projection_by_locator(
            &ctx.state,
            GetProjectionByLocatorParams {
                locator: locator.clone(),
            },
        )
        .await
        .unwrap();

        let readback_value = serde_json::to_value(&readback_res).unwrap();
        let readback_text = readback_value["content"][0]["text"].as_str().unwrap();
        let readback_json: serde_json::Value = serde_json::from_str(readback_text).unwrap();

        assert_eq!(readback_json["locator"]["locator"], locator);
        assert_eq!(
            readback_json["locator"]["locator_kind"],
            "ephemeral_projection_handle"
        );
        assert_eq!(readback_json["locator"]["lookup"]["state"], "resolved");
        assert_eq!(readback_json["locator"]["lookup"]["found"], true);
        assert!(readback_json["locator"]["lookup"]["reason_code"].is_null());
        assert_eq!(
            readback_json["projection"]["project_id"],
            json["projection"]["project_id"]
        );
        assert_eq!(
            readback_json["projection"]["request"],
            json["projection"]["request"]
        );
        assert_eq!(
            readback_json["projection"]["summary"],
            json["projection"]["summary"]
        );
        assert_eq!(
            readback_json["projection"]["counts"],
            json["projection"]["counts"]
        );
        assert_eq!(
            readback_json["projection"]["nodes"],
            json["projection"]["nodes"]
        );
        assert_eq!(
            readback_json["projection"]["edges"],
            json["projection"]["edges"]
        );
    }

    #[tokio::test]
    async fn get_projection_by_locator_returns_not_found_for_unknown_locator() {
        let ctx = TestContext::new().await;

        let res = super::get_project_projection_by_locator(
            &ctx.state,
            GetProjectionByLocatorParams {
                locator: "projection:missing:all:canonical:0".to_string(),
            },
        )
        .await
        .unwrap();

        let value = serde_json::to_value(&res).unwrap();
        let text = value["content"][0]["text"].as_str().unwrap();
        let json: serde_json::Value = serde_json::from_str(text).unwrap();

        assert_eq!(json["reason_code"], "invalid_locator");
        assert!(json["error"]
            .as_str()
            .unwrap()
            .contains("Projection locator not found in this process"));
        assert_eq!(json["locator"]["lookup"]["state"], "missing");
        assert_eq!(json["locator"]["lookup"]["found"], false);
        assert_eq!(json["locator"]["lookup"]["reason_code"], "invalid_locator");
        assert_eq!(json["locator"]["lifecycle"]["same_process_only"], true);
        assert_eq!(json["locator"]["lifecycle"]["client_persistable"], false);
    }

    fn tool_result_json(result: &rmcp::model::CallToolResult) -> serde_json::Value {
        let rmcp::model::RawContent::Text(text) = &result.content[0].raw else {
            panic!("Expected text content");
        };
        serde_json::from_str(&text.text).expect("tool response should be valid json")
    }

    fn recall_params(project_id: &str, query: &str) -> RecallCodeParams {
        RecallCodeParams {
            query: query.to_string(),
            project_id: Some(project_id.to_string()),
            limit: Some(10),
            mode: None,
            vector_weight: None,
            bm25_weight: None,
            ppr_weight: None,
            path_prefix: None,
            language: None,
            chunk_type: None,
        }
    }

    fn symbol_params(project_id: &str, query: &str) -> SearchSymbolsParams {
        SearchSymbolsParams {
            query: query.to_string(),
            project_id: Some(project_id.to_string()),
            limit: Some(10),
            offset: Some(0),
            symbol_type: None,
            path_prefix: None,
        }
    }

    fn symbol_graph_params(symbol_id: &str) -> SymbolGraphParams {
        SymbolGraphParams {
            symbol_id: symbol_id.to_string(),
            action: "related".to_string(),
            depth: Some(1),
            direction: Some("both".to_string()),
        }
    }

    #[allow(dead_code)]
    fn completed_status(project_id: &str, generation: u64) -> IndexStatus {
        let mut status = IndexStatus::new(project_id.to_string());
        status.status = IndexState::Completed;
        status.total_files = 1;
        status.indexed_files = 1;
        status.total_chunks = 1;
        status.total_symbols = 1;
        status.structural_generation = generation;
        status.semantic_generation = generation;
        status.refresh_lifecycle_states();
        status
    }

    #[allow(dead_code)]
    fn indexing_status_with_serving(project_id: &str, serving: u64, indexing: u64) -> IndexStatus {
        let mut status = IndexStatus::new(project_id.to_string());
        status.status = IndexState::Indexing;
        status.total_files = 2;
        status.indexed_files = 1;
        status.total_chunks = 1;
        status.total_symbols = 1;
        status.structural_generation = indexing;
        status.semantic_generation = serving;
        status.error_message = Some(format!(
            "serving_generation={serving}; indexing_generation={indexing}; capability freshness is stale while active indexing is running"
        ));
        status.refresh_lifecycle_states();
        status
    }

    #[allow(dead_code)]
    fn indexing_status_without_serving(project_id: &str, indexing: u64) -> IndexStatus {
        let mut status = IndexStatus::new(project_id.to_string());
        status.status = IndexState::Indexing;
        status.total_files = 2;
        status.indexed_files = 0;
        status.structural_generation = indexing;
        status.error_message = Some(format!(
            "serving_generation missing; indexing_generation={indexing}; reason_code=missing; reason=no_serving_generation"
        ));
        status.refresh_lifecycle_states();
        status
    }

    #[allow(dead_code)]
    fn interrupted_status_with_prior_serving(
        project_id: &str,
        serving: u64,
        interrupted: u64,
    ) -> IndexStatus {
        let mut status = indexing_status_with_serving(project_id, serving, interrupted);
        status.status = IndexState::Failed;
        status.error_message = Some("interrupted_generation_not_promoted".to_string());
        status.refresh_lifecycle_states();
        status
    }

    async fn persist_generation_fixture(
        ctx: &TestContext,
        project_id: &str,
        generation: u64,
        marker: &str,
    ) -> String {
        let chunk = CodeChunk {
            id: None,
            file_path: format!("src/{marker}.rs"),
            content: format!("pub fn {marker}() {{ /* {marker} searchable marker */ }}"),
            language: crate::types::Language::Rust,
            start_line: 1,
            end_line: 1,
            chunk_type: crate::types::ChunkType::Function,
            name: Some(marker.to_string()),
            context_path: None,
            embedding: Some(vec![0.1; 768]),
            content_hash: format!("hash-{marker}-{generation}"),
            project_id: Some(project_id.to_string()),
            generation: Some(generation),
            indexed_at: crate::types::Datetime::default(),
        };
        ctx.state.storage.create_code_chunk(chunk).await.unwrap();

        let mut symbol = CodeSymbol::new(
            marker.to_string(),
            SymbolType::Function,
            format!("src/{marker}.rs"),
            1,
            1,
            project_id.to_string(),
        );
        symbol.embedding = Some(vec![0.2; 768]);
        symbol.generation = Some(generation);
        ctx.state.storage.create_code_symbol(symbol).await.unwrap()
    }

    fn checkpoint(project_id: &str, generation: u64, file_path: &str) -> IndexFileCheckpoint {
        IndexFileCheckpoint {
            id: None,
            job_id: format!("job-{project_id}-{generation}"),
            project_id: project_id.to_string(),
            generation,
            relative_file_path: file_path.to_string(),
            file_path: file_path.to_string(),
            content_hash: format!("hash-{project_id}-{generation}-{file_path}"),
            checkpoint_generation: generation,
            phase: IndexJobPhase::Promote,
            completed: true,
            completed_at: Datetime::default(),
            chunks_written: 1,
            symbols_written: 1,
            updated_at: Datetime::default(),
        }
    }

    async fn persist_call_relation_fixture(
        ctx: &TestContext,
        project_id: &str,
        generation: u64,
        caller: &str,
        callee: &str,
    ) -> (String, String) {
        let caller_id = persist_generation_fixture(ctx, project_id, generation, caller).await;
        let callee_id = persist_generation_fixture(ctx, project_id, generation, callee).await;
        ctx.state
            .storage
            .upsert_file_checkpoint(&checkpoint(
                project_id,
                generation,
                &format!("src/{caller}.rs"),
            ))
            .await
            .unwrap();
        ctx.state
            .storage
            .upsert_file_checkpoint(&checkpoint(
                project_id,
                generation,
                &format!("src/{callee}.rs"),
            ))
            .await
            .unwrap();
        let caller_thing = crate::types::safe_thing::symbol_thing(
            project_id,
            &format!("src/{caller}.rs"),
            caller,
            1,
        );
        let callee_thing = crate::types::safe_thing::symbol_thing(
            project_id,
            &format!("src/{callee}.rs"),
            callee,
            1,
        );
        ctx.state
            .storage
            .create_symbol_relation(SymbolRelation::new(
                caller_thing,
                callee_thing,
                CodeRelationType::Calls,
                RelationClass::Observed,
                RelationProvenance::ParserExtracted,
                ConfidenceClass::Extracted,
                generation,
                StalenessState::Current,
                format!("src/{caller}.rs"),
                1,
                project_id.to_string(),
            ))
            .await
            .unwrap();
        (caller_id, callee_id)
    }

    async fn assert_capability_contract_fields(
        ctx: &TestContext,
        project_id: &str,
        json: &serde_json::Value,
        expected_serving: Option<u64>,
        expected_indexing: u64,
        expected_reason_codes: &[&str],
    ) {
        assert_eq!(
            json["capability_readiness"]["serving_generation"],
            serde_json::to_value(expected_serving).unwrap(),
            "capability serving_generation mismatch for response: {json}"
        );
        assert_eq!(
            json["capability_readiness"]["indexing_generation"], expected_indexing,
            "capability indexing_generation mismatch for response: {json}"
        );
        assert!(json["capability_readiness"]["capabilities"].is_array());
        assert_eq!(
            json["serving_generation"],
            serde_json::to_value(expected_serving).unwrap()
        );
        assert_eq!(json["indexing_generation"], expected_indexing);
        for expected_reason_code in expected_reason_codes {
            assert!(
                json["capability_readiness"]["capabilities"]
                    .as_array()
                    .expect("capabilities should be an array")
                    .iter()
                    .any(|capability| capability["reason_code"].as_str()
                        == Some(*expected_reason_code)),
                "missing response capability reason_code={expected_reason_code:?}: {json}"
            );
        }
    }

    fn assert_item_freshness(
        json: &serde_json::Value,
        expected_generation: u64,
        expected_freshness: &str,
    ) {
        let items = json["results"]
            .as_array()
            .expect("results should be an array");
        assert!(!items.is_empty(), "expected at least one result item");
        for item in items {
            assert_eq!(item["freshness"]["generation"], expected_generation);
            assert_eq!(item["freshness"]["serving_generation"], expected_generation);
            assert_eq!(item["freshness"]["state"], expected_freshness);
        }
    }

    async fn four_tool_contract_responses(
        ctx: &TestContext,
        project_id: &str,
        symbol_id: &str,
        query: &str,
    ) -> Vec<(&'static str, serde_json::Value)> {
        vec![
            (
                "recall_code",
                tool_result_json(
                    &super::recall_code(&ctx.state, recall_params(project_id, query))
                        .await
                        .unwrap(),
                ),
            ),
            (
                "search_symbols",
                tool_result_json(
                    &super::search_symbols(&ctx.state, symbol_params(project_id, query))
                        .await
                        .unwrap(),
                ),
            ),
            (
                "symbol_graph",
                tool_result_json(
                    &super::symbol_graph(&ctx.state, symbol_graph_params(symbol_id))
                        .await
                        .unwrap(),
                ),
            ),
            (
                "project_info_stats",
                tool_result_json(
                    &super::get_project_stats(
                        &ctx.state,
                        GetProjectStatsParams {
                            project_id: project_id.to_string(),
                        },
                    )
                    .await
                    .unwrap(),
                ),
            ),
        ]
    }

    #[tokio::test]
    async fn capability_readiness_contract_fresh() {
        let ctx = TestContext::new().await;
        let project_id = "capability-readiness-fresh";
        let symbol_id =
            persist_generation_fixture(&ctx, project_id, 1, "fresh_contract_marker").await;
        ctx.state
            .storage
            .set_active_generation(project_id, 1)
            .await
            .unwrap();

        for (tool, json) in
            four_tool_contract_responses(&ctx, project_id, &symbol_id, "fresh_contract_marker")
                .await
        {
            assert_eq!(json["summary"]["partial"]["is_partial"], false, "{tool}");
            assert!(
                json["summary"]["partial"]["reason_code"].is_null()
                    || json["summary"]["partial"]["reason_code"] == "fresh",
                "{tool} should not report degraded reason_code: {json}"
            );
            assert_capability_contract_fields(&ctx, project_id, &json, Some(1), 1, &[]).await;
            if tool != "project_info_stats" && tool != "symbol_graph" {
                assert_item_freshness(&json, 1, "fresh");
            }
        }
    }

    #[tokio::test]
    async fn capability_readiness_contract_stale_serving_generation() {
        let ctx = TestContext::new().await;
        let project_id = "capability-readiness-stale";
        let other_project_id = "capability-readiness-stale-other";
        let symbol_id =
            persist_generation_fixture(&ctx, project_id, 1, "stale_contract_marker").await;
        persist_generation_fixture(&ctx, other_project_id, 1, "other_project_marker").await;
        ctx.state
            .storage
            .set_active_generation(project_id, 1)
            .await
            .unwrap();
        ctx.state
            .storage
            .set_indexing_generation(project_id, Some(2))
            .await
            .unwrap();

        for (tool, json) in
            four_tool_contract_responses(&ctx, project_id, &symbol_id, "stale_contract_marker")
                .await
        {
            if tool != "project_info_stats" {
                assert!(
                    json["summary"]["partial"]["is_partial"]
                        .as_bool()
                        .unwrap_or(false),
                    "{tool}"
                );
                assert!(
                    matches!(
                        json["summary"]["partial"]["reason_code"].as_str(),
                        Some("stale") | Some("partial")
                    ),
                    "{tool} must expose stale/partial reason_code: {json}"
                );
            }
            assert_capability_contract_fields(
                &ctx,
                project_id,
                &json,
                Some(1),
                2,
                &["stale", "partial"],
            )
            .await;
            if tool != "project_info_stats" && tool != "symbol_graph" {
                assert_item_freshness(&json, 1, "stale");
                assert!(!json.to_string().contains("other_project_marker"));
            }
        }
    }

    #[tokio::test]
    async fn capability_readiness_contract_no_serving_generation() {
        let ctx = TestContext::new().await;
        let project_id = "capability-readiness-no-serving";
        let other_project_id = "capability-readiness-no-serving-other";
        let other_symbol_id =
            persist_generation_fixture(&ctx, other_project_id, 1, "no_serving_other_marker").await;
        ctx.state
            .storage
            .set_active_generation(other_project_id, 1)
            .await
            .unwrap();
        ctx.state
            .storage
            .set_indexing_generation(project_id, Some(1))
            .await
            .unwrap();
        let session_id = "capability-readiness-no-serving-session";
        ctx.state
            .session_bindings
            .bind(session_id.to_string(), project_id.to_string())
            .await;
        let context = Some(CodeToolContext::from_session_id(Some(
            session_id.to_string(),
        )));

        let recall = tool_result_json(
            &super::recall_code_with_context(
                &ctx.state,
                RecallCodeParams {
                    project_id: None,
                    ..recall_params(project_id, "no_serving_other_marker")
                },
                context.clone(),
            )
            .await
            .unwrap(),
        );
        let symbols = tool_result_json(
            &super::search_symbols_with_context(
                &ctx.state,
                SearchSymbolsParams {
                    project_id: None,
                    ..symbol_params(project_id, "no_serving_other_marker")
                },
                context,
            )
            .await
            .unwrap(),
        );
        let graph = tool_result_json(
            &super::symbol_graph(&ctx.state, symbol_graph_params(&other_symbol_id))
                .await
                .unwrap(),
        );
        let stats = tool_result_json(
            &super::get_project_stats(
                &ctx.state,
                GetProjectStatsParams {
                    project_id: project_id.to_string(),
                },
            )
            .await
            .unwrap(),
        );

        for (tool, json) in [
            ("recall_code", recall),
            ("search_symbols", symbols),
            ("symbol_graph", graph),
            ("project_info_stats", stats),
        ] {
            if tool != "symbol_graph" && tool != "project_info_stats" {
                assert_eq!(json["summary"]["partial"]["is_partial"], true, "{tool}");
                assert!(
                    matches!(
                        json["summary"]["partial"]["reason_code"].as_str(),
                        Some("missing") | Some("no_serving_generation")
                    ),
                    "{tool} must expose missing/no_serving_generation reason_code: {json}"
                );
                assert_eq!(
                    json["count"]
                        .as_u64()
                        .or_else(|| json["symbol_count"].as_u64()),
                    Some(0),
                    "{tool}"
                );
            }
            if tool != "symbol_graph" && tool != "project_info_stats" {
                assert_capability_contract_fields(&ctx, project_id, &json, None, 1, &["missing"])
                    .await;
                assert!(!json.to_string().contains("no_serving_other_marker"));
            }
            if let Some(project_resolution) = json.get("project_resolution") {
                assert_ne!(project_resolution["source"], "cross_project", "{tool}");
                assert_eq!(project_resolution["project_id"], project_id, "{tool}");
            }
        }
    }

    #[tokio::test]
    async fn capability_readiness_contract_interrupted_generation() {
        let ctx = TestContext::new().await;
        let project_id = "capability-readiness-interrupted";
        let symbol_id =
            persist_generation_fixture(&ctx, project_id, 1, "prior_serving_marker").await;
        persist_generation_fixture(&ctx, project_id, 2, "interrupted_generation_marker").await;
        ctx.state
            .storage
            .set_active_generation(project_id, 1)
            .await
            .unwrap();

        for (tool, json) in
            four_tool_contract_responses(&ctx, project_id, &symbol_id, "prior_serving_marker").await
        {
            if tool != "project_info_stats" {
                assert_eq!(json["summary"]["partial"]["is_partial"], true, "{tool}");
                assert!(
                    matches!(
                        json["summary"]["partial"]["reason_code"].as_str(),
                        Some("stale") | Some("partial") | Some("degraded")
                    ),
                    "{tool} must expose interrupted/degraded capability reason_code: {json}"
                );
            }
            assert_capability_contract_fields(
                &ctx,
                project_id,
                &json,
                Some(1),
                2,
                &["stale", "partial", "degraded"],
            )
            .await;
            assert!(!json.to_string().contains("interrupted_generation_marker"));
            if tool != "project_info_stats" && tool != "symbol_graph" {
                assert_item_freshness(&json, 1, "stale");
            }
        }
    }

    #[tokio::test]
    async fn symbol_graph_serves_stale_graph_generation() {
        let ctx = TestContext::new().await;
        let (_caller_id, _callee_id) = persist_call_relation_fixture(
            &ctx,
            "project",
            1,
            "stale_graph_caller",
            "stale_graph_callee",
        )
        .await;
        persist_generation_fixture(&ctx, "project", 2, "indexing_only_symbol").await;
        ctx.state
            .storage
            .set_serving_generation("project", CapabilityKind::Symbols, 1)
            .await
            .unwrap();
        ctx.state
            .storage
            .set_serving_generation("project", CapabilityKind::Graph, 1)
            .await
            .unwrap();
        ctx.state
            .storage
            .set_indexing_generation("project", Some(2))
            .await
            .unwrap();
        assert_eq!(
            ctx.state
                .storage
                .get_serving_metadata("project")
                .await
                .unwrap()
                .graph,
            Some(1)
        );

        let response = tool_result_json(
            &super::symbol_graph(
                &ctx.state,
                SymbolGraphParams {
                    symbol_id: format!(
                        "code_symbols:{}",
                        crate::types::safe_thing::symbol_hash(
                            "project",
                            "src/stale_graph_caller.rs",
                            "stale_graph_caller",
                            1,
                        )
                    ),
                    action: "related".to_string(),
                    depth: Some(1),
                    direction: Some("outgoing".to_string()),
                },
            )
            .await
            .unwrap(),
        );
        eprintln!("symbol_graph stale response: {response}");
        assert_eq!(response["serving_generation"], 1);
        assert_eq!(response["summary"]["serving_generation"]["graph"], 1);
        assert_eq!(response["indexing_generation"], 2);
        assert_eq!(response["symbol_count"], 1);
        assert_eq!(response["relation_count"], 1);
        assert!(response.to_string().contains("stale_graph_callee"));
        assert!(!response.to_string().contains("indexing_only_symbol"));
        assert_eq!(response["nodes"][0]["freshness"]["state"], "fresh");
        assert_eq!(response["nodes"][0]["freshness"]["serving_generation"], 1);
        assert_eq!(response["edges"][0]["freshness"]["state"], "fresh");
    }

    #[tokio::test]
    async fn symbol_graph_falls_back_to_symbol_frontier_when_graph_missing() {
        let ctx = TestContext::new().await;
        let project_id = "symbol-graph-frontier-missing";
        let symbol_id = persist_generation_fixture(&ctx, project_id, 7, "frontier_symbol").await;
        ctx.state
            .storage
            .upsert_file_checkpoint(&checkpoint(project_id, 7, "src/frontier_symbol.rs"))
            .await
            .unwrap();
        ctx.state
            .storage
            .set_serving_generation(project_id, CapabilityKind::Symbols, 7)
            .await
            .unwrap();

        let response = tool_result_json(
            &super::symbol_graph(
                &ctx.state,
                SymbolGraphParams {
                    symbol_id: symbol_id.clone(),
                    action: "related".to_string(),
                    depth: Some(1),
                    direction: Some("both".to_string()),
                },
            )
            .await
            .unwrap(),
        );

        assert_eq!(response["summary"]["partial"]["is_partial"], true);
        assert_eq!(response["summary"]["partial"]["reason"], "missing_graph");
        assert_eq!(response["fallback_path"], "symbol_frontier");
        assert_eq!(response["serving_generation"], serde_json::Value::Null);
        assert_eq!(response["symbol_serving_generation"], 7);
        assert_eq!(response["relation_count"], 0);
        assert_eq!(response["frontier"][0], symbol_id);
        assert_eq!(response["nodes"][0]["freshness"]["state"], "fresh");
        assert_eq!(response["nodes"][0]["freshness"]["serving_generation"], 7);
        assert_eq!(response["relation_count"], 0);
        assert_eq!(response["frontier"][0], symbol_id);
        assert_eq!(response["nodes"][0]["freshness"]["state"], "fresh");
        assert_eq!(response["nodes"][0]["freshness"]["serving_generation"], 7);
    }

    #[tokio::test]
    async fn search_symbols_serves_stale_symbol_generation() {
        let ctx = TestContext::new().await;
        let project_id = "search-symbols-stale-gen";
        persist_generation_fixture(&ctx, project_id, 1, "stale_sym_marker").await;
        persist_generation_fixture(&ctx, project_id, 2, "indexing_only_sym_marker").await;
        ctx.state
            .storage
            .set_serving_generation(project_id, CapabilityKind::Symbols, 1)
            .await
            .unwrap();
        ctx.state
            .storage
            .set_indexing_generation(project_id, Some(2))
            .await
            .unwrap();

        let response = tool_result_json(
            &super::search_symbols(
                &ctx.state,
                SearchSymbolsParams {
                    query: "stale_sym_marker".to_string(),
                    project_id: Some(project_id.to_string()),
                    limit: None,
                    offset: None,
                    symbol_type: None,
                    path_prefix: None,
                },
            )
            .await
            .unwrap(),
        );

        assert_eq!(response["summary"]["partial"]["is_partial"], true);
        assert!(
            matches!(
                response["summary"]["partial"]["reason_code"].as_str(),
                Some("stale") | Some("partial") | Some("degraded")
            ),
            "expected stale/partial/degraded reason_code, got: {}",
            response
        );
        assert_eq!(response["serving_generation"], 1);
        assert_eq!(response["indexing_generation"], 2);
        assert!(response.to_string().contains("stale_sym_marker"));
        assert!(!response.to_string().contains("indexing_only_sym_marker"));
        let results = response["results"].as_array().expect("results array");
        for item in results {
            assert!(item.get("embedding").is_none() || item["embedding"].is_null());
        }
    }

    #[tokio::test]
    async fn search_symbols_marks_unchanged_symbols_fresh_during_indexing() {
        let ctx = TestContext::new().await;
        let project_id = "search-symbols-freshness-mix";
        persist_generation_fixture(&ctx, project_id, 1, "fresh_sym_a").await;
        ctx.state
            .storage
            .upsert_file_checkpoint(&checkpoint(project_id, 1, "src/fresh_sym_a.rs"))
            .await
            .unwrap();
        persist_generation_fixture(&ctx, project_id, 1, "stale_sym_b").await;

        ctx.state
            .storage
            .set_serving_generation(project_id, CapabilityKind::Symbols, 1)
            .await
            .unwrap();
        ctx.state
            .storage
            .set_indexing_generation(project_id, Some(2))
            .await
            .unwrap();

        let response = tool_result_json(
            &super::search_symbols(
                &ctx.state,
                SearchSymbolsParams {
                    query: "sym".to_string(),
                    project_id: Some(project_id.to_string()),
                    limit: Some(10),
                    offset: None,
                    symbol_type: None,
                    path_prefix: None,
                },
            )
            .await
            .unwrap(),
        );

        let results = response["results"].as_array().expect("results array");
        assert!(!results.is_empty(), "expected results");

        let fresh_item = results
            .iter()
            .find(|item| item["name"].as_str() == Some("fresh_sym_a"))
            .expect("fresh_sym_a not found in results");
        let stale_item = results
            .iter()
            .find(|item| item["name"].as_str() == Some("stale_sym_b"))
            .expect("stale_sym_b not found in results");

        assert_eq!(
            fresh_item["freshness"]["state"], "fresh",
            "fresh_sym_a should be fresh (has checkpoint): {}",
            fresh_item
        );
        assert!(
            matches!(
                stale_item["freshness"]["state"].as_str(),
                Some("stale") | Some("unknown")
            ),
            "stale_sym_b should be stale or unknown (no checkpoint): {}",
            stale_item
        );
    }

    #[tokio::test]
    async fn search_symbols_no_serving_generation_partial_contract() {
        let ctx = TestContext::new().await;
        let project_id = "search-symbols-no-serving";
        persist_generation_fixture(&ctx, project_id, 1, "no_serving_sym_marker").await;

        let response = tool_result_json(
            &super::search_symbols(
                &ctx.state,
                SearchSymbolsParams {
                    query: "no_serving_sym_marker".to_string(),
                    project_id: Some(project_id.to_string()),
                    limit: None,
                    offset: None,
                    symbol_type: None,
                    path_prefix: None,
                },
            )
            .await
            .unwrap(),
        );

        assert_eq!(response["results"], serde_json::Value::Array(vec![]));
        assert_eq!(response["count"], 0);
        assert_eq!(response["summary"]["partial"]["is_partial"], true);
        assert_eq!(response["summary"]["partial"]["reason_code"], "missing");
        assert_eq!(
            response["summary"]["partial"]["reason"],
            "no_serving_generation"
        );
    }

    #[tokio::test]
    async fn indexing_does_not_replace_serving_generation_until_promote() {
        let ctx = TestContext::new().await;
        let project_id = "atomic-promote-waits";

        persist_generation_fixture(&ctx, project_id, 1, "prior").await;
        ctx.state
            .storage
            .set_serving_generation(project_id, CapabilityKind::ProjectInfo, 1)
            .await
            .unwrap();
        ctx.state
            .storage
            .set_indexing_generation(project_id, Some(2))
            .await
            .unwrap();
        persist_generation_fixture(&ctx, project_id, 2, "building").await;

        let serving_generation = ctx
            .state
            .storage
            .get_serving_generation(project_id, CapabilityKind::ProjectInfo)
            .await
            .unwrap();

        assert_eq!(serving_generation, Some(1));
    }

    #[tokio::test]
    async fn interrupted_generation_is_never_served() {
        let ctx = TestContext::new().await;
        let project_id = "interrupted-generation-never-served";

        persist_generation_fixture(&ctx, project_id, 1, "serving_interrupt_marker").await;
        persist_generation_fixture(&ctx, project_id, 2, "interrupted_interrupt_marker").await;
        ctx.state
            .storage
            .set_serving_generation(project_id, CapabilityKind::ProjectInfo, 1)
            .await
            .unwrap();
        ctx.state
            .storage
            .set_serving_generation(project_id, CapabilityKind::Bm25, 1)
            .await
            .unwrap();
        ctx.state
            .storage
            .set_serving_generation(project_id, CapabilityKind::Symbols, 1)
            .await
            .unwrap();
        ctx.state
            .storage
            .set_indexing_generation(project_id, Some(2))
            .await
            .unwrap();

        let symbols = tool_result_json(
            &super::search_symbols_with_context(
                &ctx.state,
                symbol_params(project_id, "interrupt_marker"),
                None,
            )
            .await
            .unwrap(),
        );
        let recall = tool_result_json(
            &super::recall_code_with_context(
                &ctx.state,
                recall_params(project_id, "interrupt_marker"),
                None,
            )
            .await
            .unwrap(),
        );

        for (tool, response) in [("search_symbols", symbols), ("recall_code", recall)] {
            let response_text = response.to_string();
            assert!(
                response_text.contains("serving_interrupt_marker"),
                "{tool} should serve prior generation: {response}"
            );
            assert!(
                !response_text.contains("interrupted_interrupt_marker"),
                "{tool} must not serve interrupted generation: {response}"
            );
            assert!(
                matches!(
                    response["summary"]["partial"]["reason_code"].as_str(),
                    Some("stale") | Some("partial") | Some("degraded")
                ),
                "{tool} should expose stale/interrupted partial state: {response}"
            );
        }
    }

    #[tokio::test]
    async fn capability_promotion_is_independent() {
        let ctx = TestContext::new().await;
        let project_id = "capability-promotion-independent";

        persist_generation_fixture(&ctx, project_id, 1, "semantic_gap_prior").await;
        persist_generation_fixture(&ctx, project_id, 2, "semantic_gap_current").await;
        ctx.state
            .storage
            .set_serving_generation(project_id, CapabilityKind::ProjectInfo, 2)
            .await
            .unwrap();
        ctx.state
            .storage
            .set_serving_generation(project_id, CapabilityKind::Bm25, 2)
            .await
            .unwrap();
        ctx.state
            .storage
            .set_serving_generation(project_id, CapabilityKind::Symbols, 2)
            .await
            .unwrap();
        ctx.state
            .storage
            .set_serving_generation(project_id, CapabilityKind::Semantic, 1)
            .await
            .unwrap();

        let symbols = tool_result_json(
            &super::search_symbols_with_context(
                &ctx.state,
                symbol_params(project_id, "semantic_gap_current"),
                None,
            )
            .await
            .unwrap(),
        );
        assert!(
            symbols.to_string().contains("semantic_gap_current"),
            "symbols should serve independently promoted generation: {symbols}"
        );
        assert_eq!(symbols["serving_generation"], 2);

        let recall = tool_result_json(
            &super::recall_code_with_context(
                &ctx.state,
                recall_params(project_id, "semantic_gap_current"),
                None,
            )
            .await
            .unwrap(),
        );
        assert!(
            recall.to_string().contains("semantic_gap_current"),
            "recall_code should fall back to BM25/structural serving generation: {recall}"
        );
        assert_eq!(recall["summary"]["serving_generation"]["bm25"], 2);
        assert_eq!(recall["summary"]["serving_generation"]["symbols"], 2);
        assert_eq!(recall["summary"]["serving_generation"]["semantic"], 1);
        assert_eq!(recall["summary"]["partial"]["is_partial"], true);
        assert_eq!(recall["summary"]["partial"]["reason_code"], "degraded");
        assert_eq!(
            recall["summary"]["fallback_path"],
            "bm25_lexical_symbol_hydration"
        );
    }
}
