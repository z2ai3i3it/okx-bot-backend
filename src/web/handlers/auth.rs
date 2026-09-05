use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension, Json,
};
use serde_json::json;
use uuid::Uuid;

use chrono::Utc;
use crate::domain::user::{
    AuthResponse, ChangePasswordRequest, Claims, LoginRequest, RegisterRequest,
    UpdateProfileRequest, UserResponse, UserStatus,
};
use crate::users::auth_service::AuthError;
use crate::web::state::AppState;

/// แปลง AuthError หรือข้อผิดพลาดทั่วไปให้เป็น HTTP Response พร้อม JSON payload
fn handle_auth_error(err: AuthError) -> Response {
    let (status, msg) = match err {
        AuthError::InvalidCredentials => (StatusCode::UNAUTHORIZED, "Invalid email or password".to_string()),
        AuthError::AccountInactive => (StatusCode::FORBIDDEN, "Account is suspended or inactive".to_string()),
        AuthError::UserAlreadyExists(detail) => (StatusCode::CONFLICT, detail),
        AuthError::ValidationError(detail) => (StatusCode::BAD_REQUEST, detail),
        AuthError::TokenError(detail) => (StatusCode::UNAUTHORIZED, detail),
        AuthError::UserNotFound => (StatusCode::NOT_FOUND, "User not found".to_string()),
        AuthError::HashError(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "An internal error occurred while processing password".to_string(),
        ),
    };

    (
        status,
        Json(json!({
            "success": false,
            "error": msg
        })),
    )
        .into_response()
}

/// Helper แปลง DB error ให้เป็น Internal Server Error
fn handle_db_error(msg: String) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({
            "success": false,
            "error": format!("Database error: {}", msg)
        })),
    )
        .into_response()
}

/// POST /api/auth/register
/// สมัครสมาชิกผู้ใช้ใหม่ (ตรวจสอบซ้ำ และบันทึกลง MongoDB จริง)
#[utoipa::path(
    post,
    path = "/api/auth/register",
    tag = "Authentication",
    request_body = RegisterRequest,
    responses(
        (status = 201, description = "User registered successfully", body = AuthResponse),
        (status = 400, description = "Validation error"),
        (status = 409, description = "Email or username already exists")
    )
)]
pub async fn register(
    State(state): State<AppState>,
    Json(payload): Json<RegisterRequest>,
) -> Result<(StatusCode, Response), Response> {
    // 1. ตรวจสอบว่า Email ถูกใช้ไปแล้วหรือยัง
    let existing_email = state
        .user_repo
        .find_by_email(&payload.email)
        .await
        .map_err(|e| handle_db_error(e.to_string()))?;

    if existing_email.is_some() {
        return Err(handle_auth_error(AuthError::UserAlreadyExists(
            "Email is already registered".into(),
        )));
    }

    // 2. ตรวจสอบว่า Username ถูกใช้ไปแล้วหรือยัง
    let existing_username = state
        .user_repo
        .find_by_username(&payload.username)
        .await
        .map_err(|e| handle_db_error(e.to_string()))?;

    if existing_username.is_some() {
        return Err(handle_auth_error(AuthError::UserAlreadyExists(
            "Username is already taken".into(),
        )));
    }

    // 3. เตรียม User Entity พร้อมแฮชรหัสผ่าน
    let user_id = Uuid::new_v4().to_string();
    let new_user = state
        .auth_service
        .create_user_entity(user_id, payload, None)
        .map_err(handle_auth_error)?;

    // 4. บันทึกลง MongoDB จริง
    state
        .user_repo
        .create(&new_user)
        .await
        .map_err(|e| handle_db_error(e.to_string()))?;

    // 5. ออก JWT Token
    let token = state
        .auth_service
        .generate_token(&new_user)
        .map_err(handle_auth_error)?;

    let res = Json(json!({
        "success": true,
        "data": {
            "access_token": token,
            "token_type": "Bearer",
            "user": new_user.to_response()
        }
    }))
    .into_response();

    Ok((StatusCode::CREATED, res))
}

/// POST /api/auth/login
/// เข้าสู่ระบบด้วย Email & Password (ค้นหาจาก MongoDB และตรวจสอบความถูกต้องจริง)
#[utoipa::path(
    post,
    path = "/api/auth/login",
    tag = "Authentication",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "Login successful", body = AuthResponse),
        (status = 400, description = "Missing email or password"),
        (status = 401, description = "Invalid credentials"),
        (status = 403, description = "Account is inactive or suspended")
    )
)]
pub async fn login(
    State(state): State<AppState>,
    Json(payload): Json<LoginRequest>,
) -> Result<Response, Response> {
    if payload.email.trim().is_empty() || payload.password.trim().is_empty() {
        return Err(handle_auth_error(AuthError::ValidationError(
            "Email and password are required".into(),
        )));
    }

    // 1. ค้นหาผู้ใช้ตาม Email จาก MongoDB
    let user = state
        .user_repo
        .find_by_email(&payload.email)
        .await
        .map_err(|e| handle_db_error(e.to_string()))?;

    let user = match user {
        Some(u) => u,
        None => return Err(handle_auth_error(AuthError::InvalidCredentials)),
    };

    // 2. ตรวจสอบรหัสผ่านและสร้าง AuthResponse
    let auth_res = state
        .auth_service
        .authenticate_user(&user, &payload.password)
        .map_err(handle_auth_error)?;

    Ok(Json(json!({
        "success": true,
        "data": auth_res
    }))
    .into_response())
}

/// GET /api/auth/me
/// ดึงข้อมูล Profile ของผู้ใช้ปัจจุบันจาก Database
#[utoipa::path(
    get,
    path = "/api/auth/me",
    tag = "Authentication",
    security(
        ("bearer_auth" = [])
    ),
    responses(
        (status = 200, description = "Current user profile", body = UserResponse),
        (status = 401, description = "Unauthorized - Missing or invalid JWT token"),
        (status = 404, description = "User not found")
    )
)]
pub async fn get_current_user(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> Result<Response, Response> {
    let user = state
        .user_repo
        .find_by_id(&claims.sub)
        .await
        .map_err(|e| handle_db_error(e.to_string()))?;

    match user {
        Some(u) => Ok(Json(json!({
            "success": true,
            "data": u.to_response()
        }))
        .into_response()),
        None => Err(handle_auth_error(AuthError::UserNotFound)),
    }
}

/// PUT /api/auth/profile
/// อัปเดตข้อมูลส่วนตัว (Username, Email)
#[utoipa::path(
    put,
    path = "/api/auth/profile",
    tag = "Authentication",
    security(
        ("bearer_auth" = [])
    ),
    request_body = UpdateProfileRequest,
    responses(
        (status = 200, description = "Profile updated successfully", body = UserResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Unauthorized"),
        (status = 409, description = "Email or username already taken")
    )
)]
pub async fn update_profile(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(payload): Json<UpdateProfileRequest>,
) -> Result<Response, Response> {
    let mut user = state
        .user_repo
        .find_by_id(&claims.sub)
        .await
        .map_err(|e| handle_db_error(e.to_string()))?
        .ok_or_else(|| handle_auth_error(AuthError::UserNotFound))?;

    // ถ้ามีการเปลี่ยน email ให้ตรวจสอบว่าซ้ำกับคนอื่นหรือไม่
    if let Some(ref new_email) = payload.email {
        if new_email != &user.email {
            if new_email.trim().is_empty() || !new_email.contains('@') {
                return Err(handle_auth_error(AuthError::ValidationError(
                    "Invalid email address".into(),
                )));
            }
            let existing = state
                .user_repo
                .find_by_email(new_email)
                .await
                .map_err(|e| handle_db_error(e.to_string()))?;
            if existing.is_some() {
                return Err(handle_auth_error(AuthError::UserAlreadyExists(
                    "Email is already taken".into(),
                )));
            }
            user.email = new_email.clone();
        }
    }

    // ถ้ามีการเปลี่ยน username ให้ตรวจสอบว่าซ้ำกับคนอื่นหรือไม่
    if let Some(ref new_username) = payload.username {
        if new_username != &user.username {
            if new_username.trim().is_empty() {
                return Err(handle_auth_error(AuthError::ValidationError(
                    "Username cannot be empty".into(),
                )));
            }
            let existing = state
                .user_repo
                .find_by_username(new_username)
                .await
                .map_err(|e| handle_db_error(e.to_string()))?;
            if existing.is_some() {
                return Err(handle_auth_error(AuthError::UserAlreadyExists(
                    "Username is already taken".into(),
                )));
            }
            user.username = new_username.clone();
        }
    }

    user.updated_at = Utc::now();

    state
        .user_repo
        .update(&user)
        .await
        .map_err(|e| handle_db_error(e.to_string()))?;

    Ok(Json(json!({
        "success": true,
        "data": user.to_response()
    }))
    .into_response())
}

/// PUT /api/auth/password
/// เปลี่ยนรหัสผ่านของผู้ใช้ (ตรวจสอบรหัสผ่านเดิมก่อน)
#[utoipa::path(
    put,
    path = "/api/auth/password",
    tag = "Authentication",
    security(
        ("bearer_auth" = [])
    ),
    request_body = ChangePasswordRequest,
    responses(
        (status = 200, description = "Password changed successfully", body = crate::domain::user::GenericMessageResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Invalid old password or unauthorized")
    )
)]
pub async fn change_password(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(payload): Json<ChangePasswordRequest>,
) -> Result<Response, Response> {
    let mut user = state
        .user_repo
        .find_by_id(&claims.sub)
        .await
        .map_err(|e| handle_db_error(e.to_string()))?
        .ok_or_else(|| handle_auth_error(AuthError::UserNotFound))?;

    let new_hash = state
        .auth_service
        .process_change_password(&user, &payload.old_password, &payload.new_password)
        .map_err(handle_auth_error)?;

    user.password_hash = new_hash;
    user.updated_at = Utc::now();

    state
        .user_repo
        .update(&user)
        .await
        .map_err(|e| handle_db_error(e.to_string()))?;

    Ok(Json(json!({
        "success": true,
        "message": "Password changed successfully"
    }))
    .into_response())
}

/// DELETE /api/auth/account
/// ปิดการใช้งานบัญชี (Soft Delete ปรับสถานะเป็น Suspended)
#[utoipa::path(
    delete,
    path = "/api/auth/account",
    tag = "Authentication",
    security(
        ("bearer_auth" = [])
    ),
    responses(
        (status = 200, description = "Account deactivated successfully", body = crate::domain::user::GenericMessageResponse),
        (status = 401, description = "Unauthorized")
    )
)]
pub async fn delete_account(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> Result<Response, Response> {
    let mut user = state
        .user_repo
        .find_by_id(&claims.sub)
        .await
        .map_err(|e| handle_db_error(e.to_string()))?
        .ok_or_else(|| handle_auth_error(AuthError::UserNotFound))?;

    // ทำ Soft Delete โดยการตั้งสถานะเป็น Suspended
    user.status = UserStatus::Suspended;
    user.updated_at = Utc::now();

    state
        .user_repo
        .update(&user)
        .await
        .map_err(|e| handle_db_error(e.to_string()))?;

    Ok(Json(json!({
        "success": true,
        "message": "Account has been deactivated successfully"
    }))
    .into_response())
}

/// POST /api/auth/logout
/// ออกจากระบบ (Invalidate Token ทั้งหมดที่ออกก่อนหน้านี้โดยอัปเดต last_logout_at)
#[utoipa::path(
    post,
    path = "/api/auth/logout",
    tag = "Authentication",
    security(
        ("bearer_auth" = [])
    ),
    responses(
        (status = 200, description = "Logged out successfully", body = crate::domain::user::GenericMessageResponse),
        (status = 401, description = "Unauthorized")
    )
)]
pub async fn logout(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> Result<Response, Response> {
    let mut user = state
        .user_repo
        .find_by_id(&claims.sub)
        .await
        .map_err(|e| handle_db_error(e.to_string()))?
        .ok_or_else(|| handle_auth_error(AuthError::UserNotFound))?;

    let now = Utc::now();
    user.last_logout_at = Some(now);
    user.updated_at = now;

    state
        .user_repo
        .update(&user)
        .await
        .map_err(|e| handle_db_error(e.to_string()))?;

    Ok(Json(json!({
        "success": true,
        "message": "Logged out successfully. Token is now invalidated."
    }))
    .into_response())
}

