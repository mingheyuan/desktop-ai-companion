use crate::contracts::AppError;

const KEYCHAIN_SERVICE: &str = "com.mingheyuan.desktop-ai-companion";
const LLM_ACCOUNT: &str = "llm.default";

#[derive(Debug, Clone, Default)]
pub struct SecretStore;

impl SecretStore {
    pub fn resolve_llm_api_key(&self) -> Result<Option<String>, AppError> {
        if let Ok(value) = std::env::var("DESKTOP_AI_LLM_API_KEY") {
            if !value.trim().is_empty() {
                return Ok(Some(value));
            }
        }
        self.get_stored_llm_api_key()
    }

    pub fn get_stored_llm_api_key(&self) -> Result<Option<String>, AppError> {
        match self.entry()?.get_password() {
            Ok(value) if !value.trim().is_empty() => Ok(Some(value)),
            Ok(_) | Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(keychain_error("读取", error)),
        }
    }

    pub fn set_llm_api_key(&self, value: &str) -> Result<(), AppError> {
        if value.trim().is_empty() {
            return Err(AppError::new(
                "invalid_request",
                "API key 不能为空。",
                false,
            ));
        }
        self.entry()?
            .set_password(value)
            .map_err(|error| keychain_error("写入", error))
    }

    pub fn delete_llm_api_key(&self) -> Result<(), AppError> {
        match self.entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(keychain_error("删除", error)),
        }
    }

    fn entry(&self) -> Result<keyring::Entry, AppError> {
        keyring::Entry::new(KEYCHAIN_SERVICE, LLM_ACCOUNT)
            .map_err(|error| keychain_error("访问", error))
    }
}

fn keychain_error(action: &str, error: keyring::Error) -> AppError {
    AppError::new(
        "keychain_error",
        format!("无法{action} macOS Keychain：{error}"),
        false,
    )
}
