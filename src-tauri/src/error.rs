use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("未找到微信数据目录：{0}")]
    DataDirNotFound(String),
    #[error("未找到取密钥用的原生库 wx_key.dll：{0}")]
    DllNotFound(String),
    #[error("加载原生库失败：{0}")]
    DllLoad(String),
    #[error("获取数据库密钥失败：{0}")]
    KeyFailed(String),
    #[error("密钥无法解密数据库（可能微信版本不兼容或密钥过期）")]
    KeyMismatch,
    #[error("解密失败：{0}")]
    Decrypt(String),
    #[error("读取失败：{0}")]
    Read(String),
    #[error("未初始化，请先调用 init_wechat")]
    NotInitialized,
    #[error("{0}")]
    Other(String),
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::Other(e.to_string())
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        AppError::Read(e.to_string())
    }
}

impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;
