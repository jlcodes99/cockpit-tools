use serde::Serialize;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[cfg(feature = "desktop")]
    #[error("Tauri error: {0}")]
    Tauri(#[from] tauri::Error),

    #[error("OAuth error: {0}")]
    OAuth(String),

    #[error("Account error: {0}")]
    Account(String),

    #[error("File corrupted: {file_name}")]
    FileCorrupted {
        file_name: String,
        file_path: String,
        original_error: String,
    },

    #[error("Unknown error: {0}")]
    Unknown(String),
}

impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap;

        match self {
            AppError::FileCorrupted {
                file_name,
                file_path,
                original_error,
            } => {
                let mut map = serializer.serialize_map(Some(4))?;
                map.serialize_entry("error_type", "file_corrupted")?;
                map.serialize_entry("file_name", file_name)?;
                map.serialize_entry("file_path", file_path)?;
                map.serialize_entry("original_error", original_error)?;
                map.end()
            }
            _ => serializer.serialize_str(self.to_string().as_str()),
        }
    }
}

/// 创建文件损坏错误的辅助函数
pub fn file_corrupted_error(file_name: &str, file_path: &str, original_error: &str) -> String {
    serde_json::json!({
        "error_type": "file_corrupted",
        "file_name": file_name,
        "file_path": file_path,
        "original_error": original_error
    })
    .to_string()
}

pub type AppResult<T> = Result<T, AppError>;
