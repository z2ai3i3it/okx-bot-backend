use reqwest::{Client, header::HeaderMap};
use serde::de::DeserializeOwned;
use std::collections::HashSet;
use thiserror::Error;

use crate::okx::{
    dto::account::{AccountVerificationResult, OkxAccountBalanceData, OkxApiResponse},
    dto::trade::{OkxMyTradeData, OkxOrderUpdateData},
    signer::OkxSigner,
};

#[derive(Clone)]
pub struct OkxRestCredentials {
    pub api_key: String,
    pub secret_key: String,
    pub passphrase: String,
    pub is_simulated: bool,
}

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

    async fn signed_get<T: DeserializeOwned + Default>(
        &self,
        credentials: &OkxRestCredentials,
        request_path: &str,
    ) -> Result<Vec<T>, OkxClientError> {
        let timestamp = OkxSigner::generate_timestamp();
        let signature = OkxSigner::sign(
            &timestamp,
            "GET",
            request_path,
            None,
            &credentials.secret_key,
        )
        .map_err(OkxClientError::SignerError)?;

        let mut headers = HeaderMap::new();
        headers.insert("OK-ACCESS-KEY", credentials.api_key.parse().unwrap());
        headers.insert("OK-ACCESS-SIGN", signature.parse().unwrap());
        headers.insert("OK-ACCESS-TIMESTAMP", timestamp.parse().unwrap());
        headers.insert(
            "OK-ACCESS-PASSPHRASE",
            credentials.passphrase.parse().unwrap(),
        );
        if credentials.is_simulated {
            headers.insert("x-simulated-trading", "1".parse().unwrap());
        }

        let url = format!(
            "{}{}",
            self.get_base_url(credentials.is_simulated),
            request_path
        );
        let body = self
            .client
            .get(url)
            .headers(headers)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        let response: OkxApiResponse<T> = serde_json::from_str(&body)?;
        if response.code != "0" {
            return Err(OkxClientError::ApiError {
                code: response.code,
                msg: response.msg,
            });
        }

        Ok(response.data)
    }

    pub async fn get_order_detail(
        &self,
        credentials: &OkxRestCredentials,
        inst_id: &str,
        client_order_id: &str,
    ) -> Result<OkxOrderUpdateData, OkxClientError> {
        let request_path =
            format!("/api/v5/trade/order?instId={inst_id}&clOrdId={client_order_id}");
        self.signed_get::<OkxOrderUpdateData>(credentials, &request_path)
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| OkxClientError::ApiError {
                code: "empty_data".to_string(),
                msg: format!("No order returned for clOrdId {client_order_id}"),
            })
    }

    pub async fn get_order_fills(
        &self,
        credentials: &OkxRestCredentials,
        inst_id: &str,
        order_id: &str,
    ) -> Result<Vec<OkxMyTradeData>, OkxClientError> {
        const PAGE_SIZE: usize = 100;
        const MAX_PAGES: usize = 100;

        let mut fills = Vec::new();
        let mut seen_trade_ids = HashSet::new();
        let mut after: Option<String> = None;

        for _ in 0..MAX_PAGES {
            let mut request_path =
                format!("/api/v5/trade/fills?instId={inst_id}&ordId={order_id}&limit={PAGE_SIZE}");
            if let Some(cursor) = &after {
                request_path.push_str("&after=");
                request_path.push_str(cursor);
            }

            let page: Vec<OkxMyTradeData> = self.signed_get(credentials, &request_path).await?;
            let page_len = page.len();
            if page_len == 0 {
                break;
            }

            let next_cursor = page
                .last()
                .map(|fill| fill.trade_id.clone())
                .unwrap_or_default();
            for fill in page {
                if !fill.trade_id.is_empty() && seen_trade_ids.insert(fill.trade_id.clone()) {
                    fills.push(fill);
                }
            }

            if page_len < PAGE_SIZE
                || next_cursor.is_empty()
                || after.as_deref() == Some(&next_cursor)
            {
                break;
            }
            after = Some(next_cursor);
        }

        Ok(fills)
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

        let response = self.client.get(&url).headers(headers).send().await?;

        let body_text = response.text().await?;
        let api_response: OkxApiResponse<OkxAccountBalanceData> = serde_json::from_str(&body_text)?;

        if api_response.code != "0" {
            return Err(OkxClientError::ApiError {
                code: api_response.code,
                msg: api_response.msg,
            });
        }

        let balance_data =
            api_response
                .data
                .into_iter()
                .next()
                .unwrap_or_else(|| OkxAccountBalanceData {
                    total_equity_usd: "0".to_string(),
                    details: Vec::new(),
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

    pub async fn get_instrument_details(
        &self,
        credentials: &OkxRestCredentials,
        inst_type: &str,
        inst_id: &str,
    ) -> Result<serde_json::Value, OkxClientError> {
        let request_path =
            format!("/api/v5/public/instruments?instType={inst_type}&instId={inst_id}");
        let response_data: Vec<serde_json::Value> =
            self.signed_get(credentials, &request_path).await?;
        response_data
            .into_iter()
            .next()
            .ok_or_else(|| OkxClientError::ApiError {
                code: "empty_data".to_string(),
                msg: format!("No instrument details returned for instId {inst_id}"),
            })
    }

    pub async fn get_instrument_code(
        &self,
        credentials: &OkxRestCredentials,
        inst_type: &str,
        inst_id: &str,
    ) -> Result<u64, OkxClientError> {
        let request_path =
            format!("/api/v5/public/instruments?instType={inst_type}&instId={inst_id}");

        let response_data: Vec<serde_json::Value> =
            self.signed_get(credentials, &request_path).await?;

        // หาแถวที่ instId ตรงกับที่ขอ (กันกรณี API คืนหลายแถว)
        let details = response_data
            .into_iter()
            .find(|item| item.get("instId").and_then(|v| v.as_str()) == Some(inst_id))
            .ok_or_else(|| OkxClientError::ApiError {
                code: "empty_data".to_string(),
                msg: format!("No instrument details returned for instId {inst_id}"),
            })?;

        // ต้อง live เท่านั้น
        let state = details.get("state").and_then(|v| v.as_str()).unwrap_or("");
        if state != "live" {
            return Err(OkxClientError::ApiError {
                code: "instrument_not_live".to_string(),
                msg: format!("Instrument {inst_id} state is '{state}', expected 'live'"),
            });
        }

        // instIdCode อาจเป็น number หรือ string รองรับทั้งสองแบบ
        let code = details
            .get("instIdCode")
            .and_then(|v| {
                v.as_u64()
                    .or_else(|| v.as_str().and_then(|s| s.parse::<u64>().ok()))
            })
            .ok_or_else(|| OkxClientError::ApiError {
                code: "missing_inst_id_code".to_string(),
                msg: format!("instIdCode missing or invalid for instId {inst_id}"),
            })?;

        Ok(code)
    }
}
