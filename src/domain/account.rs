pub struct Account {
    pub id: String,
    pub uid: String,
    pub api_key: String,
    pub secret_key: String,
    pub passphrase: String,
    pub api_key_name: String,
    pub permissions: Vec<String>,
}

impl Account {
    pub fn new(
        id: String,
        uid: String,
        api_key: String,
        secret_key: String,
        passphrase: String,
        api_key_name: String,
        permissions: Vec<String>,
    ) -> Self {
        Self {
            id,
            uid,
            api_key,
            secret_key,
            passphrase,
            api_key_name,
            permissions,
        }
    }
}
