use std::sync::Arc;
use thiserror::Error;
use uuid::Uuid;

use crate::{
    crypto::encryption::{CryptoError, EncryptionService},
    domain::account::{Account, AccountResponse, LinkAccountRequest},
    storage::repositories::account_repository::AccountRepository,
};

#[derive(Debug, Error)]
pub enum AccountServiceError {
    #[error("Crypto error: {0}")]
    CryptoError(#[from] CryptoError),

    #[error("Database error: {0}")]
    DatabaseError(#[from] mongodb::error::Error),

    #[error("Account not found or access denied")]
    AccountNotFound,

    #[error("Validation error: {0}")]
    ValidationError(String),
}

pub struct AccountService {
    account_repo: Arc<AccountRepository>,
    encryption_service: Arc<EncryptionService>,
}

impl AccountService {
    pub fn new(
        account_repo: Arc<AccountRepository>,
        encryption_service: Arc<EncryptionService>,
    ) -> Self {
        Self {
            account_repo,
            encryption_service,
        }
    }

    /// ผูกบัญชี OKX API Key ใหม่
    /// เข้ารหัส secret_key และ passphrase ด้วย AES-256-GCM ก่อนบันทึกลง Database
    pub async fn link_account(
        &self,
        user_id: &str,
        req: LinkAccountRequest,
    ) -> Result<AccountResponse, AccountServiceError> {
        if req.label.trim().is_empty() {
            return Err(AccountServiceError::ValidationError(
                "Account label cannot be empty".to_string(),
            ));
        }
        if req.api_key.trim().is_empty() {
            return Err(AccountServiceError::ValidationError(
                "API Key cannot be empty".to_string(),
            ));
        }
        if req.secret_key.trim().is_empty() {
            return Err(AccountServiceError::ValidationError(
                "Secret Key cannot be empty".to_string(),
            ));
        }
        if req.passphrase.trim().is_empty() {
            return Err(AccountServiceError::ValidationError(
                "Passphrase cannot be empty".to_string(),
            ));
        }

        // เข้ารหัส Secret Key และ Passphrase
        let encrypted_secret = self.encryption_service.encrypt(req.secret_key.trim())?;
        let encrypted_passphrase = self.encryption_service.encrypt(req.passphrase.trim())?;

        let account_id = Uuid::new_v4().to_string();
        let account = Account::new(
            account_id,
            user_id.to_string(),
            req.label.trim().to_string(),
            req.api_key.trim().to_string(),
            encrypted_secret,
            encrypted_passphrase,
            req.is_simulated,
        );

        self.account_repo.create(&account).await?;

        Ok(account.to_response())
    }

    /// ดึงรายการ Accounts ทั้งหมดของ User
    pub async fn list_accounts(
        &self,
        user_id: &str,
    ) -> Result<Vec<AccountResponse>, AccountServiceError> {
        let accounts = self.account_repo.find_by_user_id(user_id).await?;
        Ok(accounts.into_iter().map(|a| a.to_response()).collect())
    }

    /// ดึงข้อมูล Account เดี่ยว (เฉพาะ Response ที่ Masked แล้ว)
    pub async fn get_account(
        &self,
        account_id: &str,
        user_id: &str,
    ) -> Result<AccountResponse, AccountServiceError> {
        let account = self
            .account_repo
            .find_by_id_and_user_id(account_id, user_id)
            .await?
            .ok_or(AccountServiceError::AccountNotFound)?;

        Ok(account.to_response())
    }

    /// ลบบัญชี OKX (เช็คว่าผู้ใช้เป็นเจ้าของบัญชีจริง)
    pub async fn delete_account(
        &self,
        account_id: &str,
        user_id: &str,
    ) -> Result<(), AccountServiceError> {
        let deleted = self
            .account_repo
            .delete_by_id_and_user_id(account_id, user_id)
            .await?;

        if !deleted {
            return Err(AccountServiceError::AccountNotFound);
        }

        Ok(())
    }

    /// ถอดรหัส Credentials สำหรับ Bot Engine (Internal Use)
    /// คืนค่า (api_key, decrypted_secret, decrypted_passphrase)
    pub async fn get_decrypted_credentials(
        &self,
        account_id: &str,
        user_id: &str,
    ) -> Result<(String, String, String), AccountServiceError> {
        let account = self
            .account_repo
            .find_by_id_and_user_id(account_id, user_id)
            .await?
            .ok_or(AccountServiceError::AccountNotFound)?;

        let secret = self
            .encryption_service
            .decrypt(&account.encrypted_secret)?;
        let passphrase = self
            .encryption_service
            .decrypt(&account.encrypted_passphrase)?;

        Ok((account.api_key, secret, passphrase))
    }
}
