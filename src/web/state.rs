use std::sync::Arc;
use crate::storage::repositories::user_repository::UserRepository;
use crate::users::auth_service::AuthService;

/// Shared application state across Web handlers and middlewares
#[derive(Clone)]
pub struct AppState {
    pub auth_service: Arc<AuthService>,
    pub user_repo: Arc<UserRepository>,
}

impl AppState {
    pub fn new(auth_service: AuthService, user_repo: UserRepository) -> Self {
        Self {
            auth_service: Arc::new(auth_service),
            user_repo: Arc::new(user_repo),
        }
    }
}
