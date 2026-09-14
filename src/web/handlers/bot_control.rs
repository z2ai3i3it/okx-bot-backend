use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use serde_json::json;

use crate::{
    domain::{
        strategy::{BotResponse, CreateBotRequest, UpdateBotRequest},
        user::{Claims, GenericMessageResponse},
    },
    services::strategy_service::StrategyServiceError,
    web::state::AppState,
};

/// ฟังก์ชันแปลง StrategyServiceError เป็น HTTP Response
fn handle_strategy_error(err: StrategyServiceError) -> (StatusCode, axum::Json<serde_json::Value>) {
    match err {
        StrategyServiceError::ValidationError(msg) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "success": false, "error": msg })),
        ),
        StrategyServiceError::NotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "success": false, "error": "Bot not found or access denied" })),
        ),
        StrategyServiceError::AccountNotFound => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "success": false, "error": "Selected exchange account not found or access denied" })),
        ),
        StrategyServiceError::BotIsRunning(msg) => (
            StatusCode::CONFLICT,
            Json(json!({ "success": false, "error": msg })),
        ),
        StrategyServiceError::BotManagerError(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "success": false, "error": format!("Bot runtime error: {}", e) })),
        ),
        StrategyServiceError::DatabaseError(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "success": false, "error": format!("Database error: {}", e) })),
        ),
    }
}

/// สร้างบอทใหม่
#[utoipa::path(
    post,
    path = "/api/bots",
    tag = "Trading Bots",
    security(("bearer_auth" = [])),
    request_body = CreateBotRequest,
    responses(
        (status = 201, description = "Bot created successfully", body = BotResponse),
        (status = 400, description = "Validation error", body = GenericMessageResponse),
        (status = 401, description = "Unauthorized", body = GenericMessageResponse)
    )
)]
pub async fn create_bot(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(payload): Json<CreateBotRequest>,
) -> impl IntoResponse {
    match state.strategy_service.create_bot(&claims.sub, payload).await {
        Ok(bot) => (StatusCode::CREATED, Json(bot)).into_response(),
        Err(err) => handle_strategy_error(err).into_response(),
    }
}

/// ดูรายการบอททั้งหมดของผู้ใช้
#[utoipa::path(
    get,
    path = "/api/bots",
    tag = "Trading Bots",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "List of user's bots", body = Vec<BotResponse>),
        (status = 401, description = "Unauthorized", body = GenericMessageResponse)
    )
)]
pub async fn list_bots(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> impl IntoResponse {
    match state.strategy_service.list_bots(&claims.sub).await {
        Ok(bots) => (StatusCode::OK, Json(bots)).into_response(),
        Err(err) => handle_strategy_error(err).into_response(),
    }
}

/// ดูข้อมูลบอทเดี่ยวตาม ID
#[utoipa::path(
    get,
    path = "/api/bots/{id}",
    tag = "Trading Bots",
    security(("bearer_auth" = [])),
    params(
        ("id" = String, Path, description = "Bot ID")
    ),
    responses(
        (status = 200, description = "Bot detail", body = BotResponse),
        (status = 404, description = "Bot not found", body = GenericMessageResponse),
        (status = 401, description = "Unauthorized", body = GenericMessageResponse)
    )
)]
pub async fn get_bot(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(bot_id): Path<String>,
) -> impl IntoResponse {
    match state.strategy_service.get_bot(&bot_id, &claims.sub).await {
        Ok(bot) => (StatusCode::OK, Json(bot)).into_response(),
        Err(err) => handle_strategy_error(err).into_response(),
    }
}

/// แก้ไขข้อมูลบอท (ทำได้เฉพาะเมื่อบอทไม่ได้รันอยู่)
#[utoipa::path(
    put,
    path = "/api/bots/{id}",
    tag = "Trading Bots",
    security(("bearer_auth" = [])),
    params(
        ("id" = String, Path, description = "Bot ID")
    ),
    request_body = UpdateBotRequest,
    responses(
        (status = 200, description = "Bot updated successfully", body = BotResponse),
        (status = 409, description = "Cannot update while running", body = GenericMessageResponse),
        (status = 404, description = "Bot not found", body = GenericMessageResponse),
        (status = 401, description = "Unauthorized", body = GenericMessageResponse)
    )
)]
pub async fn update_bot(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(bot_id): Path<String>,
    Json(payload): Json<UpdateBotRequest>,
) -> impl IntoResponse {
    match state.strategy_service.update_bot(&bot_id, &claims.sub, payload).await {
        Ok(bot) => (StatusCode::OK, Json(bot)).into_response(),
        Err(err) => handle_strategy_error(err).into_response(),
    }
}

/// ลบบอท (ทำได้เฉพาะเมื่อบอทไม่ได้รันอยู่)
#[utoipa::path(
    delete,
    path = "/api/bots/{id}",
    tag = "Trading Bots",
    security(("bearer_auth" = [])),
    params(
        ("id" = String, Path, description = "Bot ID")
    ),
    responses(
        (status = 200, description = "Bot deleted successfully", body = GenericMessageResponse),
        (status = 409, description = "Cannot delete while running", body = GenericMessageResponse),
        (status = 404, description = "Bot not found", body = GenericMessageResponse),
        (status = 401, description = "Unauthorized", body = GenericMessageResponse)
    )
)]
pub async fn delete_bot(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(bot_id): Path<String>,
) -> impl IntoResponse {
    match state.strategy_service.delete_bot(&bot_id, &claims.sub).await {
        Ok(_) => (
            StatusCode::OK,
            Json(GenericMessageResponse {
                success: true,
                message: "Bot deleted successfully".to_string(),
            }),
        )
            .into_response(),
        Err(err) => handle_strategy_error(err).into_response(),
    }
}

/// สั่งเริ่มรันบอท (Start Bot)
#[utoipa::path(
    post,
    path = "/api/bots/{id}/start",
    tag = "Trading Bots",
    security(("bearer_auth" = [])),
    params(
        ("id" = String, Path, description = "Bot ID")
    ),
    responses(
        (status = 200, description = "Bot started successfully", body = BotResponse),
        (status = 409, description = "Bot already running", body = GenericMessageResponse),
        (status = 404, description = "Bot not found", body = GenericMessageResponse),
        (status = 401, description = "Unauthorized", body = GenericMessageResponse)
    )
)]
pub async fn start_bot(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(bot_id): Path<String>,
) -> impl IntoResponse {
    match state.strategy_service.start_bot(&bot_id, &claims.sub).await {
        Ok(bot) => (StatusCode::OK, Json(bot)).into_response(),
        Err(err) => handle_strategy_error(err).into_response(),
    }
}

/// สั่งหยุดการทำงานของบอท (Stop Bot)
#[utoipa::path(
    post,
    path = "/api/bots/{id}/stop",
    tag = "Trading Bots",
    security(("bearer_auth" = [])),
    params(
        ("id" = String, Path, description = "Bot ID")
    ),
    responses(
        (status = 200, description = "Bot stopped successfully", body = BotResponse),
        (status = 404, description = "Bot not found", body = GenericMessageResponse),
        (status = 401, description = "Unauthorized", body = GenericMessageResponse)
    )
)]
pub async fn stop_bot(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(bot_id): Path<String>,
) -> impl IntoResponse {
    match state.strategy_service.stop_bot(&bot_id, &claims.sub).await {
        Ok(bot) => (StatusCode::OK, Json(bot)).into_response(),
        Err(err) => handle_strategy_error(err).into_response(),
    }
}
