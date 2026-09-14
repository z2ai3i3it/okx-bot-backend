use std::{collections::HashMap, sync::Arc};
use thiserror::Error;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use crate::{
    bot::executor::BotExecutor,
    domain::strategy::Strategy,
    okx::ws_client::OkxPublicWsClient,
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
}

impl BotManager {
    pub fn new() -> Self {
        Self {
            active_tokens: Arc::new(RwLock::new(HashMap::new())),
        }
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
    ) -> Result<(), BotManagerError> {
        let bot_id = strategy.id.clone();

        let mut guard = self.active_tokens.write().await;
        if guard.contains_key(&bot_id) {
            return Err(BotManagerError::AlreadyRunning(bot_id));
        }

        let cancel_token = CancellationToken::new();
        guard.insert(bot_id.clone(), cancel_token.clone());
        drop(guard);

        // สร้างและรัน Executor ใน Background Task
        let executor = BotExecutor::new(strategy, public_ws, cancel_token);
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

impl Default for BotManager {
    fn default() -> Self {
        Self::new()
    }
}
