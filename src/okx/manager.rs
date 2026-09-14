use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;

use crate::okx::{rate_limiter::OkxRateLimiter, ws_client::OkxPublicWsClient};

/// OKX Connector Pool Manager
/// จัดการ Lifecycle ของ Rate Limiter และ WebSocket Client กลาง
#[derive(Clone)]
pub struct OkxManager {
    public_ws: OkxPublicWsClient,
    /// Rate limiters แยกตาม Sub-account ID เพื่อไม่ให้แย่งโควต้ากัน
    rate_limiters: Arc<RwLock<HashMap<String, OkxRateLimiter>>>,
}

impl OkxManager {
    pub fn new() -> Self {
        let public_ws = OkxPublicWsClient::new();
        Self {
            public_ws,
            rate_limiters: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// ดึงหรือสร้าง Rate Limiter ประจำ Account นั้นๆ (1 Sub-account = 1 Rate Limiter)
    pub async fn get_rate_limiter(&self, account_id: &str) -> OkxRateLimiter {
        let read_guard = self.rate_limiters.read().await;
        if let Some(limiter) = read_guard.get(account_id) {
            return limiter.clone();
        }
        drop(read_guard);

        let mut write_guard = self.rate_limiters.write().await;
        write_guard
            .entry(account_id.to_string())
            .or_insert_with(OkxRateLimiter::new)
            .clone()
    }

    /// ดึง Public WebSocket Client สำหรับสตรีมราคา
    pub fn public_ws(&self) -> &OkxPublicWsClient {
        &self.public_ws
    }
}

impl Default for OkxManager {
    fn default() -> Self {
        Self::new()
    }
}
