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
    order::{Side, TrackedOrder},
    strategy::{
        fixed_ratio_rebalance::FixdRatioRebalanceConfig, grid::GridConfig, BotResponse,
        CreateBotRequest, StrategyConfig, StrategyState, StrategyStatus, UpdateBotRequest,
    },
    user::{
        AuthResponse, ChangePasswordRequest, GenericMessageResponse, LoginRequest, RegisterRequest,
        Role, UpdateProfileRequest, UserResponse, UserStatus,
    },
};
use crate::okx::dto::account::{AccountVerificationResult, OkxBalanceDetail};
use crate::web::{
    handlers::{
        account::{
            self, delete_account as delete_linked_account, get_account, link_account,
            list_accounts, verify_account,
        },
        auth::{
            self as auth_handlers, change_password, delete_account, get_current_user, login,
            logout, register, update_profile,
        },
        bot_control::{
            self as bot_handlers, create_bot, delete_bot, get_bot, list_bots, start_bot,
            stop_bot, update_bot,
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
        auth_handlers::logout,
        auth_handlers::get_current_user,
        auth_handlers::update_profile,
        auth_handlers::change_password,
        auth_handlers::delete_account,
        account::link_account,
        account::list_accounts,
        account::get_account,
        account::delete_account,
        account::verify_account,
        bot_handlers::create_bot,
        bot_handlers::list_bots,
        bot_handlers::get_bot,
        bot_handlers::update_bot,
        bot_handlers::delete_bot,
        bot_handlers::start_bot,
        bot_handlers::stop_bot,
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
            AccountVerificationResult,
            OkxBalanceDetail,
            CreateBotRequest,
            UpdateBotRequest,
            BotResponse,
            StrategyConfig,
            StrategyState,
            StrategyStatus,
            FixdRatioRebalanceConfig,
            GridConfig,
            TrackedOrder,
            Side,
        )
    ),
    modifiers(&SecurityAddon),
    tags(
        (name = "Authentication", description = "OKX Web Bot User Authentication & Profile Management Endpoints"),
        (name = "Exchange Accounts", description = "OKX API Key Linking & Encrypted Account Management Endpoints"),
        (name = "Trading Bots", description = "Trading Bot Lifecycle & Strategy Configuration Endpoints")
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
        .route("/logout", post(logout))
        .route("/profile", put(update_profile))
        .route("/password", put(change_password))
        .route("/account", delete(delete_account))
        .route_layer(from_fn_with_state(state.clone(), require_auth));

    // Protected Exchange Account Routes ต้องมี Token
    let account_routes = Router::new()
        .route("/", post(link_account).get(list_accounts))
        .route("/{id}", get(get_account).delete(delete_linked_account))
        .route("/{id}/verify", post(verify_account))
        .route_layer(from_fn_with_state(state.clone(), require_auth));

    // Protected Trading Bot Routes ต้องมี Token
    let bot_routes = Router::new()
        .route("/", post(create_bot).get(list_bots))
        .route("/{id}", get(get_bot).put(update_bot).delete(delete_bot))
        .route("/{id}/start", post(start_bot))
        .route("/{id}/stop", post(stop_bot))
        .route_layer(from_fn_with_state(state.clone(), require_auth));

    // ผูก Swagger UI เข้ากับ Axum Router
    let swagger_router = SwaggerUi::new("/swagger-ui")
        .url("/api-docs/openapi.json", ApiDoc::openapi());

    Router::new()
        .merge(swagger_router)
        .nest("/api/auth", auth_public_routes.merge(auth_protected_routes))
        .nest("/api/accounts", account_routes)
        .nest("/api/bots", bot_routes)
        .with_state(state)
}
