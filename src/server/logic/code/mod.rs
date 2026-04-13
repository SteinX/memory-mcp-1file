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
    delete_project, get_degradation_info, get_index_status, get_project_stats, index_project,
    list_projects,
};
pub use search::{recall_code, search_code};
pub use symbols::{search_symbols, symbol_graph};

#[cfg(test)]
mod tests {
    use crate::server::params::{
        GetIndexStatusParams, IndexProjectParams, SearchCodeParams, SearchSymbolsParams,
    };
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
}
