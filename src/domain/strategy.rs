use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::domain::{
    order::TrackedOrder,
    strategy::{fixed_ratio_rebalance::FixdRatioRebalanceConfig, grid::GridConfig},
};

pub mod fixed_ratio_rebalance;
pub mod grid;
pub mod mode;

/// สถานะการทำงานของบอท / กลยุทธ์
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum StrategyStatus {
    #[default]
    Created,
    Running,
    Paused,
    Stopped,
    Error(String),
}

/// Static Configuration ของกลยุทธ์ (บันทึกลง Database ได้)
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", content = "parameters")]
pub enum StrategyConfig {
    FixedRatioRebalance(FixdRatioRebalanceConfig),
    Grid(GridConfig),
}

/// Dynamic Runtime State ของกลยุทธ์ใน Memory
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", content = "state")]
pub enum StrategyState {
    #[default]
    Idle,

    /// Fixed Ratio Rebalance ติดตาม 1 Buy และ 1 Sell
    FixedRatio {
        tracked_buy: Option<TrackedOrder>,
        tracked_sell: Option<TrackedOrder>,
    },

    /// Grid Strategy ติดตามหลายไม้ (Multiple Orders)
    Grid {
        tracked_orders: Vec<TrackedOrder>,
    },
}

/// Domain Entity สำหรับ Bot / Strategy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Strategy {
    pub id: String,
    pub user_id: String,
    pub account_id: String,
    pub name: String,
    pub pair: String,
    pub sandbox: bool,

    /// การตั้งค่ากลยุทธ์ (Config)
    pub config: StrategyConfig,

    /// สถานะ Runtime State ใน Memory (Active Tracked Orders)
    pub state: StrategyState,

    /// สถานะวงจรชีวิตของบอท
    pub status: StrategyStatus,

    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Strategy {
    pub fn new(
        id: String,
        user_id: String,
        account_id: String,
        name: String,
        pair: String,
        sandbox: bool,
        config: StrategyConfig,
    ) -> Self {
        let now = Utc::now();
        Self {
            id,
            user_id,
            account_id,
            name,
            pair,
            sandbox,
            config,
            state: StrategyState::Idle,
            status: StrategyStatus::Created,
            created_at: now,
            updated_at: now,
        }
    }

    /// Helper เข้าถึง Tracked Buy Order (ถ้ามี)
    pub fn get_active_buy(&self) -> Option<&TrackedOrder> {
        match &self.state {
            StrategyState::FixedRatio { tracked_buy, .. } => tracked_buy.as_ref(),
            StrategyState::Grid { tracked_orders } => tracked_orders
                .iter()
                .find(|o| o.side == crate::domain::order::Side::Buy),
            StrategyState::Idle => None,
        }
    }

    /// Helper เข้าถึง Tracked Sell Order (ถ้ามี)
    pub fn get_active_sell(&self) -> Option<&TrackedOrder> {
        match &self.state {
            StrategyState::FixedRatio { tracked_sell, .. } => tracked_sell.as_ref(),
            StrategyState::Grid { tracked_orders } => tracked_orders
                .iter()
                .find(|o| o.side == crate::domain::order::Side::Sell),
            StrategyState::Idle => None,
        }
    }

    /// แปลงเป็น BotResponse สำหรับส่งกลับทาง API
    pub fn to_response(&self) -> BotResponse {
        BotResponse {
            id: self.id.clone(),
            user_id: self.user_id.clone(),
            account_id: self.account_id.clone(),
            name: self.name.clone(),
            pair: self.pair.clone(),
            sandbox: self.sandbox,
            config: self.config.clone(),
            state: self.state.clone(),
            status: self.status.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

/// Request DTO สำหรับสร้างบอทใหม่
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateBotRequest {
    #[schema(example = "acc-uuid-1234")]
    pub account_id: String,
    #[schema(example = "BTC Fixed Ratio Bot")]
    pub name: String,
    #[schema(example = "BTC-USDT")]
    pub pair: String,
    #[schema(example = true)]
    pub sandbox: bool,
    pub config: StrategyConfig,
}

/// Request DTO สำหรับแก้ไขการตั้งค่าบอท (ทำได้เฉพาะเมื่อบอทไม่รันอยู่)
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateBotRequest {
    #[schema(example = "BTC Fixed Ratio Bot (Updated)")]
    pub name: Option<String>,
    pub config: Option<StrategyConfig>,
}

/// Response DTO สำหรับส่งข้อมูลบอทกลับให้ UI
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BotResponse {
    pub id: String,
    pub user_id: String,
    pub account_id: String,
    pub name: String,
    pub pair: String,
    pub sandbox: bool,
    pub config: StrategyConfig,
    pub state: StrategyState,
    pub status: StrategyStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
