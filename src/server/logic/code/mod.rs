//! Code indexing, search, and symbol logic.
//!
//! This module is the handler facade — it re-exports all public functions from
//! the sub-modules so that callers (`server/handler.rs`, tests) can use the
//! original unqualified paths without changes.

mod indexing;
mod search;
mod symbols;

// Re-export everything so external callers see the same flat API as before.
pub use indexing::{
    delete_project, get_degradation_info, get_index_status, get_project_projection,
    get_project_projection_by_locator, get_project_stats, index_project, list_projects,
};
pub use search::{recall_code, search_code};
pub use symbols::{search_symbols, symbol_graph};

#[cfg(test)]
mod tests {
    use crate::server::params::{
        GetIndexStatusParams, GetProjectProjectionParams, GetProjectionByLocatorParams,
        IndexProjectParams, SearchCodeParams,
        SearchSymbolsParams, SymbolGraphParams,
    };
    use crate::storage::StorageBackend;
    use crate::test_utils::TestContext;
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
            path: project_path.to_string_lossy().to_string(),
            force: None,
            confirm_failed_restart: None,
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
                path: project_path.to_string_lossy().to_string(),
                force: None,
                confirm_failed_restart: None,
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
                path: project_path.to_string_lossy().to_string(),
                force: None,
                confirm_failed_restart: None,
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
                path: project_path.to_string_lossy().to_string(),
                force: None,
                confirm_failed_restart: None,
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
            assert!(retries <= 100, "Indexing timed out for symbol graph contract test");
        }

        let symbols = ctx
            .state
            .storage
            .search_symbols("caller", Some(&unique_id), 10, 0, None, None)
            .await
            .unwrap()
            .0;
        let caller_id = symbols
            .iter()
            .find(|symbol| symbol.name == "caller")
            .and_then(|symbol| symbol.id.as_ref())
            .map(|id| crate::types::record_key_to_string(&id.key))
            .expect("caller symbol should exist");

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
        assert_eq!(json["contract"]["compatibility"]["db_shape_is_not_public_contract"], true);
        assert_eq!(json["contract"]["identity"]["stable_node_ids"], true);
        assert_eq!(json["contract"]["identity"]["edge_ids_are_local_only"], true);
        assert_eq!(json["contract"]["identity"]["node_id_semantics"], "stable_project_scoped_symbol_id");
        assert_eq!(json["contract"]["identity"]["edge_id_semantics"], "local_only_edge_reference");
        assert_eq!(json["contract"]["surface_guidance"]["preferred_response_fields"][0], "nodes");
        assert_eq!(json["contract"]["surface_guidance"]["legacy_compatibility_fields"][0], "symbols");
        assert_eq!(json["contract"]["traversal_defaults"]["frontier_semantics"], "unexpanded_symbol_boundary_for_manual_follow_up");
        assert_eq!(json["contract"]["traversal_defaults"]["frontier_items_identity_basis"], "stable_project_scoped_symbol_id");
        assert_eq!(json["contract"]["traversal_defaults"]["frontier_items_are_stable_node_ids"], true);
        assert_eq!(json["contract"]["traversal_defaults"]["frontier_items_are_project_scoped"], true);
        assert_eq!(json["contract"]["traversal_defaults"]["frontier_is_cursor"], false);
        assert_eq!(json["contract"]["projection_state"], "missing");
        assert_eq!(json["nodes"][0]["kind"], "function");
        assert_eq!(json["edges"][0]["relation_type"], "calls");
    }

    #[tokio::test]
    async fn code_search_exposes_contract_metadata() {
        let ctx = TestContext::new().await;
        let unique_id = format!("test_code_search_contract_{}", uuid::Uuid::new_v4().simple());
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
                path: project_path.to_string_lossy().to_string(),
                force: None,
                confirm_failed_restart: None,
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
            assert!(retries <= 100, "Indexing timed out for code search contract test");
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
        assert_eq!(search_json["contract"]["identity"]["stable_node_ids"], false);
        assert_eq!(search_json["contract"]["identity"]["node_ids_are_project_scoped"], false);
        assert_eq!(search_json["contract"]["identity"]["node_id_semantics"], "local_only_chunk_record_id; stable_local_locator_is_project_id_plus_file_path_plus_start_line_plus_end_line");
        assert_eq!(search_json["contract"]["identity"]["edge_id_semantics"], "local_only_result_edge_reference");
        assert_eq!(search_json["contract"]["surface_guidance"]["preferred_response_fields"][0], "results[].file_path");

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
        assert_eq!(recall_json["contract"]["identity"]["stable_node_ids"], false);
        assert_eq!(recall_json["contract"]["identity"]["node_ids_are_project_scoped"], false);
        assert_eq!(recall_json["contract"]["identity"]["node_id_semantics"], "local_only_chunk_record_id; stable_local_locator_is_project_id_plus_file_path_plus_start_line_plus_end_line");
        assert_eq!(recall_json["contract"]["identity"]["edge_id_semantics"], "local_only_result_edge_reference");
        assert_eq!(recall_json["contract"]["surface_guidance"]["forbidden_to_depend_fields"][0], "results[].id");
        assert_eq!(recall_json["contract"]["surface_guidance"]["preferred_response_fields"][0], "results[].file_path");
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
                path: project_path.to_string_lossy().to_string(),
                force: None,
                confirm_failed_restart: None,
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
            assert!(retries <= 100, "Indexing timed out for projection builder test");
        }

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

        assert_eq!(json["project_id"], unique_id);
        assert_eq!(json["projection"]["project_id"], json["project_id"]);
        assert_eq!(json["projection"]["request"]["relation_scope"], "all");
        assert_eq!(json["projection"]["request"]["sort_mode"], "canonical");
        assert_eq!(json["projection"]["shaping"]["relation_scope_applied"], "all");
        assert_eq!(json["projection"]["shaping"]["sort_mode_applied"], "canonical");
        assert_eq!(json["projection"]["shaping"]["node_selection_basis"], "relation_endpoint_induced_subgraph");
        assert_eq!(json["projection"]["shaping"]["edge_selection_basis"], "all_relation_edges");
        assert_eq!(json["projection"]["shaping"]["output_kind"], "induced_symbol_graph");
        assert_eq!(json["projection"]["contract"]["schema_version"], 1);
        assert_eq!(json["projection"]["contract"]["projection"]["basis"], "semantic_generation");
        assert_eq!(json["projection"]["summary"]["result_kind"], "graph");
        assert_eq!(json["projection"]["summary"]["partial"]["is_partial"], true);
        assert_eq!(json["projection"]["summary"]["partial"]["reason_code"], "stale");
        assert_eq!(json["projection"]["summary"]["partial"]["reason"], "projection_stale");
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
                path: project_path.to_string_lossy().to_string(),
                force: None,
                confirm_failed_restart: None,
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
            assert!(retries <= 100, "Indexing timed out for projection options test");
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
        assert_eq!(json["projection"]["shaping"]["relation_scope_applied"], "none");
        assert_eq!(json["projection"]["shaping"]["node_selection_basis"], "empty_graph_when_no_edges_retained");
        assert_eq!(json["projection"]["shaping"]["edge_selection_basis"], "no_edges_retained");
        assert_eq!(json["projection"]["shaping"]["output_kind"], "empty_graph");
        assert_eq!(json["projection"]["nodes"].as_array().unwrap().len(), 0);
        assert_eq!(json["projection"]["edges"].as_array().unwrap().len(), 0);
        assert_eq!(json["projection"]["counts"]["nodes"].as_u64().unwrap(), 0);
        assert_eq!(json["projection"]["counts"]["edges"].as_u64().unwrap(), 0);
        assert_eq!(json["projection"]["contract"]["schema_version"], 1);
        assert_eq!(json["projection"]["contract"]["identity"]["project_id"], unique_id);
        assert_eq!(json["projection"]["contract"]["projection"]["basis"], "semantic_generation");
        assert_eq!(json["projection"]["summary"]["partial"]["is_partial"], true);
        assert_eq!(json["projection"]["summary"]["partial"]["reason_code"], "stale");
        assert_eq!(json["projection"]["summary"]["partial"]["reason"], "projection_stale");
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
                path: project_path.to_string_lossy().to_string(),
                force: None,
                confirm_failed_restart: None,
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
            assert!(retries <= 100, "Indexing timed out for projection imports test");
        }

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

        assert_eq!(json["projection"]["request"]["relation_scope"], "imports");
        assert_eq!(json["projection"]["request"]["sort_mode"], "canonical");
        assert_eq!(json["projection"]["shaping"]["relation_scope_applied"], "imports");
        assert_eq!(json["projection"]["shaping"]["node_selection_basis"], "relation_endpoint_induced_subgraph");
        assert_eq!(json["projection"]["shaping"]["edge_selection_basis"], "only_import_edges");
        assert_eq!(json["projection"]["shaping"]["output_kind"], "induced_symbol_graph");
        let edges = json["projection"]["edges"].as_array().unwrap();
        assert!(edges.iter().all(|edge| edge["relation_type"] == "imports"));
        assert_eq!(json["projection"]["counts"]["edges"].as_u64().unwrap(), edges.len() as u64);
        let nodes = json["projection"]["nodes"].as_array().unwrap();
        assert_eq!(json["projection"]["counts"]["nodes"].as_u64().unwrap(), nodes.len() as u64);
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
        assert_eq!(json["projection"]["contract"]["identity"]["project_id"], unique_id);
        assert_eq!(json["projection"]["summary"]["partial"]["is_partial"], true);
        assert_eq!(json["projection"]["summary"]["partial"]["reason_code"], "stale");
        assert_eq!(json["projection"]["summary"]["partial"]["reason"], "projection_stale");
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
                path: project_path.to_string_lossy().to_string(),
                force: None,
                confirm_failed_restart: None,
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
            assert!(retries <= 100, "Indexing timed out for locator projection test");
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

        assert_eq!(json["locator"]["locator_kind"], "ephemeral_projection_handle");
        assert_eq!(json["locator"]["project_id"], unique_id);
        assert_eq!(json["locator"]["lookup"]["state"], "created");
        assert_eq!(json["locator"]["lookup"]["found"], true);
        assert_eq!(json["locator"]["lifecycle"]["same_process_only"], true);
        assert_eq!(json["locator"]["lifecycle"]["survives_process_restart"], false);
        assert_eq!(json["locator"]["lifecycle"]["survives_generation_change"], false);
        assert_eq!(json["projection"]["contract"]["projection"]["materialization"]["is_addressable"], false);

        let readback_res = super::get_project_projection_by_locator(
            &ctx.state,
            GetProjectionByLocatorParams { locator: locator.clone() },
        )
        .await
        .unwrap();

        let readback_value = serde_json::to_value(&readback_res).unwrap();
        let readback_text = readback_value["content"][0]["text"].as_str().unwrap();
        let readback_json: serde_json::Value = serde_json::from_str(readback_text).unwrap();

        assert_eq!(readback_json["locator"]["locator"], locator);
        assert_eq!(readback_json["locator"]["locator_kind"], "ephemeral_projection_handle");
        assert_eq!(readback_json["locator"]["lookup"]["state"], "resolved");
        assert_eq!(readback_json["locator"]["lookup"]["found"], true);
        assert!(readback_json["locator"]["lookup"]["reason_code"].is_null());
        assert_eq!(readback_json["projection"]["project_id"], json["projection"]["project_id"]);
        assert_eq!(readback_json["projection"]["request"], json["projection"]["request"]);
        assert_eq!(readback_json["projection"]["summary"], json["projection"]["summary"]);
        assert_eq!(readback_json["projection"]["counts"], json["projection"]["counts"]);
        assert_eq!(readback_json["projection"]["nodes"], json["projection"]["nodes"]);
        assert_eq!(readback_json["projection"]["edges"], json["projection"]["edges"]);
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
}
