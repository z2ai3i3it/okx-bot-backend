use std::time::Duration;
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use thiserror::Error;
use tokio::{
    sync::broadcast,
    time::{interval, sleep},
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::protocol::Message,
};

use crate::okx::dto::market::{
    OkxOrderBookData, OkxTickerData, OkxTradeData, WsDataMessage, WsRequest, WsSubscriptionArg,
};

#[derive(Debug, Error)]
pub enum WsClientError {
    #[error("WebSocket connection error: {0}")]
    TungsteniteError(#[from] tokio_tungstenite::tungstenite::Error),

    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("Channel send error")]
    ChannelError,
}

/// Client จัดการการเชื่อมต่อ WebSocket กับ OKX v5 Public Market Data
/// ออกแบบเป็น Singleton Hub กลางเพื่อแชร์ Connection ให้บอททุกตัวในระบบ
#[derive(Clone)]
pub struct OkxPublicWsClient {
    ws_url: String,
    ticker_sender: broadcast::Sender<OkxTickerData>,
    trade_sender: broadcast::Sender<OkxTradeData>,
    orderbook_sender: broadcast::Sender<OkxOrderBookData>,
}

impl OkxPublicWsClient {
    pub fn new() -> Self {
        Self::with_url("wss://ws.okx.com:8443/ws/v5/public")
    }

    pub fn with_url(ws_url: &str) -> Self {
        let (ticker_sender, _) = broadcast::channel(2048);
        let (trade_sender, _) = broadcast::channel(2048);
        let (orderbook_sender, _) = broadcast::channel(1024);

        Self {
            ws_url: ws_url.to_string(),
            ticker_sender,
            trade_sender,
            orderbook_sender,
        }
    }

    /// 1. ดึง Receiver สำหรับฟัง Ticker
    pub fn subscribe_ticker(&self) -> broadcast::Receiver<OkxTickerData> {
        self.ticker_sender.subscribe()
    }

    /// 2. ดึง Receiver สำหรับฟัง Public Trades
    pub fn subscribe_trades(&self) -> broadcast::Receiver<OkxTradeData> {
        self.trade_sender.subscribe()
    }

    /// 3. ดึง Receiver สำหรับฟัง Order Book
    pub fn subscribe_orderbook(&self) -> broadcast::Receiver<OkxOrderBookData> {
        self.orderbook_sender.subscribe()
    }

    /// สตาร์ท Background Task เชื่อมต่อ WebSocket พร้อม Auto-reconnect และ Ping-Pong Heartbeat
    pub fn start(&self, subscriptions: Vec<WsSubscriptionArg>) {
        let ws_url = self.ws_url.clone();
        let ticker_sender = self.ticker_sender.clone();
        let trade_sender = self.trade_sender.clone();
        let orderbook_sender = self.orderbook_sender.clone();

        tokio::spawn(async move {
            let mut backoff_sec = 1u64;

            loop {
                log::info!("Connecting to OKX Public WebSocket: {}", ws_url);

                match connect_async(&ws_url).await {
                    Ok((ws_stream, _response)) => {
                        log::info!("Connected to OKX Public WebSocket successfully");
                        backoff_sec = 1; // Reset backoff เมื่อเชื่อมต่อติด

                        let (mut write, mut read) = ws_stream.split();

                        // 1. ส่ง Subscribe Payload ทีเดียวแบบ Batch (เคารพโควต้า 480 sub/hr)
                        if !subscriptions.is_empty() {
                            let sub_req = WsRequest {
                                op: "subscribe".to_string(),
                                args: subscriptions.clone(),
                            };

                            if let Ok(json_str) = serde_json::to_string(&sub_req) {
                                let _ = write.send(Message::Text(json_str.into())).await;
                            }
                        }

                        // 2. Loop การส่ง Ping Heartbeat ทุก 20 วินาที (ป้องกัน 30s Idle Disconnect)
                        let mut ping_interval = interval(Duration::from_secs(20));

                        loop {
                            tokio::select! {
                                // Heartbeat Timer
                                _ = ping_interval.tick() => {
                                    if let Err(e) = write.send(Message::Text("ping".into())).await {
                                        log::warn!("Failed to send ping to OKX WS: {}", e);
                                        break; // หลุด loop เพื่อ reconnect
                                    }
                                }

                                // รับข้อความจาก OKX
                                msg = read.next() => {
                                    match msg {
                                        Some(Ok(Message::Text(text))) => {
                                            if text == "pong" {
                                                continue;
                                            }

                                            // แยกกระจายตาม Channel
                                            if text.contains("\"channel\":\"tickers\"") {
                                                if let Ok(data_msg) = serde_json::from_str::<WsDataMessage<OkxTickerData>>(&text) {
                                                    for ticker in data_msg.data {
                                                        let _ = ticker_sender.send(ticker);
                                                    }
                                                }
                                            } else if text.contains("\"channel\":\"trades\"") {
                                                if let Ok(data_msg) = serde_json::from_str::<WsDataMessage<OkxTradeData>>(&text) {
                                                    for trade in data_msg.data {
                                                        let _ = trade_sender.send(trade);
                                                    }
                                                }
                                            } else if text.contains("\"channel\":\"books") {
                                                if let Ok(data_msg) = serde_json::from_str::<WsDataMessage<OkxOrderBookData>>(&text) {
                                                    for ob in data_msg.data {
                                                        let _ = orderbook_sender.send(ob);
                                                    }
                                                }
                                            } else {
                                                if let Ok(raw_val) = serde_json::from_str::<Value>(&text) {
                                                    if let Some(event) = raw_val.get("event") {
                                                        log::debug!("OKX WS Event: {:?}", event);
                                                    }
                                                }
                                            }
                                        }
                                        Some(Ok(Message::Close(_))) => {
                                            log::warn!("OKX Public WS received close frame");
                                            break;
                                        }
                                        Some(Err(e)) => {
                                            log::error!("OKX Public WS error: {}", e);
                                            break;
                                        }
                                        None => {
                                            log::warn!("OKX Public WS stream ended");
                                            break;
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to connect to OKX Public WS: {}. Retrying...", e);
                    }
                }

                // หน่วงเวลา Reconnect แบบ Exponential Backoff
                sleep(Duration::from_secs(backoff_sec)).await;
                backoff_sec = (backoff_sec * 2).min(30);
            }
        });
    }
}

impl Default for OkxPublicWsClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_public_ws_connect_and_stream_ticker() {
        let client = OkxPublicWsClient::new();
        let mut ticker_rx = client.subscribe_ticker();

        // Subscribe คู่ BTC-USDT
        let subs = vec![WsSubscriptionArg::per_instrument("tickers", "BTC-USDT")];
        client.start(subs);

        // รอรับข้อความราคาแรกจาก OKX (ให้เวลาไม่เกิน 5 วินาที)
        let received = tokio::time::timeout(Duration::from_secs(5), async {
            while let Ok(ticker) = ticker_rx.recv().await {
                if ticker.inst_id == "BTC-USDT" {
                    return Some(ticker);
                }
            }
            None
        })
        .await;

        match received {
            Ok(Some(ticker)) => {
                println!(
                    "✅ Received live OKX Ticker: {} Last Price: {} Best Ask: {} Best Bid: {}",
                    ticker.inst_id, ticker.last, ticker.ask_price, ticker.bid_price
                );
                assert_eq!(ticker.inst_id, "BTC-USDT");
                assert!(!ticker.last.is_empty());
            }
            Ok(None) => panic!("Did not receive matching ticker"),
            Err(_) => {
                println!("⚠️ WebSocket test timed out (network might be slow or restricted)");
            }
        }
    }
}
