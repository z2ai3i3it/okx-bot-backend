use serde::{Deserialize, Serialize};

/// Payload สำหรับ Login เข้า Private WebSocket
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsLoginArg {
    #[serde(rename = "apiKey")]
    pub api_key: String,
    pub passphrase: String,
    pub timestamp: String,
    pub sign: String,
}

/// คำสั่งยิง Place Order ผ่าน WebSocket Trade API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsPlaceOrderArg {
    pub id: Option<String>,
    #[serde(rename = "instId")]
    pub inst_id: String,
    #[serde(rename = "tdMode")]
    pub td_mode: String, // "cash", "cross", "isolated"
    pub side: String,   // "buy", "sell"
    #[serde(rename = "ordType")]
    pub ord_type: String, // "limit", "market", "post_only"
    pub sz: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub px: Option<String>,
    #[serde(rename = "clOrdId", skip_serializing_if = "Option::is_none")]
    pub client_order_id: Option<String>,
}

/// ผลลัพธ์การตอบรับจาก OKX เมื่อยิงคำสั่งผ่าน WebSocket
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsOrderResult {
    #[serde(rename = "clOrdId")]
    pub client_order_id: Option<String>,
    #[serde(rename = "ordId")]
    pub order_id: Option<String>,
    #[serde(rename = "sCode")]
    pub status_code: String, // "0" สำเร็จ
    #[serde(rename = "sMsg")]
    pub status_msg: String,
    pub tag: Option<String>,
}
