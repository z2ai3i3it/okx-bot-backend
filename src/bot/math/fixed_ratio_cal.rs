//! แปลงมาจาก fixed_ratio_cal.py แบบ 1:1 (logic เดิมทุกจุด)
//! ใช้ rust_decimal::Decimal แทน python Decimal เพื่อ precision เท่ากัน

use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};


/// เทียบเท่า 1e-10 ใน python (guard กัน division by ~0)
fn epsilon_guard() -> Decimal {
    Decimal::from_str("0.0000000001").unwrap()
}

/// เทียบเท่า 1e-16 ใน python (ใช้ใน cal_target_amount_improved)
fn epsilon_tiny() -> Decimal {
    Decimal::from_str("0.0000000000000001").unwrap()
}

use crate::bots_config::Side;

/// python: rounddown()
/// พฤติกรรมจริงของโค้ดเดิม = ปัดเข้าใกล้ 0 (truncate towards zero) เป็นทวีคูณของ decimal_factor
/// (บวก: floor ธรรมดา / ลบ: floor ค่า abs แล้วใส่ลบกลับ ซึ่งเท่ากับ truncate towards zero ทั้งคู่)
pub(crate) fn rounddown(value: Decimal, decimal_factor: Decimal) -> Decimal {
    if decimal_factor.is_zero() {
        return value;
    }
    let sign = if value.is_sign_negative() {
        -Decimal::ONE
    } else {
        Decimal::ONE
    };
    let quotient = (value.abs() / decimal_factor).floor();
    quotient * decimal_factor * sign
}

/// python: roundup()
/// พฤติกรรมจริง = ปัดออกจาก 0 (round away from zero) เป็นทวีคูณของ decimal_factor
pub(crate) fn roundup(value: Decimal, decimal_factor: Decimal) -> Decimal {
    if decimal_factor.is_zero() {
        return value;
    }
    let sign = if value.is_sign_negative() {
        -Decimal::ONE
    } else {
        Decimal::ONE
    };
    let quotient = (value.abs() / decimal_factor).ceil();
    quotient * decimal_factor * sign
}

/// python: min_size_action()
#[allow(clippy::too_many_arguments)]
pub fn min_size_action(
    side: Side,
    filled_price_qb: Decimal,
    base_balance_b: Decimal,
    quote_value_q: Decimal,
    base_additional_b: Decimal,
    quote_additional_q: Decimal,
    base_value_ratio_target: Decimal,
    min_size_b: Decimal,
) -> Decimal {
    let new_base_balance_b = match side {
        Side::Buy => {
            (base_balance_b + base_additional_b) * (Decimal::ONE - base_value_ratio_target)
                + min_size_b
        }
        Side::Sell => {
            (base_balance_b + base_additional_b) * (Decimal::ONE - base_value_ratio_target)
                - min_size_b
        }
    };

    if new_base_balance_b <= Decimal::ZERO {
        return Decimal::ZERO;
    }

    let new_quote_value_q = (quote_value_q + quote_additional_q) * base_value_ratio_target;

    if new_base_balance_b.abs() < epsilon_guard() {
        return Decimal::ZERO;
    }

    ((new_quote_value_q / new_base_balance_b) - filled_price_qb) / filled_price_qb
}

/// python: threshold_action()
#[allow(clippy::too_many_arguments)]
fn threshold_action(
    side: Side,
    filled_price_qb: Decimal,
    base_balance_b: Decimal,
    quote_value_q: Decimal,
    base_additional_b: Decimal,
    buy_threshold_ratio_target: Decimal,
    sell_threshold_ratio_target: Decimal,
    quote_additional_q: Decimal,
    base_value_ratio_target: Decimal,
) -> Decimal {
    let threshold_toggle = match side {
        Side::Buy => base_value_ratio_target - buy_threshold_ratio_target,
        Side::Sell => base_value_ratio_target + sell_threshold_ratio_target,
    };

    let new_base_value_q = filled_price_qb * (base_balance_b + base_additional_b);
    let new_quote_value_q = quote_value_q + quote_additional_q;
    let port_value_q = new_base_value_q + new_quote_value_q;

    if port_value_q <= Decimal::ZERO {
        return Decimal::ZERO;
    }

    let base_port_value_percent = new_base_value_q / port_value_q;
    let denominator = base_port_value_percent * (threshold_toggle - Decimal::ONE);

    if denominator.abs() < epsilon_guard() {
        log::warn!(
            "Division by ~0 avoided in threshold_action: base_port_value_percent={base_port_value_percent}, threshold_toggle={threshold_toggle}"
        );
        return Decimal::ZERO;
    }

    (base_port_value_percent - threshold_toggle) / denominator
}

/// python: cal_target_price()
#[allow(clippy::too_many_arguments)]
pub(crate) fn cal_target_price(
    side: Side,
    filled_price_qb: Decimal,
    base_balance_b: Decimal,
    quote_value_q: Decimal,
    base_value_ratio_target: Decimal,
    sell_threshold_ratio_target: Decimal,
    buy_threshold_ratio_target: Decimal,
    min_size_b: Decimal,
    decimal_place_price: Decimal,
    base_additional_b: Decimal,
    quote_additional_q: Decimal,
) -> Option<Decimal> {
    if filled_price_qb <= Decimal::ZERO {
        return None;
    }
    if (base_value_ratio_target - buy_threshold_ratio_target) <= Decimal::ZERO {
        return None;
    }
    if (base_value_ratio_target + sell_threshold_ratio_target) >= Decimal::ONE {
        return None;
    }

    let m = min_size_action(
        side,
        filled_price_qb,
        base_balance_b,
        quote_value_q,
        base_additional_b,
        quote_additional_q,
        base_value_ratio_target,
        min_size_b,
    );
    let t = threshold_action(
        side,
        filled_price_qb,
        base_balance_b,
        quote_value_q,
        base_additional_b,
        buy_threshold_ratio_target,
        sell_threshold_ratio_target,
        quote_additional_q,
        base_value_ratio_target,
    );

    let target_price = match side {
        Side::Buy => rounddown(
            (m.min(t) + Decimal::ONE) * filled_price_qb,
            decimal_place_price,
        ),
        Side::Sell => roundup(
            (m.max(t) + Decimal::ONE) * filled_price_qb,
            decimal_place_price,
        ),
    };

    Some(target_price)
}

/// python: cal_base_value_q()
fn cal_base_value_q(
    price_qb: Decimal,
    base_balance_b: Decimal,
    base_additional_b: Decimal,
) -> Decimal {
    price_qb * (base_balance_b + base_additional_b)
}

/// python: cal_quote_value_q()
fn cal_quote_value_q(quote_value_q: Decimal, quote_additional_q: Decimal) -> Decimal {
    quote_value_q + quote_additional_q
}

/// python: cal_target_amount_improved()
/// หมายเหตุ: ต้นฉบับหาร amount_needed ด้วย (1 - 0) เสมอ (fee ถูก comment ทิ้งไว้)
/// ทำให้ค่า fee ที่รับเข้ามาไม่มีผลต่อผลลัพธ์จริง ๆ คงพฤติกรรมนี้ไว้เพื่อความเข้ากันได้
#[allow(clippy::too_many_arguments)]
pub(crate) fn cal_target_amount_improved(
    side: Side,
    next_price_qb: Decimal,
    base_balance_b: Decimal,
    quote_value_q: Decimal,
    base_value_ratio_target: Decimal,
    sell_threshold_ratio_target: Decimal,
    buy_threshold_ratio_target: Decimal,
    min_size_b: Decimal,
    decimal_place_amount: Decimal,
    _fee: Decimal,
    base_additional_b: Decimal,
    quote_additional_q: Decimal,
) -> Decimal {
    log::debug!(
        "cal_target_amount_improved: side={:?}, next_price_qb={next_price_qb}, base_balance_b={base_balance_b}, quote_value_q={quote_value_q}, base_value_ratio_target={base_value_ratio_target}, sell_threshold_ratio_target={sell_threshold_ratio_target}, buy_threshold_ratio_target={buy_threshold_ratio_target}, min_size_b={min_size_b}, decimal_place_amount={decimal_place_amount}, base_additional_b={base_additional_b}, quote_additional_q={quote_additional_q}",
        side
    );
    let new_base_value_q = cal_base_value_q(next_price_qb, base_balance_b, base_additional_b);
    let new_quote_value_q = cal_quote_value_q(quote_value_q, quote_additional_q);
    let new_port_value_q = new_base_value_q + new_quote_value_q;

    if new_port_value_q <= Decimal::ZERO {
        log::warn!("Portfolio value <= 0");
        return Decimal::ZERO;
    }

    let base_port_value_percent = new_base_value_q / new_port_value_q;

    let threshold_met = match side {
        Side::Sell => {
            let deviation = base_port_value_percent - base_value_ratio_target + epsilon_tiny();
            deviation >= sell_threshold_ratio_target
        }
        Side::Buy => {
            let deviation = base_value_ratio_target - base_port_value_percent + epsilon_tiny();
            deviation >= buy_threshold_ratio_target
        }
    };

    if threshold_met {
        let target_base_value = new_port_value_q * base_value_ratio_target;

        let amount_needed = (target_base_value - new_base_value_q) / next_price_qb;
        // สำหรับ คำนวณเ amount เผื่อค่าธรรมเนียม (1-fee) ถ้าไม่เผื่อให้เอาออก หรือใส่ _fee = 0
        // แยก line เพื่อให้เข้าใจง่าย
        let amount_needed = amount_needed / (Decimal::ONE - Decimal::ZERO);

        if amount_needed.abs() >= min_size_b {
            return rounddown(amount_needed, decimal_place_amount);
        }
    }

    Decimal::ZERO
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// สร้าง clOrdId ตามกติกาของ OKX: alphanumeric ล้วน ยาวไม่เกิน 32 ตัวอักษร
/// ใช้ prefix "BOT" + hex จาก uuid v4 (ตัด "-" ออกเพราะ OKX ไม่รับ "-") ตัดให้พอดี 32 ตัว
pub(crate) fn generate_cl_ord_id() -> String {
    let hex = uuid::Uuid::new_v4().simple().to_string(); // 32 hex chars, alnum ล้วน
    let mut id = format!("BOT{hex}");
    id.truncate(32);
    id
}

/// ค่า jitter factor สุ่ม 0.5–1.0 — ใช้กระจายเวลารอ (back off) ให้แต่ละ bot ไม่รัวพร้อมกัน
/// (กัน thundering herd ตอน network กลับมา) — สุ่มจาก UUID v4 (OS RNG)
pub(crate) fn jitter_factor() -> f64 {
    let r = (uuid::Uuid::new_v4().as_u128() % 500 + 500) as f64 / 1000.0;
    r
}

// #[derive(Debug, Clone, Serialize, Deserialize)]
// pub struct TargetAnchorOrder {
//     pub side: Side,
//     pub price_qb: Decimal,
//     pub amount_b: Decimal,
// }

// /// record เดียวกับที่ python คืนเป็น dict แล้วเอาไป insert DB
// /// ที่นี่คือ record ที่จะถูก append ลง /logs/{bot_name}.json (ไฟล์แยกตาม bot)
// #[derive(Debug, Clone, Serialize, Deserialize)]
// pub struct RebalanceRecord {
//     #[serde(rename = "_id")]
//     pub id: String,
//     #[serde(rename = "clOrdId")]
//     pub cl_ord_id: String,
//     pub sequence: u64,
//     pub symbol: String,
//     #[serde(rename = "type")]
//     pub order_type: String,
//     pub side: String,
//     pub fee_type: String,
//     pub target_anchor_buy: Option<TargetAnchorOrder>,
//     pub target_anchor_sell: Option<TargetAnchorOrder>,
//     pub price_qb: Decimal,
//     pub base_balance_before_b: Decimal,
//     pub base_value_before_q: Decimal,
//     pub quote_value_before_q: Decimal,
//     pub port_value_before_q: Decimal,
//     pub got_b: Decimal,
//     pub net_base_balance_b: Decimal,
//     pub base_value_after_q: Decimal,
//     pub quote_value_after_q: Decimal,
//     pub port_value_after_q: Decimal,
//     pub base_additional_b: Decimal,
//     pub quote_additional_q: Decimal,
//     #[serde(rename = "createdAt")]
//     pub created_at: String,
// }

/// python: save_to_db_first_filled_rebalance()
/// ใช้ตอน rebalance ครั้งแรกสุด (ยังไม่มี snapshot ก่อนหน้า)
/// _id และ clOrdId ถูกสร้างขึ้นภายในฟังก์ชันนี้เอง (uuid v4 / OKX-compatible id)
#[allow(clippy::too_many_arguments)]
pub(crate) fn save_to_db_first_filled_rebalance(
    order_type: &str,
    side: &str,
    symbol: &str,
    fee_type: &str,
    initial_capital_b: Decimal,
    initial_capital_q: Decimal,
    price_qb: Decimal,
    amount_b: Decimal, // + = ได้ base มา (buy), - = เสีย base ไป (sell)
    fee: Decimal,
    target_anchor_buy: Option<TargetAnchorOrder>,
    target_anchor_sell: Option<TargetAnchorOrder>,
) -> RebalanceRecord {
    let base_balance_before_b = initial_capital_b;
    let base_value_before_q = base_balance_before_b * price_qb;
    let quote_balance_before_q = initial_capital_q;
    let port_value_before_q = base_value_before_q + quote_balance_before_q;

    let net_base_balance_b = initial_capital_b
        + if amount_b > Decimal::ZERO {
            amount_b * (Decimal::ONE - Decimal::ZERO) // นี่เป็ค่าธรรมเนียมแบบ QQ: Buy ใช้ Quote และ Sell ใช้ Quote
        } else {
            amount_b
        };

    let base_value_after_q = net_base_balance_b * price_qb;

    let quote_value_after_q = quote_balance_before_q
        + if amount_b < Decimal::ZERO {
            (price_qb * (-amount_b)) * (Decimal::ONE - fee)
        } else {
            (price_qb * (-amount_b)) * (Decimal::ONE + fee) // นี่เป็ค่าธรรมเนียมแบบ QQ: Buy ใช้ Quote และ Sell ใช้ Quote
        };

    let port_value_after_q = base_value_after_q + quote_value_after_q;

    RebalanceRecord {
        id: uuid::Uuid::new_v4().to_string(),
        cl_ord_id: generate_cl_ord_id(),
        sequence: 1,
        symbol: symbol.to_string(),
        order_type: order_type.to_string(),
        side: side.to_string(),
        fee_type: fee_type.to_string(),
        target_anchor_buy,
        target_anchor_sell,
        price_qb,
        base_balance_before_b,
        base_value_before_q,
        quote_value_before_q: quote_balance_before_q,
        port_value_before_q,
        got_b: amount_b,
        net_base_balance_b,
        base_value_after_q,
        quote_value_after_q,
        port_value_after_q,
        base_additional_b: Decimal::ZERO,
        quote_additional_q: Decimal::ZERO,
        created_at: now_ms().to_string(),
    }
}

/// python: save_to_db_filled_rebalance()
/// ใช้ต่อจาก record ก่อนหน้า (last_*) เพื่อคำนวณ snapshot ใหม่หลัง fill
/// _id และ clOrdId ถูกสร้างขึ้นภายในฟังก์ชันนี้เอง (uuid v4 / OKX-compatible id)
#[allow(clippy::too_many_arguments)]
pub(crate) fn save_to_db_filled_rebalance(
    order_type: &str,
    side: &str,
    symbol: &str,
    fee_type: &str,
    next_sequence: u64,
    last_net_base_balance_b: Decimal,
    price_qb: Decimal,
    amount_b: Decimal,
    fee: Decimal,
    last_quote_value_after_q: Decimal,
    last_base_additional_b: Decimal,
    last_quote_additional_q: Decimal,
    target_anchor_buy: Option<TargetAnchorOrder>,
    target_anchor_sell: Option<TargetAnchorOrder>,
) -> RebalanceRecord {
    let new_base_balance_before_b = last_net_base_balance_b + last_base_additional_b;
    let new_base_value_before_q = price_qb * new_base_balance_before_b;
    let new_quote_value_before_q = last_quote_value_after_q + last_quote_additional_q;
    let new_port_value_before_q = new_base_value_before_q + new_quote_value_before_q;

    let new_net_base_balance_b = new_base_balance_before_b
        + if amount_b > Decimal::ZERO {
            amount_b * (Decimal::ONE - Decimal::ZERO) // นี่เป็ค่าธรรมเนียมแบบ QQ: Buy ใช้ Quote และ Sell ใช้ Quote
        } else {
            amount_b
        };

    let new_base_value_after_q = price_qb * new_net_base_balance_b;

    let new_quote_value_after_q = new_quote_value_before_q
        + if amount_b < Decimal::ZERO {
            (price_qb * (-amount_b)) * (Decimal::ONE - fee)
        } else {
            (price_qb * (-amount_b)) * (Decimal::ONE + fee) // นี่เป็ค่าธรรมเนียมแบบ QQ: Buy ใช้ Quote และ Sell ใช้ Quote
        };

    let new_port_value_after_q = new_base_value_after_q + new_quote_value_after_q;

    RebalanceRecord {
        id: uuid::Uuid::new_v4().to_string(),
        cl_ord_id: generate_cl_ord_id(),
        sequence: next_sequence,
        symbol: symbol.to_string(),
        order_type: order_type.to_string(),
        side: side.to_string(),
        fee_type: fee_type.to_string(),
        target_anchor_buy,
        target_anchor_sell,
        price_qb,
        base_balance_before_b: new_base_balance_before_b,
        base_value_before_q: new_base_value_before_q,
        quote_value_before_q: new_quote_value_before_q,
        port_value_before_q: new_port_value_before_q,
        got_b: amount_b,
        net_base_balance_b: new_net_base_balance_b,
        base_value_after_q: new_base_value_after_q,
        quote_value_after_q: new_quote_value_after_q,
        port_value_after_q: new_port_value_after_q,
        base_additional_b: Decimal::ZERO,
        quote_additional_q: Decimal::ZERO,
        created_at: now_ms().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_rounddown_truncate_towards_zero() {
        assert_eq!(rounddown(dec!(7.8), dec!(1)), dec!(7));
        assert_eq!(rounddown(dec!(-7.8), dec!(1)), dec!(-7));
        assert_eq!(rounddown(dec!(123.456), dec!(0.01)), dec!(123.45));
    }

    #[test]
    fn test_roundup_away_from_zero() {
        assert_eq!(roundup(dec!(7.1), dec!(1)), dec!(8));
        assert_eq!(roundup(dec!(-7.1), dec!(1)), dec!(-8));
        assert_eq!(roundup(dec!(123.451), dec!(0.01)), dec!(123.46));
    }

    #[test]
    fn test_cal_target_price_buy_and_sell() {
        let base_price = dec!(100000);
        // ต้องให้พอร์ตสมดุลตาม ratio_target (0.5) ก่อน เพื่อให้เทสมีความหมาย:
        // base_value = base_balance * price = 0.5 * 100000 = 50000 = quote_value
        let base_balance = dec!(0.5);
        let quote_value = dec!(50000);
        let ratio_target = dec!(0.5);
        let sell_th = dec!(0.02);
        let buy_th = dec!(0.02);
        let min_size = dec!(0.0001);
        let decimal_place = dec!(0.1);

        let buy_price = cal_target_price(
            Side::Buy,
            base_price,
            base_balance,
            quote_value,
            ratio_target,
            sell_th,
            buy_th,
            min_size,
            decimal_place,
            dec!(0),
            dec!(0),
        );
        let sell_price = cal_target_price(
            Side::Sell,
            base_price,
            base_balance,
            quote_value,
            ratio_target,
            sell_th,
            buy_th,
            min_size,
            decimal_place,
            dec!(0),
            dec!(0),
        );

        assert!(buy_price.is_some());
        assert!(sell_price.is_some());
        // ราคาซื้อ target ต้องต่ำกว่าราคาปัจจุบัน, ราคาขาย target ต้องสูงกว่า
        assert!(buy_price.unwrap() <= base_price);
        assert!(sell_price.unwrap() >= base_price);
    }

    #[test]
    fn test_cal_target_price_invalid_guard() {
        // filled_price <= 0 -> None
        assert_eq!(
            cal_target_price(
                Side::Buy,
                dec!(0),
                dec!(1),
                dec!(1),
                dec!(0.5),
                dec!(0.02),
                dec!(0.02),
                dec!(0.0001),
                dec!(0.1),
                dec!(0),
                dec!(0)
            ),
            None
        );
    }
}
