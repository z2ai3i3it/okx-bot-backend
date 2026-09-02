use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

use crate::domain::user::{Claims, Role};
use crate::web::state::AppState;

/// Helper response สำหรับส่ง HTTP 401 Unauthorized พร้อม JSON error message
fn unauthorized(msg: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({
            "success": false,
            "error": msg
        })),
    )
        .into_response()
}

/// Helper response สำหรับส่ง HTTP 403 Forbidden
fn forbidden(msg: &str) -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(json!({
            "success": false,
            "error": msg
        })),
    )
        .into_response()
}

/// Middleware หลักสำหรับตรวจสอบ JWT Token จาก Header: `Authorization: Bearer <token>`
/// หากผ่าน จะแนบ `Claims` เข้าไปใน Request Extension ให้ Handler เรียกใช้ได้
pub async fn require_auth(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Result<Response, Response> {
    let auth_header = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok());

    let auth_header = match auth_header {
        Some(header) => header,
        None => return Err(unauthorized("Missing Authorization header")),
    };

    // ต้องเป็นรูปแบบ Bearer <token>
    if !auth_header.starts_with("Bearer ") {
        return Err(unauthorized("Invalid authorization format, must be Bearer token"));
    }

    let token = &auth_header[7..]; // ตัด "Bearer " ออก
    match state.auth_service.verify_token(token) {
        Ok(claims) => {
            // แนบ Claims ลงใน request extensions เพื่อให้ handlers ถัดไปดึงไปใช้ได้ (Extension(claims))
            req.extensions_mut().insert(claims);
            Ok(next.run(req).await)
        }
        Err(e) => Err(unauthorized(&format!("Invalid or expired token: {}", e))),
    }
}

/// Middleware เสริมสำหรับตรวจสอบสิทธิ์เฉพาะผู้ใช้ระดับ Admin
pub async fn require_admin(
    req: Request,
    next: Next,
) -> Result<Response, Response> {
    let claims = req.extensions().get::<Claims>();

    match claims {
        Some(c) if c.role == Role::Admin => Ok(next.run(req).await),
        Some(_) => Err(forbidden("Access denied: Administrator role required")),
        None => Err(unauthorized("Authentication required")),
    }
}
