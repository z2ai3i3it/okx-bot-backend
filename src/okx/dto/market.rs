use serde::{Deserialize, Serialize};

/// Payload โครงสร้าง Subscription สำหรับ OKX WebSocket
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsSubscriptionArg {
    pub channel: String,
    #[serde(rename = "instId")]
    pub inst_id: String,
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

/// ข้อมูล Ticker ราคาตลาดสดจาก OKX v5 Public Channel `tickers`
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OkxTickerData {
    #[serde(rename = "instType")]
    pub inst_type: String,
    #[serde(rename = "instId")]
    pub inst_id: String,
    /// ราคาล่าสุด (Last traded price)
    pub last: String,
    /// ขนาดของไม้ล่าสุด
    #[serde(rename = "lastSz")]
    pub last_size: String,
    /// ราคา Best Ask (ราคาเสนอขายที่ดีที่สุด)
    #[serde(rename = "askPx")]
    pub ask_price: String,
    /// ปริมาณ Best Ask
    #[serde(rename = "askSz")]
    pub ask_size: String,
    /// ราคา Best Bid (ราคาเสนอซื้อที่ดีที่สุด)
    #[serde(rename = "bidPx")]
    pub bid_price: String,
    /// ปริมาณ Best Bid
    #[serde(rename = "bidSz")]
    pub bid_size: String,
    /// ราคาสูงสุดรอบ 24 ชั่วโมง
    pub high24h: String,
    /// ราคาต่ำสุดรอบ 24 ชั่วโมง
    pub low24h: String,
    /// ปริมาณการซื้อขายรอบ 24 ชั่วโมง (base currency)
    pub vol24h: String,
    /// เวลา Timestamp บนกระดาน OKX (Unix milliseconds)
    pub ts: String,
}

/// Wrapper สำหรับ Data Message ที่ OKX สตรีมกลับมา
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsDataMessage<T> {
    pub arg: WsSubscriptionArg,
    #[serde(default)]
    pub data: Vec<T>,
}
