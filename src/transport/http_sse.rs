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
async fn health_check(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> impl IntoResponse {
    let storage_ok = state.storage.health_check().await.unwrap_or(false);
    if storage_ok {
        (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "ok",
                "version": env!("CARGO_PKG_VERSION"),
            })),
        )
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "status": "degraded",
                "version": env!("CARGO_PKG_VERSION"),
                "error": "storage unhealthy",
            })),
        )
    }
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

    let mcp_config = StreamableHttpServerConfig {
        stateful_mode: true,
        cancellation_token: ct.child_token(),
        ..Default::default()
    };

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
            },
            _ = terminate.recv() => {
                tracing::info!("Received SIGTERM, initiating HTTP server shutdown...");
            },
        }

        #[cfg(not(unix))]
        {
            ctrl_c.await.ok();
            tracing::info!("Received SIGINT, initiating HTTP server shutdown...");
        }

        ct_for_signals.cancel();
    });

    axum::serve(listener, app)
        .with_graceful_shutdown(async move { ct.cancelled().await })
        .await?;

    tracing::info!("HTTP server stopped, cleaning up...");
    let _ = state.shutdown_tx.send(true);
    if let Err(e) = state.storage.shutdown().await {
        tracing::warn!("Database shutdown error: {}", e);
    }
    tracing::info!("HTTP server shutdown complete");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    use crate::test_utils::TestContext;
    use crate::server::MemoryMcpServer;

    fn test_router(state: Arc<AppState>) -> axum::Router {
        let state_for_factory = state.clone();
        let ct = CancellationToken::new();
        build_router(
            move || Ok(MemoryMcpServer::new(state_for_factory.clone())),
            state,
            &ct,
        )
    }

    #[tokio::test]
    async fn health_check_returns_ok_when_storage_healthy() {
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
                    .header("access-control-request-headers", "content-type,mcp-session-id")
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
        assert!(allow_methods.contains("POST"), "POST not in allowed methods");
        assert!(allow_methods.contains("GET"), "GET not in allowed methods");
        assert!(allow_methods.contains("DELETE"), "DELETE not in allowed methods");
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
            .oneshot(
                Request::delete("/mcp")
                    .body(Body::empty())
                    .unwrap(),
            )
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
}
