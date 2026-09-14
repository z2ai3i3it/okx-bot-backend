pub mod bot;
pub mod config;
pub mod crypto;
pub mod domain;
pub mod observability;
pub mod okx;
pub mod services;
pub mod storage;
pub mod users;
pub mod web;

use std::net::SocketAddr;
use std::sync::Arc;
use bot::manager::BotManager;
use config::AppConfig;
use crypto::encryption::EncryptionService;
use okx::manager::OkxManager;
use okx::rest_client::OkxRestClient;
use services::strategy_service::StrategyService;
use storage::db::init_db;
use storage::repositories::account_repository::AccountRepository;
use storage::repositories::strategy_repository::StrategyRepository;
use storage::repositories::user_repository::UserRepository;
use users::account_service::AccountService;
use users::auth_service::{AuthConfig, AuthService};
use web::routes::create_router;
use web::state::AppState;

#[tokio::main]
async fn main() {
    // ตั้งค่า Logger (flexi_logger) ก่อนทุกอย่าง
    observability::logging::setup_logger().expect("Failed to initialize logger");

    log::info!("Loading application configuration...");
    let app_config = AppConfig::load();

    log::info!(
        "Connecting to MongoDB at {} (DB: {})...",
        app_config.mongodb_uri, app_config.mongodb_db_name
    );

    let db = init_db(&app_config.mongodb_uri, &app_config.mongodb_db_name)
        .await
        .expect("Failed to connect to MongoDB. Please ensure MongoDB Compass / Server is running.");

    log::info!("MongoDB connected successfully!");

    let user_repo = UserRepository::new(&db);
    let account_repo = Arc::new(AccountRepository::new(&db));
    let strategy_repo = Arc::new(StrategyRepository::new(&db));

    let encryption_service = Arc::new(
        EncryptionService::new(&app_config.encryption_key)
            .expect("Failed to initialize EncryptionService with provided ENCRYPTION_KEY"),
    );

    let okx_rest_client = Arc::new(OkxRestClient::new());
    let okx_manager = OkxManager::new();
    let bot_manager = BotManager::new();

    let account_service = AccountService::new(
        account_repo.clone(),
        encryption_service.clone(),
        okx_rest_client.clone(),
    );

    let strategy_service = StrategyService::new(
        strategy_repo,
        account_repo,
        bot_manager,
        okx_manager,
    );

    let auth_config = AuthConfig {
        jwt_secret: app_config.jwt_secret,
        jwt_expiration_hours: app_config.jwt_expiration_hours,
    };
    let auth_service = AuthService::new(auth_config);

    let app_state = AppState::new(auth_service, user_repo, account_service, strategy_service);
    let app = create_router(app_state);

    let addr = SocketAddr::from(([127, 0, 0, 1], app_config.port));
    log::info!("Web server running at http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Failed to bind TCP listener");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("Failed to start server");

    // flush log ที่ค้างใน buffer ก่อนปิดโปรแกรม
    observability::logging::flush_log();
    log::info!("Server shutdown complete. Goodbye!");
}

/// รอรับสัญญาณ Ctrl+C แล้ว trigger graceful shutdown
async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to install Ctrl+C signal handler");
    log::info!("Ctrl+C received — initiating graceful shutdown...");
}
