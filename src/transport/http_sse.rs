use axum::{http::StatusCode, response::IntoResponse, routing::get, Json};
use rmcp::{
    service::{RoleServer, Service},
    transport::streamable_http_server::{
        session::local::LocalSessionManager,
        tower::{StreamableHttpServerConfig, StreamableHttpService},
    },
};
use std::{net::SocketAddr, sync::Arc};
use tokio_util::sync::CancellationToken;
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::config::AppState;
use crate::lifecycle::record_runtime_event_with_details;
use crate::storage::StorageBackend;

pub struct HttpServerConfig {
    pub bind: String,
    pub port: u16,
}

impl Default for HttpServerConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1".to_string(),
            port: 8080,
        }
    }
}

/// Health check endpoint for liveness probes.
async fn health_check() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "version": env!("CARGO_PKG_VERSION"),
        })),
    )
}

/// Build the axum Router with all middleware layers.
///
/// Extracted from `serve_http_sse` to enable unit testing of routes,
/// CORS, body limits, and health checks without starting a TCP listener.
fn build_router<S, F>(
    service_factory: F,
    state: Arc<AppState>,
    ct: &CancellationToken,
) -> axum::Router
where
    S: Service<RoleServer> + Send + 'static,
    F: Fn() -> Result<S, std::io::Error> + Send + Sync + 'static,
{
    let session_manager = Arc::new(LocalSessionManager::default());

    let mcp_config = StreamableHttpServerConfig::default()
        .with_stateful_mode(true)
        .with_cancellation_token(ct.child_token())
        .disable_allowed_hosts();

    let mcp_service = StreamableHttpService::new(service_factory, session_manager, mcp_config);

    let mcp_session_id = axum::http::HeaderName::from_static("mcp-session-id");

    // CORS: permissive for development — allows any origin with common headers.
    // The MCP Streamable HTTP spec requires Accept and mcp-session-id headers.
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::any())
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::DELETE,
            axum::http::Method::OPTIONS,
        ])
        .allow_headers([
            axum::http::header::CONTENT_TYPE,
            axum::http::header::ACCEPT,
            axum::http::header::AUTHORIZATION,
            mcp_session_id.clone(),
        ])
        .expose_headers([mcp_session_id])
        .max_age(std::time::Duration::from_secs(3600));

    axum::Router::new()
        .route("/health", get(health_check))
        .with_state(state)
        .nest_service("/mcp", mcp_service)
        .layer(axum::extract::DefaultBodyLimit::max(4 * 1024 * 1024))
        .layer(tower::limit::ConcurrencyLimitLayer::new(64))
        .layer(cors)
        .layer(tower_http::trace::TraceLayer::new_for_http())
}

pub async fn serve_http_sse<S, F>(
    service_factory: F,
    config: HttpServerConfig,
    state: Arc<AppState>,
) -> anyhow::Result<()>
where
    S: Service<RoleServer> + Send + 'static,
    F: Fn() -> Result<S, std::io::Error> + Send + Sync + 'static,
{
    let addr: SocketAddr = format!("{}:{}", config.bind, config.port)
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid address: {}", e))?;

    tracing::info!("Starting HTTP SSE server on http://{}", addr);

    let ct = CancellationToken::new();

    let app = build_router(service_factory, state.clone(), &ct);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    let local_addr = listener.local_addr()?;

    tracing::info!("HTTP SSE server listening on http://{}", local_addr);

    let ct_for_signals = ct.clone();
    let signal_data_dir = state.config.data_dir.clone();
    tokio::spawn(async move {
        let ctrl_c = tokio::signal::ctrl_c();

        #[cfg(unix)]
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("failed to install SIGTERM handler");

        #[cfg(unix)]
        tokio::select! {
            _ = ctrl_c => {
                tracing::info!("Received SIGINT, initiating HTTP server shutdown...");
                record_runtime_event_with_details(
                    &signal_data_dir,
                    "last_signal.json",
                    "signal_received",
                    "http_sse",
                    serde_json::json!({"signal": "SIGINT"}),
                );
            },
            _ = terminate.recv() => {
                tracing::info!("Received SIGTERM, initiating HTTP server shutdown...");
                record_runtime_event_with_details(
                    &signal_data_dir,
                    "last_signal.json",
                    "signal_received",
                    "http_sse",
                    serde_json::json!({"signal": "SIGTERM"}),
                );
            },
        }

        #[cfg(not(unix))]
        {
            ctrl_c.await.ok();
            tracing::info!("Received SIGINT, initiating HTTP server shutdown...");
            record_runtime_event_with_details(
                &signal_data_dir,
                "last_signal.json",
                "signal_received",
                "http_sse",
                serde_json::json!({"signal": "SIGINT"}),
            );
        }

        ct_for_signals.cancel();
    });

    let serve_result = axum::serve(listener, app)
        .with_graceful_shutdown(async move { ct.cancelled().await })
        .await;

    if let Err(e) = serve_result {
        record_runtime_event_with_details(
            &state.config.data_dir,
            "last_error.json",
            "http_server_error",
            "http_sse",
            serde_json::json!({"error": e.to_string()}),
        );
        return Err(e.into());
    }

    tracing::info!("HTTP server stopped, cleaning up...");
    record_runtime_event_with_details(
        &state.config.data_dir,
        "last_shutdown.json",
        "graceful_shutdown_started",
        "http_sse",
        serde_json::json!({"reason": "server_stopped_or_signal"}),
    );
    let _ = state.shutdown_tx.send(true);
    if let Err(e) = state.storage.shutdown().await {
        tracing::warn!("Database shutdown error: {}", e);
    }
    tracing::info!("HTTP server shutdown complete");
    record_runtime_event_with_details(
        &state.config.data_dir,
        "last_shutdown.json",
        "graceful_shutdown_complete",
        "http_sse",
        serde_json::json!({"reason": "server_stopped_or_signal"}),
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use serde_json::{json, Value};
    use tower::ServiceExt;

    use crate::server::params::{DeleteProjectParams, GetIndexStatusParams, IndexProjectParams};
    use crate::server::MemoryMcpServer;
    use crate::test_utils::TestContext;

    fn test_router(state: Arc<AppState>) -> axum::Router {
        let state_for_factory = state.clone();
        let ct = CancellationToken::new();
        build_router(
            move || Ok(MemoryMcpServer::new(state_for_factory.clone())),
            state,
            &ct,
        )
    }

    async fn post_mcp_request(
        app: &axum::Router,
        payload: &Value,
        session_id: Option<&str>,
    ) -> axum::response::Response {
        let mut request = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("host", "127.0.0.1")
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream");

        if let Some(session_id) = session_id {
            request = request.header("mcp-session-id", session_id);
        }

        app.clone()
            .oneshot(
                request
                    .body(Body::from(payload.to_string()))
                    .expect("request should build"),
            )
            .await
            .expect("mcp request should complete")
    }

    fn response_session_id(response: &axum::response::Response) -> Option<String> {
        response
            .headers()
            .get("mcp-session-id")
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    }

    async fn response_json(response: axum::response::Response) -> Value {
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("response body should decode");
        if bytes.is_empty() {
            return json!({});
        }

        if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
            return value;
        }

        let body_text = String::from_utf8_lossy(&bytes);
        let sse_data_line = body_text
            .lines()
            .rev()
            .find_map(|line| line.strip_prefix("data:"))
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .expect("response should contain JSON or SSE data line");

        serde_json::from_str(sse_data_line)
            .unwrap_or_else(|_| panic!("response should decode from JSON or SSE data: {body_text}"))
    }

    fn parse_tool_result_json(response_json: &Value) -> Value {
        let text = response_json
            .get("result")
            .and_then(|result| result.get("content"))
            .and_then(|content| content.get(0))
            .and_then(|item| item.get("text"))
            .and_then(Value::as_str)
            .expect("tool response should contain result.content[0].text");
        serde_json::from_str(text).expect("tool result text should be valid JSON")
    }

    async fn initialize_http_session(app: &axum::Router, request_id: u64) -> String {
        let initialize_payload = json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "task-9-http-test",
                    "version": "0.1"
                }
            }
        });
        let initialize_response = post_mcp_request(app, &initialize_payload, None).await;
        assert!(
            initialize_response.status().is_success(),
            "initialize should succeed"
        );
        let session_id = response_session_id(&initialize_response)
            .expect("initialize response should issue mcp-session-id");

        let initialized_payload = json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        });
        let initialized_response =
            post_mcp_request(app, &initialized_payload, Some(&session_id)).await;
        assert!(
            initialized_response.status().is_success(),
            "notifications/initialized should succeed"
        );

        session_id
    }

    async fn index_project_and_wait(ctx: &TestContext, project_path: &std::path::Path) -> String {
        let project_id = project_path
            .file_name()
            .and_then(|value| value.to_str())
            .expect("project path should have file name")
            .to_string();

        let _ = crate::server::logic::code::index_project(
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
        .expect("index_project should succeed");

        let status_params = GetIndexStatusParams {
            project_id: project_id.clone(),
        };
        for _ in 0..120 {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            let status =
                crate::server::logic::code::get_index_status(&ctx.state, status_params.clone())
                    .await
                    .expect("get_index_status should succeed")
                    .into_typed::<Value>()
                    .expect("status should be JSON");

            let current = status
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if current == "completed" || current == "embedding_pending" {
                return project_id;
            }
        }

        panic!("Indexing timed out for project_id={project_id}");
    }

    fn recall_code_tool_call_payload(id: u64, query: &str) -> Value {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": {
                "name": "recall_code",
                "arguments": {
                    "query": query,
                    "mode": "vector",
                    "limit": 20
                }
            }
        })
    }

    #[tokio::test]
    async fn health_check_returns_ok_for_liveness() {
        let ctx = TestContext::new().await;
        let app = test_router(ctx.state.clone());

        let resp = app
            .oneshot(Request::get("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "ok");
        assert!(json["version"].is_string());
    }

    #[tokio::test]
    async fn health_check_returns_version() {
        let ctx = TestContext::new().await;
        let app = test_router(ctx.state.clone());

        let resp = app
            .oneshot(Request::get("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();

        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["version"], env!("CARGO_PKG_VERSION"));
    }

    #[tokio::test]
    async fn cors_preflight_returns_allow_headers() {
        let ctx = TestContext::new().await;
        let app = test_router(ctx.state.clone());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("OPTIONS")
                    .uri("/mcp")
                    .header("origin", "http://example.com")
                    .header("access-control-request-method", "POST")
                    .header(
                        "access-control-request-headers",
                        "content-type,mcp-session-id",
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);

        let allow_origin = resp.headers().get("access-control-allow-origin");
        assert!(allow_origin.is_some(), "CORS allow-origin header missing");
        assert_eq!(allow_origin.unwrap(), "*");

        let allow_methods = resp
            .headers()
            .get("access-control-allow-methods")
            .map(|v| v.to_str().unwrap().to_string())
            .unwrap_or_default();
        assert!(
            allow_methods.contains("POST"),
            "POST not in allowed methods"
        );
        assert!(allow_methods.contains("GET"), "GET not in allowed methods");
        assert!(
            allow_methods.contains("DELETE"),
            "DELETE not in allowed methods"
        );
    }

    #[tokio::test]
    async fn cors_exposes_mcp_session_id_header() {
        let ctx = TestContext::new().await;
        let app = test_router(ctx.state.clone());

        // expose-headers is sent on actual cross-origin requests, not preflight
        let resp = app
            .oneshot(
                Request::get("/health")
                    .header("origin", "http://example.com")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let expose = resp
            .headers()
            .get("access-control-expose-headers")
            .map(|v| v.to_str().unwrap().to_lowercase())
            .unwrap_or_default();
        assert!(
            expose.contains("mcp-session-id"),
            "mcp-session-id not in exposed headers: {expose}"
        );
    }

    #[tokio::test]
    async fn unknown_route_returns_404() {
        let ctx = TestContext::new().await;
        let app = test_router(ctx.state.clone());

        let resp = app
            .oneshot(Request::get("/nonexistent").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn mcp_post_without_body_returns_error() {
        let ctx = TestContext::new().await;
        let app = test_router(ctx.state.clone());

        let resp = app
            .oneshot(
                Request::post("/mcp")
                    .header("content-type", "application/json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert!(
            resp.status().is_client_error() || resp.status().is_server_error(),
            "Expected error status for empty POST to /mcp, got {}",
            resp.status()
        );
    }

    #[tokio::test]
    async fn mcp_get_without_session_id_returns_error() {
        let ctx = TestContext::new().await;
        let app = test_router(ctx.state.clone());

        let resp = app
            .oneshot(
                Request::get("/mcp")
                    .header("accept", "text/event-stream")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert!(
            resp.status().is_client_error(),
            "Expected 4xx for GET /mcp without session-id, got {}",
            resp.status()
        );
    }

    #[tokio::test]
    async fn mcp_delete_without_session_id_returns_error() {
        let ctx = TestContext::new().await;
        let app = test_router(ctx.state.clone());

        let resp = app
            .oneshot(Request::delete("/mcp").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert!(
            resp.status().is_client_error(),
            "Expected 4xx for DELETE /mcp without session-id, got {}",
            resp.status()
        );
    }

    #[tokio::test]
    async fn body_size_limit_rejects_oversized_request() {
        let ctx = TestContext::new().await;
        let app = test_router(ctx.state.clone());

        let oversized = vec![b'x'; 5 * 1024 * 1024];

        let resp = app
            .oneshot(
                Request::post("/mcp")
                    .header("content-type", "application/json")
                    .header("accept", "application/json, text/event-stream")
                    .body(Body::from(oversized))
                    .unwrap(),
            )
            .await
            .unwrap();

        // DefaultBodyLimit triggers 413 when axum extracts the body,
        // but nest_service passes the body through to rmcp which may
        // reject with its own error first. Either way, it must not succeed.
        assert!(
            resp.status().is_client_error() || resp.status().is_server_error(),
            "Expected error for oversized body, got {}",
            resp.status()
        );
    }

    #[tokio::test]
    async fn health_endpoint_not_affected_by_post_method() {
        let ctx = TestContext::new().await;
        let app = test_router(ctx.state.clone());

        let resp = app
            .oneshot(Request::post("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[test]
    fn http_server_config_defaults() {
        let config = HttpServerConfig::default();
        assert_eq!(config.bind, "127.0.0.1");
        assert_eq!(config.port, 8080);
    }

    #[tokio::test]
    async fn mcp_http_two_sessions_bind_and_search_isolated() {
        let ctx = TestContext::new().await;
        let app = test_router(ctx.state.clone());

        let project_a_path = ctx._temp_dir.path().join("task9-http-two-sessions-a");
        let project_b_path = ctx._temp_dir.path().join("task9-http-two-sessions-b");
        std::fs::create_dir_all(&project_a_path).expect("project A path should exist");
        std::fs::create_dir_all(&project_b_path).expect("project B path should exist");
        std::fs::write(
            project_a_path.join("lib.rs"),
            "fn task9_http_shared_search() { println!(\"task9-marker-alpha-only\"); }\n",
        )
        .expect("project A source should write");
        std::fs::write(
            project_b_path.join("lib.rs"),
            "fn task9_http_shared_search() { println!(\"task9-marker-beta-only\"); }\n",
        )
        .expect("project B source should write");

        let project_a = index_project_and_wait(&ctx, &project_a_path).await;
        let project_b = index_project_and_wait(&ctx, &project_b_path).await;

        let session_a = initialize_http_session(&app, 1).await;
        let session_b = initialize_http_session(&app, 2).await;
        assert_ne!(session_a, session_b, "sessions should receive distinct ids");

        let bind_a_payload = json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "project_info",
                "arguments": {
                    "action": "bind",
                    "project_id": project_a
                }
            }
        });
        let bind_a_response = post_mcp_request(&app, &bind_a_payload, Some(&session_a)).await;
        assert!(bind_a_response.status().is_success());
        let bind_a_json = parse_tool_result_json(&response_json(bind_a_response).await);
        assert_eq!(bind_a_json["binding"]["session_id"], session_a);
        assert_eq!(bind_a_json["binding"]["project_id"], project_a);

        let bind_b_payload = json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "project_info",
                "arguments": {
                    "action": "bind",
                    "project_id": project_b
                }
            }
        });
        let bind_b_response = post_mcp_request(&app, &bind_b_payload, Some(&session_b)).await;
        assert!(bind_b_response.status().is_success());
        let bind_b_json = parse_tool_result_json(&response_json(bind_b_response).await);
        assert_eq!(bind_b_json["binding"]["session_id"], session_b);
        assert_eq!(bind_b_json["binding"]["project_id"], project_b);

        let query = "task9_http_shared_search";
        let search_a_response = post_mcp_request(
            &app,
            &recall_code_tool_call_payload(5, query),
            Some(&session_a),
        )
        .await;
        assert!(search_a_response.status().is_success());
        let search_a_json = parse_tool_result_json(&response_json(search_a_response).await);
        assert_eq!(
            search_a_json["project_resolution"]["source"],
            "session_binding"
        );
        assert_eq!(search_a_json["project_resolution"]["project_id"], project_a);
        let search_a_text = search_a_json.to_string();
        assert!(search_a_text.contains("task9-marker-alpha-only"));
        assert!(!search_a_text.contains("task9-marker-beta-only"));

        let search_b_response = post_mcp_request(
            &app,
            &recall_code_tool_call_payload(6, query),
            Some(&session_b),
        )
        .await;
        assert!(search_b_response.status().is_success());
        let search_b_json = parse_tool_result_json(&response_json(search_b_response).await);
        assert_eq!(
            search_b_json["project_resolution"]["source"],
            "session_binding"
        );
        assert_eq!(search_b_json["project_resolution"]["project_id"], project_b);
        let search_b_text = search_b_json.to_string();
        assert!(search_b_text.contains("task9-marker-beta-only"));
        assert!(!search_b_text.contains("task9-marker-alpha-only"));
    }

    #[tokio::test]
    async fn mcp_http_stale_binding_and_rebind_last_write_wins() {
        let ctx = TestContext::new().await;
        let app = test_router(ctx.state.clone());

        let project_a_path = ctx._temp_dir.path().join("task9-http-stale-a");
        let project_b_path = ctx._temp_dir.path().join("task9-http-stale-b");
        std::fs::create_dir_all(&project_a_path).expect("project A path should exist");
        std::fs::create_dir_all(&project_b_path).expect("project B path should exist");
        std::fs::write(
            project_a_path.join("lib.rs"),
            "fn task9_stale_shared_probe() { println!(\"task9-stale-alpha-only\"); }\n",
        )
        .expect("project A source should write");
        std::fs::write(
            project_b_path.join("lib.rs"),
            "fn task9_stale_shared_probe() { println!(\"task9-stale-beta-only\"); }\n",
        )
        .expect("project B source should write");

        let project_a = index_project_and_wait(&ctx, &project_a_path).await;
        let project_b = index_project_and_wait(&ctx, &project_b_path).await;

        let session_id = initialize_http_session(&app, 10).await;

        for (request_id, action, project_id) in [
            (11_u64, "bind", Some(project_a.clone())),
            (12_u64, "bind", Some(project_b.clone())),
            (13_u64, "unbind", None),
            (14_u64, "bind", Some(project_a.clone())),
        ] {
            let payload = json!({
                "jsonrpc": "2.0",
                "id": request_id,
                "method": "tools/call",
                "params": {
                    "name": "project_info",
                    "arguments": {
                        "action": action,
                        "project_id": project_id
                    }
                }
            });
            let response = post_mcp_request(&app, &payload, Some(&session_id)).await;
            assert!(response.status().is_success(), "{action} should succeed");
        }

        let status_payload = json!({
            "jsonrpc": "2.0",
            "id": 15,
            "method": "tools/call",
            "params": {
                "name": "project_info",
                "arguments": {
                    "action": "binding_status"
                }
            }
        });
        let status_response = post_mcp_request(&app, &status_payload, Some(&session_id)).await;
        assert!(status_response.status().is_success());
        let status_json = parse_tool_result_json(&response_json(status_response).await);
        assert_eq!(status_json["binding"]["project_id"], project_a);

        let delete_response = crate::server::logic::code::delete_project(
            &ctx.state,
            DeleteProjectParams {
                project_id: project_a.clone(),
            },
        )
        .await
        .expect("delete project should succeed")
        .into_typed::<Value>()
        .expect("delete response should be json");
        assert_eq!(delete_response["project_id"], project_a);

        let stale_query_response = post_mcp_request(
            &app,
            &recall_code_tool_call_payload(16, "task9_stale_shared_probe"),
            Some(&session_id),
        )
        .await;
        assert!(stale_query_response.status().is_success());
        let stale_json = parse_tool_result_json(&response_json(stale_query_response).await);
        assert_eq!(
            stale_json["project_resolution"]["source"],
            "session_binding"
        );
        assert_eq!(
            stale_json["project_resolution"]["project_id"], project_a,
            "stale response should preserve original bound project id"
        );
        assert_eq!(stale_json["project_resolution"]["reason_code"], "stale");
        assert_eq!(
            stale_json["project_resolution"]["binding_state"],
            "stale_binding"
        );
        assert_eq!(stale_json["summary"]["partial"]["reason_code"], "stale");
        assert_eq!(stale_json["reason_code"], "stale");
        assert_eq!(stale_json["count"], 0);
        assert!(stale_json["results"]
            .as_array()
            .expect("results should be array")
            .is_empty());
        assert!(
            !stale_json.to_string().contains("task9-stale-beta-only"),
            "stale binding should not silently broaden to other projects"
        );
    }
}
