//! Simulate Cancel Plan Module
//!
//! Refactored from legacy Python CEM (`cem_simulate_cancel_plan.py`).
//! Pure function for calculating capital efficiency & smart cancellation plans using mid prices.

use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const DEC_ZERO: Decimal = dec!(0);
pub const HIGH_DISTANCE: Decimal = dec!(999999);
pub const HIGH_PRIORITY: Decimal = dec!(999999);

/// ข้อมูล Mid Price ของแต่ละคู่เทรด
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MidPrice {
    pub pair: String, // e.g. "BTC/USDT", "SOL/USDT", "SOL/BTC"
    pub price: Decimal,
}

impl MidPrice {
    pub fn new(pair: impl Into<String>, price: Decimal) -> Self {
        Self {
            pair: pair.into(),
            price,
        }
    }
}

/// สถานะของ Intent ใน CEM Pool
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IntentStatus {
    Approved,
    Parked,
    CancelPending,
}

impl Default for IntentStatus {
    fn default() -> Self {
        Self::Approved
    }
}

/// ข้อมูล Order ที่อยู่ในระบบสำหรับนำมาประเมิน Cancel Plan
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackedOrderForCem {
    pub intent_id: String,
    pub symbol: String,         // e.g. "BTC-USDT" or "BTC/USDT"
    pub required_asset: String, // e.g. "USDT", "BTC"
    pub price: Decimal,
    pub locked_amount: Decimal,
    pub can_be_canceled: bool,
    pub cem_priority: Option<Decimal>,
    #[serde(default)]
    pub status: IntentStatus,
    #[serde(default)]
    pub bot_id: String,
    #[serde(default)]
    pub created_at: Option<i64>,
    #[serde(default)]
    pub inst_id_code: Option<u64>,
    /// ยืนยันแล้วว่า order มีตัวตน live บน OKX จริง (มี order event สถานะ non-terminal เข้ามา)
    /// ใช้คัดผู้ถูก preempt: ห้าม cancel plan เลือก order ที่ place ยังไม่ confirm
    /// (cancel จะได้ 51400 "order does not exist" — เคสจริง zzz.log 03:00:00.637)
    #[serde(default)]
    pub live_confirmed: bool,
}

/// ข้อมูล Order Intent ที่ต้องการขอใช้เงินทุน
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderIntentForCem {
    pub intent_id: String,
    pub symbol: String,
    pub required_asset: String,
    pub price: Decimal,
    pub amount: Decimal,
    pub estimated_cost: Decimal,
}

/// ข้อมูลคะแนนและเหตุผลสำหรับ Order ที่ถูก Park
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ParkedOrderScore {
    pub order_id: String,
    pub distance: Decimal,
    pub converted_price: Decimal,
    pub cem_priority: Decimal,
    pub reason: String,
}

/// ผลลัพธ์จากการจำลอง Cancel Plan
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancelPlanResult {
    pub can_approve_intent: bool,
    pub intent_distance: Decimal,
    pub orders_to_cancel: Vec<String>,
    pub orders_to_keep: Vec<String>,
    pub orders_to_park: Vec<ParkedOrderScore>,
    pub simulated_free_balance_after_cancel: Decimal,
    pub reason: String,
    pub decision_trace: Vec<String>,
}

// ================================================================
// Price Conversion Helper Functions
// ================================================================

/// Normalize symbol: "BTC-USDT" -> "BTC/USDT"
pub fn normalize_symbol(symbol: &str) -> String {
    symbol.replace('-', "/")
}

/// Split symbol into (Base, Quote)
pub fn split_symbol(symbol: &str) -> Option<(String, String)> {
    let normalized = normalize_symbol(symbol);
    let parts: Vec<&str> = normalized.split('/').collect();
    if parts.len() == 2 {
        Some((parts[0].to_string(), parts[1].to_string()))
    } else {
        None
    }
}

/// Convert order price to "Price at BTCUSDT" scale
///
/// Cases:
/// 1. BTC/USDT: No conversion needed
/// 2. XXX/BTC: `mid_xxxusdt / order_price_xxxbtc`
/// 3. XXX/USDT: `(order_price_xxxusdt / mid_price_xxxusdt) * mid_price_btcusdt`
/// 4. XXX/YYY: `((order_price * quote_usdt_mid) / base_usdt_mid) * btc_usdt_mid`
pub fn convert_to_btcusdt_price(
    order_pair: &str,
    order_price: Decimal,
    mid_price_map: &HashMap<String, Decimal>,
) -> Option<Decimal> {
    if order_price <= DEC_ZERO {
        return None;
    }

    let norm_pair = normalize_symbol(order_pair);
    let (base, quote) = split_symbol(&norm_pair)?;

    // Case 1: Already BTC/USDT
    if norm_pair == "BTC/USDT" {
        return Some(order_price);
    }

    // Case 2: XXX/BTC
    if quote == "BTC" {
        let base_usdt_pair = format!("{}/USDT", base);
        let base_usdt_mid = mid_price_map.get(&base_usdt_pair)?;
        if *base_usdt_mid <= DEC_ZERO {
            return None;
        }
        return Some(base_usdt_mid / order_price);
    }

    let btc_usdt_mid = mid_price_map.get("BTC/USDT")?;
    if *btc_usdt_mid <= DEC_ZERO {
        return None;
    }

    // Case 3: XXX/USDT
    if quote == "USDT" {
        let base_usdt_mid = mid_price_map.get(&norm_pair)?;
        if *base_usdt_mid <= DEC_ZERO {
            return None;
        }
        return Some((order_price / base_usdt_mid) * btc_usdt_mid);
    }

    // Case 4: XXX/YYY (other quote currencies)
    let base_usdt_pair = format!("{}/USDT", base);
    let quote_usdt_pair = format!("{}/USDT", quote);

    let base_usdt_mid = mid_price_map.get(&base_usdt_pair)?;
    let quote_usdt_mid = mid_price_map.get(&quote_usdt_pair)?;

    if *base_usdt_mid <= DEC_ZERO || *quote_usdt_mid <= DEC_ZERO {
        return None;
    }

    let order_value_usdt = order_price * quote_usdt_mid;
    Some((order_value_usdt / base_usdt_mid) * btc_usdt_mid)
}

/// Calculate distance from BTC/USDT mid price
pub fn calculate_distance_from_mid(converted_price: Decimal, mid_btcusdt: Decimal) -> Decimal {
    (converted_price - mid_btcusdt).abs()
}

pub fn build_mid_price_map(mid_prices: &[MidPrice]) -> HashMap<String, Decimal> {
    mid_prices
        .iter()
        .map(|mp| (normalize_symbol(&mp.pair), mp.price))
        .collect()
}

// ================================================================
// Core Simulation Function (Pure Function)
// ================================================================

/// Simulate cancel plan based on market distances from mid price.
///
/// Flow:
/// 1. Filter orders matching `intent.required_asset` and `can_be_canceled`
/// 2. Convert prices to BTC/USDT scale
/// 3. Filter orders whose distance >= intent distance
/// 4. Cumulative sum from furthest until deficit covered
/// 5. Smart parking: use surplus to keep closest orders
/// 6. Return plan with approved, parked, and keep orders
pub fn simulate_cancel_plan(
    intent: &OrderIntentForCem,
    current_free_balance: Decimal,
    pending_orders: &[TrackedOrderForCem],
    mid_prices: &[MidPrice],
) -> CancelPlanResult {
    let required_asset = &intent.required_asset;
    let capital_required = intent.estimated_cost;

    let mid_map = build_mid_price_map(mid_prices);
    let mid_btcusdt = match mid_map.get("BTC/USDT") {
        Some(price) if *price > DEC_ZERO => *price,
        _ => {
            log::warn!("[CEM] Missing BTC/USDT mid price - falling back to simple plan");
            return fallback_simple_plan(
                intent,
                current_free_balance,
                pending_orders,
                intent_distance_from_mid(intent, &mid_map),
            );
        }
    };

    // Calculate intent distance
    let convert_price_intent = convert_to_btcusdt_price(&intent.symbol, intent.price, &mid_map);
    let distance_intent = convert_price_intent
        .map(|p| calculate_distance_from_mid(p, mid_btcusdt))
        .unwrap_or(DEC_ZERO);

    // Step 1: Filter relevant orders (same required_asset & can_be_canceled & status == Approved)
    let relevant_orders: Vec<&TrackedOrderForCem> = pending_orders
        .iter()
        .filter(|o| {
            o.required_asset.eq_ignore_ascii_case(required_asset)
                && o.can_be_canceled
                && o.status == IntentStatus::Approved
        })
        .collect();

    if relevant_orders.is_empty() {
        log::warn!(
            "[CEM] No cancelable orders found for asset {}",
            required_asset
        );
        return CancelPlanResult {
            can_approve_intent: false,
            intent_distance: distance_intent,
            orders_to_cancel: Vec::new(),
            orders_to_keep: Vec::new(),
            orders_to_park: Vec::new(),
            simulated_free_balance_after_cancel: current_free_balance,
            reason: format!("No cancelable orders using {}", required_asset),
            decision_trace: vec![
                format!("Required: {} {}", capital_required, required_asset),
                format!("Available: {} {}", current_free_balance, required_asset),
                "No cancelable orders found".to_string(),
            ],
        };
    }

    // Step 2: Convert prices and filter distance >= intent_distance
    struct OrderScore<'a> {
        order: &'a TrackedOrderForCem,
        converted_price: Decimal,
        distance: Decimal,
    }

    let mut orders_with_price: Vec<OrderScore> = Vec::new();

    for order in &relevant_orders {
        if let Some(conv_price) = convert_to_btcusdt_price(&order.symbol, order.price, &mid_map) {
            let distance = calculate_distance_from_mid(conv_price, mid_btcusdt);
            if distance >= distance_intent {
                orders_with_price.push(OrderScore {
                    order,
                    converted_price: conv_price,
                    distance,
                });
            }
        }
    }

    if orders_with_price.is_empty() {
        log::warn!(
            "[CEM] No orders found with distance >= {}. Falling back to simple priority plan.",
            distance_intent
        );
        return fallback_simple_plan(
            intent,
            current_free_balance,
            &relevant_orders.into_iter().cloned().collect::<Vec<_>>(),
            Some(distance_intent),
        );
    }

    // legacy แต่ยังไม่ยืนยันว่าถูกหรือผิด
    // Step 3: Sort by converted_price ascending (closest to mid first)
    // orders_with_price.sort_by(|a, b| a.converted_price.cmp(&b.converted_price));

    // version ใหม่ (ที่ใช้และมั่นใจว่าถูก) - เก็บไว้เป็น reference
    // Step 3: Sort by distance ascending (closest to mid first)
    // ใช้ distance (|converted_price - mid|) เพราะทำงานถูกต้องทั้ง Buy และ Sell
    // (converted_price ทำงานถูกเฉพาะ Sell เพราะ Sell ที่ไกล = ราคาสูงกว่า
    //  แต่ Buy ที่ไกล = ราคาต่ำกว่า — เรียงด้วย converted_price จะสลับลำดับผิด)
    orders_with_price.sort_by(|a, b| a.distance.cmp(&b.distance));

    // Step 4: Calculate deficit
    let deficit = capital_required - current_free_balance;
    if deficit <= DEC_ZERO {
        let keep_ids: Vec<String> = relevant_orders
            .iter()
            .map(|o| o.intent_id.clone())
            .collect();
        return CancelPlanResult {
            can_approve_intent: true,
            intent_distance: distance_intent,
            orders_to_cancel: Vec::new(),
            orders_to_keep: keep_ids,
            orders_to_park: Vec::new(),
            simulated_free_balance_after_cancel: current_free_balance,
            reason: "Sufficient balance available".to_string(),
            decision_trace: vec![
                format!("Required: {} {}", capital_required, required_asset),
                format!("Available: {} {}", current_free_balance, required_asset),
                "No cancellation needed".to_string(),
            ],
        };
    }

    // Step 5: Cumulative sum from FURTHEST (reversed)
    // orders_with_price is ascending by converted_price, so reverse starts from furthest
    let mut prepare_orders: Vec<&OrderScore> = Vec::new();
    let mut freed_amount = DEC_ZERO;

    for item in orders_with_price.iter().rev() {
        prepare_orders.push(item);
        freed_amount += item.order.locked_amount;
        if freed_amount >= deficit {
            break;
        }
    }

    // If still not enough even after all candidate orders
    if freed_amount < deficit {
        let all_cancel_ids: Vec<String> = prepare_orders
            .iter()
            .map(|item| item.order.intent_id.clone())
            .collect();

        // Sort by converted_price for parking priority
        let mut sorted_prepare = prepare_orders.clone();
        sorted_prepare.sort_by(|a, b| a.converted_price.cmp(&b.converted_price));

        let all_park_scores: Vec<ParkedOrderScore> = sorted_prepare
            .iter()
            .map(|item| ParkedOrderScore {
                order_id: item.order.intent_id.clone(),
                distance: item.distance,
                converted_price: item.converted_price,
                cem_priority: item.distance,
                reason: "insufficient_capital".to_string(),
            })
            .collect();

        return CancelPlanResult {
            can_approve_intent: false,
            intent_distance: distance_intent,
            orders_to_cancel: all_cancel_ids,
            orders_to_keep: Vec::new(),
            orders_to_park: all_park_scores,
            simulated_free_balance_after_cancel: current_free_balance + freed_amount,
            reason: format!(
                "Insufficient capital (deficit: {} {})",
                deficit - freed_amount,
                required_asset
            ),
            decision_trace: vec![
                format!("Required: {} {}", capital_required, required_asset),
                format!("Available: {} {}", current_free_balance, required_asset),
                format!("Deficit: {} {}", deficit, required_asset),
                format!("Canceled all {} orders", prepare_orders.len()),
                format!("Freed: {} {}", freed_amount, required_asset),
                format!(
                    "Still short by: {} {}",
                    deficit - freed_amount,
                    required_asset
                ),
            ],
        };
    }

    // Step 6: Smart Parking - Use surplus to keep closest orders
    let surplus = freed_amount - deficit;
    let mut prepare_sorted_by_closeness = prepare_orders.clone();
    prepare_sorted_by_closeness.sort_by(|a, b| a.converted_price.cmp(&b.converted_price));

    let mut remaining_surplus = surplus;
    let mut final_status: HashMap<String, &'static str> = HashMap::new();

    for item in &prepare_sorted_by_closeness {
        let order = item.order;
        if remaining_surplus >= order.locked_amount {
            final_status.insert(order.intent_id.clone(), "approved");
            remaining_surplus -= order.locked_amount;
        } else {
            final_status.insert(order.intent_id.clone(), "parked");
        }
    }

    let approved_in_prepare: Vec<String> = final_status
        .iter()
        .filter(|(_, status)| **status == "approved")
        .map(|(id, _)| id.clone())
        .collect();

    let parked_ids: Vec<String> = final_status
        .iter()
        .filter(|(_, status)| **status == "parked")
        .map(|(id, _)| id.clone())
        .collect();

    let prepare_order_id_set: std::collections::HashSet<&str> = prepare_orders
        .iter()
        .map(|item| item.order.intent_id.as_str())
        .collect();

    let keep_ids: Vec<String> = relevant_orders
        .iter()
        .filter(|o| !prepare_order_id_set.contains(o.intent_id.as_str()))
        .map(|o| o.intent_id.clone())
        .collect();

    let mut final_approved = keep_ids.clone();
    final_approved.extend(approved_in_prepare.clone());

    let mut parked_items: Vec<&OrderScore> = prepare_orders
        .iter()
        .filter(|item| final_status.get(&item.order.intent_id) == Some(&"parked"))
        .copied()
        .collect();
    parked_items.sort_by(|a, b| a.converted_price.cmp(&b.converted_price));

    let parked_scores: Vec<ParkedOrderScore> = parked_items
        .iter()
        .map(|item| ParkedOrderScore {
            order_id: item.order.intent_id.clone(),
            distance: item.distance,
            converted_price: item.converted_price,
            cem_priority: item.distance,
            reason: "capital_needed_for_plan".to_string(),
        })
        .collect();

    let simulated_free_after = current_free_balance + freed_amount - deficit;

    CancelPlanResult {
        can_approve_intent: true,
        intent_distance: distance_intent,
        orders_to_cancel: parked_ids.clone(),
        orders_to_keep: final_approved.clone(),
        orders_to_park: parked_scores,
        simulated_free_balance_after_cancel: simulated_free_after,
        reason: format!(
            "Market-aware plan: cancel {}, keep {} orders",
            parked_ids.len(),
            final_approved.len()
        ),
        decision_trace: vec![
            format!("Required: {} {}", capital_required, required_asset),
            format!("Available: {} {}", current_free_balance, required_asset),
            format!("Deficit: {} {}", deficit, required_asset),
            format!("Freed: {} {}", freed_amount, required_asset),
            format!("Surplus: {} {}", surplus, required_asset),
            format!("Approved (kept): {}", approved_in_prepare.len()),
            format!("Parked: {}", parked_ids.len()),
            format!("Kept untouched: {}", keep_ids.len()),
        ],
    }
}

// ================================================================
// Fallback: Simple Priority Algorithm
// ================================================================

/// distance ของ intent จาก Mid ของคู่ตัวเอง (หน่วย: สัดส่วนของราคา เช่น 0.07 = 7%)
/// ใช้เทียบกับ `TrackedOrderForCem.cem_priority` ที่คำนวณตอน track ด้วยสเกลเดียวกัน
fn intent_distance_from_mid(
    intent: &OrderIntentForCem,
    mid_map: &HashMap<String, Decimal>,
) -> Option<Decimal> {
    mid_map
        .get(&normalize_symbol(&intent.symbol))
        .copied()
        .filter(|mid| *mid > DEC_ZERO)
        .map(|mid| (intent.price - mid).abs() / mid)
}

pub fn fallback_simple_plan(
    intent: &OrderIntentForCem,
    current_free_balance: Decimal,
    pending_orders: &[TrackedOrderForCem],
    intent_distance: Option<Decimal>,
) -> CancelPlanResult {
    let real_intent_distance = intent_distance.unwrap_or(HIGH_DISTANCE);
    let real_park_priority = intent_distance.unwrap_or(HIGH_PRIORITY);

    let _required_asset = &intent.required_asset;
    let capital_required = intent.estimated_cost;
    let deficit = capital_required - current_free_balance;

    if deficit <= DEC_ZERO {
        let keep_ids: Vec<String> = pending_orders.iter().map(|o| o.intent_id.clone()).collect();
        return CancelPlanResult {
            can_approve_intent: true,
            intent_distance: real_intent_distance,
            orders_to_cancel: Vec::new(),
            orders_to_keep: keep_ids,
            orders_to_park: Vec::new(),
            simulated_free_balance_after_cancel: current_free_balance,
            reason: "Sufficient balance (fallback)".to_string(),
            decision_trace: vec!["Fallback: Sufficient balance".to_string()],
        };
    }

    // 1) กรองเฉพาะ order ที่ล็อก "สกุลเดียวกับที่ intent ต้องการ" + Approved + ยกเลิกได้
    //    ถ้าไม่กรองสกุล จะไปยกเลิก order ผิดสกุล เช่น deficit SUI แต่ดันยกเลิก order ที่ล็อก USDT
    //    (เงินไม่ได้คืนสกุลที่ต้องการ -> plan ดูผ่านแต่ใช้จริงไม่ได้)
    let mut sorted_orders: Vec<TrackedOrderForCem> = pending_orders
        .iter()
        .filter(|o| {
            o.status == IntentStatus::Approved
                && o.can_be_canceled
                && o.required_asset
                    .eq_ignore_ascii_case(&intent.required_asset)
        })
        .cloned()
        .collect();

    // 2) รักษา order ที่ "ใกล้ตลาดกว่า" intent ไว้ (เทียบได้เฉพาะเมื่อรู้ priority ทั้งคู่)
    //    ส่วน order ที่ไม่รู้ priority ยังมีสิทธิ์ถูกยกเลิกแต่ถูกจัดไว้ท้ายๆ
    sorted_orders.retain(|o| match (o.cem_priority, intent_distance) {
        (Some(order_priority), Some(intent_priority)) => order_priority >= intent_priority,
        _ => true,
    });

    // 3) เรียงลำดับยกเลิก: ไกล mid ก่อน (cem_priority DESC)
    //    เดิม sort ASC จะไปยกเลิก order ที่ใกล้ตลาดที่สุดก่อน = ผิดทิศกับ market-aware plan
    //    ไม่รู้ priority = 0 = ถูกยกเลิกเป็นลำดับสุดท้าย
    sorted_orders.sort_by(|a, b| {
        let pa = a.cem_priority.unwrap_or(DEC_ZERO);
        let pb = b.cem_priority.unwrap_or(DEC_ZERO);
        pb.cmp(&pa)
    });

    let mut orders_to_cancel: Vec<String> = Vec::new();
    let mut freed_amount = DEC_ZERO;

    for order in &sorted_orders {
        if freed_amount >= deficit {
            break;
        }
        orders_to_cancel.push(order.intent_id.clone());
        freed_amount += order.locked_amount;
    }

    let can_approve = freed_amount >= deficit;
    let orders_to_keep: Vec<String> = sorted_orders
        .iter()
        .filter(|o| !orders_to_cancel.contains(&o.intent_id))
        .map(|o| o.intent_id.clone())
        .collect();

    let parked_scores: Vec<ParkedOrderScore> = orders_to_cancel
        .iter()
        .map(|oid| ParkedOrderScore {
            order_id: oid.clone(),
            distance: real_intent_distance,
            converted_price: DEC_ZERO,
            cem_priority: real_park_priority,
            reason: "fallback_simple_plan".to_string(),
        })
        .collect();

    let simulated_free = if can_approve {
        current_free_balance + freed_amount
    } else {
        current_free_balance
    };

    CancelPlanResult {
        can_approve_intent: can_approve,
        intent_distance: real_intent_distance,
        orders_to_cancel: orders_to_cancel.clone(),
        orders_to_keep,
        orders_to_park: parked_scores,
        simulated_free_balance_after_cancel: simulated_free,
        reason: format!(
            "Simple priority-based plan (fallback): freed {}",
            freed_amount
        ),
        decision_trace: vec![format!("Fallback: Freed {}", freed_amount)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_to_btcusdt_cases() {
        let mut mid_map = HashMap::new();
        mid_map.insert("BTC/USDT".to_string(), dec!(100000.0));
        mid_map.insert("SOL/USDT".to_string(), dec!(200.0));
        mid_map.insert("ETH/USDT".to_string(), dec!(3000.0));

        // Case 1: BTC/USDT
        let conv_btc = convert_to_btcusdt_price("BTC/USDT", dec!(105000.0), &mid_map);
        assert_eq!(conv_btc, Some(dec!(105000.0)));

        // Case 2: SOL/BTC (mid_xxxusdt / order_price_xxxbtc)
        // SOL/USDT = 200, order SOL/BTC = 0.002 -> 200 / 0.002 = 100000
        let conv_sol_btc = convert_to_btcusdt_price("SOL/BTC", dec!(0.002), &mid_map);
        assert_eq!(conv_sol_btc, Some(dec!(100000.0)));

        // Case 3: SOL/USDT ((order_price / base_usdt_mid) * btc_usdt_mid)
        // order SOL/USDT = 210, mid = 200, BTC mid = 100000 -> (210 / 200) * 100000 = 105000
        let conv_sol_usdt = convert_to_btcusdt_price("SOL-USDT", dec!(210.0), &mid_map);
        assert_eq!(conv_sol_usdt, Some(dec!(105000.0)));

        // Case 4: SOL/ETH
        // order SOL/ETH = 0.07, ETH/USDT = 3000 -> 0.07 * 3000 = 210 USDT
        // (210 / 200) * 100000 = 105000
        let conv_sol_eth = convert_to_btcusdt_price("SOL/ETH", dec!(0.07), &mid_map);
        assert_eq!(conv_sol_eth, Some(dec!(105000.0)));
    }

    #[test]
    fn test_simulate_cancel_plan_no_deficit() {
        let intent = OrderIntentForCem {
            intent_id: "int_1".to_string(),
            symbol: "BTC-USDT".to_string(),
            required_asset: "USDT".to_string(),
            price: dec!(100000.0),
            amount: dec!(0.001),
            estimated_cost: dec!(100.0),
        };

        let mid_prices = vec![MidPrice::new("BTC/USDT", dec!(100000.0))];
        let pending = vec![TrackedOrderForCem {
            intent_id: "ord_1".to_string(),
            symbol: "BTC-USDT".to_string(),
            required_asset: "USDT".to_string(),
            price: dec!(90000.0),
            locked_amount: dec!(50.0),
            can_be_canceled: true,
            cem_priority: None,
            status: IntentStatus::Approved,
            bot_id: "bot_1".to_string(),
            created_at: None,
            inst_id_code: None,
            live_confirmed: false,
        }];

        // free = 200 >= cost = 100
        let res = simulate_cancel_plan(&intent, dec!(200.0), &pending, &mid_prices);
        assert!(res.can_approve_intent);
        assert!(res.orders_to_cancel.is_empty());
        assert_eq!(res.orders_to_keep, vec!["ord_1".to_string()]);
    }

    #[test]
    fn test_simulate_cancel_plan_with_deficit_cancels_furthest() {
        let mid_prices = vec![
            MidPrice::new("BTC/USDT", dec!(100000.0)),
            MidPrice::new("SOL/USDT", dec!(200.0)),
        ];

        let intent = OrderIntentForCem {
            intent_id: "int_new".to_string(),
            symbol: "BTC-USDT".to_string(),
            required_asset: "USDT".to_string(),
            price: dec!(100050.0), // distance = 50
            amount: dec!(0.001),
            estimated_cost: dec!(100.0),
        };

        // Orders:
        // ord_near: price 100100 -> conv 100100 -> distance = 100
        // ord_far: price 101000 -> conv 101000 -> distance = 1000
        let pending = vec![
            TrackedOrderForCem {
                intent_id: "ord_near".to_string(),
                symbol: "BTC-USDT".to_string(),
                required_asset: "USDT".to_string(),
                price: dec!(100100.0),
                locked_amount: dec!(60.0),
                can_be_canceled: true,
                cem_priority: None,
                status: IntentStatus::Approved,
                bot_id: "bot_1".to_string(),
                created_at: None,
                inst_id_code: None,
                live_confirmed: false,
            },
            TrackedOrderForCem {
                intent_id: "ord_far".to_string(),
                symbol: "BTC-USDT".to_string(),
                required_asset: "USDT".to_string(),
                price: dec!(101000.0),
                locked_amount: dec!(70.0),
                can_be_canceled: true,
                cem_priority: None,
                status: IntentStatus::Approved,
                bot_id: "bot_2".to_string(),
                created_at: None,
                inst_id_code: None,
                live_confirmed: false,
            },
        ];

        // current_free = 40, deficit = 60
        // ord_far frees 70 >= 60 -> only ord_far is cancelled, ord_near is kept!
        let res = simulate_cancel_plan(&intent, dec!(40.0), &pending, &mid_prices);
        assert!(res.can_approve_intent);
        assert_eq!(res.orders_to_cancel, vec!["ord_far".to_string()]);
        assert!(res.orders_to_keep.contains(&"ord_near".to_string()));
    }

    fn tracked(
        id: &str,
        asset: &str,
        price: Decimal,
        locked: Decimal,
        priority: Option<Decimal>,
    ) -> TrackedOrderForCem {
        TrackedOrderForCem {
            intent_id: id.to_string(),
            symbol: "SUI-USDT".to_string(),
            required_asset: asset.to_string(),
            price,
            locked_amount: locked,
            can_be_canceled: true,
            cem_priority: priority,
            status: IntentStatus::Approved,
            bot_id: "bot-test".to_string(),
            created_at: Some(1),
            inst_id_code: None,
            live_confirmed: false,
        }
    }

    #[test]
    fn fallback_cancels_only_same_asset_and_furthest_priority_first() {
        // Deficit เป็น SUI -> ห้ามแตะ order ที่ล็อก USDT และต้องยกเลิกที่ "ไกล mid" ก่อน
        let intent = OrderIntentForCem {
            intent_id: "new_sell".to_string(),
            symbol: "SUI-USDT".to_string(),
            required_asset: "SUI".to_string(),
            price: dec!(1.1954),
            amount: dec!(6.4336),
            estimated_cost: dec!(6.4336),
        };
        let pending = vec![
            tracked("buy_usdt", "USDT", dec!(0.9), dec!(100.0), Some(dec!(0.3))),
            tracked(
                "sell_far",
                "SUI",
                dec!(1.2983),
                dec!(10.3665),
                Some(dec!(0.17)),
            ),
            tracked("sell_near", "SUI", dec!(1.12), dec!(5.0), Some(dec!(0.01))),
        ];

        // free = 1.1977 SUI -> deficit = 5.2359 SUI
        let plan = fallback_simple_plan(&intent, dec!(1.1977), &pending, Some(dec!(0.074)));

        assert!(plan.can_approve_intent);
        // ยกเลิกเฉพาะ SUI ที่ไกล mid ไม่แตะ order ผิดสกุล/order ที่ใกล้ตลาดกว่า intent
        assert_eq!(plan.orders_to_cancel, vec!["sell_far".to_string()]);
    }

    #[test]
    fn fallback_parks_intent_when_all_candidates_closer_to_market() {
        // intent ไกล mid กว่าทุก candidate -> ยกเลิกใครก็เสียของดีกว่า -> ต้อง park intent
        let intent = OrderIntentForCem {
            intent_id: "worst_sell".to_string(),
            symbol: "SUI-USDT".to_string(),
            required_asset: "SUI".to_string(),
            price: dec!(1.9),
            amount: dec!(6.4336),
            estimated_cost: dec!(6.4336),
        };
        let pending = vec![
            tracked(
                "sell_far",
                "SUI",
                dec!(1.2983),
                dec!(10.3665),
                Some(dec!(0.17)),
            ),
            tracked("sell_near", "SUI", dec!(1.12), dec!(5.0), Some(dec!(0.01))),
        ];

        let plan = fallback_simple_plan(&intent, dec!(1.1977), &pending, Some(dec!(0.7)));

        assert!(!plan.can_approve_intent);
        assert!(plan.orders_to_cancel.is_empty());
    }
}
