use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// สถานะการใช้งานของบัญชี OKX ที่ผูกไว้
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AccountStatus {
    Active,
    InvalidKeys,
    Suspended,
}

impl Default for AccountStatus {
    fn default() -> Self {
        Self::Active
    }
}

/// ข้อมูลบัญชี OKX ที่บันทึกลงใน MongoDB Collection `accounts`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub label: String,
    pub exchange: String,
    pub api_key: String, // Masked หรือ Full API Key
    pub encrypted_secret: String, // AES-256-GCM (Base64)
    pub encrypted_passphrase: String, // AES-256-GCM (Base64)
    pub is_simulated: bool,
    pub status: AccountStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Account {
    pub fn new(
        id: String,
        user_id: String,
        label: String,
        api_key: String,
        encrypted_secret: String,
        encrypted_passphrase: String,
        is_simulated: bool,
    ) -> Self {
        let now = Utc::now();
        Self {
            id,
            user_id,
            label,
            exchange: "okx".to_string(),
            api_key,
            encrypted_secret,
            encrypted_passphrase,
            is_simulated,
            status: AccountStatus::Active,
            created_at: now,
            updated_at: now,
        }
    }

    /// แปลงเป็น Response DTO สำหรับส่งให้ Client (ไม่มี encrypted secrets)
    pub fn to_response(&self) -> AccountResponse {
        AccountResponse {
            id: self.id.clone(),
            user_id: self.user_id.clone(),
            label: self.label.clone(),
            exchange: self.exchange.clone(),
            api_key: mask_api_key(&self.api_key),
            is_simulated: self.is_simulated,
            status: self.status.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

/// DTO สำหรับรับข้อมูลการผูกบัญชี OKX ใหม่
#[derive(Debug, Deserialize, ToSchema)]
pub struct LinkAccountRequest {
    #[schema(example = "Main OKX Demo Bot")]
    pub label: String,
    #[schema(example = "c1b2a3d4-xxxx-xxxx-xxxx-xxxxxxxxxxxx")]
    pub api_key: String,
    #[schema(example = "1A2B3C4D5E6F7G8H9I0J...")]
    pub secret_key: String,
    #[schema(example = "MySecretPassphrase123!")]
    pub passphrase: String,
    #[schema(example = true)]
    pub is_simulated: bool,
}

/// DTO สำหรับส่งรายการบัญชีให้ Client (ไม่มีข้อมูลลับ)
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AccountResponse {
    pub id: String,
    pub user_id: String,
    pub label: String,
    pub exchange: String,
    pub api_key: String, // Masked เช่น c1b2****xxxx
    pub is_simulated: bool,
    pub status: AccountStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Helper function ในการ Mask API Key เช่น "12345678-abcd-1234" -> "1234****1234"
pub fn mask_api_key(key: &str) -> String {
    if key.len() <= 8 {
        return "****".to_string();
    }
    let prefix = &key[..4];
    let suffix = &key[key.len() - 4..];
    format!("{}****{}", prefix, suffix)
}
