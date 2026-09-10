use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use thiserror::Error;
use tokio::sync::Mutex;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RateLimitError {
    #[error("Sub-account rate limit exceeded (limit: {limit}/2s, requested: {requested})")]
    SubAccountLimitExceeded { limit: u32, requested: u32 },

    #[error("Instrument rate limit exceeded for {inst_id} (op: {op:?}, limit: {limit}/2s)")]
    InstrumentLimitExceeded {
        inst_id: String,
        op: OrderOpType,
        limit: u32,
    },

    #[error("Endpoint rate limit exceeded for {endpoint} (limit: {limit})")]
    EndpointLimitExceeded { endpoint: String, limit: u32 },

    #[error("Operation timeout while waiting for rate limit permit")]
    Timeout,
}

/// ประเภทของ Order Action สำหรับนับโควต้าระดับ Instrument
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OrderOpType {
    PlaceSingle,
    PlaceBatch,
    Cancel,
    Amend,
}

/// Token Bucket ที่คำนวณการเติม Token แบบ Continuous Refill ตามระยะเวลาที่ผ่านไป
#[derive(Debug, Clone)]
pub struct TokenBucket {
    capacity: f64,
    tokens: f64,
    refill_rate_per_sec: f64,
    last_update: Instant,
}

impl TokenBucket {
    pub fn new(capacity: u32, window: Duration) -> Self {
        let cap = capacity as f64;
        let refill_rate = cap / window.as_secs_f64();
        Self {
            capacity: cap,
            tokens: cap,
            refill_rate_per_sec: refill_rate,
            last_update: Instant::now(),
        }
    }

    /// เติม Token ตามเวลาที่ผ่านไป
    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_update).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_rate_per_sec).min(self.capacity);
        self.last_update = now;
    }

    /// พยายามดึง token (Non-blocking)
    pub fn try_acquire(&mut self, required: u32) -> bool {
        self.refill();
        let req = required as f64;
        if self.tokens >= req {
            self.tokens -= req;
            true
        } else {
            false
        }
    }

    /// คำนวณระยะเวลาที่ต้องรอจนกว่าจะมี token เพียงพอ
    pub fn wait_time_for(&mut self, required: u32) -> Duration {
        self.refill();
        let req = required as f64;
        if self.tokens >= req {
            Duration::ZERO
        } else {
            let missing = req - self.tokens;
            let secs = missing / self.refill_rate_per_sec;
            Duration::from_secs_f64(secs)
        }
    }
}

/// State ภายในของ OkxRateLimiter จัดเก็บ Bucket แยกตาม Scope
struct LimiterInner {
    /// เพดานรวม Sub-account (1,000 req / 2s สำหรับ Tier 1)
    sub_account_bucket: TokenBucket,
    /// โควต้าระดับคู่เหรียญ + ประเภทคำสั่ง (inst_id, op_type)
    instrument_buckets: HashMap<(String, OrderOpType), TokenBucket>,
    /// โควต้า REST Endpoint ทั่วไป
    endpoint_buckets: HashMap<String, TokenBucket>,
}

/// Pure OKX v5 Rate Limiter Engine
/// จัดการสิทธิ์การยิงคำสั่งตามข้อกำหนดของ OKX ทั้งระดับ Sub-account และ Instrument
#[derive(Clone)]
pub struct OkxRateLimiter {
    inner: Arc<Mutex<LimiterInner>>,
    sub_account_cap: u32,
}

impl OkxRateLimiter {
    /// สร้าง Rate Limiter ด้วยค่ามาตรฐาน Default ของ OKX (Tier 1: 1,000 req / 2s)
    pub fn new() -> Self {
        Self::with_sub_account_cap(1000)
    }

    /// สร้าง Rate Limiter พร้อมกำหนด Sub-account cap (เช่น VIP Tier 1-8: 1,000 - 10,000)
    pub fn with_sub_account_cap(sub_account_cap: u32) -> Self {
        let window_2s = Duration::from_secs(2);
        let inner = LimiterInner {
            sub_account_bucket: TokenBucket::new(sub_account_cap, window_2s),
            instrument_buckets: HashMap::new(),
            endpoint_buckets: HashMap::new(),
        };

        Self {
            inner: Arc::new(Mutex::new(inner)),
            sub_account_cap,
        }
    }

    /// ตรวจสอบและขอ Permit ก่อนยิง Order (Non-blocking)
    /// - order_count: จำนวนคำสั่งจริงใน Request (Single = 1, Batch = 2..=20)
    pub async fn try_acquire_order(
        &self,
        inst_id: &str,
        op: OrderOpType,
        order_count: u32,
    ) -> Result<(), RateLimitError> {
        let mut inner = self.inner.lock().await;

        // 1. ตรวจสอบ Sub-account bucket (หักตาม order_count จริง)
        if !inner.sub_account_bucket.try_acquire(order_count) {
            return Err(RateLimitError::SubAccountLimitExceeded {
                limit: self.sub_account_cap,
                requested: order_count,
            });
        }

        // 2. ตรวจสอบ Instrument bucket (หัก 1 request สำหรับ endpoint นั้น)
        let inst_cap = match op {
            OrderOpType::PlaceSingle => 60,
            OrderOpType::PlaceBatch => 300,
            OrderOpType::Cancel => 60,
            OrderOpType::Amend => 60,
        };

        let key = (inst_id.to_string(), op);
        let bucket = inner
            .instrument_buckets
            .entry(key.clone())
            .or_insert_with(|| TokenBucket::new(inst_cap, Duration::from_secs(2)));

        if !bucket.try_acquire(1) {
            // คืนค่าที่หักจาก sub-account ไปเมื่อกี้ (Rollback)
            inner.sub_account_bucket.tokens =
                (inner.sub_account_bucket.tokens + order_count as f64).min(self.sub_account_cap as f64);

            return Err(RateLimitError::InstrumentLimitExceeded {
                inst_id: inst_id.to_string(),
                op,
                limit: inst_cap,
            });
        }

        Ok(())
    }

    /// ขอ Permit แบบ Async Wait (รอจนกว่าโควต้าจะว่าง หรือจนหมดเวลา timeout)
    pub async fn acquire_order(
        &self,
        inst_id: &str,
        op: OrderOpType,
        order_count: u32,
        max_wait: Duration,
    ) -> Result<(), RateLimitError> {
        let start = Instant::now();

        loop {
            // ลองขอแบบ non-blocking ดูก่อน
            match self.try_acquire_order(inst_id, op, order_count).await {
                Ok(()) => return Ok(()),
                Err(_) => {
                    let elapsed = start.elapsed();
                    if elapsed >= max_wait {
                        return Err(RateLimitError::Timeout);
                    }

                    // คำนวณเวลาที่ต้องรอสั้นๆ แล้ว sleep ก่อนลองใหม่
                    let sleep_dur = Duration::from_millis(10).min(max_wait - elapsed);
                    tokio::time::sleep(sleep_dur).await;
                }
            }
        }
    }

    /// ขอ Permit สำหรับ REST Endpoint ทั่วไป (เช่น GET /account/balance = 10 req / 2s)
    pub async fn try_acquire_endpoint(
        &self,
        endpoint: &str,
        capacity: u32,
        window: Duration,
    ) -> Result<(), RateLimitError> {
        let mut inner = self.inner.lock().await;
        let bucket = inner
            .endpoint_buckets
            .entry(endpoint.to_string())
            .or_insert_with(|| TokenBucket::new(capacity, window));

        if bucket.try_acquire(1) {
            Ok(())
        } else {
            Err(RateLimitError::EndpointLimitExceeded {
                endpoint: endpoint.to_string(),
                limit: capacity,
            })
        }
    }
}

impl Default for OkxRateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_sub_account_limit_enforcement() {
        // สร้าง Limiter ที่มี Cap ต่ำสำหรับการทดสอบ (เช่น 5 req / 2s)
        let limiter = OkxRateLimiter::with_sub_account_cap(5);

        // ยิงชุดแรก 3 orders -> สำเร็จ
        assert!(limiter
            .try_acquire_order("BTC-USDT", OrderOpType::PlaceSingle, 3)
            .await
            .is_ok());

        // ยิงชุดสอง 2 orders -> รวมเป็น 5 สำเร็จ
        assert!(limiter
            .try_acquire_order("BTC-USDT", OrderOpType::PlaceSingle, 2)
            .await
            .is_ok());

        // ยิงเพิ่มอีก 1 order -> ต้องติด Error SubAccountLimitExceeded ทันที
        let err = limiter
            .try_acquire_order("BTC-USDT", OrderOpType::PlaceSingle, 1)
            .await
            .unwrap_err();

        match err {
            RateLimitError::SubAccountLimitExceeded { limit, requested } => {
                assert_eq!(limit, 5);
                assert_eq!(requested, 1);
            }
            _ => panic!("Expected SubAccountLimitExceeded error"),
        }
    }

    #[tokio::test]
    async fn test_instrument_single_vs_batch_quota_isolation() {
        let limiter = OkxRateLimiter::new();

        // Single order quota (60) กับ Batch order quota (300) ต้องแยก Bucket กัน
        let res_single = limiter
            .try_acquire_order("BTC-USDT", OrderOpType::PlaceSingle, 1)
            .await;
        assert!(res_single.is_ok());

        let res_batch = limiter
            .try_acquire_order("BTC-USDT", OrderOpType::PlaceBatch, 10)
            .await;
        assert!(res_batch.is_ok());
    }

    #[tokio::test]
    async fn test_endpoint_rate_limit() {
        let limiter = OkxRateLimiter::new();
        let endpoint = "/api/v5/account/balance";

        // อนุญาต 2 ครั้งใน 2 วินาที
        assert!(limiter
            .try_acquire_endpoint(endpoint, 2, Duration::from_secs(2))
            .await
            .is_ok());
        assert!(limiter
            .try_acquire_endpoint(endpoint, 2, Duration::from_secs(2))
            .await
            .is_ok());

        // ครั้งที่ 3 ต้องถูก Reject
        assert!(limiter
            .try_acquire_endpoint(endpoint, 2, Duration::from_secs(2))
            .await
            .is_err());
    }
}
