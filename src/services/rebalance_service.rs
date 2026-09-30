use std::sync::Arc;
use thiserror::Error;

use crate::{
    bot::math::fixed_ratio_cal::RebalanceRecord,
    domain::rebalance::Rebalance,
    storage::repositories::rebalance_repository::RebalanceRepository,
};

#[derive(Debug, Error)]
pub enum RebalanceServiceError {
    #[error("Database error: {0}")]
    DatabaseError(#[from] mongodb::error::Error),

    #[error("Rebalance record validation error: {0}")]
    ValidationError(String),
}

/// Service สำหรับจัดการ Rebalance Persistence Lifecycle (Layer 9)
#[derive(Clone)]
pub struct RebalanceService {
    rebalance_repo: Arc<RebalanceRepository>,
}

impl RebalanceService {
    pub fn new(rebalance_repo: Arc<RebalanceRepository>) -> Self {
        Self { rebalance_repo }
    }

    /// บันทึก Rebalance Snapshot ที่เกิดขึ้นจากการ Match Trade หรือ Seed Order
    pub async fn save_trade_fill(
        &self,
        bot_id: &str,
        order_id: &str,
        record: &RebalanceRecord,
    ) -> Result<Rebalance, RebalanceServiceError> {
        let entity = Rebalance::from_record(bot_id, order_id, record);
        self.rebalance_repo.insert_record(&entity).await?;
        Ok(entity)
    }

    /// ดึง Snapshot ล่าสุดของบอทเพื่อนำไปเป็น Anchor สำหรับรอบถัดไป หรือใช้ Recovery State
    pub async fn get_last_snapshot(
        &self,
        bot_id: &str,
    ) -> Result<Option<RebalanceRecord>, RebalanceServiceError> {
        let entity_opt = self.rebalance_repo.get_last_record(bot_id).await?;
        Ok(entity_opt.map(|e| e.to_record()))
    }

    /// นับจำนวน Rebalance ทั้งหมดของบอท
    pub async fn count_by_bot(&self, bot_id: &str) -> Result<u64, RebalanceServiceError> {
        let count = self.rebalance_repo.count_by_bot(bot_id).await?;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rebalance_service_interface() {
        let _ = std::mem::size_of::<RebalanceService>();
    }
}
