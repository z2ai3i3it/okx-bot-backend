use std::sync::Arc;
use thiserror::Error;
use uuid::Uuid;

use crate::{
    bot::manager::{BotManager, BotManagerError},
    domain::strategy::{
        BotResponse, CreateBotRequest, Strategy, StrategyStatus, UpdateBotRequest,
    },
    okx::manager::OkxManager,
    storage::repositories::{
        account_repository::AccountRepository, strategy_repository::StrategyRepository,
    },
};

#[derive(Debug, Error)]
pub enum StrategyServiceError {
    #[error("Strategy not found or access denied")]
    NotFound,

    #[error("Account not found or access denied")]
    AccountNotFound,

    #[error("Cannot perform operation while bot is running: {0}")]
    BotIsRunning(String),

    #[error("Bot manager error: {0}")]
    BotManagerError(#[from] BotManagerError),

    #[error("Database error: {0}")]
    DatabaseError(#[from] mongodb::error::Error),

    #[error("Validation error: {0}")]
    ValidationError(String),
}

/// Service สำหรับจัดการ Business Logic ของ Bot / Strategy
#[derive(Clone)]
pub struct StrategyService {
    strategy_repo: Arc<StrategyRepository>,
    account_repo: Arc<AccountRepository>,
    bot_manager: BotManager,
    okx_manager: OkxManager,
}

impl StrategyService {
    pub fn new(
        strategy_repo: Arc<StrategyRepository>,
        account_repo: Arc<AccountRepository>,
        bot_manager: BotManager,
        okx_manager: OkxManager,
    ) -> Self {
        Self {
            strategy_repo,
            account_repo,
            bot_manager,
            okx_manager,
        }
    }

    /// สร้างบอทใหม่
    pub async fn create_bot(
        &self,
        user_id: &str,
        req: CreateBotRequest,
    ) -> Result<BotResponse, StrategyServiceError> {
        // 1. ตรวจสอบว่า account_id มีอยู่จริงและเป็นของ User คนนี้
        let account = self
            .account_repo
            .find_by_id_and_user_id(&req.account_id, user_id)
            .await?;

        if account.is_none() {
            return Err(StrategyServiceError::AccountNotFound);
        }

        // 2. Validate ชื่อและคู่เหรียญ
        if req.name.trim().is_empty() {
            return Err(StrategyServiceError::ValidationError(
                "Bot name cannot be empty".to_string(),
            ));
        }

        if req.pair.trim().is_empty() {
            return Err(StrategyServiceError::ValidationError(
                "Trading pair cannot be empty".to_string(),
            ));
        }

        let bot_id = format!("bot-{}", Uuid::new_v4());
        let strategy = Strategy::new(
            bot_id,
            user_id.to_string(),
            req.account_id,
            req.name,
            req.pair,
            req.sandbox,
            req.config,
        );

        self.strategy_repo.create(&strategy).await?;

        Ok(strategy.to_response())
    }

    /// ดึงรายการบอททั้งหมดของ User
    pub async fn list_bots(&self, user_id: &str) -> Result<Vec<BotResponse>, StrategyServiceError> {
        let strategies = self.strategy_repo.find_by_user_id(user_id).await?;
        let responses = strategies.into_iter().map(|s| s.to_response()).collect();
        Ok(responses)
    }

    /// ดึงข้อมูลบอทเดี่ยวตาม ID
    pub async fn get_bot(
        &self,
        bot_id: &str,
        user_id: &str,
    ) -> Result<BotResponse, StrategyServiceError> {
        let strategy = self
            .strategy_repo
            .find_by_id_and_user_id(bot_id, user_id)
            .await?
            .ok_or(StrategyServiceError::NotFound)?;

        Ok(strategy.to_response())
    }

    /// แก้ไขข้อมูลบอท (ทำได้เฉพาะเมื่อบอทไม่รันอยู่)
    pub async fn update_bot(
        &self,
        bot_id: &str,
        user_id: &str,
        req: UpdateBotRequest,
    ) -> Result<BotResponse, StrategyServiceError> {
        let strategy = self
            .strategy_repo
            .find_by_id_and_user_id(bot_id, user_id)
            .await?
            .ok_or(StrategyServiceError::NotFound)?;

        // Guard: ตรวจสอบว่าบอทไม่รันอยู่
        if self.bot_manager.is_running(bot_id).await || strategy.status == StrategyStatus::Running {
            return Err(StrategyServiceError::BotIsRunning(
                "Please stop the bot before updating its configuration".to_string(),
            ));
        }

        self.strategy_repo
            .update_config_and_name(bot_id, user_id, req.name, req.config)
            .await?;

        let updated_strategy = self
            .strategy_repo
            .find_by_id_and_user_id(bot_id, user_id)
            .await?
            .ok_or(StrategyServiceError::NotFound)?;

        Ok(updated_strategy.to_response())
    }

    /// ลบบอท (ทำได้เฉพาะเมื่อบอทไม่รันอยู่)
    pub async fn delete_bot(
        &self,
        bot_id: &str,
        user_id: &str,
    ) -> Result<(), StrategyServiceError> {
        let strategy = self
            .strategy_repo
            .find_by_id_and_user_id(bot_id, user_id)
            .await?
            .ok_or(StrategyServiceError::NotFound)?;

        // Guard: ตรวจสอบว่าบอทไม่รันอยู่
        if self.bot_manager.is_running(bot_id).await || strategy.status == StrategyStatus::Running {
            return Err(StrategyServiceError::BotIsRunning(
                "Please stop the bot before deleting it".to_string(),
            ));
        }

        self.strategy_repo
            .delete_by_id_and_user_id(bot_id, user_id)
            .await?;

        Ok(())
    }

    /// สั่งเริ่มรันบอท (Start Bot)
    pub async fn start_bot(
        &self,
        bot_id: &str,
        user_id: &str,
    ) -> Result<BotResponse, StrategyServiceError> {
        let mut strategy = self
            .strategy_repo
            .find_by_id_and_user_id(bot_id, user_id)
            .await?
            .ok_or(StrategyServiceError::NotFound)?;

        if self.bot_manager.is_running(bot_id).await {
            return Err(StrategyServiceError::BotIsRunning(
                "Bot is already active in memory".to_string(),
            ));
        }

        // อัปเดตสถานะใน DB
        self.strategy_repo
            .update_status(bot_id, StrategyStatus::Running)
            .await?;
        strategy.status = StrategyStatus::Running;

        // สั่ง BotManager รัน Background Task โดยใช้ Public WS ร่วมกัน
        let public_ws = self.okx_manager.public_ws().clone();
        self.bot_manager.start_bot(strategy.clone(), public_ws).await?;

        Ok(strategy.to_response())
    }

    /// สั่งหยุดการทำงานของบอท (Stop Bot)
    pub async fn stop_bot(
        &self,
        bot_id: &str,
        user_id: &str,
    ) -> Result<BotResponse, StrategyServiceError> {
        let mut strategy = self
            .strategy_repo
            .find_by_id_and_user_id(bot_id, user_id)
            .await?
            .ok_or(StrategyServiceError::NotFound)?;

        // ส่งสัญญาณ cancel ไปยัง Background Task
        let _ = self.bot_manager.stop_bot(bot_id).await;

        // อัปเดตสถานะใน DB
        self.strategy_repo
            .update_status(bot_id, StrategyStatus::Stopped)
            .await?;
        strategy.status = StrategyStatus::Stopped;

        Ok(strategy.to_response())
    }
}
