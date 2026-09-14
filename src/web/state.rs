use std::sync::Arc;
use crate::{
    services::strategy_service::StrategyService,
    storage::repositories::user_repository::UserRepository,
    users::{account_service::AccountService, auth_service::AuthService},
};

/// Shared application state across Web handlers and middlewares
#[derive(Clone)]
pub struct AppState {
    pub auth_service: Arc<AuthService>,
    pub user_repo: Arc<UserRepository>,
    pub account_service: Arc<AccountService>,
    pub strategy_service: Arc<StrategyService>,
}

impl AppState {
    pub fn new(
        auth_service: AuthService,
        user_repo: UserRepository,
        account_service: AccountService,
        strategy_service: StrategyService,
    ) -> Self {
        Self {
            auth_service: Arc::new(auth_service),
            user_repo: Arc::new(user_repo),
            account_service: Arc::new(account_service),
            strategy_service: Arc::new(strategy_service),
        }
    }
}
