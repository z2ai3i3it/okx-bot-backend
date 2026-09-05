use reqwest::{header::HeaderMap, Client};
use thiserror::Error;

use crate::okx::{
    dto::account::{AccountVerificationResult, OkxAccountBalanceData, OkxApiResponse},
    signer::OkxSigner,
};

#[derive(Debug, Error)]
pub enum OkxClientError {
    #[error("HTTP request error: {0}")]
    ReqwestError(#[from] reqwest::Error),

    #[error("Signer error: {0}")]
    SignerError(String),

    #[error("OKX API Error (code: {code}): {msg}")]
    ApiError { code: String, msg: String },

    #[error("JSON decode error: {0}")]
    JsonError(#[from] serde_json::Error),
}

#[derive(Clone)]
pub struct OkxRestClient {
    client: Client,
    live_base_url: String,
    demo_base_url: String,
}

impl Default for OkxRestClient {
    fn default() -> Self {
        Self::new()
    }
}

impl OkxRestClient {
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap_or_else(|_| Client::new()),
            live_base_url: "https://www.okx.com".to_string(),
            demo_base_url: "https://www.okx.com".to_string(),
        }
    }

    /// Helper ดึง Base URL ตามโหมด is_simulated
    fn get_base_url(&self, is_simulated: bool) -> &str {
        if is_simulated {
            &self.demo_base_url
        } else {
            &self.live_base_url
        }
    }

    /// ตรวจสอบความถูกต้องของ API Key + ดึง Asset Balance จาก OKX v5
    /// Endpoint: `GET /api/v5/account/balance`
    pub async fn get_balance(
        &self,
        api_key: &str,
        secret_key: &str,
        passphrase: &str,
        is_simulated: bool,
    ) -> Result<OkxAccountBalanceData, OkxClientError> {
        let request_path = "/api/v5/account/balance";
        let timestamp = OkxSigner::generate_timestamp();
        let method = "GET";

        let signature = OkxSigner::sign(&timestamp, method, request_path, None, secret_key)
            .map_err(OkxClientError::SignerError)?;

        let mut headers = HeaderMap::new();
        headers.insert("OK-ACCESS-KEY", api_key.parse().unwrap());
        headers.insert("OK-ACCESS-SIGN", signature.parse().unwrap());
        headers.insert("OK-ACCESS-TIMESTAMP", timestamp.parse().unwrap());
        headers.insert("OK-ACCESS-PASSPHRASE", passphrase.parse().unwrap());

        if is_simulated {
            headers.insert("x-simulated-trading", "1".parse().unwrap());
        }

        let url = format!("{}{}", self.get_base_url(is_simulated), request_path);

        let response = self
            .client
            .get(&url)
            .headers(headers)
            .send()
            .await?;

        let body_text = response.text().await?;
        let api_response: OkxApiResponse<OkxAccountBalanceData> = serde_json::from_str(&body_text)?;

        if api_response.code != "0" {
            return Err(OkxClientError::ApiError {
                code: api_response.code,
                msg: api_response.msg,
            });
        }

        let balance_data = api_response.data.into_iter().next().unwrap_or_else(|| {
            OkxAccountBalanceData {
                total_equity_usd: "0".to_string(),
                details: Vec::new(),
            }
        });

        Ok(balance_data)
    }

    /// Verify Credentials โดยทดสอบยิงไปดึง Balance หากสำเร็จจะคืนค่า VerificationResult
    pub async fn verify_credentials(
        &self,
        api_key: &str,
        secret_key: &str,
        passphrase: &str,
        is_simulated: bool,
    ) -> AccountVerificationResult {
        match self
            .get_balance(api_key, secret_key, passphrase, is_simulated)
            .await
        {
            Ok(balance) => AccountVerificationResult {
                is_valid: true,
                message: "OKX API Credentials verified successfully".to_string(),
                total_equity_usd: Some(balance.total_equity_usd),
                balances: balance.details,
            },
            Err(e) => AccountVerificationResult {
                is_valid: false,
                message: format!("Verification failed: {}", e),
                total_equity_usd: None,
                balances: Vec::new(),
            },
        }
    }
}
