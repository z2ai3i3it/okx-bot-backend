use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use serde_json::json;

use crate::{
    domain::{
        account::{AccountResponse, LinkAccountRequest},
        user::{Claims, GenericMessageResponse},
    },
    users::account_service::AccountServiceError,
    web::state::AppState,
};

/// ฟังก์ชันแปลง AccountServiceError เป็น HTTP Response
fn handle_account_error(err: AccountServiceError) -> (StatusCode, axum::Json<serde_json::Value>) {
    match err {
        AccountServiceError::ValidationError(msg) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "success": false, "error": msg })),
        ),
        AccountServiceError::AccountNotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "success": false, "error": "Account not found or access denied" })),
        ),
        AccountServiceError::CryptoError(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "success": false, "error": format!("Encryption error: {}", e) })),
        ),
        AccountServiceError::DatabaseError(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "success": false, "error": format!("Database error: {}", e) })),
        ),
    }
}

/// ผูกบัญชี OKX API Key ใหม่
#[utoipa::path(
    post,
    path = "/api/accounts",
    tag = "Exchange Accounts",
    security(("bearer_auth" = [])),
    request_body = LinkAccountRequest,
    responses(
        (status = 201, description = "OKX Account linked successfully", body = AccountResponse),
        (status = 400, description = "Validation error", body = GenericMessageResponse),
        (status = 401, description = "Unauthorized", body = GenericMessageResponse)
    )
)]
pub async fn link_account(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(payload): Json<LinkAccountRequest>,
) -> impl IntoResponse {
    match state.account_service.link_account(&claims.sub, payload).await {
        Ok(account_response) => (StatusCode::CREATED, Json(account_response)).into_response(),
        Err(err) => handle_account_error(err).into_response(),
    }
}

/// ดึงรายการ OKX Accounts ทั้งหมดของผู้ใช้ปัจจุบัน
#[utoipa::path(
    get,
    path = "/api/accounts",
    tag = "Exchange Accounts",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "List of OKX Accounts", body = Vec<AccountResponse>),
        (status = 401, description = "Unauthorized", body = GenericMessageResponse)
    )
)]
pub async fn list_accounts(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> impl IntoResponse {
    match state.account_service.list_accounts(&claims.sub).await {
        Ok(accounts) => (StatusCode::OK, Json(accounts)).into_response(),
        Err(err) => handle_account_error(err).into_response(),
    }
}

/// ดึงข้อมูลบัญชี OKX ตาม ID
#[utoipa::path(
    get,
    path = "/api/accounts/{id}",
    tag = "Exchange Accounts",
    security(("bearer_auth" = [])),
    params(
        ("id" = String, Path, description = "Account ID")
    ),
    responses(
        (status = 200, description = "Account detail", body = AccountResponse),
        (status = 404, description = "Account not found", body = GenericMessageResponse),
        (status = 401, description = "Unauthorized", body = GenericMessageResponse)
    )
)]
pub async fn get_account(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(account_id): Path<String>,
) -> impl IntoResponse {
    match state.account_service.get_account(&account_id, &claims.sub).await {
        Ok(account) => (StatusCode::OK, Json(account)).into_response(),
        Err(err) => handle_account_error(err).into_response(),
    }
}

/// ลบการผูกบัญชี OKX ตาม ID
#[utoipa::path(
    delete,
    path = "/api/accounts/{id}",
    tag = "Exchange Accounts",
    security(("bearer_auth" = [])),
    params(
        ("id" = String, Path, description = "Account ID")
    ),
    responses(
        (status = 200, description = "Account unlinked successfully", body = GenericMessageResponse),
        (status = 404, description = "Account not found", body = GenericMessageResponse),
        (status = 401, description = "Unauthorized", body = GenericMessageResponse)
    )
)]
pub async fn delete_account(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(account_id): Path<String>,
) -> impl IntoResponse {
    match state.account_service.delete_account(&account_id, &claims.sub).await {
        Ok(_) => (
            StatusCode::OK,
            Json(GenericMessageResponse {
                success: true,
                message: "OKX Account unlinked successfully".to_string(),
            }),
        )
            .into_response(),
        Err(err) => handle_account_error(err).into_response(),
    }
}
