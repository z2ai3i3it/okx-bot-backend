use axum::{
    middleware::from_fn_with_state,
    routing::{get, post},
    Router,
};
use utoipa::{
    openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme},
    Modify, OpenApi,
};
use utoipa_swagger_ui::SwaggerUi;

use crate::domain::user::{
    AuthResponse, ChangePasswordRequest, GenericMessageResponse, LoginRequest, RegisterRequest,
    Role, UpdateProfileRequest, UserResponse, UserStatus,
};
use crate::web::{
    handlers::auth::{
        self, change_password, delete_account, get_current_user, login, register, update_profile,
    },
    middlewares::auth_middleware::require_auth,
    state::AppState,
};

/// Modifier สำหรับเพิ่ม Bearer Token Security Scheme ลงใน OpenAPI Spec
struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = openapi.components.as_mut() {
            components.add_security_scheme(
                "bearer_auth",
                SecurityScheme::Http(
                    HttpBuilder::new()
                        .scheme(HttpAuthScheme::Bearer)
                        .bearer_format("JWT")
                        .description(Some("Enter JWT Bearer token here"))
                        .build(),
                ),
            );
        }
    }
}

/// OpenAPI Documentation Structure
#[derive(OpenApi)]
#[openapi(
    paths(
        auth::register,
        auth::login,
        auth::get_current_user,
        auth::update_profile,
        auth::change_password,
        auth::delete_account
    ),
    components(
        schemas(
            Role,
            UserStatus,
            UserResponse,
            RegisterRequest,
            LoginRequest,
            AuthResponse,
            UpdateProfileRequest,
            ChangePasswordRequest,
            GenericMessageResponse
        )
    ),
    modifiers(&SecurityAddon),
    tags(
        (name = "Authentication", description = "OKX Web Bot User Authentication & Profile Management Endpoints")
    ),
    info(
        title = "OKX Web Bot API",
        version = "0.1.0",
        description = "High-Performance Pure Rust Trading Engine API for OKX v5"
    )
)]
pub struct ApiDoc;

/// รวม Route ทั้งหมดของแอปพลิเคชัน พร้อมติดตั้ง Swagger UI
pub fn create_router(state: AppState) -> Router {
    // Public Routes ไม่ต้องมี Token
    let auth_public_routes = Router::new()
        .route("/register", post(register))
        .route("/login", post(login));

    // Protected Auth Routes ต้องมี Token
    let auth_protected_routes = Router::new()
        .route("/me", get(get_current_user))
        .route("/profile", axum::routing::put(update_profile))
        .route("/password", axum::routing::put(change_password))
        .route("/account", axum::routing::delete(delete_account))
        .route_layer(from_fn_with_state(state.clone(), require_auth));

    // ผูก Swagger UI เข้ากับ Axum Router
    let swagger_router = SwaggerUi::new("/swagger-ui")
        .url("/api-docs/openapi.json", ApiDoc::openapi());

    Router::new()
        .merge(swagger_router)
        .nest("/api/auth", auth_public_routes.merge(auth_protected_routes))
        .with_state(state)
}
