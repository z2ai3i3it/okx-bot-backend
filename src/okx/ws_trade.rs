use thiserror::Error;
use tokio::sync::{broadcast, mpsc};

use crate::okx::{
    dto::trade::{
        OkxAccountBalanceUpdate, OkxMyTradeData, OkxOrderUpdateData, WsAmendOrderArg,
        WsCancelOrderArg, WsLoginArg, WsPlaceOrderArg,
    },
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

    #[error("Queue send error: {0}")]
    QueueError(String),

    #[error("Authentication failed: {0}")]
    AuthFailed(String),
}

/// คำสั่ง Trading Action ที่ส่งผ่าน WebSocket Trade API
#[derive(Debug, Clone)]
pub enum WsTradingOp {
    /// 1. Create Order (Single)
    CreateOrder(WsPlaceOrderArg),
    /// 2. Batch Create Orders
    BatchCreateOrders(Vec<WsPlaceOrderArg>),
    /// 3. Edit Order
    EditOrder(WsAmendOrderArg),
    /// 4. Cancel Order (Single)
    CancelOrder(WsCancelOrderArg),
    /// 5. Batch Cancel Orders
    BatchCancelOrders(Vec<WsCancelOrderArg>),
}

/// WebSocket Client สำหรับยิงคำสั่งซื้อขายความเร็วสูงและรับ Private Streams
#[derive(Clone)]
pub struct OkxWsTradeClient {
    ws_url: String,
    trading_op_tx: mpsc::Sender<WsTradingOp>,

    // Broadcast channels สำหรับ 3 Private Streams
    orders_sender: broadcast::Sender<OkxOrderUpdateData>,
    my_trades_sender: broadcast::Sender<OkxMyTradeData>,
    balance_sender: broadcast::Sender<OkxAccountBalanceUpdate>,
}

impl OkxWsTradeClient {
    /// สร้าง Trade Client เชื่อมต่อไปยัง Private WS
    pub fn new() -> (Self, mpsc::Receiver<WsTradingOp>) {
        let (trading_op_tx, trading_op_rx) = mpsc::channel(1000);
        let (orders_sender, _) = broadcast::channel(1024);
        let (my_trades_sender, _) = broadcast::channel(1024);
        let (balance_sender, _) = broadcast::channel(512);

        let client = Self {
            ws_url: "wss://ws.okx.com:8443/ws/v5/private".to_string(),
            trading_op_tx,
            orders_sender,
            my_trades_sender,
            balance_sender,
        };

        (client, trading_op_rx)
    }

    pub fn ws_url(&self) -> &str {
        &self.ws_url
    }

    // ==========================================
    // 🔐 Subscriptions สำหรับ 3 Private Streams
    // ==========================================

    /// 1. ฟัง Order Update สด (Orders stream)
    pub fn subscribe_orders(&self) -> broadcast::Receiver<OkxOrderUpdateData> {
        self.orders_sender.subscribe()
    }

    /// 2. ฟัง Execution Fill สด (My Trades stream)
    pub fn subscribe_my_trades(&self) -> broadcast::Receiver<OkxMyTradeData> {
        self.my_trades_sender.subscribe()
    }

    /// 3. ฟัง Balance & Margin สด (Balance stream)
    pub fn subscribe_balance(&self) -> broadcast::Receiver<OkxAccountBalanceUpdate> {
        self.balance_sender.subscribe()
    }

    // ==========================================
    // 🔧 5 Trading Operations (Non-blocking Dispatch)
    // ==========================================

    /// 1. Create Order (Single)
    pub async fn create_order_ws(&self, order: WsPlaceOrderArg) -> Result<(), WsTradeError> {
        self.trading_op_tx
            .send(WsTradingOp::CreateOrder(order))
            .await
            .map_err(|e| WsTradeError::QueueError(e.to_string()))
    }

    /// 2. Batch Create Orders (สูงสุด 20 ไม้)
    pub async fn create_orders_ws(&self, orders: Vec<WsPlaceOrderArg>) -> Result<(), WsTradeError> {
        self.trading_op_tx
            .send(WsTradingOp::BatchCreateOrders(orders))
            .await
            .map_err(|e| WsTradeError::QueueError(e.to_string()))
    }

    /// 3. Edit Order
    pub async fn edit_order_ws(&self, amend: WsAmendOrderArg) -> Result<(), WsTradeError> {
        self.trading_op_tx
            .send(WsTradingOp::EditOrder(amend))
            .await
            .map_err(|e| WsTradeError::QueueError(e.to_string()))
    }

    /// 4. Cancel Order (Single)
    pub async fn cancel_order_ws(&self, cancel: WsCancelOrderArg) -> Result<(), WsTradeError> {
        self.trading_op_tx
            .send(WsTradingOp::CancelOrder(cancel))
            .await
            .map_err(|e| WsTradeError::QueueError(e.to_string()))
    }

    /// 5. Batch Cancel Orders
    pub async fn cancel_orders_ws(&self, cancels: Vec<WsCancelOrderArg>) -> Result<(), WsTradeError> {
        self.trading_op_tx
            .send(WsTradingOp::BatchCancelOrders(cancels))
            .await
            .map_err(|e| WsTradeError::QueueError(e.to_string()))
    }

    /// Helper สร้าง Login Payload ด้วย HMAC-SHA256
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
}
