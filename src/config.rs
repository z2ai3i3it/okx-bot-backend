use std::env;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub mongodb_uri: String,
    pub mongodb_db_name: String,
    pub jwt_secret: String,
    pub jwt_expiration_hours: i64,
    pub port: u16,
    pub encryption_key: String,
}

impl AppConfig {
    pub fn load() -> Self {
        // พยายามโหลดจาก secrets.env หรือ .env
        let _ = dotenvy::from_filename("config/secrets.env");
        let _ = dotenvy::dotenv();

        Self {
            mongodb_uri: env::var("MONGODB_URI")
                .unwrap_or_else(|_| "mongodb://localhost:27017".to_string()),
            mongodb_db_name: env::var("MONGODB_DB_NAME")
                .unwrap_or_else(|_| "okx-bot".to_string()),
            jwt_secret: env::var("JWT_SECRET")
                .unwrap_or_else(|_| "default-dev-jwt-secret-key-32chars-min".to_string()),
            jwt_expiration_hours: env::var("JWT_EXPIRATION_HOURS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(24),
            port: env::var("PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(3000),
            encryption_key: env::var("ENCRYPTION_KEY")
                .unwrap_or_else(|_| "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string()),
        }
    }
}
