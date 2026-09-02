pub mod config;
pub mod domain;
pub mod storage;
pub mod users;
pub mod web;

use std::net::SocketAddr;
use config::AppConfig;
use storage::db::init_db;
use storage::repositories::user_repository::UserRepository;
use users::auth_service::{AuthConfig, AuthService};
use web::routes::create_router;
use web::state::AppState;

#[tokio::main]
async fn main() {
    println!("Loading application configuration...");
    let app_config = AppConfig::load();

    println!(
        "Connecting to MongoDB at {} (DB: {})...",
        app_config.mongodb_uri, app_config.mongodb_db_name
    );

    let db = init_db(&app_config.mongodb_uri, &app_config.mongodb_db_name)
        .await
        .expect("Failed to connect to MongoDB. Please ensure MongoDB Compass / Server is running.");

    println!("MongoDB connected successfully!");

    let user_repo = UserRepository::new(&db);

    let auth_config = AuthConfig {
        jwt_secret: app_config.jwt_secret,
        jwt_expiration_hours: app_config.jwt_expiration_hours,
    };
    let auth_service = AuthService::new(auth_config);

    let app_state = AppState::new(auth_service, user_repo);
    let app = create_router(app_state);

    let addr = SocketAddr::from(([127, 0, 0, 1], app_config.port));
    println!("Web server running at http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Failed to bind TCP listener");

    axum::serve(listener, app)
        .await
        .expect("Failed to start server");
}
