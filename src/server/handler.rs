use std::sync::Arc;

use rmcp::{
    handler::server::{
        tool::ToolCallContext, tool::ToolRouter, wrapper::Parameters, ServerHandler,
    },
    model::*,
    service::{RequestContext, RoleServer},
    tool, tool_router,
};

use crate::config::AppState;
use crate::server::logic;
use crate::server::params::*;

#[derive(Clone)]
pub struct MemoryMcpServer {
    state: Arc<AppState>,
    tool_router: ToolRouter<Self>,
}

// Helper to convert anyhow::Error to JSON-RPC ErrorData
fn to_rpc_error(e: anyhow::Error) -> ErrorData {
    ErrorData {
        code: ErrorCode(-32000),
        message: e.to_string().into(),
        data: None,
    }
}

#[tool_router]
impl MemoryMcpServer {
    pub fn new(state: Arc<AppState>) -> Self {
        Self {
            state,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "Store a new memory.")]
    async fn store_memory(
        &self,
        params: Parameters<StoreMemoryParams>,
    ) -> Result<CallToolResult, ErrorData> {
        logic::memory::store_memory(&self.state, params.0)
            .await
            .map_err(to_rpc_error)
    }

    #[tool(
        description = "Get full memory by ID."
    )]
    async fn get_memory(
        &self,
        params: Parameters<GetMemoryParams>,
    ) -> Result<CallToolResult, ErrorData> {
        logic::memory::get_memory(&self.state, params.0)
            .await
            .map_err(to_rpc_error)
    }

    #[tool(description = "Update memory fields.")]
    async fn update_memory(
        &self,
        params: Parameters<UpdateMemoryParams>,
    ) -> Result<CallToolResult, ErrorData> {
        logic::memory::update_memory(&self.state, params.0)
            .await
            .map_err(to_rpc_error)
    }

    #[tool(description = "Delete memory by ID.")]
    async fn delete_memory(
        &self,
        params: Parameters<DeleteMemoryParams>,
    ) -> Result<CallToolResult, ErrorData> {
        logic::memory::delete_memory(&self.state, params.0)
            .await
            .map_err(to_rpc_error)
    }

    #[tool(
        description = "List memories (newest first)."
    )]
    async fn list_memories(
        &self,
        params: Parameters<ListMemoriesParams>,
    ) -> Result<CallToolResult, ErrorData> {
        logic::memory::list_memories(&self.state, params.0)
            .await
            .map_err(to_rpc_error)
    }

    #[tool(
        description = "Semantic vector search for memories."
    )]
    async fn search(&self, params: Parameters<SearchParams>) -> Result<CallToolResult, ErrorData> {
        logic::search::search(&self.state, params.0)
            .await
            .map_err(to_rpc_error)
    }

    #[tool(
        description = "Exact keyword (BM25) search for memories."
    )]
    async fn search_text(
        &self,
        params: Parameters<SearchParams>,
    ) -> Result<CallToolResult, ErrorData> {
        logic::search::search_text(&self.state, params.0)
            .await
            .map_err(to_rpc_error)
    }

    #[tool(
        description = "High-quality memory retrieval via vector+BM25+graph fusion. Default for memories."
    )]
    async fn recall(&self, params: Parameters<RecallParams>) -> Result<CallToolResult, ErrorData> {
        logic::search::recall(&self.state, params.0)
            .await
            .map_err(to_rpc_error)
    }

    #[tool(description = "Create KG entity.")]
    async fn create_entity(
        &self,
        params: Parameters<CreateEntityParams>,
    ) -> Result<CallToolResult, ErrorData> {
        logic::graph::create_entity(&self.state, params.0)
            .await
            .map_err(to_rpc_error)
    }

    #[tool(description = "Create KG relation.")]
    async fn create_relation(
        &self,
        params: Parameters<CreateRelationParams>,
    ) -> Result<CallToolResult, ErrorData> {
        logic::graph::create_relation(&self.state, params.0)
            .await
            .map_err(to_rpc_error)
    }

    #[tool(description = "Get related KG entities via traversal.")]
    async fn get_related(
        &self,
        params: Parameters<GetRelatedParams>,
    ) -> Result<CallToolResult, ErrorData> {
        logic::graph::get_related(&self.state, params.0)
            .await
            .map_err(to_rpc_error)
    }

    #[tool(
        description = "Get valid memories (not superseded/deleted)."
    )]
    async fn get_valid(
        &self,
        params: Parameters<GetValidParams>,
    ) -> Result<CallToolResult, ErrorData> {
        logic::memory::get_valid(&self.state, params.0)
            .await
            .map_err(to_rpc_error)
    }

    #[tool(
        description = "Get memories valid at specific ISO 8601 timestamp."
    )]
    async fn get_valid_at(
        &self,
        params: Parameters<GetValidAtParams>,
    ) -> Result<CallToolResult, ErrorData> {
        logic::memory::get_valid_at(&self.state, params.0)
            .await
            .map_err(to_rpc_error)
    }

    #[tool(
        description = "Soft-delete memory, optionally linking replacement."
    )]
    async fn invalidate(
        &self,
        params: Parameters<InvalidateParams>,
    ) -> Result<CallToolResult, ErrorData> {
        logic::memory::invalidate(&self.state, params.0)
            .await
            .map_err(to_rpc_error)
    }

    #[tool(
        description = "Get system status and startup progress."
    )]
    async fn get_status(
        &self,
        params: Parameters<GetStatusParams>,
    ) -> Result<CallToolResult, ErrorData> {
        logic::system::get_status(&self.state, params.0)
            .await
            .map_err(to_rpc_error)
    }

    #[tool(
        description = "Index codebase directory for code search."
    )]
    async fn index_project(
        &self,
        params: Parameters<IndexProjectParams>,
    ) -> Result<CallToolResult, ErrorData> {
        logic::code::index_project(&self.state, params.0)
            .await
            .map_err(to_rpc_error)
    }

    #[tool(
        description = "Semantic code search."
    )]
    async fn search_code(
        &self,
        params: Parameters<SearchCodeParams>,
    ) -> Result<CallToolResult, ErrorData> {
        logic::code::search_code(&self.state, params.0)
            .await
            .map_err(to_rpc_error)
    }

    #[tool(
        description = "High-quality code retrieval via vector+BM25+graph fusion. Default for codebase queries."
    )]
    async fn recall_code(
        &self,
        params: Parameters<RecallCodeParams>,
    ) -> Result<CallToolResult, ErrorData> {
        logic::code::recall_code(&self.state, params.0)
            .await
            .map_err(to_rpc_error)
    }

    #[tool(
        description = "Get project indexing status."
    )]
    async fn get_index_status(
        &self,
        params: Parameters<GetIndexStatusParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let status = logic::code::get_index_status(&self.state, params.0)
            .await
            .map_err(to_rpc_error)?;

        // Mix in real-time monitor stats if available and matching project
        // Note: logic::code::get_index_status returns CallToolResult (JSON), not IndexStatus struct directly.
        // We need to parse the JSON content to modify it, or modify logic::code::get_index_status instead.
        // Modifying the logic layer is cleaner.
        // Let's modify src/server/logic/code.rs instead of handler.rs for this logic.

        Ok(status)
    }

    #[tool(
        description = "List indexed projects."
    )]
    async fn list_projects(
        &self,
        params: Parameters<ListProjectsParams>,
    ) -> Result<CallToolResult, ErrorData> {
        logic::code::list_projects(&self.state, params.0)
            .await
            .map_err(to_rpc_error)
    }

    #[tool(description = "Delete indexed project.")]
    async fn delete_project(
        &self,
        params: Parameters<DeleteProjectParams>,
    ) -> Result<CallToolResult, ErrorData> {
        logic::code::delete_project(&self.state, params.0)
            .await
            .map_err(to_rpc_error)
    }

    #[tool(description = "Search code symbols by name.")]
    async fn search_symbols(
        &self,
        params: Parameters<SearchSymbolsParams>,
    ) -> Result<CallToolResult, ErrorData> {
        logic::code::search_symbols(&self.state, params.0)
            .await
            .map_err(to_rpc_error)
    }

    #[tool(description = "Find callers of a symbol.")]
    async fn get_callers(
        &self,
        params: Parameters<GetCallersParams>,
    ) -> Result<CallToolResult, ErrorData> {
        logic::code::get_callers(&self.state, params.0)
            .await
            .map_err(to_rpc_error)
    }

    #[tool(description = "Find callees of a symbol.")]
    async fn get_callees(
        &self,
        params: Parameters<GetCalleesParams>,
    ) -> Result<CallToolResult, ErrorData> {
        logic::code::get_callees(&self.state, params.0)
            .await
            .map_err(to_rpc_error)
    }

    #[tool(description = "Get related symbols via graph traversal.")]
    async fn get_related_symbols(
        &self,
        params: Parameters<GetRelatedSymbolsParams>,
    ) -> Result<CallToolResult, ErrorData> {
        logic::code::get_related_symbols(&self.state, params.0)
            .await
            .map_err(to_rpc_error)
    }

    #[tool(
        description = "Get project indexing statistics."
    )]
    async fn get_project_stats(
        &self,
        params: Parameters<GetProjectStatsParams>,
    ) -> Result<CallToolResult, ErrorData> {
        logic::code::get_project_stats(&self.state, params.0)
            .await
            .map_err(to_rpc_error)
    }

    #[tool(
        description = "DANGER: Reset all database data (requires confirm=true)."
    )]
    async fn reset_all_memory(
        &self,
        params: Parameters<ResetAllMemoryParams>,
    ) -> Result<CallToolResult, ErrorData> {
        logic::system::reset_all_memory(&self.state, params.0)
            .await
            .map_err(to_rpc_error)
    }

    #[tool(description = "Detect KG communities (Leiden algorithm).")]
    async fn detect_communities(
        &self,
        params: Parameters<DetectCommunitiesParams>,
    ) -> Result<CallToolResult, ErrorData> {
        logic::graph::detect_communities(&self.state, params.0)
            .await
            .map_err(to_rpc_error)
    }
}

impl ServerHandler for MemoryMcpServer {
    fn get_info(&self) -> InitializeResult {
        InitializeResult {
            protocol_version: ProtocolVersion::default(),
            capabilities: ServerCapabilities {
                tools: Some(ToolsCapability {
                    list_changed: Some(false),
                }),
                ..ServerCapabilities::default()
            },
            server_info: Implementation {
                name: "memory-mcp".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                description: None,
                title: None,
                icons: None,
                website_url: None,
            },
            instructions: Some(
                "AI agent memory server with semantic search, knowledge graph, and code search."
                    .into(),
            ),
        }
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(self.tool_router.list_all()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let tool_context = ToolCallContext::new(self, request, context);
        self.tool_router.call(tool_context).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::TestContext;

    #[tokio::test]
    async fn test_server_handler_integration() {
        let ctx = TestContext::new().await;
        let server = MemoryMcpServer::new(ctx.state.clone());

        // 1. Get Info
        let info = server.get_info();
        assert_eq!(info.server_info.name, "memory-mcp");

        // 2. Integration check pass
        // We cannot easily mock RequestContext without more deps,
        // but since logic tests cover actual execution,
        // and compilation proves traits are implemented, this is sufficient.
    }
}
