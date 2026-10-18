use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use rust_decimal::Decimal;

use crate::{
    bot::strategies::base::StrategyAction,
    okx::rate_limiter::OrderOpType,
    pipeline::{
        context::PipelineContext,
        simulate_cancel_plan::{
            simulate_cancel_plan, IntentStatus, MidPrice, OrderIntentForCem, TrackedOrderForCem,
            DEC_ZERO,
        },
    },
};

/// บันทึกสถานะยอดเงินคงเหลือและ Virtual Lock สำหรับสินทรัพย์หนึ่งๆ
#[derive(Debug, Clone)]
pub struct AssetBalance {
    pub free: Decimal,
    pub used: Decimal,
    pub total: Decimal,
}

impl AssetBalance {
    pub fn new(free: Decimal) -> Self {
        Self {
            free,
            used: DEC_ZERO,
            total: free,
        }
    }
}

/// CEM Core Manager จัดการ Pool เงินและ Order ของทั้ง Account (Shared State ข้าม Bots)
#[derive(Debug, Clone, Default)]
pub struct CemManager {
    /// ยอดคงเหลือแยกตาม Currency (e.g. "USDT", "BTC")
    pub balances: Arc<Mutex<HashMap<String, AssetBalance>>>,
    /// Order ที่อยู่ระหว่างเปิดใช้งาน (Tracked Orders Pool)
    pub tracked_orders: Arc<Mutex<HashMap<String, TrackedOrderForCem>>>,
    /// Mid Prices ปัจจุบันของแต่ละตลาดสำหรับใช้ทำ Market-Aware Cancel Plan
    pub mid_prices: Arc<Mutex<HashMap<String, Decimal>>>,
    /// รายการ client_order_id ที่ CEM วางแผนสั่ง Cancel เพื่อ Preempt เงินทุน
    pub planned_preemption_cancels: Arc<Mutex<std::collections::HashSet<String>>>,
    /// Route สำหรับส่ง Reactivated Order กลับไปให้ executor ของ "บอทเจ้าของ" วางใหม่
    /// (private WS ทำให้ทุก executor เห็น event เดียวกัน — ต้อง route กลับเจ้าของเท่านั้น
    ///  ไม่งั้น executor ผิดตัวเป็นคน consume แล้ว strategy เจ้าของไม่เคยถูกเรียก -> ไม่มีใครวาง order จริง)
    pub reactivation_routes:
        Arc<Mutex<HashMap<String, tokio::sync::mpsc::UnboundedSender<TrackedOrderForCem>>>>,
    /// OKX Private WS Trade Client สำหรับส่ง cancel preemption
    pub trade_client: Arc<Mutex<Option<crate::okx::ws_trade::OkxWsTradeClient>>>,
    /// Backoff ของ intent ที่ place เพิ่งล้ม (Dedup block / rate-limit / OKX reject หลัง dispatch)
    /// — กันลูป `approve -> place ล้ม -> revert เป็น Parked -> reevaluate ปลุกซ้ำ` ที่ถูก
    /// trigger ทุกครั้งที่มี ticker (เคสจริง 1,298 รอบ ใน 49 วินาที ที่ log 03:55:49)
    place_retry_backoff: Arc<Mutex<HashMap<String, PlaceRetryBackoff>>>,
}

/// สถานะ backoff ของ intent ที่ place ล้มล่าสุด
#[derive(Debug, Clone, Copy)]
struct PlaceRetryBackoff {
    /// จำนวนครั้งที่ล้มติดกัน (ใช้คำนวณ delay แบบ exponential)
    failures: u32,
    /// เวลา (ms) ก่อนที่ reevaluate จะปลุก intent นี้ได้อีกครั้ง
    next_allowed_ms: i64,
}

/// Base delay (ms) หลัง place ล้มครั้งแรก — สั้นพอที่ bot จะ retry เร็ว แต่หยุดลูป ~65 รอบ/วินาที
const PLACE_RETRY_BASE_DELAY_MS: i64 = 500;
/// เพดาน delay (ms) ของ place retry backoff
const PLACE_RETRY_MAX_DELAY_MS: i64 = 30_000;

impl CemManager {
    pub fn new() -> Self {
        Self {
            balances: Arc::new(Mutex::new(HashMap::new())),
            tracked_orders: Arc::new(Mutex::new(HashMap::new())),
            mid_prices: Arc::new(Mutex::new(HashMap::new())),
            planned_preemption_cancels: Arc::new(Mutex::new(std::collections::HashSet::new())),
            reactivation_routes: Arc::new(Mutex::new(HashMap::new())),
            trade_client: Arc::new(Mutex::new(None)),
            place_retry_backoff: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// ตั้งค่า trade client ให้ CEM กลางสำหรับใช้ยิงคำสั่ง cancel preemption
    pub async fn set_trade_client(&self, client: crate::okx::ws_trade::OkxWsTradeClient) {
        let mut guard = self.trade_client.lock().await;
        *guard = Some(client);
    }

    /// บันทึกว่า place ของ intent นี้เพิ่งล้ม -> หน่วงเวลา reevaluate ปลุกซ้ำ (exponential backoff)
    /// เรียกเมื่อ: place ถูก block หลังจองเงิน (dedup/rate-limit/dispatch fail) หรือ OKX ปฏิเสธหลัง dispatch
    pub async fn note_place_failure(&self, intent_id: &str) {
        let mut map = self.place_retry_backoff.lock().await;
        let now = chrono::Utc::now().timestamp_millis();
        let entry = map
            .entry(intent_id.to_string())
            .or_insert(PlaceRetryBackoff { failures: 0, next_allowed_ms: 0 });
        // ถ้า backoff เดิมหมดอายุไปแล้ว = เคยถูกหน่วงให้ retry จริงครั้งหนึ่งแล้ว -> เริ่มสายใหม่
        if entry.next_allowed_ms <= now {
            entry.failures = 0;
        }
        entry.failures = (entry.failures + 1).min(16);
        let delay = PLACE_RETRY_BASE_DELAY_MS
            .saturating_mul(1i64 << (entry.failures - 1))
            .min(PLACE_RETRY_MAX_DELAY_MS);
        entry.next_allowed_ms = now + delay;
        log::warn!(
            "[CEM] ⏳ Place failed for {} ({} consecutive) — delaying reactivation for {}ms",
            intent_id, entry.failures, delay
        );
    }

    /// ล้าง backoff เมื่อ place ของ intent นี้ถูก dispatch สำเร็จ (หรือ entry ถูกเก็บกวาดแล้ว)
    pub async fn clear_place_failure(&self, intent_id: &str) {
        self.place_retry_backoff.lock().await.remove(intent_id);
    }

    /// true ถ้า intent นี้ยังอยู่ในช่วง backoff (ห้าม reevaluate ปลุกซ้ำ)
    async fn place_retry_blocked(&self, intent_id: &str) -> bool {
        let map = self.place_retry_backoff.lock().await;
        map.get(intent_id)
            .is_some_and(|b| b.next_allowed_ms > chrono::Utc::now().timestamp_millis())
    }

    /// Approve + lock แบบ atomic ในการ lock ครั้งเดียว (กัน TOCTOU กับ reevaluate/terminal
    /// release ของบอทอื่น) — เดิม fast-path อ่าน `free` ก่อนแล้วค่อย `lock_capital()` ทีหลัง
    /// โดย "ละเลย" ผลลัพธ์ ถ้าเงินถูกแย่งไป lock ก็ไม่สำเร็จ แต่ยัง track เป็น Approved +
    /// log "Fast-path APPROVED" โดยไม่มีบรรทัด Virtual Lock (เคสจริง log 03:55:42.055 ->
    /// entry ถือ lock ที่ไม่มีจริง -> เมื่อ order terminal จะ release เกิน -> `used` ต่ำกว่าจริง
    /// -> CEM approve เกิน capital limit)
    ///
    /// คืน true เฉพาะเมื่อ lock สำเร็จจริง (entry ถูกบันทึกเป็น Approved พร้อม lock)
    pub async fn try_fast_approve(&self, order: TrackedOrderForCem) -> bool {
        if order.locked_amount <= DEC_ZERO {
            return false;
        }
        let mut pool = self.tracked_orders.lock().await;
        let mut balances = self.balances.lock().await;
        let asset_key = order.required_asset.to_uppercase();
        let entry = balances
            .entry(asset_key.clone())
            .or_insert_with(|| AssetBalance::new(DEC_ZERO));
        if entry.free < order.locked_amount {
            return false;
        }
        entry.free -= order.locked_amount;
        entry.used += order.locked_amount;
        log::info!(
            "[CEM] 🔒 Virtual Lock {} {}: remaining_free={}, used={}",
            order.locked_amount, asset_key, entry.free, entry.used
        );
        pool.insert(order.intent_id.clone(), order);
        true
    }

    /// OKX ปฏิเสธ place "หลัง" dispatch (เช่น 51008 insufficient balance) — order ไม่เคยขึ้น
    /// exchange และจะไม่มี terminal event มาเอง -> ต้องคืน lock (ถ้าถืออยู่) + ตั้ง backoff
    /// กัน reevaluate ปลุกซ้ำทันที (ไม่งั้นกลายเป็นลูป place ล้มไม่รู้จบ)
    pub async fn handle_place_reject(&self, intent_id: &str) -> Option<(String, Decimal)> {
        if let Some(released) = self.revert_reservation(intent_id).await {
            return Some(released);
        }
        // ไม่ได้ถือ reservation (เช่น entry ถูก revert/เปลี่ยนสถานะไปแล้ว) -> ยังต้องหน่วง retry
        self.note_place_failure(intent_id).await;
        None
    }

    /// อัปเดต Balance จาก WebSocket (Ground Truth)
    pub async fn update_balance_from_ws(&self, asset: &str, free: Decimal, total: Decimal) {
        let mut balances = self.balances.lock().await;
        let entry = balances.entry(asset.to_uppercase()).or_insert_with(|| AssetBalance {
            free,
            used: DEC_ZERO,
            total,
        });

        // อัปเดต total และคำนวณ free โดยหัก virtual locked (`used`) ออก
        entry.total = total;
        if entry.used > total {
            entry.used = total;
        }
        entry.free = (total - entry.used).max(DEC_ZERO);
        log::debug!(
            "[CEM] 💰 WS Balance updated for {}: total={}, used={}, effective_free={}",
            asset, entry.total, entry.used, entry.free
        );
    }

    /// อัปเดต Mid Price ของคู่เหรียญ
    pub async fn update_mid_price(&self, pair: &str, price: Decimal) {
        let mut mids = self.mid_prices.lock().await;
        mids.insert(crate::pipeline::simulate_cancel_plan::normalize_symbol(pair), price);
    }

    /// ตรวจสอบและขอล็อกเงินทุน (Virtual Lock)
    pub async fn lock_capital(&self, asset: &str, amount: Decimal) -> bool {
        let mut balances = self.balances.lock().await;
        let entry = balances.entry(asset.to_uppercase()).or_insert_with(|| AssetBalance::new(DEC_ZERO));
        if entry.free >= amount {
            entry.free -= amount;
            entry.used += amount;
            log::info!(
                "[CEM] 🔒 Virtual Lock {} {}: remaining_free={}, used={}",
                amount, asset, entry.free, entry.used
            );
            true
        } else {
            false
        }
    }

    /// คลายล็อกเงินทุน (Virtual Unlock) เมื่อ Fill / Cancel / Reject
    pub async fn release_capital(&self, asset: &str, amount: Decimal) {
        let mut balances = self.balances.lock().await;
        let entry = balances.entry(asset.to_uppercase()).or_insert_with(|| AssetBalance::new(DEC_ZERO));
        let release_amt = amount.min(entry.used);
        entry.used -= release_amt;
        entry.free += release_amt;
        log::info!(
            "[CEM] 🔓 Virtual Unlock {} {}: free={}, used={}",
            release_amt, asset, entry.free, entry.used
        );
    }

    /// บันทึก Order เข้าสู่ Tracked Pool
    pub async fn track_order(&self, order: TrackedOrderForCem) {
        let mut pool = self.tracked_orders.lock().await;
        pool.insert(order.intent_id.clone(), order);
    }

    /// บันทึก intent สถานะ Parked เฉพาะเมื่อ "ยังไม่มี entry" หรือ entry เดิมไม่ได้ถือ reservation
    /// (Approved/CancelPending) — กัน lock ถูกเขียนทับแบบไม่คืนเงิน
    /// (เคสจริง zzz.log 03:00:00: reactivation ถูก preempt flip เป็น CancelPending ระหว่าง place
    ///  กำลังบิน แล้ว re-place โดน reject -> เดิม track_order(Parked) ทับ CancelPending
    ///  -> lock 10.3665 ลอยกลาง session ไม่มีใครคืน)
    /// คืน true ถ้าบันทึกจริง / false ถ้าถูกกันไว้เพราะมี reservation อยู่
    pub async fn track_parked_intent(&self, order: TrackedOrderForCem) -> bool {
        let mut pool = self.tracked_orders.lock().await;
        if let Some(existing) = pool.get(&order.intent_id) {
            if matches!(
                existing.status,
                IntentStatus::Approved | IntentStatus::CancelPending
            ) {
                log::warn!(
                    "[CEM] ⚠️ Keeping existing reservation for {} (status={:?}, locked {} {}) instead of overwriting with Parked",
                    order.intent_id, existing.status, existing.locked_amount, existing.required_asset
                );
                return false;
            }
        }
        pool.insert(order.intent_id.clone(), order);
        true
    }

    /// นำ Order ออกจาก Tracked Pool
    pub async fn untrack_order(&self, intent_id: &str) -> Option<TrackedOrderForCem> {
        let mut pool = self.tracked_orders.lock().await;
        pool.remove(intent_id)
    }

    /// ยืนยันว่า order มีตัวตน live บน OKX แล้ว (มี order event สถานะ non-terminal)
    /// — เรียกจาก executor ทุกตัวที่เห็น event (idempotent)
    /// ใช้คัดผู้ถูก preempt: order ที่ place ยังไม่ confirm ห้ามถูก cancel plan เลือก
    pub async fn mark_order_live(&self, intent_id: &str) {
        let mut pool = self.tracked_orders.lock().await;
        if let Some(order) = pool.get_mut(intent_id) {
            order.live_confirmed = true;
        }
    }

    /// จัดการ order ที่มีสถานะ terminal (filled/canceled) ในมุม CEM
    ///
    /// - `CancelPending` (ถูกยกเลิกโดย CEM preemption plan) -> เปลี่ยนเป็น `Parked` + ปลดล็อกเงิน
    ///   คง entry ไว้ใน pool เพื่อรอ `reevaluate_parked_orders` ปลุกกลับมาวางใหม่เมื่อมีเงินว่าง
    ///   (ถ้า untrack เฉยๆ เงินจะคืนแต่ entry หาย -> order ของบอทที่ถูก preempt
    ///    จะไม่มีวันถูก reactivate — ดู log 19:15:19 stop bot2 ที่ sell 3 ไม่กลับมาวาง)
    /// - `Approved` -> untrack (เอาออกจาก pool) + ปลดล็อกเงิน — จบที่นี่ ไม่รอ reactivation
    /// - `Parked` / ไม่มี entry -> no-op (executor อื่นที่เห็นก่อนทำไปแล้ว — กัน release ซ้ำ)
    ///
    /// คืน `Some((asset, amount, parked))` เพื่อให้ executor log ได้ถูกต้อง
    pub async fn complete_terminal_order(&self, intent_id: &str) -> Option<(String, Decimal, bool)> {
        let mut pool = self.tracked_orders.lock().await;
        let order = pool.get_mut(intent_id)?;

        let parked = match order.status {
            IntentStatus::CancelPending => {
                order.status = IntentStatus::Parked;
                true
            }
            IntentStatus::Approved => false,
            IntentStatus::Parked => return None,
        };
        let asset = order.required_asset.to_uppercase();
        let amount = order.locked_amount;
        if !parked {
            pool.remove(intent_id);
        }
        drop(pool);

        // entry ถูกจัดการแล้ว -> backoff ของ intent นี้ไม่มีความหมายอีก
        self.place_retry_backoff.lock().await.remove(intent_id);

        // ปลดล็อกเงิน (lock order: pool -> balances ตรงกับ reevaluate_parked_orders)
        self.release_capital(&asset, amount).await;
        Some((asset, amount, parked))
    }

    /// คืนเงินจองเมื่อ place ถูกปฏิเสธหลัง CEM เคยจองไว้แล้ว
    /// (เช่น DedupStep block หรือ rate-limit/okx dispatch fail)
    /// -> เปลี่ยน `Approved` กลับเป็น `Parked` เพื่อ:
    ///    1) state ตรงกับ strategy (on_action_failed ย้าย order ไป parked)
    ///    2) reevaluate ปลุก retry ได้ในรอบถัดไป ไม่ lock ลอย
    /// no-op ถ้าไม่มี entry หรือไม่ใช่ Approved (เช่น slow-path denial ที่ไม่เคย lock)
    pub async fn revert_reservation(&self, intent_id: &str) -> Option<(String, Decimal)> {
        let mut pool = self.tracked_orders.lock().await;
        let order = pool.get_mut(intent_id)?;
        if order.status != IntentStatus::Approved {
            return None;
        }
        order.status = IntentStatus::Parked;
        let asset = order.required_asset.to_uppercase();
        let amount = order.locked_amount;
        drop(pool);

        self.release_capital(&asset, amount).await;
        log::warn!(
            "[CEM] ↩️ Place failed after reservation — reverted {} to Parked, {} {} returned to free",
            intent_id, amount, asset
        );
        // หน่วง reevaluate ปลุกซ้ำทันที (กันลูป approve -> place ล้ม -> Parked -> approve...)
        self.note_place_failure(intent_id).await;
        Some((asset, amount))
    }

    /// Strategy ไม่รู้จัก reactivated order (เช่น บอทถูกรีสตาร์ทแล้ว strategy state หาย)
    /// -> เก็บกวาด entry ออกจาก pool พร้อมคืนเงินจอง กันเงินถูกจองค้างตายโดยไม่มีใครวาง order
    pub async fn abandon_reactivated_order(&self, intent_id: &str) -> Option<(String, Decimal)> {
        let mut pool = self.tracked_orders.lock().await;
        let order = pool.get(intent_id)?;
        // เฉพาะ entry ที่เพิ่งถูก reserve โดย reevaluate เท่านั้น (ไม่งั้นแตะ order ที่ยัง live)
        if order.status != IntentStatus::Approved {
            return None;
        }
        let asset = order.required_asset.to_uppercase();
        let amount = order.locked_amount;
        pool.remove(intent_id);
        drop(pool);

        self.release_capital(&asset, amount).await;
        Some((asset, amount))
    }

    /// เก็บกวาด Entry ของ bot ที่หยุดทำงานออกจาก pool แล้วคืน lock ตามสถานะ
    /// - `Approved` / `CancelPending` -> คืน lock (เรียกหลัง recovery ยืนยันว่า order terminal แล้วเท่านั้น)
    /// - `Parked` -> เอาออกอย่างเดียว (ไม่ได้ถือ lock ตั้งแต่ตอน preempt)
    /// รวมถึงลบ flag `planned_preemption_cancel` ของ order เหล่านั้นด้วย
    ///
    /// เหตุผลที่จำเป็น: ปกติการปลดล็อกทำโดย event loop ของ executor ที่ยังรันอยู่คนใดคนหนึ่ง
    /// (executor แรกที่เห็น terminal event เรียก `complete_terminal_order`) — ถ้าเป็นบอทตัวสุดท้าย
    /// ของ account ไม่มีใครประมวลผล event ตอน stop -> lock ค้าง และเนื่องจาก CemManager เป็น
    /// shared ต่อ account ระดับ process -> `used` ค้างข้าม restart จน lock ขายไม่ได้
    ///
    /// คืนจำนวน entry ที่ถูกเก็บกวาด
    pub async fn release_bot_orders_on_stop(&self, bot_id: &str) -> usize {
        let mut pool = self.tracked_orders.lock().await;
        let ids: Vec<String> = pool
            .values()
            .filter(|o| o.bot_id == bot_id)
            .map(|o| o.intent_id.clone())
            .collect();
        if ids.is_empty() {
            return 0;
        }

        let mut cleaned = 0usize;
        let mut releases: Vec<(String, String, Decimal)> = Vec::new(); // (intent_id, asset, amount)
        for id in ids {
            let Some(order) = pool.remove(&id) else {
                continue;
            };
            cleaned += 1;
            // flag preemption ของ order นี้ไม่มีความหมายอีกต่อไป (บอทหยุดแล้ว)
            self.planned_preemption_cancels.lock().await.remove(&order.intent_id);
            match order.status {
                IntentStatus::Approved | IntentStatus::CancelPending => {
                    releases.push((
                        order.intent_id.clone(),
                        order.required_asset.to_uppercase(),
                        order.locked_amount,
                    ));
                }
                IntentStatus::Parked => {
                    log::info!(
                        "[CEM] 🛑 Stop cleanup: dropped parked order {} of bot {} (no lock held)",
                        order.intent_id, bot_id
                    );
                }
            }
        }
        drop(pool);

        for (intent_id, asset, amount) in releases {
            log::info!(
                "[CEM] 🛑 Stop cleanup: bot {} order {} is terminal on stop — releasing lock {} {}",
                bot_id, intent_id, amount, asset
            );
            self.release_capital(&asset, amount).await;
        }
        cleaned
    }

    /// ตรวจสอบ invariants ของ CEM state (bug report ข้อ 5) — คืนรายการที่ผิด (ว่าง = ผ่าน)
    /// 1. `free + used == total` ทุก asset
    /// 2. ผลรวม `locked_amount` ของ entry สถานะ Approved/CancelPending ต่อ asset == `used`
    ///    (จับได้ทั้ง "lock ลอย" ที่ไม่มี entry รองรับ และ entry ที่ถือ lock เกิน `used`)
    /// 3. entry ที่ถือ lock ต้องมี balance record ของ asset นั้นด้วย
    pub async fn invariant_violations(&self) -> Vec<String> {
        let pool = self.tracked_orders.lock().await;
        let balances = self.balances.lock().await;
        let mut violations = Vec::new();

        for (asset, entry) in balances.iter() {
            if entry.free + entry.used != entry.total {
                violations.push(format!(
                    "{asset}: free({}) + used({}) != total({})",
                    entry.free, entry.used, entry.total
                ));
            }
            let locked: Decimal = pool
                .values()
                .filter(|o| {
                    o.required_asset.eq_ignore_ascii_case(asset)
                        && matches!(
                            o.status,
                            IntentStatus::Approved | IntentStatus::CancelPending
                        )
                })
                .map(|o| o.locked_amount)
                .sum();
            if locked != entry.used {
                violations.push(format!(
                    "{asset}: sum of active-order locks ({locked}) != used ({})",
                    entry.used
                ));
            }
        }

        for order in pool.values() {
            if matches!(order.status, IntentStatus::Approved | IntentStatus::CancelPending)
                && !balances.contains_key(&order.required_asset.to_uppercase())
            {
                violations.push(format!(
                    "{}: active order {} holds lock {} but asset has no balance entry",
                    order.required_asset, order.intent_id, order.locked_amount
                ));
            }
        }
        violations
    }

    /// ดึงรายการ Tracked Orders ทั้งหมด
    pub async fn get_all_tracked_orders(&self) -> Vec<TrackedOrderForCem> {
        let pool = self.tracked_orders.lock().await;
        pool.values().cloned().collect()
    }

    /// ตรวจสอบว่า order_id นี้เป็นคำสั่ง Cancel ที่เกิดจากแผน Preemption ของ CEM หรือไม่
    pub async fn is_planned_preemption_cancel(&self, intent_id: &str) -> bool {
        let list = self.planned_preemption_cancels.lock().await;
        list.contains(intent_id)
    }

    /// บันทึก order_id ว่าถูกวางแผน Cancel เพื่อ Preempt เงินทุน
    pub async fn mark_preemption_cancel(&self, intent_id: &str) {
        let mut list = self.planned_preemption_cancels.lock().await;
        list.insert(intent_id.to_string());
    }

    /// ลบ order_id ออกจากรายการ Preemption Cancel เมื่อการ Cancel เสร็จสมบูรณ์
    pub async fn remove_preemption_cancel(&self, intent_id: &str) {
        let mut list = self.planned_preemption_cancels.lock().await;
        list.remove(intent_id);
    }

    /// ตรวจสอบและปลุกคำสั่งที่ถูก Park ไว้เมื่อ Balance เพิ่มขึ้น (Fill / Cancel / External Buy / Release)
    /// คืนรายการ Order ที่ได้รับการ Approve และล็อกเงินทุนเรียบร้อยแล้ว
    ///
    /// ปลุกแบบ global เรียงตาม Priority (ใกล้ mid ก่อน / LIFO) เพราะ:
    /// - ผู้แพ้ preemption ก็เป็นคนปลดล็อกเงินด้วย ถ้า owner-scoped จะปลุกตัวเองก่อนแล้ว
    ///  แย่งเงินคืนจากผู้ชนะ (priority สูงกว่า)
    /// - ผลที่ได้ต้องถูก route กลับบอทเจ้าของผ่าน `route_reactivation`
    pub async fn reevaluate_parked_orders(&self) -> Vec<TrackedOrderForCem> {
        let mut pool = self.tracked_orders.lock().await;
        let mut balances = self.balances.lock().await;

        // 1. คัดกรองเฉพาะคำสั่งที่อยู่ในสถานะ Parked
        let mut parked: Vec<TrackedOrderForCem> = pool
            .values()
            .filter(|o| o.status == crate::pipeline::simulate_cancel_plan::IntentStatus::Parked)
            .cloned()
            .collect();

        if parked.is_empty() {
            return Vec::new();
        }

        // 1.1 Refresh cem_priority ด้วย Mid Price ปัจจุบัน ป้องกัน Scale Mismatch
        // (เช่น ตอนสตาร์ทบอทแรกยังไม่มี BTC mid price ทำให้ได้ค่า ratio 0.xx ซึ่งจะแซงผิดคิว)
        {
            let mids = self.mid_prices.lock().await;
            let mid_btcusdt = mids.get("BTC/USDT").or_else(|| mids.get("BTC-USDT")).copied();
            if let Some(btc_mid) = mid_btcusdt {
                if btc_mid > DEC_ZERO {
                    for order in &mut parked {
                        if let Some(conv_px) = crate::pipeline::simulate_cancel_plan::convert_to_btcusdt_price(
                            &order.symbol,
                            order.price,
                            &mids,
                        ) {
                            let score = crate::pipeline::simulate_cancel_plan::calculate_distance_from_mid(conv_px, btc_mid);
                            order.cem_priority = Some(score);
                            if let Some(in_pool) = pool.get_mut(&order.intent_id) {
                                in_pool.cem_priority = Some(score);
                            }
                        }
                    }
                }
            }
        }

        // 2. เรียงลำดับตาม Priority (cem_priority ASC: เลขน้อยกว่าใกล้ราคาตลาดกว่า / created_at DESC)
        parked.sort_by(|a, b| {
            let pa = a.cem_priority.unwrap_or(crate::pipeline::simulate_cancel_plan::HIGH_PRIORITY);
            let pb = b.cem_priority.unwrap_or(crate::pipeline::simulate_cancel_plan::HIGH_PRIORITY);
            match pa.cmp(&pb) {
                std::cmp::Ordering::Equal => {
                    let ca = a.created_at.unwrap_or(0);
                    let cb = b.created_at.unwrap_or(0);
                    cb.cmp(&ca) // LIFO / newer first
                }
                other => other,
            }
        });

        let mut reactivated = Vec::new();

        // 3. วนลูปตรวจสอบยอดเงินคงเหลือทีละรายการ
        for order in &parked {
            // ข้าม intent ที่ place เพิ่งล้ม — อยู่ในช่วง backoff (กัน ticker ถี่ๆ ปลุกซ้ำไม่รู้จบ)
            if self.place_retry_blocked(&order.intent_id).await {
                log::debug!(
                    "[CEM] ⏳ Skipping reactivation of {} during place-failure backoff",
                    order.intent_id
                );
                continue;
            }
            let asset_key = order.required_asset.to_uppercase();
            let entry = balances.entry(asset_key.clone()).or_insert_with(|| AssetBalance::new(DEC_ZERO));

            if entry.free >= order.locked_amount && order.locked_amount > DEC_ZERO {
                // เงินพอ -> ล็อกเงิน (Virtual Lock)
                entry.free -= order.locked_amount;
                entry.used += order.locked_amount;

                // เปลี่ยนสถานะเป็น Approved ใน pool
                if let Some(target) = pool.get_mut(&order.intent_id) {
                    target.status = crate::pipeline::simulate_cancel_plan::IntentStatus::Approved;
                    reactivated.push(target.clone());
                }

                // ลบออกจาก planned_preemption_cancels (หากเคยติด flag preemption cancel มาก่อน)
                // เพื่อให้ออเดอร์ที่ถูกปลุกกลับมาวางใหม่ไม่ถูก In-Flight Abort Guard บล็อก
                self.planned_preemption_cancels.lock().await.remove(&order.intent_id);

                log::info!(
                    "[CEM] 🚀 [Reactivate] Parked order {} (bot: {}) APPROVED! Locked {} {}. Remaining free: {}",
                    order.intent_id, order.bot_id, order.locked_amount, order.required_asset, entry.free
                );
            }
        }

        // ถ้ามีออเดอร์ที่ได้รับการอนุมัติ (Reactivated) ในรอบนี้ ให้ส่งกลับไปวางทันที
        // ห้ามเข้า Preemption Phase ซ้ำในรอบเดียวกัน เพื่อป้องกันการ Cancel ทับออเดอร์ที่กำลังจะวาง
        if !reactivated.is_empty() {
            return reactivated;
        }

        // 4. Preemption Phase: ถ้ามี parked order ที่ยังค้างอยู่เพราะเงินไม่พอ
        // และไม่มี order ใดกำลังรอ cancel อยู่ (ไม่มี CancelPending ค้าง)
        let has_cancel_pending = pool.values().any(|o| {
            o.status == crate::pipeline::simulate_cancel_plan::IntentStatus::CancelPending
        });

        if !has_cancel_pending {
            // หา parked order ตัวแรกที่ยังเป็น Parked (เพราะ parked ถูก sort ตาม priority ASC แล้ว)
            if let Some(candidate) = parked.iter().find(|o| {
                pool.get(&o.intent_id).is_some_and(|p| {
                    p.status == crate::pipeline::simulate_cancel_plan::IntentStatus::Parked
                })
            }) {
                let current_free = balances
                    .get(&candidate.required_asset.to_uppercase())
                    .map(|b| b.free)
                    .unwrap_or(DEC_ZERO);

                // คัดกรอง orders ที่สามารถถูก preempt/cancel ได้:
                // - ต้องเป็น asset เดียวกัน
                // - live_confirmed บน OKX แล้ว
                // - can_be_canceled = true
                // - status = Approved
                let cancelable_candidates: Vec<TrackedOrderForCem> = pool
                    .values()
                    .filter(|o| {
                        o.intent_id != candidate.intent_id
                            && o.live_confirmed
                            && o.can_be_canceled
                            && o.status == crate::pipeline::simulate_cancel_plan::IntentStatus::Approved
                            && o.required_asset.eq_ignore_ascii_case(&candidate.required_asset)
                    })
                    .cloned()
                    .collect();

                if !cancelable_candidates.is_empty() {
                    let mid_prices: Vec<MidPrice> = {
                        let mids = self.mid_prices.lock().await;
                        mids.iter()
                            .map(|(pair, price)| MidPrice::new(pair.clone(), *price))
                            .collect()
                    };

                    let intent = OrderIntentForCem {
                        intent_id: candidate.intent_id.clone(),
                        symbol: candidate.symbol.clone(),
                        required_asset: candidate.required_asset.clone(),
                        price: candidate.price,
                        amount: candidate.locked_amount,
                        estimated_cost: candidate.locked_amount,
                    };

                    let plan = simulate_cancel_plan(
                        &intent,
                        current_free,
                        &cancelable_candidates,
                        &mid_prices,
                    );

                    if plan.can_approve_intent && !plan.orders_to_cancel.is_empty() {
                        log::info!(
                            "[CEM] 🚫 [Parked Preemption] Parked order {} (bot: {}, priority: {:?}) triggered preemption! Canceling orders: {:?} (reason: {})",
                            candidate.intent_id, candidate.bot_id, candidate.cem_priority, plan.orders_to_cancel, plan.reason
                        );

                        let trade_client = self.trade_client.lock().await.clone();

                        for cancel_id in &plan.orders_to_cancel {
                            let cancel_inst_id_code =
                                pool.get(cancel_id).and_then(|t| t.inst_id_code);
                            let cancel_symbol = pool
                                .get(cancel_id)
                                .map(|t| t.symbol.clone())
                                .unwrap_or_else(|| candidate.symbol.clone());

                            if let Some(target) = pool.get_mut(cancel_id) {
                                target.status =
                                    crate::pipeline::simulate_cancel_plan::IntentStatus::CancelPending;
                            }
                            self.mark_preemption_cancel(cancel_id).await;

                            if let Some(client) = &trade_client {
                                let cancel_arg = crate::okx::dto::trade::WsCancelOrderArg {
                                    inst_id: cancel_symbol,
                                    inst_id_code: cancel_inst_id_code,
                                    order_id: None,
                                    client_order_id: Some(cancel_id.clone()),
                                };
                                let client_clone = client.clone();
                                let cid_log = cancel_id.clone();
                                tokio::spawn(async move {
                                    if let Err(e) = client_clone.cancel_order_ws(cancel_arg).await {
                                        log::error!(
                                            "❌ [CEM Parked Preemption] Failed to cancel {}: {}",
                                            cid_log, e
                                        );
                                    }
                                });
                            }
                        }
                    }
                }
            }
        }

        reactivated
    }

    /// ลงทะเบียนรับ Reactivated Orders ของบอทนี้ (เรียกตอน executor เริ่ม)
    pub async fn register_reactivation_route(
        &self,
        bot_id: String,
        tx: tokio::sync::mpsc::UnboundedSender<TrackedOrderForCem>,
    ) {
        let mut routes = self.reactivation_routes.lock().await;
        routes.insert(bot_id, tx);
    }

    /// ยกเลิก route เมื่อ executor หยุด
    pub async fn unregister_reactivation_route(&self, bot_id: &str) {
        let mut routes = self.reactivation_routes.lock().await;
        routes.remove(bot_id);
    }

    /// ส่ง Reactivated Order ไปให้ executor ของบอทเจ้าของวางใหม่
    /// ถ้าไม่มี route (บอทหยุดไปแล้ว) -> คืนเงินจอง ปลุกกลับเป็น Parked เพื่อรอรอบหน้า
    pub async fn route_reactivation(&self, order: TrackedOrderForCem) {
        let intent_id = order.intent_id.clone();
        let bot_id = order.bot_id.clone();
        let tx = {
            let routes = self.reactivation_routes.lock().await;
            routes.get(&bot_id).cloned()
        };
        let delivered = match tx {
            Some(tx) => tx.send(order).is_ok(),
            None => false,
        };
        if delivered {
            return;
        }
        log::warn!(
            "[CEM] ⚠️ No reactivation route for bot {} (order {}) — reverting reservation",
            bot_id,
            intent_id
        );
        self.unreserve_reactivated_order(&intent_id).await;
    }

    /// คืนเงินจองของ reactivated order ที่ไม่มีเจ้ามารับ -> ปลุกกลับเป็น Parked
    /// (เรียกเฉพาะกับ order ที่ `reevaluate_parked_orders` เพิ่งปลุกเองเท่านั้น)
    pub async fn unreserve_reactivated_order(&self, intent_id: &str) {
        let mut pool = self.tracked_orders.lock().await;
        let mut balances = self.balances.lock().await;

        let Some(order) = pool.get_mut(intent_id) else {
            return;
        };
        if order.status != crate::pipeline::simulate_cancel_plan::IntentStatus::Approved {
            return;
        }
        order.status = crate::pipeline::simulate_cancel_plan::IntentStatus::Parked;

        let asset_key = order.required_asset.to_uppercase();
        let amount = order.locked_amount;
        let entry = balances
            .entry(asset_key.clone())
            .or_insert_with(|| AssetBalance::new(DEC_ZERO));
        entry.used = (entry.used - amount).max(DEC_ZERO);
        entry.free += amount;
        log::info!(
            "[CEM] ↩️ Unreserved reactivated order {}: returned {} {} to free",
            intent_id, amount, asset_key
        );
    }

    /// ตรวจสอบว่า order intent ถูก Preempt หรือถูกสั่ง Cancel ตัดหน้าไปแล้วหรือไม่
    pub async fn is_preempted_or_cancelled(&self, intent_id: &str) -> bool {
        if self.planned_preemption_cancels.lock().await.contains(intent_id) {
            return true;
        }
        let pool = self.tracked_orders.lock().await;
        if let Some(order) = pool.get(intent_id) {
            return matches!(
                order.status,
                crate::pipeline::simulate_cancel_plan::IntentStatus::CancelPending
                    | crate::pipeline::simulate_cancel_plan::IntentStatus::Parked
            );
        }
        false
    }
}

impl CemStep {
    /// ตรวจสอบว่า order ถูก Preempt หรือ Cancel ตัดหน้าก่อนส่งลง WebSocket หรือไม่
    pub async fn is_preempted_or_cancelled(&self, intent_id: &str) -> bool {
        match &self.manager {
            Some(manager) => manager.is_preempted_or_cancelled(intent_id).await,
            None => false,
        }
    }
    /// Delegate: คืนเงินจองเมื่อ place ถูกปฏิเสธหลังจอง (ไม่ทำอะไรถ้าไม่ได้ inject manager)
    pub async fn revert_reservation(&self, intent_id: &str) -> Option<(String, Decimal)> {
        match &self.manager {
            Some(manager) => manager.revert_reservation(intent_id).await,
            None => None,
        }
    }

    /// Delegate: OKX ปฏิเสธ place หลัง dispatch -> คืน lock + ตั้ง backoff ห้ามปลุกซ้ำทันที
    pub async fn handle_place_reject(&self, intent_id: &str) -> Option<(String, Decimal)> {
        match &self.manager {
            Some(manager) => manager.handle_place_reject(intent_id).await,
            None => None,
        }
    }

    /// Delegate: place ถูก dispatch สำเร็จ -> ล้าง place-failure backoff ของ intent นี้
    pub async fn note_place_succeeded(&self, intent_id: &str) {
        match &self.manager {
            Some(manager) => manager.clear_place_failure(intent_id).await,
            None => {}
        }
    }
}

/// Step 2: CEM Gatekeeper (Capital Efficiency Manager)
/// - ตรวจสอบความพร้อมของยอดเงินทุน (Balance Gate)
/// - ปล่อยผ่านคำสั่งที่ไม่ต้องใช้ทุนเพิ่ม (Cancel / Hold)
/// - คำนวณ Cancel Plan เมื่อเงินไม่พอ สำหรับ Place Order
#[derive(Clone, Default)]
pub struct CemStep {
    manager: Option<Arc<CemManager>>,
    trade_client: Option<crate::okx::ws_trade::OkxWsTradeClient>,
}

impl CemStep {
    pub fn new() -> Self {
        Self { manager: None, trade_client: None }
    }

    pub fn with_manager(manager: Arc<CemManager>) -> Self {
        Self {
            manager: Some(manager),
            trade_client: None,
        }
    }

    pub fn with_manager_and_client(
        manager: Arc<CemManager>,
        trade_client: crate::okx::ws_trade::OkxWsTradeClient,
    ) -> Self {
        if let Ok(mut guard) = manager.trade_client.try_lock() {
            *guard = Some(trade_client.clone());
        }
        Self {
            manager: Some(manager),
            trade_client: Some(trade_client),
        }
    }

    pub fn manager(&self) -> Option<Arc<CemManager>> {
        self.manager.clone()
    }

    pub async fn execute(&self, ctx: &mut PipelineContext) -> Result<(), String> {
        // หากยังไม่ได้เสียบ CemManager เข้ามา ให้ทำงานเป็น Pass-through เพื่อ backward-compatibility
        let manager = match &self.manager {
            Some(m) => m,
            None => {
                log::info!(
                    "[Bot #{}] 🛡️ [CEM Gatekeeper] Pass-through (no manager injected) for clOrdId: {}",
                    ctx.bot_id, ctx.client_order_id
                );
                return Ok(());
            }
        };

        // 1. CancelOrder: ปล่อยผ่านเสมอ ไม่ใช้เงินทุนเพิ่ม
        if ctx.op_type == OrderOpType::Cancel {
            log::info!(
                "[Bot #{}] 🛡️ [CEM Gatekeeper] Cancel order approved unconditionally: clOrdId={}",
                ctx.bot_id, ctx.client_order_id
            );
            return Ok(());
        }

        // 2. PlaceOrder: ตรวจสอบและจัดสรรเงินทุน
        if let StrategyAction::PlaceOrder { arg, .. } = &ctx.action {
            let symbol = arg.inst_id.clone();
            let side = arg.side.to_lowercase();
            let price_dec = arg.px.as_deref().unwrap_or("0").parse::<Decimal>().unwrap_or(DEC_ZERO);
            let size_dec = arg.sz.parse::<Decimal>().unwrap_or(DEC_ZERO);

            // แยก required_asset: Buy ใช้ Quote, Sell ใช้ Base
            let (base, quote) = match crate::pipeline::simulate_cancel_plan::split_symbol(&symbol) {
                Some(pair) => pair,
                None => (symbol.clone(), "USDT".to_string()),
            };

            let (required_asset, estimated_cost) = if side == "buy" {
                (quote, (price_dec * size_dec))
            } else {
                (base, size_dec)
            };

            // Reactivation path: เงินถูกจองไว้แล้วโดย `reevaluate_parked_orders`
            // -> ปล่อยผ่านทันที (Early Exit) โดยไม่ต้อง lock mid_prices หรือคำนวณ distance ซ้ำซ้อน
            {
                let pool = manager.tracked_orders.lock().await;
                if let Some(existing) = pool.get(&ctx.client_order_id) {
                    if existing.status
                        == crate::pipeline::simulate_cancel_plan::IntentStatus::Approved
                    {
                        log::info!(
                            "[Bot #{}] 🛡️ [CEM Gatekeeper] Reactivated order {} already reserved {} {} — passing without re-lock",
                            ctx.bot_id,
                            ctx.client_order_id,
                            existing.locked_amount,
                            existing.required_asset
                        );
                        return Ok(());
                    }
                }
            }

            // Fast Path: ล็อก แบบ atomic กับการตรวจเงิน (กัน race กับ reevaluate/terminal release
            // ของบอทอื่น) — เดิมเช็ค `current_free` ก่อนแล้วค่อย lock ทีหลังโดยละเลยผลลัพธ์
            // -> ถ้าเงินถูกแย่งไป lock ไม่สำเร็จ แต่ยัง approve + track เป็น Approved (order ผี)
            let now_ms = chrono::Utc::now().timestamp_millis();

            // Cem Priority = distance จาก BTC/USDT Mid (เทียบเท่า Python Legacy: calculate_order_distance_score)
            // ใช้จัดลำดับ: ยกเลิก "ไกล mid" ก่อน / ปลุก "ใกล้ mid" ก่อน
            let cem_priority = {
                let mids = manager.mid_prices.lock().await;
                let mid_btcusdt = mids.get("BTC/USDT").or_else(|| mids.get("BTC-USDT")).copied();
                match mid_btcusdt {
                    Some(btc_mid) if btc_mid > DEC_ZERO => {
                        crate::pipeline::simulate_cancel_plan::convert_to_btcusdt_price(&symbol, price_dec, &mids)
                            .map(|conv_px| crate::pipeline::simulate_cancel_plan::calculate_distance_from_mid(conv_px, btc_mid))
                    }
                    _ => {
                        // Fallback ถ้ายังไม่มีราคา BTC/USDT: ใช้สัดส่วนของคู่ตัวเอง
                        mids.get(&crate::pipeline::simulate_cancel_plan::normalize_symbol(&symbol))
                            .copied()
                            .filter(|mid| *mid > DEC_ZERO)
                            .map(|mid| (price_dec - mid).abs() / mid)
                    }
                }
            };

            if manager
                .try_fast_approve(TrackedOrderForCem {
                    intent_id: ctx.client_order_id.clone(),
                    symbol: symbol.clone(),
                    required_asset: required_asset.clone(),
                    price: price_dec,
                    locked_amount: estimated_cost,
                    can_be_canceled: true,
                    cem_priority,
                    status: crate::pipeline::simulate_cancel_plan::IntentStatus::Approved,
                    bot_id: ctx.bot_id.clone(),
                    created_at: Some(now_ms),
                    inst_id_code: arg.inst_id_code,
                    // เพิ่งสั่งวาง — ยังไม่ confirm ว่าถึง OKX; executor จะ mark เมื่อเห็น order event non-terminal
                    live_confirmed: false,
                })
                .await
            {
                // lock สำเร็จจริงเท่านั้น ค่อยอนุมัติ (ถ้าเงินถูกแย่งไปจะ fall through ไป slow path)
                log::info!(
                    "[Bot #{}] 🛡️ [CEM Gatekeeper] Fast-path APPROVED for {} (cost: {} {})",
                    ctx.bot_id, ctx.client_order_id, estimated_cost, required_asset
                );
                return Ok(());
            }

            // Slow Path: เงินไม่พอ (รวมเคส lock ไม่สำเร็จเพราะเงินถูกแย่งไประหว่างเช็ค)
            // -> อ่าน free ใหม่ ณ ตอนนี้ ก่อนจำลอง Cancel Plan (ค่าเก่าอาจ stale ไปแล้ว)
            let current_free = {
                let balances = manager.balances.lock().await;
                balances.get(&required_asset.to_uppercase()).map(|b| b.free).unwrap_or(DEC_ZERO)
            };
            log::warn!(
                "[Bot #{}] 🛡️ [CEM Gatekeeper] Deficit detected! Needed: {} {}, Free: {} {}. Simulating Cancel Plan...",
                ctx.bot_id, estimated_cost, required_asset, current_free, required_asset
            );

            let intent = OrderIntentForCem {
                intent_id: ctx.client_order_id.clone(),
                symbol: symbol.clone(),
                required_asset: required_asset.clone(),
                price: price_dec,
                amount: size_dec,
                estimated_cost,
            };

            let pending_orders = manager.get_all_tracked_orders().await;
            // คัดเฉพาะ order ที่ confirm live บน OKX จริง: ห้าม plan เลือก
            // - order ที่ place ยังไม่ถึง exchange (cancel ได้ 51400 "does not exist")
            // - ที่ถูก cancel ไปแล้ว (CancelPending — cancel ถูกยิงไปแล้ว)
            // - ตัว intent เอง (เคส reactivation: clOrdId ซ้ำกัน)
            let pending_orders: Vec<TrackedOrderForCem> = pending_orders
                .into_iter()
                .filter(|o| {
                    o.intent_id != ctx.client_order_id
                        && o.live_confirmed
                        && o.status == crate::pipeline::simulate_cancel_plan::IntentStatus::Approved
                })
                .collect();
            let mid_prices: Vec<MidPrice> = {
                let mids = manager.mid_prices.lock().await;
                mids.iter()
                    .map(|(pair, price)| MidPrice::new(pair.clone(), *price))
                    .collect()
            };

            let plan = simulate_cancel_plan(&intent, current_free, &pending_orders, &mid_prices);
            log::info!(
                "[Bot #{}] 🛡️ [CEM Gatekeeper] Cancel Plan result: can_approve={}, to_cancel={:?}, reason={}",
                ctx.bot_id, plan.can_approve_intent, plan.orders_to_cancel, plan.reason
            );

            // บันทึกคำสั่งนี้เป็น Parked ลงใน Tracked Orders Pool ของ CEM เพื่อรอ reactivate เมื่อ balance พร้อม
            // (ใช้ track_parked_intent: ห้ามเขียนทับ entry ที่ถือ reservation อยู่ — see track_parked_intent)
            manager
                .track_parked_intent(TrackedOrderForCem {
                    intent_id: ctx.client_order_id.clone(),
                    symbol: symbol.clone(),
                    required_asset: required_asset.clone(),
                    price: price_dec,
                    locked_amount: estimated_cost,
                    can_be_canceled: true,
                    cem_priority,
                    status: crate::pipeline::simulate_cancel_plan::IntentStatus::Parked,
                    bot_id: ctx.bot_id.clone(),
                    created_at: Some(now_ms),
                    inst_id_code: arg.inst_id_code,
                    live_confirmed: false,
                })
                .await;

            // บันทึก Plan ลงใน Context
            ctx.cancel_plan = Some(plan.clone());

            if plan.can_approve_intent {
                // Mark orders to cancel as CancelPending in pool and planned_preemption_cancels
                {
                    let mut pool = manager.tracked_orders.lock().await;
                    for cancel_id in &plan.orders_to_cancel {
                        let cancel_inst_id_code = pool.get(cancel_id).and_then(|t| t.inst_id_code);
                        if let Some(target) = pool.get_mut(cancel_id) {
                            target.status = crate::pipeline::simulate_cancel_plan::IntentStatus::CancelPending;
                        }
                        manager.mark_preemption_cancel(cancel_id).await;

                        // ถ้ามี trade_client เสียบเข้ามา ให้ dispatch CancelOrder ไปที่ exchange ทันที
                        if let Some(trade_client) = &self.trade_client {
                            let cancel_arg = crate::okx::dto::trade::WsCancelOrderArg {
                                inst_id: symbol.clone(),
                                inst_id_code: cancel_inst_id_code,
                                order_id: None,
                                client_order_id: Some(cancel_id.clone()),
                            };
                            log::info!(
                                "[Bot #{}] 🚫 [CEM Plan Execution] Dispatching cancel for preemption: clOrdId={}, instIdCode={:?}",
                                ctx.bot_id, cancel_id, cancel_inst_id_code
                            );
                            let client = trade_client.clone();
                            let cid_log = cancel_id.clone();
                            tokio::spawn(async move {
                                if let Err(e) = client.cancel_order_ws(cancel_arg).await {
                                    log::error!("❌ [CEM Plan Execution] Failed to cancel {}: {}", cid_log, e);
                                }
                            });
                        }
                    }
                }

                let err_msg = format!(
                    "CEM Gatekeeper requires canceling [{}] orders before approving clOrdId={}",
                    plan.orders_to_cancel.join(", "), ctx.client_order_id
                );
                ctx.reject(err_msg.clone());
                return Err(err_msg);
            } else {
                let err_msg = format!(
                    "CEM Gatekeeper rejected clOrdId={}: Insufficient capital (Parked with distance={})",
                    ctx.client_order_id, plan.intent_distance
                );
                ctx.reject(err_msg.clone());
                return Err(err_msg);
            }
        }

        // 3. AmendOrder: ปล่อยผ่าน (ในอนาคตคำนวณ cost diff)
        log::info!(
            "[Bot #{}] 🛡️ [CEM Gatekeeper] Amend order passed for clOrdId: {}",
            ctx.bot_id, ctx.client_order_id
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;
    use crate::okx::dto::trade::WsPlaceOrderArg;

    #[tokio::test]
    async fn test_cem_step_fast_path_approves_when_balance_sufficient() {
        let manager = Arc::new(CemManager::new());
        manager.update_balance_from_ws("USDT", dec!(1000.0), dec!(1000.0)).await;

        let cem_step = CemStep::with_manager(manager.clone());

        let action = StrategyAction::PlaceOrder {
            arg: WsPlaceOrderArg {
                id: None,
                inst_id: "BTC-USDT".to_string(),
                inst_id_code: None,
                td_mode: "cash".to_string(),
                side: "buy".to_string(),
                ord_type: "limit".to_string(),
                sz: "0.001".to_string(),
                px: Some("50000.0".to_string()),
                client_order_id: Some("test_buy_1".to_string()),
                tag: None,
            },
            reference_bid: None,
            reference_ask: None,
        };

        let mut ctx = PipelineContext::new(
            "bot1".to_string(),
            "acc1".to_string(),
            "BTC-USDT".to_string(),
            action,
        ).unwrap();

        let res = cem_step.execute(&mut ctx).await;
        assert!(res.is_ok());

        // ตรวจสอบว่าเงิน USDT ถูกล็อกไป 50 USDT (0.001 * 50000)
        let balances = manager.balances.lock().await;
        let usdt = balances.get("USDT").unwrap();
        assert_eq!(usdt.free, dec!(950.0));
        assert_eq!(usdt.used, dec!(50.0));
    }

    #[tokio::test]
    async fn test_cem_step_slow_path_generates_cancel_plan_when_deficit() {
        let manager = Arc::new(CemManager::new());
        manager.update_balance_from_ws("USDT", dec!(20.0), dec!(20.0)).await;
        manager.update_mid_price("BTC-USDT", dec!(50000.0)).await;

        // มี order ไกลเปิดอยู่บนกระดานจริง (live) ล็อก USDT ไว้ 100
        manager.track_order(TrackedOrderForCem {
            intent_id: "far_order_1".to_string(),
            symbol: "BTC-USDT".to_string(),
            required_asset: "USDT".to_string(),
            price: dec!(55000.0),
            locked_amount: dec!(100.0),
            can_be_canceled: true,
            cem_priority: None,
            status: crate::pipeline::simulate_cancel_plan::IntentStatus::Approved,
            bot_id: "bot1".to_string(),
            created_at: None,
            inst_id_code: None,
            // order นี้ live อยู่บน exchange จริง (มี state=live event แล้ว) -> เลือก preempt ได้
            live_confirmed: true,
        }).await;

        let cem_step = CemStep::with_manager(manager.clone());

        let action = StrategyAction::PlaceOrder {
            arg: WsPlaceOrderArg {
                id: None,
                inst_id: "BTC-USDT".to_string(),
                inst_id_code: None,
                td_mode: "cash".to_string(),
                side: "buy".to_string(),
                ord_type: "limit".to_string(),
                sz: "0.001".to_string(),
                px: Some("50100.0".to_string()), // cost = 50.1 > free 20
                client_order_id: Some("test_buy_2".to_string()),
                tag: None,
            },
            reference_bid: None,
            reference_ask: None,
        };

        let mut ctx = PipelineContext::new(
            "bot1".to_string(),
            "acc1".to_string(),
            "BTC-USDT".to_string(),
            action,
        ).unwrap();

        let res = cem_step.execute(&mut ctx).await;
        assert!(res.is_err());
        assert!(ctx.is_rejected());
        assert!(ctx.cancel_plan.is_some());

        let plan = ctx.cancel_plan.unwrap();
        assert_eq!(plan.orders_to_cancel, vec!["far_order_1".to_string()]);

        // ตรวจสอบว่า order test_buy_2 ถูก park ไว้ใน pool
        let tracked = manager.get_all_tracked_orders().await;
        let parked_order = tracked.iter().find(|o| o.intent_id == "test_buy_2");
        assert!(parked_order.is_some());
        assert_eq!(parked_order.unwrap().status, crate::pipeline::simulate_cancel_plan::IntentStatus::Parked);
    }

    #[tokio::test]
    async fn test_cem_reevaluate_parked_orders_when_balance_freed() {
        let manager = Arc::new(CemManager::new());
        manager.update_balance_from_ws("USDT", dec!(10.0), dec!(10.0)).await;

        // ใส่ Parked order ใน pool
        manager.track_order(TrackedOrderForCem {
            intent_id: "parked_1".to_string(),
            symbol: "BTC-USDT".to_string(),
            required_asset: "USDT".to_string(),
            price: dec!(50000.0),
            locked_amount: dec!(50.0),
            can_be_canceled: true,
            cem_priority: Some(dec!(100.0)),
            status: crate::pipeline::simulate_cancel_plan::IntentStatus::Parked,
            bot_id: "bot1".to_string(),
            created_at: Some(1000),
            inst_id_code: None,
            live_confirmed: false,
        }).await;

        // ก่อน balance เพิ่ม -> reevaluate ต้องไม่ปลุก
        let reactivated = manager.reevaluate_parked_orders().await;
        assert!(reactivated.is_empty());

        // จำลองมีเงินเข้ามา 100 USDT (เช่น จาก fill, cancel หรือ market buy)
        manager.update_balance_from_ws("USDT", dec!(110.0), dec!(110.0)).await;

        // หลัง balance เพิ่ม -> reevaluate ต้องปลุก parked_1
        let reactivated = manager.reevaluate_parked_orders().await;
        assert_eq!(reactivated.len(), 1);
        assert_eq!(reactivated[0].intent_id, "parked_1");
        assert_eq!(reactivated[0].status, crate::pipeline::simulate_cancel_plan::IntentStatus::Approved);

        // ตรวจสอบเงินถูกล็อก
        let balances = manager.balances.lock().await;
        let usdt = balances.get("USDT").unwrap();
        assert_eq!(usdt.free, dec!(60.0));
        assert_eq!(usdt.used, dec!(50.0));
    }

    #[tokio::test]
    async fn test_cem_multi_bot_preemption_and_reactivation_flow() {
        // จำลองกระบวนการจาก rqm.log:
        // 1. Bot #3 วาง Sell SUI ที่ราคาไกล 0.9967 (ใช้ SUI ทั้งหมด 50)
        // 2. Bot #2 ต้องการ Sell SUI ที่ราคาใกล้ 0.9190 (ต้องการ 50 SUI) แต่ SUI ขาดแคลน
        // 3. CEM ตัดสินให้ Bot #2 ชนะ (Preempt) -> สั่ง Cancel Bot #3 และ Park Bot #2
        // 4. เมื่อ Bot #3 ถูก Cancel สำเร็จ -> เงิน SUI คืนสู่ Pool -> Bot #2 ถูก Reactivate ทันที
        let manager = Arc::new(CemManager::new());
        manager.update_balance_from_ws("SUI", dec!(50.0), dec!(50.0)).await;

        // Step 1: Bot #3 ถือออเดอร์ Active บนกระดาน
        manager.track_order(TrackedOrderForCem {
            intent_id: "bot3_sell_far".to_string(),
            symbol: "SUI-USDT".to_string(),
            required_asset: "SUI".to_string(),
            price: dec!(0.9967),
            locked_amount: dec!(50.0),
            can_be_canceled: true,
            cem_priority: Some(dec!(0.9967)),
            status: crate::pipeline::simulate_cancel_plan::IntentStatus::Approved,
            bot_id: "bot_3".to_string(),
            created_at: Some(1000),
            inst_id_code: Some(2021032621970896),
            // จำลองว่า order นี้ live อยู่บนกระดานจริง (มี state=live event แล้ว)
            live_confirmed: true,
        }).await;
        manager.lock_capital("SUI", dec!(50.0)).await;

        let cem_step = CemStep::with_manager(manager.clone());

        // Step 2: Bot #2 ขอวาง Order ใกล้ตลาด
        let mut ctx = PipelineContext::new(
            "bot_2".to_string(),
            "acc1".to_string(),
            "SUI-USDT".to_string(),
            StrategyAction::PlaceOrder {
                arg: crate::okx::dto::trade::WsPlaceOrderArg {
                    id: None,
                    inst_id: "SUI-USDT".to_string(),
                    inst_id_code: None,
                    td_mode: "cash".to_string(),
                    side: "sell".to_string(),
                    ord_type: "post_only".to_string(),
                    sz: "50.0".to_string(),
                    px: Some("0.9190".to_string()),
                    client_order_id: Some("bot2_sell_near".to_string()),
                    tag: None,
                },
                reference_bid: None,
                reference_ask: None,
            },
        ).unwrap();

        // Step 3: Run CEM step -> สั่ง Cancel Bot #3 และ Park Bot #2
        let result = cem_step.execute(&mut ctx).await;
        assert!(result.is_err());
        assert!(manager.is_planned_preemption_cancel("bot3_sell_far").await);

        // Step 4: จำลอง Bot #3 Cancel เสร็จบน Exchange -> release_capital
        manager.remove_preemption_cancel("bot3_sell_far").await;
        let removed = manager.untrack_order("bot3_sell_far").await;
        assert!(removed.is_some());
        manager.release_capital("SUI", dec!(50.0)).await;

        // Step 5: CEM Reevaluate -> Bot #2 ถูกปลุกกลับมาวางใหม่
        let reactivated = manager.reevaluate_parked_orders().await;
        assert_eq!(reactivated.len(), 1);
        assert_eq!(reactivated[0].intent_id, "bot2_sell_near");
        assert_eq!(reactivated[0].status, crate::pipeline::simulate_cancel_plan::IntentStatus::Approved);
    }

    #[tokio::test]
    async fn test_cem_reactivated_place_passes_without_double_lock() {
        let manager = Arc::new(CemManager::new());
        manager.update_balance_from_ws("USDT", dec!(100.0), dec!(100.0)).await;

        // reevaluate จองเงิน 50 ไว้ให้ intent ที่ปลุกกลับมาแล้ว
        assert!(manager.lock_capital("USDT", dec!(50.0)).await);
        manager.track_order(TrackedOrderForCem {
            intent_id: "reactivated_1".to_string(),
            symbol: "BTC-USDT".to_string(),
            required_asset: "USDT".to_string(),
            price: dec!(50000.0),
            locked_amount: dec!(50.0),
            can_be_canceled: true,
            cem_priority: Some(dec!(0.0)),
            status: crate::pipeline::simulate_cancel_plan::IntentStatus::Approved,
            bot_id: "bot1".to_string(),
            created_at: Some(1),
            inst_id_code: None,
            live_confirmed: false,
        }).await;

        let cem_step = CemStep::with_manager(manager.clone());
        let action = StrategyAction::PlaceOrder {
            arg: WsPlaceOrderArg {
                id: None,
                inst_id: "BTC-USDT".to_string(),
                inst_id_code: None,
                td_mode: "cash".to_string(),
                side: "buy".to_string(),
                ord_type: "limit".to_string(),
                sz: "0.001".to_string(),
                px: Some("50000.0".to_string()),
                client_order_id: Some("reactivated_1".to_string()),
                tag: None,
            },
            reference_bid: None,
            reference_ask: None,
        };
        let mut ctx = PipelineContext::new(
            "bot1".to_string(),
            "acc1".to_string(),
            "BTC-USDT".to_string(),
            action,
        ).unwrap();

        let res = cem_step.execute(&mut ctx).await;
        assert!(res.is_ok());

        // เงินต้องถูกจองแค่รอบเดียว (place ของ reactivated order ห้าม lock ซ้ำ)
        let balances = manager.balances.lock().await;
        let usdt = balances.get("USDT").unwrap();
        assert_eq!(usdt.free, dec!(50.0));
        assert_eq!(usdt.used, dec!(50.0));
    }

    #[tokio::test]
    async fn test_reactivation_routes_to_owner_and_reverts_when_unregistered() {
        let manager = Arc::new(CemManager::new());
        manager.update_balance_from_ws("SUI", dec!(20.0), dec!(20.0)).await;
        manager.track_order(TrackedOrderForCem {
            intent_id: "parked_a".to_string(),
            symbol: "SUI-USDT".to_string(),
            required_asset: "SUI".to_string(),
            price: dec!(1.2),
            locked_amount: dec!(5.0),
            can_be_canceled: true,
            cem_priority: Some(dec!(0.05)),
            status: crate::pipeline::simulate_cancel_plan::IntentStatus::Parked,
            bot_id: "bot_a".to_string(),
            created_at: Some(1),
            inst_id_code: None,
            live_confirmed: false,
        }).await;

        // ยังไม่มี route -> route ต้องคืนเงินจองและปลุกกลับเป็น Parked (ไม่ lock เงินฟรี)
        let reactivated = manager.reevaluate_parked_orders().await;
        assert_eq!(reactivated.len(), 1);
        manager.route_reactivation(reactivated[0].clone()).await;
        let pool = manager.get_all_tracked_orders().await;
        assert_eq!(pool[0].status, crate::pipeline::simulate_cancel_plan::IntentStatus::Parked);
        {
            let balances = manager.balances.lock().await;
            let sui = balances.get("SUI").unwrap();
            assert_eq!(sui.free, dec!(20.0));
            assert_eq!(sui.used, dec!(0.0));
        }

        // ลงทะเบียน route -> ต้องส่ง order กลับให้บอทเจ้าของพร้อมเงินที่ถูกจองไว้
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        manager.register_reactivation_route("bot_a".to_string(), tx).await;
        let reactivated = manager.reevaluate_parked_orders().await;
        assert_eq!(reactivated.len(), 1);
        manager.route_reactivation(reactivated[0].clone()).await;
        let received = rx.try_recv().expect("owner must receive its reactivated order");
        assert_eq!(received.intent_id, "parked_a");
        let pool = manager.get_all_tracked_orders().await;
        assert_eq!(pool[0].status, crate::pipeline::simulate_cancel_plan::IntentStatus::Approved);
        let balances = manager.balances.lock().await;
        assert_eq!(balances.get("SUI").unwrap().used, dec!(5.0));
    }

    #[tokio::test]
    async fn test_preempted_order_parks_on_terminal_and_reactivates_when_funds_free() {
        let manager = Arc::new(CemManager::new());
        manager.update_balance_from_ws("SUI", dec!(20.0), dec!(20.0)).await;

        // Bot #3 ถือ sell ที่ล็อก SUI 10.3665 และถูก CEM preemption plan ยกเลิก
        assert!(manager.lock_capital("SUI", dec!(10.3665)).await);
        manager.track_order(TrackedOrderForCem {
            intent_id: "preempted_sell".to_string(),
            symbol: "SUI-USDT".to_string(),
            required_asset: "SUI".to_string(),
            price: dec!(1.2983),
            locked_amount: dec!(10.3665),
            can_be_canceled: true,
            cem_priority: Some(dec!(0.17)),
            status: IntentStatus::CancelPending, // ถูก mark โดย CEM plan execution
            bot_id: "bot_3".to_string(),
            created_at: Some(1),
            inst_id_code: None,
            live_confirmed: false,
        }).await;

        // 1) terminal -> ต้อง "คง entry ไว้เป็น Parked" ไม่ใช่ untrack ทิ้ง พร้อมคืนเงิน
        let completed = manager.complete_terminal_order("preempted_sell").await;
        assert!(completed.is_some());
        let (_, _, parked) = completed.unwrap();
        assert!(parked, "preempted order ต้องถูกเก็บเป็น Parked ไม่ใช่หายจาก pool");
        {
            let balances = manager.balances.lock().await;
            let sui = balances.get("SUI").unwrap();
            assert_eq!(sui.free, dec!(20.0));
            assert_eq!(sui.used, dec!(0.0));
        }
        let pool = manager.get_all_tracked_orders().await;
        assert_eq!(pool.len(), 1);
        assert_eq!(pool[0].status, IntentStatus::Parked);

        // 2) เรียกซ้ำ -> no-op (executor อื่นเห็นก่อนทำไปแล้ว) ห้าม release ซ้ำ
        assert!(manager.complete_terminal_order("preempted_sell").await.is_none());

        // 3) มีเงินว่าง -> reevaluate ต้องปลุก entry เดิมกลับมา
        let reactivated = manager.reevaluate_parked_orders().await;
        assert_eq!(reactivated.len(), 1);
        assert_eq!(reactivated[0].intent_id, "preempted_sell");
        assert_eq!(reactivated[0].status, IntentStatus::Approved);
        {
            let balances = manager.balances.lock().await;
            let sui = balances.get("SUI").unwrap();
            assert_eq!(sui.used, dec!(10.3665));
        }
    }

    #[tokio::test]
    async fn test_abandon_reactivated_order_returns_reservation() {
        let manager = Arc::new(CemManager::new());
        manager.update_balance_from_ws("SUI", dec!(20.0), dec!(20.0)).await;
        manager.track_order(TrackedOrderForCem {
            intent_id: "orphaned".to_string(),
            symbol: "SUI-USDT".to_string(),
            required_asset: "SUI".to_string(),
            price: dec!(1.2),
            locked_amount: dec!(5.0),
            can_be_canceled: true,
            cem_priority: Some(dec!(0.05)),
            status: IntentStatus::Approved, // เพิ่งถูก reserve โดย reevaluate
            bot_id: "bot_a".to_string(),
            created_at: Some(1),
            inst_id_code: None,
            live_confirmed: false,
        }).await;
        {
            let mut balances = manager.balances.lock().await;
            balances.get_mut("SUI").unwrap().free -= dec!(5.0);
            balances.get_mut("SUI").unwrap().used += dec!(5.0);
        }

        let abandoned = manager.abandon_reactivated_order("orphaned").await;
        assert!(abandoned.is_some());
        assert!(manager.get_all_tracked_orders().await.is_empty());
        {
            let balances = manager.balances.lock().await;
            let sui = balances.get("SUI").unwrap();
            assert_eq!(sui.free, dec!(20.0));
            assert_eq!(sui.used, dec!(0.0));
        }

        // เรียกซ้ำ / ไม่มี entry -> None
        assert!(manager.abandon_reactivated_order("orphaned").await.is_none());
    }

    /// Regression ของ incident app_2026-10-13_03-55 (สมมติฐาน H1):
    /// Fast-path ต้อง lock ให้สำเร็จ "ก่อน" ค่อยอนุมัติ — เคสจริงใน log:
    /// - place ของ bot #2 อ่าน free=11.5642 (≥ 6.4336) ผ่านเงื่อนไข fast-path
    /// - ระหว่างนั้น reevaluate ของ bot #3 แย่ง lock 10.3665 ไปก่อน (free เหลือ 1.1977)
    /// - เดิม `lock_capital()` คืน false แต่ถูกละเลย -> ยัง `track_order(Approved)`
    ///   + log "Fast-path APPROVED" โดยไม่มีบรรทัด Virtual Lock (ตรงกับ log 03:55:42.055)
    /// - ผล: entry ถือ lock ที่ไม่มีจริง, เมื่อ order terminal จะ release เกิน ->
    ///   `used` ต่ำกว่าจริง -> CEM approve เกิน capital limit (invariant ตอน stop ผิด)
    #[tokio::test]
    async fn test_fast_path_race_never_approves_without_lock() {
        let manager = Arc::new(CemManager::new());
        manager.update_balance_from_ws("SUI", dec!(20.0), dec!(20.0)).await;

        // order ของ bot #1 ที่ live จริง ถือ lock 8.4358 -> free = 11.5642
        assert!(manager.lock_capital("SUI", dec!(8.4358)).await);
        manager
            .track_order(TrackedOrderForCem {
                intent_id: "bot1_live".to_string(),
                symbol: "SUI-USDT".to_string(),
                required_asset: "SUI".to_string(),
                price: dec!(1.2537),
                locked_amount: dec!(8.4358),
                can_be_canceled: true,
                cem_priority: Some(dec!(0.01)),
                status: IntentStatus::Approved,
                bot_id: "bot_1".to_string(),
                created_at: Some(1),
                inst_id_code: None,
                live_confirmed: true,
            })
            .await;

        let cem_step = CemStep::with_manager(manager.clone());

        // ถือ pool lock เพื่อหยุด fast-path ของ place ใหม่ไว้ "หลังจาก" ที่มันเห็น free=11.5642
        let pool_guard = manager.tracked_orders.lock().await;

        let step = cem_step.clone();
        let place_task = tokio::spawn(async move {
            let mut ctx = PipelineContext::new(
                "bot_2".to_string(),
                "acc1".to_string(),
                "SUI-USDT".to_string(),
                StrategyAction::PlaceOrder {
                    arg: WsPlaceOrderArg {
                        id: None,
                        inst_id: "SUI-USDT".to_string(),
                        inst_id_code: None,
                        td_mode: "cash".to_string(),
                        side: "sell".to_string(),
                        ord_type: "post_only".to_string(),
                        sz: "6.4336".to_string(),
                        px: Some("1.2054".to_string()),
                        client_order_id: Some("bot2_heal".to_string()),
                        tag: None,
                    },
                    reference_bid: None,
                    reference_ask: None,
                },
            )
            .unwrap();
            step.execute(&mut ctx).await
        });

        // ปล่อยให้ place task อ่าน current_free (11.5642 >= 6.4336) แล้วค้างรอ pool lock
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // "reevaluate ของ bot #3" แย่ง lock 10.3665 ไปก่อน -> free เหลือ 1.1977 < 6.4336
        // (แก้ balances ขณะถือ pool lock อยู่ — track_order ของ bot3 ต้องทำหลังปล่อย pool lock
        //  ไม่งั้น deadlock กับ guard ตัวเอง)
        {
            let mut balances = manager.balances.lock().await;
            let sui = balances.get_mut("SUI").unwrap();
            sui.free -= dec!(10.3665);
            sui.used += dec!(10.3665);
        }
        drop(pool_guard);
        manager
            .track_order(TrackedOrderForCem {
                intent_id: "bot3_reactivated".to_string(),
                symbol: "SUI-USDT".to_string(),
                required_asset: "SUI".to_string(),
                price: dec!(1.2983),
                locked_amount: dec!(10.3665),
                can_be_canceled: true,
                cem_priority: Some(dec!(0.02)),
                status: IntentStatus::Approved,
                bot_id: "bot_3".to_string(),
                created_at: Some(2),
                inst_id_code: None,
                live_confirmed: false,
            })
            .await;

        let res = place_task.await.unwrap();
        assert!(
            res.is_err(),
            "lock ไม่สำเร็จ -> ห้าม fast-path approve ต้อง fall through ไป slow path (deficit)"
        );

        // invariant: entry ทุกตัวที่ถือ lock ต้องมีเงินจองจริงใน `used`
        let violations = manager.invariant_violations().await;
        assert!(
            violations.is_empty(),
            "ห้ามมี entry Approved โดยไม่มี lock จริง: {violations:?}"
        );
        let balances = manager.balances.lock().await;
        assert_eq!(balances.get("SUI").unwrap().free, dec!(1.1977));
    }

    /// Regression ของ incident ลูป 65 รอบ/วินาที (สมมติฐาน H4):
    /// เมื่อ place ล้มหลังจองเงิน (เช่น DedupStep block) แล้วกลับเป็น Parked —
    /// reevaluate ห้ามปลุกซ้ำทันที ไม่งั้น ticker ที่มาถี่ๆ จะทำลูป
    /// approve -> place ล้ม -> revert -> approve ... ไม่รู้จบ (เคสจริง 1,298 รอบใน 49 วินาที)
    #[tokio::test]
    async fn test_place_failure_backoff_limits_reactivation_loop() {
        let manager = Arc::new(CemManager::new());
        manager.update_balance_from_ws("SUI", dec!(20.0), dec!(20.0)).await;
        manager
            .track_order(TrackedOrderForCem {
                intent_id: "cl_loop".to_string(),
                symbol: "SUI-USDT".to_string(),
                required_asset: "SUI".to_string(),
                price: dec!(1.2983),
                locked_amount: dec!(10.3665),
                can_be_canceled: true,
                cem_priority: Some(dec!(0.05)),
                status: IntentStatus::Parked,
                bot_id: "bot_3".to_string(),
                created_at: Some(1),
                inst_id_code: None,
                live_confirmed: false,
            })
            .await;

        // ปลุกครั้งแรก -> จองเงิน
        assert_eq!(manager.reevaluate_parked_orders().await.len(), 1);

        // place ล้ม (dedup/rate-limit) -> คืนจองเป็น Parked
        assert!(manager.revert_reservation("cl_loop").await.is_some());

        // ต้องมี backoff: ห้ามปลุกซ้ำทันที
        let again = manager.reevaluate_parked_orders().await;
        assert!(
            again.is_empty(),
            "ต้องมี backoff ห้าม reevaluate ปลุก order ที่เพิ่ง place ล้มซ้ำทันที"
        );

        // หลัง backoff หมด -> ปลุกใหม่ได้ตามปกติ
        tokio::time::sleep(std::time::Duration::from_millis(700)).await;
        assert_eq!(
            manager.reevaluate_parked_orders().await.len(),
            1,
            "backoff ต้องหมดอายุแล้วปลุก retry ได้"
        );
    }

    #[tokio::test]
    async fn test_regular_terminal_order_untracks_and_releases() {
        let manager = Arc::new(CemManager::new());
        manager.update_balance_from_ws("SUI", dec!(20.0), dec!(20.0)).await;
        assert!(manager.lock_capital("SUI", dec!(8.4358)).await);
        manager.track_order(TrackedOrderForCem {
            intent_id: "normal_sell".to_string(),
            symbol: "SUI-USDT".to_string(),
            required_asset: "SUI".to_string(),
            price: dec!(1.2437),
            locked_amount: dec!(8.4358),
            can_be_canceled: true,
            cem_priority: Some(dec!(0.12)),
            status: IntentStatus::Approved,
            bot_id: "bot_1".to_string(),
            created_at: Some(1),
            inst_id_code: None,
            live_confirmed: false,
        }).await;

        let completed = manager.complete_terminal_order("normal_sell").await;
        let (_, _, parked) = completed.unwrap();
        assert!(!parked, "order ปกติ (ไม่ใช่ preemption) ต้อง untrack ออกจาก pool");
        assert!(manager.get_all_tracked_orders().await.is_empty());
        {
            let balances = manager.balances.lock().await;
            let sui = balances.get("SUI").unwrap();
            assert_eq!(sui.free, dec!(20.0));
            assert_eq!(sui.used, dec!(0.0));
        }
    }

    /// Helper สร้าง entry สำหรับ test stop-cleanup / invariant
    fn make_order(
        intent_id: &str,
        bot_id: &str,
        asset: &str,
        amount: Decimal,
        status: IntentStatus,
    ) -> TrackedOrderForCem {
        TrackedOrderForCem {
            intent_id: intent_id.to_string(),
            symbol: format!("{}-USDT", asset),
            required_asset: asset.to_string(),
            price: dec!(1.0),
            locked_amount: amount,
            can_be_canceled: true,
            cem_priority: Some(dec!(0.1)),
            status,
            bot_id: bot_id.to_string(),
            created_at: Some(1),
            inst_id_code: None,
            live_confirmed: false,
        }
    }

    /// Bug report ข้อ 1: stop bot ตัวสุดท้าย (ไม่มี bot อื่นรัน event loop) ต้องคืน lock
    /// ของตัวเองทั้งหมด — ไม่งั้น used ค้างข้าม restart (CemManager เป็น shared ต่อ account
    /// ระดับ process) แล้ว restart โดยไม่ restart app จะ lock ขายไม่ได้
    ///
    /// สเปกตัวเลขจาก bug report: capital SUI=20, sell=10.3665
    /// หลัง stop+restart ต้อง lock ใหม่ได้ -> free=9.6335, used=10.3665
    #[tokio::test]
    async fn test_stop_last_bot_releases_locks_allowing_clean_restart() {
        let manager = Arc::new(CemManager::new());
        manager.update_balance_from_ws("SUI", dec!(20.0), dec!(20.0)).await;
        manager.update_balance_from_ws("USDT", dec!(20.0), dec!(20.0)).await;

        // start #3: sell lock SUI 10.3665 + buy lock USDT 6.4336 (Approved)
        assert!(manager.lock_capital("SUI", dec!(10.3665)).await);
        manager
            .track_order(make_order("BOTsell3", "bot-3", "SUI", dec!(10.3665), IntentStatus::Approved))
            .await;
        assert!(manager.lock_capital("USDT", dec!(6.4336)).await);
        manager
            .track_order(make_order("BOTbuy3", "bot-3", "USDT", dec!(6.4336), IntentStatus::Approved))
            .await;
        manager.mark_preemption_cancel("BOTsell3").await; // เผื่อ flag ค้างอยู่

        // === stop #3: stop cleanup ต้องเก็บกวาด lock ของ bot ตัวเอง ===
        let cleaned = manager.release_bot_orders_on_stop("bot-3").await;
        assert_eq!(cleaned, 2, "ต้องปลดทั้ง sell (SUI) และ buy (USDT)");
        {
            let balances = manager.balances.lock().await;
            let sui = balances.get("SUI").unwrap();
            assert_eq!(sui.free, dec!(20.0), "stop bot สุดท้ายต้องคืน lock SUI ทั้งหมด");
            assert_eq!(sui.used, dec!(0.0));
            let usdt = balances.get("USDT").unwrap();
            assert_eq!(usdt.free, dec!(20.0), "คืน lock USDT ด้วย");
            assert_eq!(usdt.used, dec!(0.0));
        }
        assert!(manager.get_all_tracked_orders().await.is_empty(), "pool ต้องว่างหลัง stop");
        assert!(
            !manager.is_planned_preemption_cancel("BOTsell3").await,
            "preemption flag ของบอทที่หยุดต้องถูกลบด้วย"
        );
        assert!(manager.invariant_violations().await.is_empty());

        // === restart #3 โดยไม่ restart app: REST init ยอดใหม่ ===
        manager.update_balance_from_ws("SUI", dec!(20.0), dec!(20.0)).await;

        // lock sell ใหม่ -> ต้องได้ตามสเปก free=9.6335, used=10.3665
        assert!(
            manager.lock_capital("SUI", dec!(10.3665)).await,
            "lock หลัง restart ต้องสำเร็จ — ถ้า fail แปลว่า used ค้างจากก่อนหน้า (leak)"
        );
        {
            let balances = manager.balances.lock().await;
            let sui = balances.get("SUI").unwrap();
            assert_eq!(sui.free, dec!(9.6335));
            assert_eq!(sui.used, dec!(10.3665));
        }
    }

    /// Bug report ข้อ 4 (เคส 1): stop bot ที่ order ยังอยู่ใน PARKED (ถูก preempt แต่ยังไม่ถูกวางคืน)
    /// -> entry ต้องถูกเอาออกจาก pool และห้ามถูก reactivate "ผี" ภายหลัง
    #[tokio::test]
    async fn test_stop_with_parked_order_removes_entry_without_ghost_reactivation() {
        let manager = Arc::new(CemManager::new());
        manager.update_balance_from_ws("SUI", dec!(20.0), dec!(20.0)).await;

        // sell ถูก preempt -> Parked (lock คืนแล้วตอน terminal เหลือ entry ใน pool รอ reactivation)
        manager
            .track_order(make_order("BOTpreempted", "bot-3", "SUI", dec!(10.3665), IntentStatus::Parked))
            .await;

        // stop bot-3 ขณะ order อยู่ใน PARKED
        let cleaned = manager.release_bot_orders_on_stop("bot-3").await;
        assert_eq!(cleaned, 1);
        assert!(
            manager.get_all_tracked_orders().await.is_empty(),
            "Parked order ของบอทที่หยุดต้องถูกเอาออกจาก pool (กัน ghost reactivation)"
        );
        {
            let balances = manager.balances.lock().await;
            let sui = balances.get("SUI").unwrap();
            assert_eq!(sui.free, dec!(20.0), "Parked ไม่ถือ lock -> balance ห้ามเปลี่ยน");
            assert_eq!(sui.used, dec!(0.0));
        }
        // reevaluate ห้ามปลุก order ผีของบอทที่หยุดแล้วกลับมา
        assert!(manager.reevaluate_parked_orders().await.is_empty());
        assert!(manager.invariant_violations().await.is_empty());
    }

    /// Bug report ข้อ 4 (เคส race): stop ระหว่างที่ preemption cancel ยังไม่ confirm จาก OKX
    /// (entry CancelPending ยังถือ lock + flag ค้าง) -> ต้องคืน lock และลบ flag
    /// โดยไม่แตะ entry ของบอทอื่น
    #[tokio::test]
    async fn test_stop_with_cancel_pending_in_flight_releases_lock_and_keeps_other_bots() {
        let manager = Arc::new(CemManager::new());
        manager.update_balance_from_ws("SUI", dec!(20.0), dec!(20.0)).await;

        // bot-3: preempt cancel ยังไม่ confirm -> CancelPending ยังถือ lock
        assert!(manager.lock_capital("SUI", dec!(10.3665)).await);
        manager
            .track_order(make_order("BOTinflight", "bot-3", "SUI", dec!(10.3665), IntentStatus::CancelPending))
            .await;
        manager.mark_preemption_cancel("BOTinflight").await;

        // bot-2: มี order live ค้าง (lock 5.0) — ห้ามโดน stop ของ bot-3 ไปด้วย
        assert!(manager.lock_capital("SUI", dec!(5.0)).await);
        manager
            .track_order(make_order("BOTother", "bot-2", "SUI", dec!(5.0), IntentStatus::Approved))
            .await;

        let cleaned = manager.release_bot_orders_on_stop("bot-3").await;
        assert_eq!(cleaned, 1, "ต้องเก็บกวาดเฉพาะของ bot-3");
        {
            let balances = manager.balances.lock().await;
            let sui = balances.get("SUI").unwrap();
            assert_eq!(sui.used, dec!(5.0), "lock ของ bot-2 ต้องอยู่ครบ");
            assert_eq!(sui.free, dec!(15.0));
        }
        assert!(!manager.is_planned_preemption_cancel("BOTinflight").await);
        let pool = manager.get_all_tracked_orders().await;
        assert_eq!(pool.len(), 1);
        assert_eq!(pool[0].intent_id, "BOTother");
        assert!(manager.invariant_violations().await.is_empty());
    }

    /// Bug report ข้อ 5: helper invariant ต้องจับ leak ได้จริง (ไม่ใช่เขียวมั่ว)
    #[tokio::test]
    async fn test_invariant_violations_detects_lock_without_entry_and_passes_when_clean() {
        let manager = Arc::new(CemManager::new());
        manager.update_balance_from_ws("SUI", dec!(20.0), dec!(20.0)).await;

        // สถานะสะอาด: free+used==total ไม่มี entry ไม่มี lock
        assert!(manager.invariant_violations().await.is_empty());

        // lock ค้างโดยไม่มี entry รองรับ (ลักษณะของ leak หลัง stop ที่ไม่เก็บกวาด)
        assert!(manager.lock_capital("SUI", dec!(10.3665)).await);
        let violations = manager.invariant_violations().await;
        assert!(
            violations.iter().any(|v| v.contains("SUI")),
            "ต้องจับ used ที่ไม่มี entry ใน pool: {violations:?}"
        );

        // มี entry ตรงกัน -> ผ่าน
        manager
            .track_order(make_order("BOTok", "bot-1", "SUI", dec!(10.3665), IntentStatus::Approved))
            .await;
        assert!(manager.invariant_violations().await.is_empty());
    }

    /// Regression จาก zzz.log 2026-10-13 03:00:00 (lock ลอย 10.3665 SUI):
    /// reevaluate จองเงินให้ reactivated order แล้วอีก bot preempt flip entry เป็น CancelPending
    /// ระหว่างที่ place ของ order เดิมยังอยู่ใน pipeline -> place โดน reject (deficit)
    /// -> slow-path ต้องห้ามเขียนทับ entry ที่ถือ reservation ด้วย Parked
    /// (เดิม track_order(Parked) ทับ CancelPending โดยไม่คืน lock -> used ค้าง + invariant แดง)
    #[tokio::test]
    async fn test_slow_path_reject_does_not_clobber_reserved_entry() {
        let manager = Arc::new(CemManager::new());
        manager.update_balance_from_ws("SUI", dec!(20.0), dec!(20.0)).await;

        // เงิน 10.3665 ถูกจองให้ "BOTrace" และถูก preempt flip เป็น CancelPending (ยังถือ lock)
        assert!(manager.lock_capital("SUI", dec!(10.3665)).await);
        manager
            .track_order(make_order("BOTrace", "bot-3", "SUI", dec!(10.3665), IntentStatus::CancelPending))
            .await;
        // free เหลือ 9.6335 < 10.3665 -> place ของ "BOTrace" เอง (reactivation)เข้า slow path -> reject

        let cem_step = CemStep::with_manager(manager.clone());
        let action = StrategyAction::PlaceOrder {
            arg: WsPlaceOrderArg {
                id: None,
                inst_id: "SUI-USDT".to_string(),
                inst_id_code: None,
                td_mode: "cash".to_string(),
                side: "sell".to_string(),
                ord_type: "post_only".to_string(),
                sz: "10.3665".to_string(),
                px: Some("1.2983".to_string()),
                client_order_id: Some("BOTrace".to_string()),
                tag: None,
            },
            reference_bid: None,
            reference_ask: None,
        };
        let mut ctx = PipelineContext::new(
            "bot-3".to_string(),
            "acc-1".to_string(),
            "SUI-USDT".to_string(),
            action,
        )
        .unwrap();
        let res = cem_step.execute(&mut ctx).await;
        assert!(res.is_err(), "เงินไม่พอ -> ต้อง reject");

        // entry ที่ถือ reservation ต้องอยู่ครบ ห้ามถูกเขียนทับเป็น Parked
        let pool = manager.get_all_tracked_orders().await;
        assert_eq!(pool.len(), 1);
        assert_eq!(
            pool[0].status,
            IntentStatus::CancelPending,
            "ห้ามเขียนทับ entry ที่ถือ reservation ด้วย Parked"
        );
        assert!(
            manager.invariant_violations().await.is_empty(),
            "ห้ามมี lock ลอย (used ต้องตรงกับผลรวม lock ของ entry ที่ active)"
        );

        // reconcile: cancel ได้ 51400 เพราะ order ไม่เคยถึง OKX -> complete_terminal_order คืนเงิน
        let completed = manager.complete_terminal_order("BOTrace").await;
        assert!(completed.is_some());
        {
            let balances = manager.balances.lock().await;
            let sui = balances.get("SUI").unwrap();
            assert_eq!(sui.free, dec!(20.0));
            assert_eq!(sui.used, dec!(0.0));
        }
        assert!(manager.invariant_violations().await.is_empty());
    }

    /// Regression จาก zzz.log 03:00:00.531: cancel plan เลือก order ที่ place ยังไม่ถึง OKX
    /// -> cancel ได้ 51400 (order ไม่มีอยู่จริง) -> ต้องคัดเฉพาะ entry ที่ confirm live เท่านั้น
    /// และ `mark_order_live` (จาก live event) ต้องปลดล็อกสิทธิ์ถูกเลือกเมื่อ order ถึง exchange แล้ว
    #[tokio::test]
    async fn test_cancel_plan_skips_orders_not_yet_live_and_mark_order_live_flips_eligibility() {
        let manager = Arc::new(CemManager::new());
        manager.update_balance_from_ws("SUI", dec!(12.0), dec!(12.0)).await;

        // "BOTfar": ไกลสุด (priority สูง -> fallback ยกเลิกก่อน) + live บน OKX แล้ว ล็อก 3
        assert!(manager.lock_capital("SUI", dec!(3.0)).await);
        let mut far = make_order("BOTfar", "bot-2", "SUI", dec!(3.0), IntentStatus::Approved);
        far.cem_priority = Some(dec!(0.2));
        far.live_confirmed = true;
        manager.track_order(far).await;
        // "BOTnear": ใกล้กว่า (priority ต่ำ) แต่ place ยังไม่ confirm live ล็อก 5
        assert!(manager.lock_capital("SUI", dec!(5.0)).await);
        let mut near = make_order("BOTnear", "bot-3", "SUI", dec!(5.0), IntentStatus::Approved);
        near.cem_priority = Some(dec!(0.05));
        near.live_confirmed = false;
        manager.track_order(near).await;
        // free = 12 - 8 = 4 ; place ใหม่ต้องการ 10 -> deficit 6

        let cem_step = CemStep::with_manager(manager.clone());

        let place_action = |cl: &str| StrategyAction::PlaceOrder {
            arg: WsPlaceOrderArg {
                id: None,
                inst_id: "SUI-USDT".to_string(),
                inst_id_code: None,
                td_mode: "cash".to_string(),
                side: "sell".to_string(),
                ord_type: "post_only".to_string(),
                sz: "10".to_string(),
                px: Some("1.0".to_string()),
                client_order_id: Some(cl.to_string()),
                tag: None,
            },
            reference_bid: None,
            reference_ask: None,
        };

        // 1) ยังไม่มีใคร confirm live ยกเว้น BOTfar -> ห้าม plan แตะ BOTnear (cancel จะได้ 51400)
        let mut ctx1 = PipelineContext::new(
            "bot-1".to_string(), "acc-1".to_string(), "SUI-USDT".to_string(),
            place_action("probe_buy_1"),
        ).unwrap();
        assert!(cem_step.execute(&mut ctx1).await.is_err());
        let plan1 = ctx1.cancel_plan.expect("ต้องมี cancel plan");
        assert!(
            !plan1.orders_to_cancel.contains(&"BOTnear".to_string()),
            "ห้ามเลือก order ที่ยังไม่ confirm live: {:?}",
            plan1.orders_to_cancel
        );
        assert!(plan1.orders_to_cancel.contains(&"BOTfar".to_string()));

        // 2) BOTnear ถึง OKX แล้ว (live event) -> mark_order_live -> ถูกเลือกได้ตามลำดับ priority
        manager.mark_order_live("BOTnear").await;
        {
            let pool = manager.get_all_tracked_orders().await;
            let near_entry = pool.iter().find(|o| o.intent_id == "BOTnear").unwrap();
            assert!(near_entry.live_confirmed, "mark_order_live ต้อง flip live_confirmed");
        }
        let mut ctx2 = PipelineContext::new(
            "bot-1".to_string(), "acc-1".to_string(), "SUI-USDT".to_string(),
            place_action("probe_buy_2"),
        ).unwrap();
        assert!(cem_step.execute(&mut ctx2).await.is_err());
        let plan2 = ctx2.cancel_plan.expect("ต้องมี cancel plan");
        // deficit 6: ยกเลิก BOTfar(3) ก่อน (priority สูงกว่า) แล้วค่อย BOTnear(5) = freed 8 >= 6
        assert_eq!(
            plan2.orders_to_cancel,
            vec!["BOTfar".to_string(), "BOTnear".to_string()],
            "หลัง mark_order_live แล้ว BOTnear ต้องเข้าเกณฑ์ยกเลิกตาม priority DESC"
        );
    }

    /// ทดสอบกรณีศึกษาจริง:
    /// 1. Bot #3 วาง Sell 10 (priority ต่ำสุด = 0.3) -> Approved
    /// 2. Bot #2 (priority สูงสุด = 0.05) เข้ามา -> ทุนไม่พอ -> กลายเป็น Parked
    /// 3. Bot #3 กลายเป็น Live บน OKX (`mark_order_live`)
    /// 4. ระบบเรียก `reevaluate_parked_orders` -> ตรวจพบ Bot #2 Parked อยู่
    ///    -> ต้องสั่ง Preempt ยกเลิก Bot #3 ทันที!
    #[tokio::test]
    async fn test_parked_order_triggers_preemption_when_new_order_becomes_live() {
        let manager = Arc::new(CemManager::new());
        // ทุนทั้งหมดมี 12 SUI
        manager.update_balance_from_ws("SUI", dec!(12.0), dec!(12.0)).await;

        // 1. Bot #3 วาง Sell 10 SUI (priority 0.3 ไกลตลาด) -> Approved แต่ยังไม่ live
        assert!(manager.lock_capital("SUI", dec!(10.0)).await);
        let mut bot3_order = make_order("BOT3_order", "bot-3", "SUI", dec!(10.0), IntentStatus::Approved);
        bot3_order.cem_priority = Some(dec!(0.3));
        bot3_order.live_confirmed = false;
        manager.track_order(bot3_order).await;

        // Free SUI เหลือ 2.0 SUI
        // 2. Bot #2 ต้องการวาง Sell 6 SUI (priority 0.05 ใกล้ตลาดกว่า) -> ทุนไม่พอ -> Parked
        let mut bot2_parked = make_order("BOT2_order", "bot-2", "SUI", dec!(6.0), IntentStatus::Parked);
        bot2_parked.cem_priority = Some(dec!(0.05));
        bot2_parked.live_confirmed = false;
        manager.track_order(bot2_parked).await;

        // ตอนที่ Bot #3 ยังไม่ live: reevaluate ยังไม่สามารถ preempt Bot #3 ได้
        let reactivated = manager.reevaluate_parked_orders().await;
        assert!(reactivated.is_empty(), "เงินไม่พอและ Bot #3 ยังไม่ live ต้องยังไม่ reactivate หรือ cancel");
        {
            let pool = manager.get_all_tracked_orders().await;
            let b3 = pool.iter().find(|o| o.intent_id == "BOT3_order").unwrap();
            assert_eq!(b3.status, IntentStatus::Approved);
        }

        // 3. Bot #3 ได้รับ WS confirmation ว่า Live แล้ว!
        manager.mark_order_live("BOT3_order").await;

        // 4. เรียก reevaluate_parked_orders ทันทีที่มี live event
        let _ = manager.reevaluate_parked_orders().await;

        // 5. ตรวจสอบว่า Bot #3 ถูกสั่ง Cancel (CancelPending) เพื่อเคลียร์ทางให้ Bot #2 หรือไม่
        {
            let pool = manager.get_all_tracked_orders().await;
            let b3 = pool.iter().find(|o| o.intent_id == "BOT3_order").unwrap();
            assert_eq!(
                b3.status,
                IntentStatus::CancelPending,
                "Bot #3 ต้องถูก mark เป็น CancelPending โดย Preemption Phase ของ reevaluate_parked_orders"
            );
        }

        // 6. เมื่อ Bot #3 ถูก Cancel สำเร็จ (terminal event) -> คืนเงิน 10 SUI เข้า Free
        manager.complete_terminal_order("BOT3_order").await;
        // Free SUI ตอนนี้ = 2 + 10 = 12 SUI

        // 7. reevaluate รอบสอง -> Bot #2 ต้องได้รับการ Approved และ Reactivate ทันที!
        let reactivated2 = manager.reevaluate_parked_orders().await;
        assert_eq!(reactivated2.len(), 1);
        assert_eq!(reactivated2[0].intent_id, "BOT2_order");
        assert_eq!(reactivated2[0].status, IntentStatus::Approved);
    }
}
