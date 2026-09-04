use axum::{
    middleware::from_fn_with_state,
    routing::{delete, get, post, put},
    Router,
};
use utoipa::{
    openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme},
    Modify, OpenApi,
};
use utoipa_swagger_ui::SwaggerUi;

use crate::domain::{
    account::{AccountResponse, AccountStatus, LinkAccountRequest},
    user::{
        AuthResponse, ChangePasswordRequest, GenericMessageResponse, LoginRequest, RegisterRequest,
        Role, UpdateProfileRequest, UserResponse, UserStatus,
    },
};
use crate::web::{
    handlers::{
        account::{self, delete_account as delete_linked_account, get_account, link_account, list_accounts},
        auth::{
            self as auth_handlers, change_password, delete_account, get_current_user, login,
            register, update_profile,
        },
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
        auth_handlers::register,
        auth_handlers::login,
        auth_handlers::get_current_user,
        auth_handlers::update_profile,
        auth_handlers::change_password,
        auth_handlers::delete_account,
        account::link_account,
        account::list_accounts,
        account::get_account,
        account::delete_account,
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
            GenericMessageResponse,
            AccountStatus,
            LinkAccountRequest,
            AccountResponse,
        )
    ),
    modifiers(&SecurityAddon),
    tags(
        (name = "Authentication", description = "OKX Web Bot User Authentication & Profile Management Endpoints"),
        (name = "Exchange Accounts", description = "OKX API Key Linking & Encrypted Account Management Endpoints")
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
        .route("/profile", put(update_profile))
        .route("/password", put(change_password))
        .route("/account", delete(delete_account))
        .route_layer(from_fn_with_state(state.clone(), require_auth));

    // Protected Exchange Account Routes ต้องมี Token
    let account_routes = Router::new()
        .route("/", post(link_account).get(list_accounts))
        .route("/{id}", get(get_account).delete(delete_linked_account))
        .route_layer(from_fn_with_state(state.clone(), require_auth));

    // ผูก Swagger UI เข้ากับ Axum Router
    let swagger_router = SwaggerUi::new("/swagger-ui")
        .url("/api-docs/openapi.json", ApiDoc::openapi());

    Router::new()
        .merge(swagger_router)
        .nest("/api/auth", auth_public_routes.merge(auth_protected_routes))
        .nest("/api/accounts", account_routes)
        .with_state(state)
}
