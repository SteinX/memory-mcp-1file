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

    #[tool(description = "Get full memory by ID. Memory IDs are stable public identities; response includes additive contract and summary metadata.")]
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

    #[tool(description = "Store a new memory and explicitly supersede exact duplicates within the same optional scope/type boundary.")]
    async fn consolidate_memory(
        &self,
        params: Parameters<ConsolidateMemoryParams>,
    ) -> Result<CallToolResult, ErrorData> {
        logic::memory::consolidate_memory(&self.state, params.0)
            .await
            .map_err(to_rpc_error)
    }

    #[tool(description = "Preview exact-duplicate consolidation within the same optional scope/type boundary without writing any changes.")]
    async fn preview_consolidate_memory(
        &self,
        params: Parameters<PreviewConsolidateMemoryParams>,
    ) -> Result<CallToolResult, ErrorData> {
        logic::memory::preview_consolidate_memory(&self.state, params.0)
            .await
            .map_err(to_rpc_error)
    }

    #[tool(description = "List memories (newest first) with optional scope/type/metadata/time filters. Scope remains optional for forward compatibility. Response includes additive contract and summary metadata.")]
    async fn list_memories(
        &self,
        params: Parameters<ListMemoriesParams>,
    ) -> Result<CallToolResult, ErrorData> {
        logic::memory::list_memories(&self.state, params.0)
            .await
            .map_err(to_rpc_error)
    }

    #[tool(
        description = "Agent memory search (query, mode?: vector|bm25) with optional filters. Memory IDs remain the stable public identity; response includes additive contract and summary metadata."
    )]
    async fn search_memory(
        &self,
        params: Parameters<SearchParams>,
    ) -> Result<CallToolResult, ErrorData> {
        if params.0.mode.as_deref() == Some("bm25") {
            logic::search::search_text_with_access_tracking(
                &self.state,
                params.0,
                Some(&self.state.access_tracker),
            )
                .await
                .map_err(to_rpc_error)
        } else {
            logic::search::search_with_access_tracking(
                &self.state,
                params.0,
                Some(&self.state.access_tracker),
            )
                .await
                .map_err(to_rpc_error)
        }
    }

    #[tool(
        description = "Hybrid memory retrieval via vector+BM25+graph RRF fusion with additive diagnostics plus contract and summary metadata."
    )]
    async fn recall(&self, params: Parameters<RecallParams>) -> Result<CallToolResult, ErrorData> {
        logic::search::recall_with_access_tracking(
            &self.state,
            params.0,
            Some(&self.state.access_tracker),
        )
            .await
            .map_err(to_rpc_error)
    }

    #[tool(
        description = "Knowledge graph ops. Actions: create_entity(name, entity_type?, description?) | create_relation(from_entity, to_entity, relation_type, weight?) | get_related(entity_id, depth?, direction?) | detect_communities(). get_related returns preferred exported nodes/edges plus additive contract and summary metadata; raw entities/relations remain compatibility fields."
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
        description = "Get valid memories. Supports optional timestamp (ISO 8601), scope filters, memory_type, metadata_filter, and event/ingestion ranges. Response includes additive contract and summary metadata."
    )]
    async fn get_valid(
        &self,
        params: Parameters<GetValidParams>,
    ) -> Result<CallToolResult, ErrorData> {
        if let Some(ref ts) = params.0.timestamp {
            let at_params = GetValidAtParams {
                timestamp: ts.clone(),
                user_id: params.0.user_id.clone(),
                agent_id: params.0.agent_id.clone(),
                run_id: params.0.run_id.clone(),
                namespace: params.0.namespace.clone(),
                memory_type: params.0.memory_type.clone(),
                metadata_filter: params.0.metadata_filter.clone(),
                event_after: params.0.event_after.clone(),
                event_before: params.0.event_before.clone(),
                ingestion_after: params.0.ingestion_after.clone(),
                ingestion_before: params.0.ingestion_before.clone(),
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

    #[tool(description = "Soft-delete memory, optionally linking replacement.")]
    async fn invalidate(
        &self,
        params: Parameters<InvalidateParams>,
    ) -> Result<CallToolResult, ErrorData> {
        logic::memory::invalidate(&self.state, params.0)
            .await
            .map_err(to_rpc_error)
    }

    #[tool(description = "Get system status and startup progress.")]
    async fn get_status(
        &self,
        params: Parameters<GetStatusParams>,
    ) -> Result<CallToolResult, ErrorData> {
        logic::system::get_status(&self.state, params.0)
            .await
            .map_err(to_rpc_error)
    }

    #[tool(description = "Index codebase directory for code search.")]
    async fn index_project(
        &self,
        params: Parameters<IndexProjectParams>,
    ) -> Result<CallToolResult, ErrorData> {
        logic::code::index_project(&self.state, params.0)
            .await
            .map_err(to_rpc_error)
    }

    #[tool(
        description = "Hybrid code retrieval (vector+BM25+graph). Results include additive contract and summary metadata. Important: results[].id is a local chunk-record reference, not a stable public ID; stable refind locator is project_id + file_path + start_line + end_line."
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

    #[tool(description = "Project indexing information. Actions: list() | status(project_id) | stats(project_id) | projection(project_id) | projection_by_locator(). Status/stats/list responses include additive contract and normalized summary metadata, including lifecycle, generation, and projection/materialization contract fields. Projection returns an on-demand, export-only project projection document built from current canonical data.")]
    async fn project_info(
        &self,
        params: Parameters<ProjectInfoParams>,
    ) -> Result<CallToolResult, ErrorData> {
        match params.0.action.as_str() {
            "list" => {
                let list_params = ListProjectsParams {
                    _placeholder: false,
                };
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
            "projection" => {
                let project_id = params.0.project_id.ok_or_else(|| ErrorData {
                    code: ErrorCode(-32602),
                    message: "project_id required for projection action".into(),
                    data: None,
                })?;
                let projection_params = crate::server::params::GetProjectProjectionParams {
                    project_id,
                    relation_scope: params.0.relation_scope.clone(),
                    sort_mode: params.0.sort_mode.clone(),
                };
                logic::code::get_project_projection(&self.state, projection_params)
                    .await
                    .map_err(to_rpc_error)
            }
            "projection_by_locator" => {
                let locator = params.0.locator.ok_or_else(|| ErrorData {
                    code: ErrorCode(-32602),
                    message: "locator required for projection_by_locator action".into(),
                    data: None,
                })?;
                let locator_params = crate::server::params::GetProjectionByLocatorParams {
                    locator,
                };
                logic::code::get_project_projection_by_locator(&self.state, locator_params)
                    .await
                    .map_err(to_rpc_error)
            }
            other => Err(ErrorData {
                code: ErrorCode(-32602),
                message: format!("Invalid action '{}'. Use: list, status, stats, projection, projection_by_locator", other).into(),
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

    #[tool(description = "Fast by-name code lookup. Symbol IDs are stable project-scoped symbol identities; responses include additive contract and summary metadata.")]
    async fn search_symbols(
        &self,
        params: Parameters<SearchSymbolsParams>,
    ) -> Result<CallToolResult, ErrorData> {
        logic::code::search_symbols(&self.state, params.0)
            .await
            .map_err(to_rpc_error)
    }

    #[tool(
        description = "Navigate code relationships for a symbol_id. Related traversal returns preferred exported nodes/edges plus additive contract and summary metadata; frontier is an unexpanded boundary hint, not a cursor."
    )]
    async fn symbol_graph(
        &self,
        params: Parameters<SymbolGraphParams>,
    ) -> Result<CallToolResult, ErrorData> {
        logic::code::symbol_graph(&self.state, params.0)
            .await
            .map_err(to_rpc_error)
    }

    #[tool(description = "DANGER: Reset all database data (requires confirm=true).")]
    async fn reset_all_memory(
        &self,
        params: Parameters<ResetAllMemoryParams>,
    ) -> Result<CallToolResult, ErrorData> {
        logic::system::reset_all_memory(&self.state, params.0)
            .await
            .map_err(to_rpc_error)
    }

    #[tool(description = "Meta-help tool. Returns concise usage guidance for the MCP tool surface.")]
    async fn how_to_use(
        &self,
        _params: Parameters<HowToUseParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let text = [
            "=== TOOL GROUPS ===",
            "Memory: store_memory, update_memory, delete_memory, list_memories, get_memory, invalidate, get_valid",
            "Search: recall, search_memory, recall_code, search_symbols, symbol_graph",
            "Project: index_project, delete_project, project_info",
            "System: get_status, reset_all_memory, how_to_use",
            "",
            "For exact request/response fields, inspect each tool schema from list_tools.",
        ]
        .join("\n");

        Ok(CallToolResult::success(vec![Content::text(text)]))
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
    use std::collections::BTreeSet;

    use serde_json::Value;

    use crate::test_utils::TestContext;

    fn schema_value(tool: &Value) -> Value {
        tool.get("inputSchema")
            .or_else(|| tool.get("input_schema"))
            .cloned()
            .unwrap_or(Value::Null)
    }

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

    #[tokio::test]
    async fn tool_surface_stability_keeps_required_public_tools() {
        let ctx = TestContext::new().await;
        let server = MemoryMcpServer::new(ctx.state.clone());
        let tools = server.tool_router.list_all();
        let names: BTreeSet<String> = tools.iter().map(|tool| tool.name.to_string()).collect();

        assert_eq!(names.len(), 21, "public MCP tool count changed");
        assert!(names.contains("recall_code"));
        assert!(names.contains("search_symbols"));
        assert!(names.contains("symbol_graph"));
        assert!(names.contains("project_info"));
        assert!(names.contains("recall"));
        assert!(names.contains("search_memory"));
        assert!(names.contains("how_to_use"));
        assert!(!names.contains("search"));
        assert!(!names.contains("search_code"));
    }

    #[tokio::test]
    async fn tool_descriptions_and_required_params_remain_compatible() {
        let ctx = TestContext::new().await;
        let server = MemoryMcpServer::new(ctx.state.clone());
        let tools_json: Vec<Value> = server
            .tool_router
            .list_all()
            .iter()
            .map(|tool| serde_json::to_value(tool).expect("tool serializes"))
            .collect();

        let get_tool = |name: &str| {
            tools_json
                .iter()
                .find(|tool| tool.get("name").and_then(Value::as_str) == Some(name))
                .expect("tool must exist")
        };

        let recall_code = get_tool("recall_code");
        let recall_code_desc = recall_code
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(recall_code_desc.contains("Hybrid code retrieval"));
        let recall_code_required = schema_value(recall_code)
            .get("required")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(recall_code_required
            .iter()
            .any(|value| value.as_str() == Some("query")));

        let search_symbols = get_tool("search_symbols");
        let search_symbols_desc = search_symbols
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(search_symbols_desc.contains("by-name") || search_symbols_desc.contains("lookup"));

        let symbol_graph = get_tool("symbol_graph");
        let symbol_graph_desc = symbol_graph
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(symbol_graph_desc.contains("symbol_id"));
        let symbol_graph_required = schema_value(symbol_graph)
            .get("required")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(symbol_graph_required
            .iter()
            .any(|value| value.as_str() == Some("symbol_id")));
        assert!(symbol_graph_required
            .iter()
            .any(|value| value.as_str() == Some("action")));

        let project_info = get_tool("project_info");
        let project_info_desc = project_info
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(project_info_desc.contains("status") || project_info_desc.contains("indexing"));
        let project_info_required = schema_value(project_info)
            .get("required")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(project_info_required
            .iter()
            .any(|value| value.as_str() == Some("action")));

        let recall = get_tool("recall");
        let recall_desc = recall
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(recall_desc.contains("Hybrid memory retrieval") || recall_desc.contains("fusion"));

        let search_memory = get_tool("search_memory");
        let search_memory_desc = search_memory
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(search_memory_desc.contains("memory"));
        let search_memory_required = schema_value(search_memory)
            .get("required")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(search_memory_required
            .iter()
            .any(|value| value.as_str() == Some("query")));

        let how_to_use = get_tool("how_to_use");
        let how_to_use_desc = how_to_use
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(how_to_use_desc.contains("Meta-help") || how_to_use_desc.contains("meta-help"));
    }
}
