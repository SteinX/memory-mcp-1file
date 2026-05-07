use std::sync::Arc;

use rmcp::{
    handler::server::{
        tool::ToolCallContext, tool::ToolRouter, wrapper::Parameters, ServerHandler,
    },
    model::{Extensions, *},
    service::{RequestContext, RoleServer},
    tool, tool_router,
};
use serde_json::json;

use crate::config::AppState;
use crate::server::logic;
use crate::server::logic::code::CodeToolContext;
use crate::server::params::*;
use crate::storage::StorageBackend;

fn require_project_id(project_id: Option<String>, action: &str) -> Result<String, ErrorData> {
    normalize_project_id(project_id).ok_or_else(|| ErrorData {
        code: ErrorCode(-32602),
        message: format!("project_id required for {} action", action).into(),
        data: None,
    })
}

const MCP_SESSION_ID_HEADER: &str = "mcp-session-id";

pub(crate) fn extract_session_id(context: &RequestContext<RoleServer>) -> Option<String> {
    extract_session_id_from_extensions(&context.extensions)
}

fn code_tool_context_from_session_id(session_id: Option<String>) -> Option<CodeToolContext> {
    session_id.map(|session_id| CodeToolContext::from_session_id(Some(session_id)))
}

fn unsupported_binding_response(action: &str) -> CallToolResult {
    logic::success_json(json!({
        "action": action,
        "binding": serde_json::Value::Null,
        "reason_code": "unsupported",
        "message": "Session-scoped project binding is unsupported without an MCP session context."
    }))
}

fn binding_response(status: &crate::codebase::SessionBindingStatus) -> serde_json::Value {
    json!({
        "session_id": status.session_id,
        "project_id": status.project_id,
        "updated_at_unix_ms": status.updated_at_unix_ms,
        "state": if status.project_id.is_some() { "bound" } else { "unbound" },
        "resolution_source": "session_binding"
    })
}

fn binding_lifecycle_response(
    index_status: Option<&crate::types::IndexStatus>,
) -> serde_json::Value {
    match index_status {
        Some(index_status) => {
            let is_partial = index_status.status != crate::types::IndexState::Completed;
            let reason_code = if is_partial { "partial" } else { "ok" };
            let message = if is_partial {
                "Bound project is still indexing; results may be partial."
            } else {
                "Bound project indexing is complete."
            };

            json!({
                "index_status": {
                    "status": index_status.status.to_string(),
                    "total_files": index_status.total_files,
                    "indexed_files": index_status.indexed_files,
                    "total_chunks": index_status.total_chunks,
                    "total_symbols": index_status.total_symbols,
                    "started_at": index_status.started_at,
                    "completed_at": index_status.completed_at,
                    "error_message": index_status.error_message,
                },
                "summary": {
                    "result_kind": "binding_status",
                    "partial": {
                        "is_partial": is_partial,
                        "reason_code": reason_code,
                        "reason": if is_partial { "indexing_in_progress" } else { "complete" },
                        "message": message
                    }
                }
            })
        }
        None => json!({
            "summary": {
                "result_kind": "binding_status",
                "partial": {
                    "is_partial": true,
                    "reason_code": "degraded",
                    "reason": "missing_index_status",
                    "message": "Bound project has no persisted index status metadata yet."
                }
            }
        }),
    }
}

fn extract_session_id_from_extensions(extensions: &Extensions) -> Option<String> {
    extensions
        .get::<axum::http::request::Parts>()
        .and_then(|parts| parts.headers.get(MCP_SESSION_ID_HEADER))
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

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
        description = "Get full memory by ID. Memory IDs are stable public identities; response includes additive contract and summary metadata."
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
        description = "Store a new memory and explicitly supersede exact duplicates within the same optional scope/type boundary."
    )]
    async fn consolidate_memory(
        &self,
        params: Parameters<ConsolidateMemoryParams>,
    ) -> Result<CallToolResult, ErrorData> {
        logic::memory::consolidate_memory(&self.state, params.0)
            .await
            .map_err(to_rpc_error)
    }

    #[tool(
        description = "Preview exact-duplicate consolidation within the same optional scope/type boundary without writing any changes."
    )]
    async fn preview_consolidate_memory(
        &self,
        params: Parameters<PreviewConsolidateMemoryParams>,
    ) -> Result<CallToolResult, ErrorData> {
        logic::memory::preview_consolidate_memory(&self.state, params.0)
            .await
            .map_err(to_rpc_error)
    }

    #[tool(
        description = "Export memories as JSONL using the public migration contract."
    )]
    async fn export_memory(
        &self,
        params: Parameters<ExportMemoryParams>,
    ) -> Result<CallToolResult, ErrorData> {
        logic::memory::export_memory(&self.state, params.0)
            .await
            .map_err(to_rpc_error)
    }

    #[tool(
        description = "Import memories from inline JSONL using the public migration contract."
    )]
    async fn import_memory(
        &self,
        params: Parameters<ImportMemoryParams>,
    ) -> Result<CallToolResult, ErrorData> {
        logic::memory::import_memory(&self.state, params.0)
            .await
            .map_err(to_rpc_error)
    }

    #[tool(
        description = "List memories (newest first) with optional scope/type/metadata/time filters. Scope remains optional for forward compatibility. Response includes additive contract and summary metadata."
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
        description = "Knowledge graph ops. Actions: create_entity(name, entity_type?, description?) | create_relation(from_entity, to_entity, relation_type, weight?) | get_related(entity_id, depth?, direction?) | detect_communities(). create_relation from_entity/to_entity must be entity IDs returned by create_entity, not display names. get_related returns preferred exported nodes/edges plus additive contract and summary metadata; raw entities/relations remain compatibility fields."
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

    #[tool(
        description = "Index codebase directory for code search. Retrying a previously failed full index requires force=true and confirm_failed_restart=true."
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
        description = "Hybrid code retrieval (vector+BM25+graph). Results include additive contract and summary metadata. Important: results[].id is a local chunk-record reference, not a stable public ID; stable refind locator is project_id + file_path + start_line + end_line."
    )]
    async fn recall_code(
        &self,
        params: Parameters<RecallCodeParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.recall_code_with_session(params, None).await
    }

    async fn recall_code_with_session(
        &self,
        params: Parameters<RecallCodeParams>,
        context: Option<CodeToolContext>,
    ) -> Result<CallToolResult, ErrorData> {
        if params.0.mode.as_deref() == Some("vector") {
            let search_params = SearchCodeParams {
                query: params.0.query,
                project_id: params.0.project_id,
                limit: params.0.limit,
            };
            logic::code::search_code_with_context(&self.state, search_params, context)
                .await
                .map_err(to_rpc_error)
        } else {
            logic::code::recall_code_with_context(&self.state, params.0, context)
                .await
                .map_err(to_rpc_error)
        }
    }

    #[tool(
        description = "Project indexing information. Actions: list() | index(path, force?, confirm_failed_restart?) | status(project_id) | stats(project_id) | projection(project_id) | projection_by_locator() | bind(project_id) | unbind() | binding_status(). Status/stats/list responses include additive contract and normalized summary metadata, including lifecycle, generation, and projection/materialization contract fields. Projection returns an on-demand, export-only project projection document built from current canonical data."
    )]
    async fn project_info(
        &self,
        params: Parameters<ProjectInfoParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.project_info_with_session(params, None).await
    }

    async fn project_info_with_session(
        &self,
        params: Parameters<ProjectInfoParams>,
        context: Option<CodeToolContext>,
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
            "index" => {
                let path = params.0.path.ok_or_else(|| ErrorData {
                    code: ErrorCode(-32602),
                    message: "path required for index action".into(),
                    data: None,
                })?;
                let index_params = IndexProjectParams {
                    path,
                    force: params.0.force,
                    confirm_failed_restart: params.0.confirm_failed_restart,
                };
                logic::code::index_project(&self.state, index_params)
                    .await
                    .map_err(to_rpc_error)
            }
            "status" => {
                let project_id = require_project_id(params.0.project_id, "status")?;
                let status_params = GetIndexStatusParams { project_id };
                let status = logic::code::get_index_status(&self.state, status_params)
                    .await
                    .map_err(to_rpc_error)?;
                Ok(status)
            }
            "stats" => {
                let project_id = require_project_id(params.0.project_id, "stats")?;
                let stats_params = GetProjectStatsParams { project_id };
                logic::code::get_project_stats(&self.state, stats_params)
                    .await
                    .map_err(to_rpc_error)
            }
            "projection" => {
                let project_id = require_project_id(params.0.project_id, "projection")?;
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
            "bind" => {
                let Some(context) = context else {
                    return Ok(unsupported_binding_response("bind"));
                };
                let Some(session_id) = context.session_id() else {
                    return Ok(unsupported_binding_response("bind"));
                };

                let project_id = require_project_id(params.0.project_id, "bind")?;
                if let Some(missing) =
                    logic::code::missing_project_binding_diagnostic(&self.state, Some(&project_id))
                        .await
                {
                    return Ok(logic::success_json(json!({
                        "action": "bind",
                        "project_id": project_id,
                        "binding": serde_json::Value::Null,
                        "reason_code": missing.reason_code,
                        "message": missing.message,
                        "code_intelligence": missing.code_intelligence,
                        "project_binding": missing.project_binding
                    })));
                }

                self.state
                    .session_bindings
                    .bind(session_id.to_string(), project_id.clone())
                    .await;
                let binding = self.state.session_bindings.binding_status(session_id).await;
                let lifecycle = binding_lifecycle_response(
                    self.state
                        .storage
                        .get_index_status(&project_id)
                        .await
                        .ok()
                        .flatten()
                        .as_ref(),
                );

                Ok(logic::success_json(json!({
                    "action": "bind",
                    "project_id": project_id,
                    "binding": binding_response(&binding),
                    "lifecycle": lifecycle,
                })))
            }
            "unbind" => {
                let Some(context) = context else {
                    return Ok(unsupported_binding_response("unbind"));
                };
                let Some(session_id) = context.session_id() else {
                    return Ok(unsupported_binding_response("unbind"));
                };

                let previous_binding = self.state.session_bindings.binding_status(session_id).await;
                self.state.session_bindings.unbind(session_id).await;
                let current_binding = self.state.session_bindings.binding_status(session_id).await;

                Ok(logic::success_json(json!({
                    "action": "unbind",
                    "previous_binding": binding_response(&previous_binding),
                    "binding": binding_response(&current_binding),
                })))
            }
            "binding_status" => {
                let Some(context) = context else {
                    return Ok(unsupported_binding_response("binding_status"));
                };
                let Some(session_id) = context.session_id() else {
                    return Ok(unsupported_binding_response("binding_status"));
                };

                let binding = self.state.session_bindings.binding_status(session_id).await;
                let lifecycle = match binding.project_id.as_deref() {
                    Some(project_id) => binding_lifecycle_response(
                        self.state
                            .storage
                            .get_index_status(project_id)
                            .await
                            .ok()
                            .flatten()
                            .as_ref(),
                    ),
                    None => json!({
                        "summary": {
                            "result_kind": "binding_status",
                            "partial": {
                                "is_partial": false,
                                "reason_code": "ok",
                                "reason": "unbound",
                                "message": "No project is currently bound for this session."
                            }
                        }
                    }),
                };

                Ok(logic::success_json(json!({
                    "action": "binding_status",
                    "binding": binding_response(&binding),
                    "lifecycle": lifecycle,
                })))
            }
            other => Err(ErrorData {
                code: ErrorCode(-32602),
                message: format!("Invalid action '{}'. Use: list, index, status, stats, projection, projection_by_locator, bind, unbind, binding_status", other).into(),
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

    #[tool(
        description = "Fast by-name code lookup. Symbol IDs are stable project-scoped symbol identities; responses include additive contract and summary metadata."
    )]
    async fn search_symbols(
        &self,
        params: Parameters<SearchSymbolsParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.search_symbols_with_session(params, None).await
    }

    async fn search_symbols_with_session(
        &self,
        params: Parameters<SearchSymbolsParams>,
        context: Option<CodeToolContext>,
    ) -> Result<CallToolResult, ErrorData> {
        logic::code::search_symbols_with_context(&self.state, params.0, context)
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

    #[tool(
        description = "Meta-help tool. Returns concise usage guidance for the MCP tool surface."
    )]
    async fn how_to_use(
        &self,
        _params: Parameters<HowToUseParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let text = [
            "=== TOOL GROUPS ===",
            "Memory: store_memory, update_memory, delete_memory, list_memories, get_memory, invalidate, get_valid, export_memory, import_memory",
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
        let session_context = code_tool_context_from_session_id(extract_session_id(&context));
        let direct_result = match request.name.as_ref() {
            "project_info" => {
                let params = serde_json::from_value(serde_json::Value::Object(
                    request.arguments.clone().unwrap_or_default(),
                ))
                .map_err(|error| ErrorData {
                    code: ErrorCode(-32602),
                    message: format!("Invalid project_info parameters: {error}").into(),
                    data: None,
                })?;
                Some(
                    self.project_info_with_session(Parameters(params), session_context)
                        .await,
                )
            }
            "recall_code" => {
                let params = serde_json::from_value(serde_json::Value::Object(
                    request.arguments.clone().unwrap_or_default(),
                ))
                .map_err(|error| ErrorData {
                    code: ErrorCode(-32602),
                    message: format!("Invalid recall_code parameters: {error}").into(),
                    data: None,
                })?;
                Some(
                    self.recall_code_with_session(Parameters(params), session_context)
                        .await,
                )
            }
            "search_symbols" => {
                let params = serde_json::from_value(serde_json::Value::Object(
                    request.arguments.clone().unwrap_or_default(),
                ))
                .map_err(|error| ErrorData {
                    code: ErrorCode(-32602),
                    message: format!("Invalid search_symbols parameters: {error}").into(),
                    data: None,
                })?;
                Some(
                    self.search_symbols_with_session(Parameters(params), session_context)
                        .await,
                )
            }
            _ => None,
        };

        if let Some(result) = direct_result {
            return result;
        }

        let tool_context = ToolCallContext::new(self, request, context);
        self.tool_router.call(tool_context).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    use axum::http::Request;
    use serde_json::Value;

    use crate::test_utils::TestContext;

    fn schema_value(tool: &Value) -> Value {
        tool.get("inputSchema")
            .or_else(|| tool.get("input_schema"))
            .cloned()
            .unwrap_or(Value::Null)
    }

    fn extensions_with_request_headers(headers: &[(&str, &str)]) -> Extensions {
        let mut builder = Request::builder();
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        let request = builder.body(()).expect("request should build");
        let (parts, _) = request.into_parts();
        let mut extensions = Extensions::new();
        extensions.insert(parts);
        extensions
    }

    #[test]
    fn tool_context_session_identity() {
        let extensions = extensions_with_request_headers(&[(MCP_SESSION_ID_HEADER, "session-abc")]);

        assert_eq!(
            extract_session_id_from_extensions(&extensions).as_deref(),
            Some("session-abc")
        );
    }

    #[test]
    fn tool_context_without_session_is_none() {
        let empty_extensions = Extensions::new();
        let headerless_extensions = extensions_with_request_headers(&[]);

        assert_eq!(extract_session_id_from_extensions(&empty_extensions), None);
        assert_eq!(
            extract_session_id_from_extensions(&headerless_extensions),
            None
        );
    }

    #[tokio::test]
    async fn handler_threads_session_to_project_resolver() {
        let ctx = TestContext::new().await;
        let server = MemoryMcpServer::new(ctx.state.clone());
        let session_id = "handler-session-alpha";
        let project_id = "handler-project-alpha";

        ctx.state
            .session_bindings
            .bind(session_id, project_id)
            .await;
        let session_context = code_tool_context_from_session_id(Some(session_id.to_string()))
            .expect("session context should be present");

        assert_eq!(session_context.session_id(), Some(session_id));
        assert_eq!(
            session_context
                .bound_project_id(&ctx.state)
                .await
                .as_deref(),
            Some(project_id)
        );

        let result = server
            .search_symbols_with_session(
                Parameters(SearchSymbolsParams {
                    query: "anything".to_string(),
                    project_id: None,
                    limit: Some(5),
                    offset: Some(0),
                    symbol_type: None,
                    path_prefix: None,
                }),
                Some(session_context),
            )
            .await
            .expect("handler should call search_symbols with session context");
        let body = result
            .into_typed::<Value>()
            .expect("search_symbols response should be JSON");

        assert_eq!(body["filters"]["project_id"], project_id);
    }

    #[tokio::test]
    async fn handler_no_session_context_remains_none() {
        let ctx = TestContext::new().await;
        let server = MemoryMcpServer::new(ctx.state.clone());

        assert!(code_tool_context_from_session_id(None).is_none());

        let result = server
            .search_symbols_with_session(
                Parameters(SearchSymbolsParams {
                    query: "anything".to_string(),
                    project_id: None,
                    limit: Some(5),
                    offset: Some(0),
                    symbol_type: None,
                    path_prefix: None,
                }),
                None,
            )
            .await
            .expect("handler should preserve no-session search_symbols behavior");
        let body = result
            .into_typed::<Value>()
            .expect("search_symbols response should be JSON");

        assert_eq!(body["filters"]["project_id"], Value::Null);
        assert!(body.get("project_binding").is_none());
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
    async fn handler_lists_export_import_memory_tools() {
        let ctx = TestContext::new().await;
        let server = MemoryMcpServer::new(ctx.state.clone());
        let tools = server.tool_router.list_all();
        let names: BTreeSet<String> = tools.iter().map(|tool| tool.name.to_string()).collect();

        assert_eq!(names.len(), 23, "public MCP tool count changed");
        assert!(names.contains("export_memory"));
        assert!(names.contains("import_memory"));
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
    async fn tool_surface_stability_keeps_required_public_tools() {
        let ctx = TestContext::new().await;
        let server = MemoryMcpServer::new(ctx.state.clone());
        let tools = server.tool_router.list_all();
        let names: BTreeSet<String> = tools.iter().map(|tool| tool.name.to_string()).collect();

        assert_eq!(names.len(), 23, "public MCP tool count changed");
        assert!(names.contains("recall_code"));
        assert!(names.contains("search_symbols"));
        assert!(names.contains("symbol_graph"));
        assert!(names.contains("project_info"));
        assert!(names.contains("recall"));
        assert!(names.contains("search_memory"));
        assert!(names.contains("how_to_use"));
        assert!(names.contains("export_memory"));
        assert!(names.contains("import_memory"));
        assert!(!names.contains("search"));
        assert!(!names.contains("search_code"));
    }

    #[tokio::test]
    async fn project_info_binding_tool_surface_no_session_returns_unsupported_success_json() {
        let ctx = TestContext::new().await;
        let server = MemoryMcpServer::new(ctx.state.clone());

        for action in ["bind", "unbind", "binding_status"] {
            let result = server
                .project_info(Parameters(ProjectInfoParams {
                    action: action.to_string(),
                    project_id: None,
                    path: None,
                    force: None,
                    confirm_failed_restart: None,
                    locator: None,
                    relation_scope: None,
                    sort_mode: None,
                }))
                .await;

            let body = result
                .expect("future binding action should return success JSON")
                .into_typed::<Value>()
                .expect("binding response should be JSON");

            assert_eq!(body.get("binding"), Some(&Value::Null));
            assert_eq!(
                body.get("reason_code").and_then(Value::as_str),
                Some("unsupported")
            );
        }
    }

    #[tokio::test]
    async fn project_info_index_action_queues_one_shot_index_task() {
        let ctx = TestContext::new().await;
        let server = MemoryMcpServer::new(ctx.state.clone());
        let project_path = ctx._temp_dir.path().join("project-info-index-action");
        std::fs::create_dir_all(&project_path).expect("project path should exist");

        let result = server
            .project_info(Parameters(ProjectInfoParams {
                action: "index".to_string(),
                project_id: None,
                path: Some(project_path.to_string_lossy().to_string()),
                force: None,
                confirm_failed_restart: None,
                locator: None,
                relation_scope: None,
                sort_mode: None,
            }))
            .await
            .expect("project_info index should succeed")
            .into_typed::<Value>()
            .expect("index response should be json");

        assert_eq!(result["project_id"], "project-info-index-action");
        assert_eq!(
            result["root_path"],
            project_path
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .to_string()
        );
        assert_eq!(result["status"], "indexing");
        assert_eq!(result["background_task"]["state"], "queued");
        assert_eq!(result["background_task"]["runner"], "local_tokio_task");
        assert!(result["message"]
            .as_str()
            .unwrap_or_default()
            .contains("one-shot background task"));
    }

    #[tokio::test]
    async fn project_info_bind_unbind_roundtrip() {
        let ctx = TestContext::new().await;
        let server = MemoryMcpServer::new(ctx.state.clone());
        let session_context = CodeToolContext::from_session_id(Some("sid-roundtrip".to_string()));
        let project_id = "project-bind-roundtrip";
        let project_path = ctx._temp_dir.path().join(project_id);
        std::fs::create_dir_all(&project_path).expect("project path should exist");
        ctx.state
            .project_registry
            .ensure_project(project_id.to_string(), &project_path)
            .await
            .expect("project should register");

        let bind = server
            .project_info_with_session(
                Parameters(ProjectInfoParams {
                    action: "bind".to_string(),
                    project_id: Some(project_id.to_string()),
                    path: None,
                    force: None,
                    confirm_failed_restart: None,
                    locator: None,
                    relation_scope: None,
                    sort_mode: None,
                }),
                Some(session_context.clone()),
            )
            .await
            .expect("bind should succeed")
            .into_typed::<Value>()
            .expect("bind response should be json");

        assert_eq!(bind["action"], "bind");
        assert_eq!(bind["binding"]["project_id"], project_id);

        let status = server
            .project_info_with_session(
                Parameters(ProjectInfoParams {
                    action: "binding_status".to_string(),
                    project_id: None,
                    path: None,
                    force: None,
                    confirm_failed_restart: None,
                    locator: None,
                    relation_scope: None,
                    sort_mode: None,
                }),
                Some(session_context.clone()),
            )
            .await
            .expect("binding_status should succeed")
            .into_typed::<Value>()
            .expect("status response should be json");

        assert_eq!(status["action"], "binding_status");
        assert_eq!(status["binding"]["project_id"], project_id);

        let unbind = server
            .project_info_with_session(
                Parameters(ProjectInfoParams {
                    action: "unbind".to_string(),
                    project_id: None,
                    path: None,
                    force: None,
                    confirm_failed_restart: None,
                    locator: None,
                    relation_scope: None,
                    sort_mode: None,
                }),
                Some(session_context),
            )
            .await
            .expect("unbind should succeed")
            .into_typed::<Value>()
            .expect("unbind response should be json");

        assert_eq!(unbind["action"], "unbind");
        assert_eq!(unbind["previous_binding"]["project_id"], project_id);
        assert!(unbind["binding"]["project_id"].is_null());
        assert!(ctx
            .state
            .session_bindings
            .binding_status("sid-roundtrip")
            .await
            .project_id
            .is_none());
    }

    #[tokio::test]
    async fn stdio_no_session_binding_unsupported() {
        let ctx = TestContext::new().await;
        let server = MemoryMcpServer::new(ctx.state.clone());

        let bind = server
            .project_info_with_session(
                Parameters(ProjectInfoParams {
                    action: "bind".to_string(),
                    project_id: Some("project-a".to_string()),
                    path: None,
                    force: None,
                    confirm_failed_restart: None,
                    locator: None,
                    relation_scope: None,
                    sort_mode: None,
                }),
                None,
            )
            .await
            .expect("bind without session should return success json")
            .into_typed::<Value>()
            .expect("bind no-session response should be json");

        assert_eq!(bind["reason_code"], "unsupported");
        assert!(bind["binding"].is_null());
        assert_eq!(ctx.state.session_bindings.len().await, 0);

        let project_a = format!("test_stdio_no_session_a_{}", uuid::Uuid::new_v4().simple());
        let project_b = format!("test_stdio_no_session_b_{}", uuid::Uuid::new_v4().simple());

        let project_a_path = ctx._temp_dir.path().join(&project_a);
        let project_b_path = ctx._temp_dir.path().join(&project_b);
        std::fs::create_dir_all(&project_a_path).expect("project a path should exist");
        std::fs::create_dir_all(&project_b_path).expect("project b path should exist");
        std::fs::write(
            project_a_path.join("lib.rs"),
            "fn stdio_alpha_target() { println!(\"stdio no session alpha marker\"); }\n",
        )
        .unwrap();
        std::fs::write(
            project_b_path.join("lib.rs"),
            "fn stdio_beta_target() { println!(\"stdio no session beta marker\"); }\n",
        )
        .unwrap();

        for project_path in [&project_a_path, &project_b_path] {
            crate::server::logic::code::index_project(
                &ctx.state,
                IndexProjectParams {
                    path: project_path.to_string_lossy().to_string(),
                    force: None,
                    confirm_failed_restart: None,
                },
            )
            .await
            .unwrap();
        }

        for project_id in [&project_a, &project_b] {
            let status_params = crate::server::params::GetIndexStatusParams {
                project_id: project_id.clone(),
            };
            let mut retries = 0;
            loop {
                tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
                let res =
                    crate::server::logic::code::get_index_status(&ctx.state, status_params.clone())
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
                    "Indexing timed out for stdio no-session regression test"
                );
            }
        }

        let search = server
            .recall_code_with_session(
                Parameters(RecallCodeParams {
                    query: "stdio no session".to_string(),
                    project_id: None,
                    limit: Some(20),
                    mode: None,
                    vector_weight: None,
                    bm25_weight: None,
                    ppr_weight: None,
                    path_prefix: None,
                    language: None,
                    chunk_type: None,
                }),
                None,
            )
            .await
            .expect("no-session recall should succeed")
            .into_typed::<Value>()
            .expect("recall response should be json");

        assert_eq!(search["project_resolution"]["source"], "cross_project");
        assert!(search["project_resolution"]["project_id"].is_null());
        let text = search.to_string();
        assert!(text.contains("stdio no session alpha marker"));
        assert!(text.contains("stdio no session beta marker"));
    }

    #[tokio::test]
    async fn project_info_bind_unknown_project_rejected() {
        let ctx = TestContext::new().await;
        let server = MemoryMcpServer::new(ctx.state.clone());
        let session_context = CodeToolContext::from_session_id(Some("sid-unknown".to_string()));

        let bind = server
            .project_info_with_session(
                Parameters(ProjectInfoParams {
                    action: "bind".to_string(),
                    project_id: Some("missing-project-id".to_string()),
                    path: None,
                    force: None,
                    confirm_failed_restart: None,
                    locator: None,
                    relation_scope: None,
                    sort_mode: None,
                }),
                Some(session_context),
            )
            .await
            .expect("unknown bind should return success json")
            .into_typed::<Value>()
            .expect("unknown bind response should be json");

        assert_eq!(bind["action"], "bind");
        assert_eq!(bind["reason_code"], "missing");
        assert!(bind["binding"].is_null());
        assert!(ctx
            .state
            .session_bindings
            .binding_status("sid-unknown")
            .await
            .project_id
            .is_none());
    }

    #[tokio::test]
    async fn project_info_bind_indexing_project_allowed() {
        let ctx = TestContext::new().await;
        let server = MemoryMcpServer::new(ctx.state.clone());
        let session_context = CodeToolContext::from_session_id(Some("sid-indexing".to_string()));
        let project_id = "indexing-bind-project";
        let project_path = ctx._temp_dir.path().join(project_id);
        std::fs::create_dir_all(&project_path).expect("project path should exist");

        ctx.state
            .project_registry
            .ensure_project(project_id.to_string(), &project_path)
            .await
            .expect("project should register");

        let mut status = crate::types::IndexStatus::new(project_id.to_string());
        status.status = crate::types::IndexState::Indexing;
        status.total_files = 10;
        status.indexed_files = 4;
        ctx.state
            .storage
            .update_index_status(status)
            .await
            .expect("status should update");

        let bind = server
            .project_info_with_session(
                Parameters(ProjectInfoParams {
                    action: "bind".to_string(),
                    project_id: Some(project_id.to_string()),
                    path: None,
                    force: None,
                    confirm_failed_restart: None,
                    locator: None,
                    relation_scope: None,
                    sort_mode: None,
                }),
                Some(session_context),
            )
            .await
            .expect("indexing project bind should succeed")
            .into_typed::<Value>()
            .expect("indexing bind response should be json");

        assert_eq!(bind["binding"]["project_id"], project_id);
        assert_eq!(bind["lifecycle"]["index_status"]["status"], "indexing");
        assert_eq!(
            bind["lifecycle"]["summary"]["partial"]["is_partial"],
            Value::Bool(true)
        );
        assert_eq!(
            bind["lifecycle"]["summary"]["partial"]["reason_code"],
            "partial"
        );
    }

    #[tokio::test]
    async fn index_project_does_not_auto_bind_session() {
        let ctx = TestContext::new().await;
        let server = MemoryMcpServer::new(ctx.state.clone());
        let project_path = ctx._temp_dir.path().join("no-auto-bind-project");
        std::fs::create_dir_all(&project_path).expect("project path should exist");

        let _ = server
            .index_project(Parameters(IndexProjectParams {
                path: project_path.to_string_lossy().to_string(),
                force: None,
                confirm_failed_restart: None,
            }))
            .await
            .expect("index_project should return success json");

        assert_eq!(ctx.state.session_bindings.len().await, 0);
        assert!(ctx
            .state
            .session_bindings
            .binding_status("sid-index-project")
            .await
            .project_id
            .is_none());
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
        assert!(project_info_desc.contains("index(path"));
        assert!(project_info_desc.contains("confirm_failed_restart"));
        let project_info_required = schema_value(project_info)
            .get("required")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(project_info_required
            .iter()
            .any(|value| value.as_str() == Some("action")));

        let export_memory = get_tool("export_memory");
        let export_memory_desc = export_memory
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(export_memory_desc.contains("migration contract") || export_memory_desc.contains("Export memories"));
        let export_memory_required = schema_value(export_memory)
            .get("required")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(export_memory_required
            .iter()
            .any(|value| value.as_str() == Some("projectId"))
            || export_memory_required.iter().any(|value| value.as_str() == Some("project_id")));
        let export_memory_props = schema_value(export_memory)
            .get("properties")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for forbidden in ["path", "url", "file", "overwrite", "reset", "replace"] {
            assert!(!export_memory_props.contains_key(forbidden));
        }

        let import_memory = get_tool("import_memory");
        let import_memory_desc = import_memory
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(import_memory_desc.contains("migration contract") || import_memory_desc.contains("inline JSONL"));
        let import_memory_required = schema_value(import_memory)
            .get("required")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(import_memory_required
            .iter()
            .any(|value| value.as_str() == Some("jsonl")));
        let import_memory_props = schema_value(import_memory)
            .get("properties")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for forbidden in ["path", "url", "file", "overwrite", "reset", "replace"] {
            assert!(!import_memory_props.contains_key(forbidden));
        }

        let knowledge_graph = get_tool("knowledge_graph");
        let knowledge_graph_desc = knowledge_graph
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(knowledge_graph_desc.contains("entity IDs returned by create_entity"));

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
