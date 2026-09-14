use serde::{Deserialize, Serialize};

/// Payload โครงสร้าง Subscription สำหรับ OKX WebSocket
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsSubscriptionArg {
    pub channel: String,
    #[serde(rename = "instId", skip_serializing_if = "Option::is_none")]
    pub inst_id: Option<String>,
    #[serde(rename = "instType", skip_serializing_if = "Option::is_none")]
    pub inst_type: Option<String>,
}

impl WsSubscriptionArg {
    pub fn per_instrument(channel: &str, inst_id: &str) -> Self {
        Self {
            channel: channel.to_string(),
            inst_id: Some(inst_id.to_string()),
            inst_type: None,
        }
    }

    pub fn per_account(channel: &str) -> Self {
        Self {
            channel: channel.to_string(),
            inst_id: None,
            inst_type: None,
        }
    }
}

/// คำสั่ง Request ส่งเข้า OKX WebSocket (เช่น subscribe, unsubscribe, login)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsRequest<T> {
    pub op: String,
    pub args: Vec<T>,
}

/// WebSocket Event Response ทั่วไป เช่น `{"event": "subscribe", "arg": {...}}`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsEventResponse {
    pub event: Option<String>,
    pub code: Option<String>,
    pub msg: Option<String>,
    pub conn_id: Option<String>,
}

/// 1. ข้อมูล Ticker ราคาตลาดสดจาก OKX v5 Public Channel `tickers`
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OkxTickerData {
    #[serde(rename = "instType")]
    pub inst_type: String,
    #[serde(rename = "instId")]
    pub inst_id: String,
    pub last: String,
    #[serde(rename = "lastSz")]
    pub last_size: String,
    #[serde(rename = "askPx")]
    pub ask_price: String,
    #[serde(rename = "askSz")]
    pub ask_size: String,
    #[serde(rename = "bidPx")]
    pub bid_price: String,
    #[serde(rename = "bidSz")]
    pub bid_size: String,
    pub high24h: String,
    pub low24h: String,
    pub vol24h: String,
    pub ts: String,
}

/// 2. ข้อมูล Public Trades ตลาดสดจาก OKX v5 Public Channel `trades`
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OkxTradeData {
    #[serde(rename = "instId")]
    pub inst_id: String,
    #[serde(rename = "tradeId")]
    pub trade_id: String,
    pub px: String,
    pub sz: String,
    pub side: String, // "buy" or "sell"
    pub ts: String,
}

/// 3. ข้อมูล Order Book ความลึกกระดานจาก OKX v5 Public Channel `books5` / `books`
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OkxOrderBookData {
    #[serde(rename = "instId", default)]
    pub inst_id: String,
    /// Bids: Array of [price, size, num_orders]
    #[serde(default)]
    pub bids: Vec<Vec<String>>,
    /// Asks: Array of [price, size, num_orders]
    #[serde(default)]
    pub asks: Vec<Vec<String>>,
    pub ts: String,
    #[serde(default)]
    pub checksum: Option<i64>,
}

/// Wrapper สำหรับ Data Message ที่ OKX สตรีมกลับมา
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsDataMessage<T> {
    pub arg: WsSubscriptionArg,
    #[serde(default)]
    pub data: Vec<T>,
}
