use thiserror::Error;
use tokio::sync::mpsc;

use crate::okx::{
    dto::trade::{WsLoginArg, WsPlaceOrderArg},
    signer::OkxSigner,
};

#[derive(Debug, Error)]
pub enum WsTradeError {
    #[error("WebSocket connection error: {0}")]
    TungsteniteError(#[from] tokio_tungstenite::tungstenite::Error),

    #[error("Signer error: {0}")]
    SignerError(String),

    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("Authentication failed: {0}")]
    AuthFailed(String),
}

/// WebSocket Client สำหรับยิงคำสั่งซื้อขายความเร็วสูง (Low-Latency Trade)
#[derive(Clone)]
pub struct OkxWsTradeClient {
    ws_url: String,
    order_tx: mpsc::Sender<WsPlaceOrderArg>,
}

impl OkxWsTradeClient {
    /// สร้าง Trade Client เชื่อมต่อไปยัง Private WS
    pub fn new() -> (Self, mpsc::Receiver<WsPlaceOrderArg>) {
        let (order_tx, order_rx) = mpsc::channel(100);
        let client = Self {
            ws_url: "wss://ws.okx.com:8443/ws/v5/private".to_string(),
            order_tx,
        };
        (client, order_rx)
    }

    pub fn ws_url(&self) -> &str {
        &self.ws_url
    }

    /// Helper สร้าง Login Payload
    pub fn create_login_payload(
        api_key: &str,
        secret_key: &str,
        passphrase: &str,
    ) -> Result<WsLoginArg, WsTradeError> {
        let timestamp = OkxSigner::generate_timestamp();
        let method = "GET";
        let request_path = "/users/self/verify";

        let sign = OkxSigner::sign(&timestamp, method, request_path, None, secret_key)
            .map_err(WsTradeError::SignerError)?;

        Ok(WsLoginArg {
            api_key: api_key.to_string(),
            passphrase: passphrase.to_string(),
            timestamp,
            sign,
        })
    }

    /// ส่ง Order เข้า Channel เพื่อให้ Dispatcher ยิงออกไป
    pub async fn dispatch_order(&self, order: WsPlaceOrderArg) -> Result<(), WsTradeError> {
        self.order_tx
            .send(order)
            .await
            .map_err(|_| WsTradeError::AuthFailed("Failed to queue order".to_string()))?;
        Ok(())
    }
}
