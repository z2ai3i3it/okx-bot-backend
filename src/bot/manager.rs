use std::{collections::HashMap, sync::Arc};
use thiserror::Error;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use crate::{
    bot::executor::BotExecutor,
    domain::strategy::Strategy,
    okx::ws_client::OkxPublicWsClient,
    okx::rest_client::{OkxRestClient, OkxRestCredentials},
    services::rebalance_service::RebalanceService,
    storage::repositories::strategy_repository::StrategyRepository,
};

#[derive(Debug, Error)]
pub enum BotManagerError {
    #[error("Bot '{0}' is already running")]
    AlreadyRunning(String),

    #[error("Bot '{0}' is not currently running")]
    NotRunning(String),
}

/// In-Memory Bot Lifecycle Controller
/// ควบคุมและติดตาม Background Task ของบอทที่กำลังทำงานอยู่
#[derive(Clone)]
pub struct BotManager {
    /// บันทึก CancellationToken ของบอทที่กำลังรันอยู่ (Key คือ Bot ID)
    active_tokens: Arc<RwLock<HashMap<String, CancellationToken>>>,
    /// Shared CEM Manager แยกต่อ Account ID (1 Account มี 1 CemManager ร่วมกันข้ามทุกบอท)
    cem_managers: Arc<RwLock<HashMap<String, Arc<crate::pipeline::steps::CemManager>>>>,
    rebalance_service: Arc<RebalanceService>,
    strategy_repository: Arc<StrategyRepository>,
}

impl BotManager {
    pub fn new(
        rebalance_service: Arc<RebalanceService>,
        strategy_repository: Arc<StrategyRepository>,
    ) -> Self {
        Self {
            active_tokens: Arc::new(RwLock::new(HashMap::new())),
            cem_managers: Arc::new(RwLock::new(HashMap::new())),
            rebalance_service,
            strategy_repository,
        }
    }

    /// ดึงหรือสร้าง CemManager ประจำ account_id (Shared Instance)
    pub async fn get_or_create_cem(&self, account_id: &str) -> Arc<crate::pipeline::steps::CemManager> {
        let mut guard = self.cem_managers.write().await;
        guard
            .entry(account_id.to_string())
            .or_insert_with(|| Arc::new(crate::pipeline::steps::CemManager::new()))
            .clone()
    }

    /// ตรวจสอบว่าบอทกำลังรันอยู่หรือไม่
    pub async fn is_running(&self, bot_id: &str) -> bool {
        let guard = self.active_tokens.read().await;
        guard.contains_key(bot_id)
    }

    /// สั่งเริ่มรันบอท (Spawn Tokio Task)
    pub async fn start_bot(
        &self,
        strategy: Strategy,
        public_ws: OkxPublicWsClient,
        trade_client: crate::okx::ws_trade::OkxWsTradeClient,
        rate_limiter: Arc<crate::okx::rate_limiter::OkxRateLimiter>,
        rest_client: Arc<OkxRestClient>,
        rest_credentials: OkxRestCredentials,
        inst_id_code: Option<u64>,
    ) -> Result<(), BotManagerError> {
        let bot_id = strategy.id.clone();
        let account_id = strategy.account_id.clone();

        let mut guard = self.active_tokens.write().await;
        if guard.contains_key(&bot_id) {
            return Err(BotManagerError::AlreadyRunning(bot_id));
        }

        let cancel_token = CancellationToken::new();
        guard.insert(bot_id.clone(), cancel_token.clone());
        drop(guard);

        // ดึงหรือสร้าง Shared CemManager ประจำ account นี้
        let cem_manager = self.get_or_create_cem(&account_id).await;
        let cem_step = crate::pipeline::steps::CemStep::with_manager_and_client(
            cem_manager,
            trade_client.clone(),
        );

        // สร้าง Pipeline ด้วย Shared CEM Step และ Executor
        let pipeline = crate::pipeline::order_pipeline::OrderPipeline::with_cem(
            rate_limiter,
            trade_client.clone(),
            cem_step,
        );
        let executor = BotExecutor::with_dependencies(
            strategy,
            public_ws,
            trade_client,
            pipeline,
            Some(self.rebalance_service.clone()),
            Some(self.strategy_repository.clone()),
            Some(rest_client),
            Some(rest_credentials),
            Vec::new(),
            cancel_token,
            inst_id_code,
        );
        let tokens_map = self.active_tokens.clone();
        let task_bot_id = bot_id.clone();

        tokio::spawn(async move {
            executor.run().await;

            // เมื่อ Executor จบการทำงาน (ไม่ว่าจะเกิดจาก Stop หรือ Error) นำออกจาก Map
            let mut guard = tokens_map.write().await;
            guard.remove(&task_bot_id);
        });

        Ok(())
    }

    /// สั่งหยุดการทำงานของบอท (Cancel CancellationToken)
    pub async fn stop_bot(&self, bot_id: &str) -> Result<(), BotManagerError> {
        let mut guard = self.active_tokens.write().await;
        if let Some(token) = guard.remove(bot_id) {
            token.cancel();
            Ok(())
        } else {
            Err(BotManagerError::NotRunning(bot_id.to_string()))
        }
    }
}