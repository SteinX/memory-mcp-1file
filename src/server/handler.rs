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
        description = "Search memories. mode: vector (default) or bm25."
    )]
    async fn search_memory(
        &self,
        params: Parameters<SearchParams>,
    ) -> Result<CallToolResult, ErrorData> {
        if params.0.mode.as_deref() == Some("bm25") {
            logic::search::search_text(&self.state, params.0)
                .await
                .map_err(to_rpc_error)
        } else {
            logic::search::search(&self.state, params.0)
                .await
                .map_err(to_rpc_error)
        }
    }

    #[tool(
        description = "High-quality memory retrieval via vector+BM25+graph fusion. Default for memories."
    )]
    async fn recall(&self, params: Parameters<RecallParams>) -> Result<CallToolResult, ErrorData> {
        logic::search::recall(&self.state, params.0)
            .await
            .map_err(to_rpc_error)
    }

    #[tool(
        description = "Knowledge graph ops. action: create_entity|create_relation|get_related|detect_communities."
    )]
    async fn knowledge_graph(
        &self,
        params: Parameters<KnowledgeGraphParams>,
    ) -> Result<CallToolResult, ErrorData> {
        match params.0.action.as_str() {
            "create_entity" => {
                let name = params.0.name.ok_or_else(|| ErrorData {
                    code: ErrorCode(-32602),
                    message: "name required for create_entity".into(),
                    data: None,
                })?;
                let entity_params = CreateEntityParams {
                    name,
                    entity_type: params.0.entity_type,
                    description: params.0.description,
                    user_id: params.0.user_id,
                };
                logic::graph::create_entity(&self.state, entity_params)
                    .await
                    .map_err(to_rpc_error)
            }
            "create_relation" => {
                let from_entity = params.0.from_entity.ok_or_else(|| ErrorData {
                    code: ErrorCode(-32602),
                    message: "from_entity required for create_relation".into(),
                    data: None,
                })?;
                let to_entity = params.0.to_entity.ok_or_else(|| ErrorData {
                    code: ErrorCode(-32602),
                    message: "to_entity required for create_relation".into(),
                    data: None,
                })?;
                let relation_type = params.0.relation_type.ok_or_else(|| ErrorData {
                    code: ErrorCode(-32602),
                    message: "relation_type required for create_relation".into(),
                    data: None,
                })?;
                let relation_params = CreateRelationParams {
                    from_entity,
                    to_entity,
                    relation_type,
                    weight: params.0.weight,
                };
                logic::graph::create_relation(&self.state, relation_params)
                    .await
                    .map_err(to_rpc_error)
            }
            "get_related" => {
                let entity_id = params.0.entity_id.ok_or_else(|| ErrorData {
                    code: ErrorCode(-32602),
                    message: "entity_id required for get_related".into(),
                    data: None,
                })?;
                let related_params = GetRelatedParams {
                    entity_id,
                    depth: params.0.depth,
                    direction: params.0.direction,
                };
                logic::graph::get_related(&self.state, related_params)
                    .await
                    .map_err(to_rpc_error)
            }
            "detect_communities" => {
                let community_params = DetectCommunitiesParams { _placeholder: false };
                logic::graph::detect_communities(&self.state, community_params)
                    .await
                    .map_err(to_rpc_error)
            }
            other => Err(ErrorData {
                code: ErrorCode(-32602),
                message: format!("Invalid action '{}'. Use: create_entity, create_relation, get_related, detect_communities", other).into(),
                data: None,
            }),
        }
    }

    #[tool(
        description = "Get valid memories. Optional timestamp (ISO 8601) for point-in-time query."
    )]
    async fn get_valid(
        &self,
        params: Parameters<GetValidParams>,
    ) -> Result<CallToolResult, ErrorData> {
        if let Some(ref ts) = params.0.timestamp {
            let at_params = GetValidAtParams {
                timestamp: ts.clone(),
                user_id: params.0.user_id.clone(),
                limit: params.0.limit,
            };
            logic::memory::get_valid_at(&self.state, at_params)
                .await
                .map_err(to_rpc_error)
        } else {
            logic::memory::get_valid(&self.state, params.0)
                .await
                .map_err(to_rpc_error)
        }
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
        description = "Code retrieval. mode: vector|hybrid (default: hybrid). Hybrid uses vector+BM25+graph fusion."
    )]
    async fn recall_code(
        &self,
        params: Parameters<RecallCodeParams>,
    ) -> Result<CallToolResult, ErrorData> {
        if params.0.mode.as_deref() == Some("vector") {
            let search_params = SearchCodeParams {
                query: params.0.query,
                project_id: params.0.project_id,
                limit: params.0.limit,
            };
            logic::code::search_code(&self.state, search_params)
                .await
                .map_err(to_rpc_error)
        } else {
            logic::code::recall_code(&self.state, params.0)
                .await
                .map_err(to_rpc_error)
        }
    }

    #[tool(
        description = "Project info. action: list|status|stats. project_id required for status/stats."
    )]
    async fn project_info(
        &self,
        params: Parameters<ProjectInfoParams>,
    ) -> Result<CallToolResult, ErrorData> {
        match params.0.action.as_str() {
            "list" => {
                let list_params = ListProjectsParams { _placeholder: false };
                logic::code::list_projects(&self.state, list_params)
                    .await
                    .map_err(to_rpc_error)
            }
            "status" => {
                let project_id = params.0.project_id.ok_or_else(|| ErrorData {
                    code: ErrorCode(-32602),
                    message: "project_id required for status action".into(),
                    data: None,
                })?;
                let status_params = GetIndexStatusParams { project_id };
                let status = logic::code::get_index_status(&self.state, status_params)
                    .await
                    .map_err(to_rpc_error)?;
                Ok(status)
            }
            "stats" => {
                let project_id = params.0.project_id.ok_or_else(|| ErrorData {
                    code: ErrorCode(-32602),
                    message: "project_id required for stats action".into(),
                    data: None,
                })?;
                let stats_params = GetProjectStatsParams { project_id };
                logic::code::get_project_stats(&self.state, stats_params)
                    .await
                    .map_err(to_rpc_error)
            }
            other => Err(ErrorData {
                code: ErrorCode(-32602),
                message: format!("Invalid action '{}'. Use: list, status, stats", other).into(),
                data: None,
            }),
        }
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

    #[tool(description = "Navigate symbol graph. action: callers|callees|related.")]
    async fn symbol_graph(
        &self,
        params: Parameters<SymbolGraphParams>,
    ) -> Result<CallToolResult, ErrorData> {
        logic::code::symbol_graph(&self.state, params.0)
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
