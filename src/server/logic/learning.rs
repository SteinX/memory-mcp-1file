use rmcp::model::CallToolResult;

use crate::config::AppState;
use crate::server::params::{
    LearningMemoryArchiveParams, LearningMemoryCreateParams, LearningMemoryDeleteParams,
    LearningMemoryGetParams, LearningMemoryListParams, LearningMemoryMigrateLegacyParams,
    LearningMemoryPromoteParams, LearningMemoryRejectParams, LearningMemorySearchParams,
    LearningMemorySupersededParams, LearningMemoryUpdateParams,
};

use super::success_json;

pub async fn create(
    _state: &AppState,
    _params: LearningMemoryCreateParams,
) -> anyhow::Result<CallToolResult> {
    Ok(success_json(serde_json::json!({
        "status": "not_implemented",
        "message": "learning_memory_create: logic will be implemented in Tasks 6-9"
    })))
}

pub async fn get(
    _state: &AppState,
    _params: LearningMemoryGetParams,
) -> anyhow::Result<CallToolResult> {
    Ok(success_json(serde_json::json!({
        "status": "not_implemented",
        "message": "learning_memory_get: logic will be implemented in Tasks 6-9"
    })))
}

pub async fn list(
    _state: &AppState,
    _params: LearningMemoryListParams,
) -> anyhow::Result<CallToolResult> {
    Ok(success_json(serde_json::json!({
        "status": "not_implemented",
        "message": "learning_memory_list: logic will be implemented in Tasks 6-9"
    })))
}

pub async fn search(
    _state: &AppState,
    _params: LearningMemorySearchParams,
) -> anyhow::Result<CallToolResult> {
    Ok(success_json(serde_json::json!({
        "status": "not_implemented",
        "message": "learning_memory_search: logic will be implemented in Tasks 6-9"
    })))
}

pub async fn update(
    _state: &AppState,
    _params: LearningMemoryUpdateParams,
) -> anyhow::Result<CallToolResult> {
    Ok(success_json(serde_json::json!({
        "status": "not_implemented",
        "message": "learning_memory_update: logic will be implemented in Tasks 6-9"
    })))
}

pub async fn promote(
    _state: &AppState,
    _params: LearningMemoryPromoteParams,
) -> anyhow::Result<CallToolResult> {
    Ok(success_json(serde_json::json!({
        "status": "not_implemented",
        "message": "learning_memory_promote: logic will be implemented in Tasks 6-9"
    })))
}

pub async fn reject(
    _state: &AppState,
    _params: LearningMemoryRejectParams,
) -> anyhow::Result<CallToolResult> {
    Ok(success_json(serde_json::json!({
        "status": "not_implemented",
        "message": "learning_memory_reject: logic will be implemented in Tasks 6-9"
    })))
}

pub async fn archive(
    _state: &AppState,
    _params: LearningMemoryArchiveParams,
) -> anyhow::Result<CallToolResult> {
    Ok(success_json(serde_json::json!({
        "status": "not_implemented",
        "message": "learning_memory_archive: logic will be implemented in Tasks 6-9"
    })))
}

pub async fn supersede(
    _state: &AppState,
    _params: LearningMemorySupersededParams,
) -> anyhow::Result<CallToolResult> {
    Ok(success_json(serde_json::json!({
        "status": "not_implemented",
        "message": "learning_memory_supersede: logic will be implemented in Tasks 6-9"
    })))
}

pub async fn migrate_legacy(
    _state: &AppState,
    _params: LearningMemoryMigrateLegacyParams,
) -> anyhow::Result<CallToolResult> {
    Ok(success_json(serde_json::json!({
        "status": "not_implemented",
        "message": "learning_memory_migrate_legacy: logic will be implemented in Tasks 6-9"
    })))
}

pub async fn delete(
    _state: &AppState,
    _params: LearningMemoryDeleteParams,
) -> anyhow::Result<CallToolResult> {
    Ok(success_json(serde_json::json!({
        "status": "not_implemented",
        "message": "learning_memory_delete: compatibility shim; logic will be implemented in Tasks 6-9"
    })))
}
