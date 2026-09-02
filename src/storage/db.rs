use mongodb::{Client, Database};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("Database connection failed: {0}")]
    ConnectionError(#[from] mongodb::error::Error),
}

/// เชื่อมต่อ MongoDB และคืน `Database` instance ตามชื่อที่ระบุใน config
pub async fn init_db(uri: &str, db_name: &str) -> Result<Database, DbError> {
    let client = Client::with_uri_str(uri).await?;
    let db = client.database(db_name);
    Ok(db)
}
