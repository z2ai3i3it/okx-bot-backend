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

// ==========================================
// 🔐 1. Private Streams DTOs
// ==========================================

/// 1. ข้อมูลอัปเดตสถานะคำสั่งซื้อขายจาก Private Channel `orders`
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OkxOrderUpdateData {
    #[serde(rename = "instId")]
    pub inst_id: String,
    #[serde(rename = "ordId")]
    pub order_id: String,
    #[serde(rename = "clOrdId")]
    pub client_order_id: String,
    pub tag: Option<String>,
    pub px: String,
    pub sz: String,
    pub side: String, // "buy", "sell"
    #[serde(rename = "ordType")]
    pub ord_type: String,
    pub state: String, // "live", "partially_filled", "filled", "canceled"
    #[serde(rename = "fillSz")]
    pub fill_size: String,
    #[serde(rename = "fillPx")]
    pub fill_price: String,
    #[serde(rename = "avgPx")]
    pub avg_price: String,
    #[serde(rename = "accFillSz")]
    pub acc_fill_size: String,
    pub fee: String,
    #[serde(rename = "feeCcy")]
    pub fee_currency: String,
    #[serde(rename = "uTime")]
    pub update_time: String,
    #[serde(rename = "cTime")]
    pub create_time: String,
}

/// 2. ข้อมูลการจับคู่ซื้อขายจริง (Execution Fill) จาก Private Channel `fills` / `my_trades`
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OkxMyTradeData {
    #[serde(rename = "instId")]
    pub inst_id: String,
    #[serde(rename = "tradeId")]
    pub trade_id: String,
    #[serde(rename = "ordId")]
    pub order_id: String,
    #[serde(rename = "clOrdId")]
    pub client_order_id: String,
    #[serde(rename = "fillPx")]
    pub fill_price: String,
    #[serde(rename = "fillSz")]
    pub fill_size: String,
    pub side: String, // "buy", "sell"
    pub fee: String,
    #[serde(rename = "feeCcy")]
    pub fee_currency: String,
    pub ts: String,
    #[serde(rename = "execType")]
    pub exec_type: Option<String>, // "T" (Taker), "M" (Maker)
}

/// 3. ข้อมูล Balance และ Margin สดจาก Private Channel `account` / `balance_and_position`
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OkxAccountBalanceUpdate {
    #[serde(rename = "totalEq")]
    pub total_equity: String,
    #[serde(rename = "adjEq")]
    pub adjusted_equity: Option<String>,
    #[serde(rename = "uTime")]
    pub update_time: String,
    #[serde(default)]
    pub details: Vec<OkxBalanceDetailUpdate>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OkxBalanceDetailUpdate {
    pub ccy: String,
    pub eq: String,
    #[serde(rename = "availBal")]
    pub avail_bal: String,
    #[serde(rename = "frozenBal")]
    pub frozen_bal: String,
}

// ==========================================
// 🔧 2. Trading Operations DTOs (Request / Response)
// ==========================================

/// 1. คำสั่งยิง Single Place Order ผ่าน WebSocket Trade API (`op: "order"`)
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
}

/// 2. คำสั่งแก้ไขออร์เดอร์ ผ่าน WebSocket (`op: "amend-order"`)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsAmendOrderArg {
    #[serde(rename = "instId")]
    pub inst_id: String,
    #[serde(rename = "ordId", skip_serializing_if = "Option::is_none")]
    pub order_id: Option<String>,
    #[serde(rename = "clOrdId", skip_serializing_if = "Option::is_none")]
    pub client_order_id: Option<String>,
    #[serde(rename = "newPx", skip_serializing_if = "Option::is_none")]
    pub new_price: Option<String>,
    #[serde(rename = "newSz", skip_serializing_if = "Option::is_none")]
    pub new_size: Option<String>,
}

/// 3. คำสั่งยกเลิก Single Order ผ่าน WebSocket (`op: "cancel-order"`)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsCancelOrderArg {
    #[serde(rename = "instId")]
    pub inst_id: String,
    #[serde(rename = "ordId", skip_serializing_if = "Option::is_none")]
    pub order_id: Option<String>,
    #[serde(rename = "clOrdId", skip_serializing_if = "Option::is_none")]
    pub client_order_id: Option<String>,
}

/// 4. คำสั่งยกเลิก Batch Cancel Orders ผ่าน WebSocket (`op: "batch-cancel-orders"`)
pub type WsBatchCancelOrderArg = Vec<WsCancelOrderArg>;

/// ผลลัพธ์การตอบรับจาก OKX เมื่อยิงคำสั่งผ่าน WebSocket
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
