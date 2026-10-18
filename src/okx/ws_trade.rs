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

/// Item-level ack ของ trading op ที่ OKX ตอบกลับ (sCode != 0)
/// ใช้ให้ CEM reconcile ได้ทันที เช่น cancel ได้ 51400/51603 = order ไม่มีอยู่บน exchange แล้ว
/// (โดยเฉพาะ order ที่ place ถูก reject ไปก่อน -> ไม่มี terminal event มาเอง ต้องใช้ ack นี้)
#[derive(Debug, Clone)]
pub struct WsOpAck {
    pub op: String,
    pub cl_ord_id: String,
    pub s_code: String,
    pub s_msg: String,
}

/// WebSocket Client สำหรับยิงคำสั่งซื้อขายความเร็วสูงและรับ Private Streams
#[derive(Clone, Debug)]
pub struct OkxWsTradeClient {
    ws_url: String,
    trading_op_tx: mpsc::Sender<WsTradingOp>,

    // Broadcast channels สำหรับ 3 Private Streams
    orders_sender: broadcast::Sender<OkxOrderUpdateData>,
    my_trades_sender: broadcast::Sender<OkxMyTradeData>,
    balance_sender: broadcast::Sender<OkxAccountBalanceUpdate>,
    /// Item-level failures ของ trading ops (เช่น cancel 51400) ให้ CEM/executor reconcile
    op_ack_sender: broadcast::Sender<WsOpAck>,
}

impl OkxWsTradeClient {
    /// สร้าง Trade Client
    /// `is_simulated` = true → ใช้ OKX Demo Trading URL
    pub fn new(is_simulated: bool) -> (Self, mpsc::Receiver<WsTradingOp>) {
        let (trading_op_tx, trading_op_rx) = mpsc::channel(1000);
        let (orders_sender, _) = broadcast::channel(1024);
        let (my_trades_sender, _) = broadcast::channel(1024);
        let (balance_sender, _) = broadcast::channel(512);
        let (op_ack_sender, _) = broadcast::channel(256);

        let ws_url = if is_simulated {
            "wss://wspap.okx.com:8443/ws/v5/private?brokerId=9999".to_string()
        } else {
            "wss://ws.okx.com:8443/ws/v5/private".to_string()
        };

        let client = Self {
            ws_url,
            trading_op_tx,
            orders_sender,
            my_trades_sender,
            balance_sender,
            op_ack_sender,
        };

        (client, trading_op_rx)
    }

    pub fn ws_url(&self) -> &str {
        &self.ws_url
    }

    // ==========================================
    // 🔐 Subscriptions สำหรับ 3 Private Streams
    // ==========================================

    /// ฟัง Order Update สด (Orders stream)
    pub fn subscribe_orders(&self) -> broadcast::Receiver<OkxOrderUpdateData> {
        self.orders_sender.subscribe()
    }

    /// ฟัง Execution Fill สด (My Trades stream)
    pub fn subscribe_my_trades(&self) -> broadcast::Receiver<OkxMyTradeData> {
        self.my_trades_sender.subscribe()
    }

    /// ฟัง Balance & Margin สด (Balance stream)
    pub fn subscribe_balance(&self) -> broadcast::Receiver<OkxAccountBalanceUpdate> {
        self.balance_sender.subscribe()
    }

    /// ฟัง Item-level ack ของ trading ops (เช่น cancel ที่ได้ 51400 = order ไม่มีอยู่แล้ว)
    pub fn subscribe_op_acks(&self) -> broadcast::Receiver<WsOpAck> {
        self.op_ack_sender.subscribe()
    }

    // ==========================================
    // 🔧 5 Trading Operations (Non-blocking Dispatch)
    // ==========================================

    pub async fn create_order_ws(&self, order: WsPlaceOrderArg) -> Result<(), WsTradeError> {
        self.trading_op_tx
            .send(WsTradingOp::CreateOrder(order))
            .await
            .map_err(|e| WsTradeError::QueueError(e.to_string()))
    }

    pub async fn create_orders_ws(&self, orders: Vec<WsPlaceOrderArg>) -> Result<(), WsTradeError> {
        self.trading_op_tx
            .send(WsTradingOp::BatchCreateOrders(orders))
            .await
            .map_err(|e| WsTradeError::QueueError(e.to_string()))
    }

    pub async fn edit_order_ws(&self, amend: WsAmendOrderArg) -> Result<(), WsTradeError> {
        self.trading_op_tx
            .send(WsTradingOp::EditOrder(amend))
            .await
            .map_err(|e| WsTradeError::QueueError(e.to_string()))
    }

    pub async fn cancel_order_ws(&self, cancel: WsCancelOrderArg) -> Result<(), WsTradeError> {
        self.trading_op_tx
            .send(WsTradingOp::CancelOrder(cancel))
            .await
            .map_err(|e| WsTradeError::QueueError(e.to_string()))
    }

    pub async fn cancel_orders_ws(&self, cancels: Vec<WsCancelOrderArg>) -> Result<(), WsTradeError> {
        self.trading_op_tx
            .send(WsTradingOp::BatchCancelOrders(cancels))
            .await
            .map_err(|e| WsTradeError::QueueError(e.to_string()))
    }

    /// Helper สร้าง Login Payload ด้วย HMAC-SHA256
    /// ⚠️ WebSocket Login ต้องใช้ Unix timestamp (seconds) ไม่ใช่ ISO 8601 เหมือน REST
    pub fn create_login_payload(
        api_key: &str,
        secret_key: &str,
        passphrase: &str,
    ) -> Result<WsLoginArg, WsTradeError> {
        // WS Login ต้องใช้ Unix seconds ไม่ใช่ ISO 8601
        let timestamp = OkxSigner::generate_unix_timestamp();
        let sign = OkxSigner::sign(&timestamp, "GET", "/users/self/verify", None, secret_key)
            .map_err(WsTradeError::SignerError)?;

        Ok(WsLoginArg {
            api_key: api_key.to_string(),
            passphrase: passphrase.to_string(),
            timestamp,
            sign,
        })
    }

    /// 🚀 สตาร์ท Background Task เชื่อมต่อ Private WebSocket
    /// - Login ด้วย API credentials
    /// - Subscribe: orders, fills channels
    /// - Broadcast events กลับไปยัง subscribers
    /// - อ่าน trading ops จาก channel แล้วยิงไปยัง OKX
    pub fn start(
        &self,
        mut trading_op_rx: mpsc::Receiver<WsTradingOp>,
        api_key: String,
        secret_key: String,
        passphrase: String,
    ) {
        use futures_util::{SinkExt, StreamExt};
        use serde_json::{json, Value};
        use std::time::Duration;
        use tokio::time::{interval, sleep};
        use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};

        let ws_url = self.ws_url.clone();
        let orders_sender = self.orders_sender.clone();
        let my_trades_sender = self.my_trades_sender.clone();
        let balance_sender = self.balance_sender.clone();
        let op_ack_sender = self.op_ack_sender.clone();

        tokio::spawn(async move {
            let mut backoff_sec = 1u64;

            loop {
                log::info!("Connecting to OKX Private WebSocket: {}", ws_url);

                let conn = connect_async(&ws_url).await;
                match conn {
                    Err(e) => {
                        log::error!("Failed to connect to OKX Private WS: {}. Retrying in {}s...", e, backoff_sec);
                        sleep(Duration::from_secs(backoff_sec)).await;
                        backoff_sec = (backoff_sec * 2).min(30);
                        continue;
                    }
                    Ok((ws_stream, _)) => {
                        log::info!("Connected to OKX Private WebSocket successfully");
                        backoff_sec = 1;

                        let (mut write, mut read) = ws_stream.split();

                        // --- Step 1: Login ---
                        let login_result = Self::create_login_payload(&api_key, &secret_key, &passphrase);
                        match login_result {
                            Err(e) => {
                                log::error!("Failed to build OKX WS login payload: {}", e);
                                sleep(Duration::from_secs(5)).await;
                                continue;
                            }
                            Ok(login_payload) => {
                                let login_msg = json!({ "op": "login", "args": [login_payload] });
                                if let Ok(s) = serde_json::to_string(&login_msg) {
                                    let _ = write.send(Message::Text(s.into())).await;
                                }
                            }
                        }

                        // --- Step 2: Wait for login ack ---
                        let mut login_ok = false;
                        'login_wait: loop {
                            match read.next().await {
                                Some(Ok(Message::Text(text))) => {
                                    if let Ok(val) = serde_json::from_str::<Value>(&text) {
                                        if val.get("event").and_then(|v| v.as_str()) == Some("login") {
                                            let code = val.get("code").and_then(|v| v.as_str()).unwrap_or("");
                                            if code == "0" {
                                                log::info!("OKX Private WS Login success ✅");
                                                login_ok = true;
                                            } else {
                                                let msg = val.get("msg").and_then(|v| v.as_str()).unwrap_or("unknown");
                                                log::error!("OKX Private WS Login FAILED: code={} msg={}", code, msg);
                                            }
                                            break 'login_wait;
                                        }
                                    }
                                }
                                Some(Ok(Message::Close(_))) | None => break 'login_wait,
                                _ => {}
                            }
                        }

                        if !login_ok {
                            sleep(Duration::from_secs(backoff_sec)).await;
                            backoff_sec = (backoff_sec * 2).min(30);
                            continue;
                        }

                        // --- Step 3: Subscribe private channels ---
                        // หมายเหตุ: `fills` channel ต้องการ VIP tier จึงตัดออก
                        let sub_msg = json!({
                            "op": "subscribe",
                            "args": [
                                { "channel": "orders", "instType": "SPOT" },
                                { "channel": "account" }
                            ]
                        });
                        if let Ok(s) = serde_json::to_string(&sub_msg) {
                            let _ = write.send(Message::Text(s.into())).await;
                        }
                        log::info!("OKX Private WS: subscribed to orders and account channels");

                        // --- Step 4: Main event loop ---
                        let mut ping_interval = interval(Duration::from_secs(20));
                        let should_reconnect;

                        loop {
                            tokio::select! {
                                _ = ping_interval.tick() => {
                                    if let Err(e) = write.send(Message::Text("ping".into())).await {
                                        log::warn!("OKX Private WS ping failed: {}", e);
                                        should_reconnect = true;
                                        break;
                                    }
                                }

                                op = trading_op_rx.recv() => {
                                    match op {
                                        None => {
                                            log::info!("OKX Private WS: trading op channel closed — bot stopped");
                                            return; // clean exit, no reconnect
                                        }
                                        Some(trading_op) => {
                                            // OKX WS id: alphanumeric เท่านั้น ไม่มี "-", max 32 chars
                                            let req_id = format!("{:032x}", uuid::Uuid::new_v4().as_u128());
                                            let payload = match &trading_op {
                                                WsTradingOp::CreateOrder(arg) =>
                                                    json!({ "id": req_id, "op": "order", "args": [arg] }),
                                                WsTradingOp::BatchCreateOrders(args) =>
                                                    json!({ "id": req_id, "op": "batch-orders", "args": args }),
                                                WsTradingOp::EditOrder(arg) =>
                                                    json!({ "id": req_id, "op": "amend-order", "args": [arg] }),
                                                WsTradingOp::CancelOrder(arg) =>
                                                    json!({ "id": req_id, "op": "cancel-order", "args": [arg] }),
                                                WsTradingOp::BatchCancelOrders(args) =>
                                                    json!({ "id": req_id, "op": "batch-cancel-orders", "args": args }),
                                            };

                                            match serde_json::to_string(&payload) {
                                                Ok(json_str) => {
                                                    log::debug!("OKX Private WS → Sending: {}", json_str);
                                                    if let Err(e) = write.send(Message::Text(json_str.into())).await {
                                                        log::error!("OKX Private WS send failed: {}", e);
                                                        should_reconnect = true;
                                                        break;
                                                    }
                                                }
                                                Err(e) => log::error!("Failed to serialize trading op: {}", e),
                                            }
                                        }
                                    }
                                }

                                msg = read.next() => {
                                    match msg {
                                        Some(Ok(Message::Text(text))) => {
                                            if text == "pong" { continue; }
                                            if let Ok(val) = serde_json::from_str::<Value>(&text) {
                                                Self::handle_private_message(
                                                    &val, &orders_sender, &my_trades_sender, &balance_sender, &op_ack_sender
                                                );
                                            }
                                        }
                                        Some(Ok(Message::Close(_))) => {
                                            log::warn!("OKX Private WS: server closed connection");
                                            should_reconnect = true;
                                            break;
                                        }
                                        Some(Err(e)) => {
                                            log::error!("OKX Private WS read error: {}", e);
                                            should_reconnect = true;
                                            break;
                                        }
                                        None => {
                                            log::warn!("OKX Private WS stream ended");
                                            should_reconnect = true;
                                            break;
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }

                        if !should_reconnect {
                            return;
                        }
                    }
                }

                sleep(Duration::from_secs(backoff_sec)).await;
                backoff_sec = (backoff_sec * 2).min(30);
            }
        });
    }

    /// แยกประมวลผล events จาก Private WS แล้ว broadcast ไปยัง receivers
    fn handle_private_message(
        val: &serde_json::Value,
        orders_sender: &broadcast::Sender<OkxOrderUpdateData>,
        my_trades_sender: &broadcast::Sender<OkxMyTradeData>,
        balance_sender: &broadcast::Sender<OkxAccountBalanceUpdate>,
        op_ack_sender: &broadcast::Sender<WsOpAck>,
    ) {
        let channel = val
            .get("arg")
            .and_then(|a| a.get("channel"))
            .and_then(|c| c.as_str())
            .unwrap_or("");

        if channel == "orders" {
            log::debug!("📨 OKX raw orders payload: {}", val);
            if let Ok(data) = serde_json::from_value::<Vec<OkxOrderUpdateData>>(val["data"].clone()) {
                for order in data {
                    // debug เพราะทุก bot เปิด private WS แยกกันบน account เดียวกัน -> event เดียวกัน
                    // ถูก log ซ้ำ N เท่าของจำนวน connection (log ของเจ้าของ order ยังมีที่ strategy ระดับ INFO)
                    log::debug!(
                        "📋 Order Update: {} {} {} @ {} → state={}",
                        order.inst_id, order.side, order.sz, order.px, order.state
                    );
                    let _ = orders_sender.send(order);
                }
            }
        } else if channel == "fills" {
            if let Ok(data) = serde_json::from_value::<Vec<OkxMyTradeData>>(val["data"].clone()) {
                for fill in data {
                    log::info!(
                        "✅ Fill: {} {} {} @ {}",
                        fill.inst_id, fill.side, fill.fill_size, fill.fill_price
                    );
                    let _ = my_trades_sender.send(fill);
                }
            }
        } else if channel == "account" || channel == "balance_and_position" {
            if let Ok(data) = serde_json::from_value::<Vec<OkxAccountBalanceUpdate>>(val["data"].clone()) {
                for balance in data {
                    let _ = balance_sender.send(balance);
                }
            }
        } else if let Some(event) = val.get("event") {
            if event.as_str() == Some("error") {
                let code = val.get("code").and_then(|v| v.as_str()).unwrap_or("");
                let msg = val.get("msg").and_then(|v| v.as_str()).unwrap_or("");
                log::error!("OKX Private WS error event: code={} msg={}", code, msg);
            } else {
                log::debug!("OKX Private WS event: {:?}", event);
            }
        } else if let Some(op) = val.get("op") {
            // Trade op ack (order/cancel result)
            let code = val.get("code").and_then(|v| v.as_str()).unwrap_or("");
            let msg = val.get("msg").and_then(|v| v.as_str()).unwrap_or("");

            // Item-level failures (sCode != 0) -> broadcast ack ให้ subscriber (เช่น CEM ใช้ reconcile
            // cancel 51400 = order ไม่มีอยู่แล้ว) — ไม่ว่า op-level code จะเป็น 0 หรือไม่
            // (เคสจริง zzz.log 03:00:00.637 cancel 51400 มาพร้อม op-level code=1 -> ถ้ารอเฉพาะ code==0 จะตกหล่น)
            if let Some(data_arr) = val.get("data").and_then(|d| d.as_array()) {
                for item in data_arr {
                    let s_code = item.get("sCode").and_then(|c| c.as_str()).unwrap_or("0");
                    if s_code != "0" {
                        let _ = op_ack_sender.send(WsOpAck {
                            op: op.as_str().unwrap_or("?").to_string(),
                            cl_ord_id: item
                                .get("clOrdId")
                                .and_then(|c| c.as_str())
                                .unwrap_or("-")
                                .to_string(),
                            s_code: s_code.to_string(),
                            s_msg: item
                                .get("sMsg")
                                .and_then(|m| m.as_str())
                                .unwrap_or("")
                                .to_string(),
                        });
                    }
                }
            }
            if code == "0" {
                // เช็คว่าใน data array มี sCode != "0" หรือไม่
                if let Some(data_arr) = val.get("data").and_then(|d| d.as_array()) {
                    for item in data_arr {
                        let s_code = item.get("sCode").and_then(|c| c.as_str()).unwrap_or("0");
                        let s_msg = item.get("sMsg").and_then(|m| m.as_str()).unwrap_or("");
                        let cl_ord_id = item.get("clOrdId").and_then(|c| c.as_str()).unwrap_or("-");
                        if s_code != "0" {
                            let op_name = op.as_str().unwrap_or("?");
                            // Cancel ที่ได้ 51400/51603 = order ไม่มีอยู่แล้วบน exchange (terminal ไปก่อน)
                            // เป็น flow ปกติของ recovery กวาด order เก่า / preempt cancel ที่ซ้อนกับ terminal event
                            // -> ไม่ใช่ error (แต่ถ้าเป็น fill/partial-fill ที่ผิดพลาดต้องยังเป็น error เหมือนเดิม)
                            if op_name.contains("cancel") && matches!(s_code, "51400" | "51603") {
                                log::info!(
                                    "ℹ️ OKX WS op='{}' order already gone: clOrdId={} code={}",
                                    op_name, cl_ord_id, s_code
                                );
                            } else {
                                log::error!("❌ OKX WS op='{}' item error: clOrdId={} code={} msg='{}'", op_name, cl_ord_id, s_code, s_msg);
                            }
                        } else {
                            log::info!("✅ OKX WS op='{}' success: clOrdId={}", op.as_str().unwrap_or("?"), cl_ord_id);
                        }
                    }
                } else {
                    log::info!("✅ OKX WS op='{}' success", op.as_str().unwrap_or("?"));
                }
            } else {
                // op-level code != 0 แต่ item-level บอก cancel 51400/51603 ทุกตัว
                // = order ไม่มีอยู่บน exchange แล้ว (race กับ terminal event / recovery กวาด order เก่า)
                // -> flow ปกติ -> WARN สั้น ไม่พิมพ์ JSON dump (กัน log ท่วม เคส zzz.log มี 13 ERROR/90 วินาที)
                let op_name = op.as_str().unwrap_or("?");
                let data = val.get("data").and_then(|d| d.as_array());
                let benign_cancel_gone = op_name.contains("cancel")
                    && data.map_or(false, |items| {
                        !items.is_empty()
                            && items.iter().all(|item| {
                                matches!(
                                    item.get("sCode").and_then(|c| c.as_str()),
                                    Some("51400" | "51603")
                                )
                            })
                    });
                if benign_cancel_gone {
                    let items = data.expect("benign_cancel_gone implies data exists");
                    let cl_ord_id = items[0].get("clOrdId").and_then(|c| c.as_str()).unwrap_or("-");
                    let s_code = items[0].get("sCode").and_then(|c| c.as_str()).unwrap_or("?");
                    log::warn!(
                        "⚠️ OKX WS op='{}' order already gone: clOrdId={} code={} (benign — order terminal ไปก่อนหน้านี้แล้ว)",
                        op_name, cl_ord_id, s_code
                    );
                } else {
                    log::error!("❌ OKX WS op='{}' failed: {} (code={}) full={:?}", op_name, msg, code, val);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ยืนยันว่า item-level failure (sCode != 0) ถูก broadcast เป็น WsOpAck ทั้งสองเคส:
    /// - op-level code=1 พร้อม item sCode != 0 (รูปแบบจริงจาก zzz.log 03:00:00.637 cancel 51400)
    /// - op-level code=0 แต่ item sCode != 0
    /// และ sCode=0 ห้ามส่ง ack
    #[test]
    fn handle_private_message_broadcasts_item_level_op_acks() {
        let (orders_sender, _rx) = broadcast::channel(16);
        let (trades_sender, _rx) = broadcast::channel(16);
        let (balance_sender, _rx) = broadcast::channel(16);
        let (ack_sender, mut ack_rx) = broadcast::channel(16);

        // เคส 1: op-level code=1 + item sCode=51400 (รูปแบบจริงจาก OKX demo)
        let val: serde_json::Value = serde_json::json!({
            "id": "1d5d23e9a51e4d119d4da7c6ecaad174",
            "op": "cancel-order",
            "code": "1",
            "msg": "",
            "data": [{
                "ordId": "",
                "clOrdId": "BOTf98ec14c8439470cb06eeebbce3ca",
                "sCode": "51400",
                "sMsg": "Order cancellation failed as the order has been filled, canceled or does not exist."
            }]
        });
        OkxWsTradeClient::handle_private_message(
            &val, &orders_sender, &trades_sender, &balance_sender, &ack_sender,
        );
        let ack = ack_rx.try_recv().expect("ต้องได้ ack จาก op-level code=1");
        assert_eq!(ack.op, "cancel-order");
        assert_eq!(ack.cl_ord_id, "BOTf98ec14c8439470cb06eeebbce3ca");
        assert_eq!(ack.s_code, "51400");

        // เคส 2: op-level code=0 แต่ item sCode != 0
        let val2: serde_json::Value = serde_json::json!({
            "id": "abc",
            "op": "cancel-order",
            "code": "0",
            "data": [{ "clOrdId": "BOT-cancel-2", "sCode": "51603", "sMsg": "Order does not exist" }]
        });
        OkxWsTradeClient::handle_private_message(
            &val2, &orders_sender, &trades_sender, &balance_sender, &ack_sender,
        );
        let ack2 = ack_rx.try_recv().expect("ต้องได้ ack จาก op-level code=0");
        assert_eq!(ack2.cl_ord_id, "BOT-cancel-2");
        assert_eq!(ack2.s_code, "51603");

        // เคส 3: place ถูกปฏิเสธหลัง dispatch (op='order', sCode=51008 — payload จริงจาก
        // app_2026-10-13_03-55: BOT4d81... insufficient SUI balance) ต้อง broadcast ack ด้วย
        // (ไม่งั้น executor ไม่มีทางคืน lock / ล้าง dedup -> order ผี + ลูป reactivate)
        let val4: serde_json::Value = serde_json::json!({
            "id": "694b414aa5674cf1bda4da3866f8a7ca",
            "op": "order",
            "code": "1",
            "msg": "",
            "data": [{
                "ordId": "",
                "clOrdId": "BOT4d81b438a42342219e3059fc79fe9",
                "sCode": "51008",
                "sMsg": "Order failed. Insufficient SUI balance in account.",
                "subCode": "1000"
            }]
        });
        OkxWsTradeClient::handle_private_message(
            &val4, &orders_sender, &trades_sender, &balance_sender, &ack_sender,
        );
        let ack4 = ack_rx.try_recv().expect("place reject (op=order) ต้อง broadcast ack");
        assert_eq!(ack4.op, "order");
        assert_eq!(ack4.cl_ord_id, "BOT4d81b438a42342219e3059fc79fe9");
        assert_eq!(ack4.s_code, "51008");

        // success item (sCode=0) ห้าม broadcast ack
        let val3: serde_json::Value = serde_json::json!({
            "id": "def",
            "op": "cancel-order",
            "code": "0",
            "data": [{ "clOrdId": "BOT-ok", "sCode": "0", "sMsg": "" }]
        });
        OkxWsTradeClient::handle_private_message(
            &val3, &orders_sender, &trades_sender, &balance_sender, &ack_sender,
        );
        assert!(ack_rx.try_recv().is_err(), "sCode=0 ห้ามส่ง ack");
    }
}
