//! WorkBuddy 网站会话：本机草稿，供用量采集使用（不对 IPC 回传 Cookie 明文）。

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// `%AppData%\Roaming\Ulanzi\D50\workbuddy-session.json`
pub const WORKBUDDY_SESSION_RELATIVE: &str = r"Ulanzi\D50\workbuddy-session.json";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WorkbuddySessionError {
    #[error("WORKBUDDY_SESSION_JSON")]
    Json,
    #[error("WORKBUDDY_SESSION_EMPTY")]
    Empty,
    #[error("WORKBUDDY_SESSION_IO")]
    Io,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WorkbuddySession {
    /// 浏览器开发者工具里复制的完整 `Cookie` 请求头值。
    #[serde(default)]
    pub cookie: String,
    /// 可选：Chrome 主版本，用于 UA 绑定；缺省由采集端自定。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chrome_major: Option<u32>,
}

impl WorkbuddySession {
    pub fn is_configured(&self) -> bool {
        !self.cookie.trim().is_empty()
    }

    pub fn parse(json: &str) -> Result<Self, WorkbuddySessionError> {
        serde_json::from_str(json).map_err(|_| WorkbuddySessionError::Json)
    }

    pub fn to_json(&self) -> Result<String, WorkbuddySessionError> {
        serde_json::to_string_pretty(self).map_err(|_| WorkbuddySessionError::Json)
    }

    pub fn load(path: &Path) -> Self {
        match fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(path: &Path, session: &Self) -> Result<(), WorkbuddySessionError> {
        if !session.is_configured() {
            return Err(WorkbuddySessionError::Empty);
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|_| WorkbuddySessionError::Io)?;
        }
        let json = session.to_json()?;
        fs::write(path, json).map_err(|_| WorkbuddySessionError::Io)
    }

    pub fn clear(path: &Path) -> Result<(), WorkbuddySessionError> {
        if path.exists() {
            fs::remove_file(path).map_err(|_| WorkbuddySessionError::Io)?;
        }
        Ok(())
    }

    pub fn default_path() -> Option<PathBuf> {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .map(|p| p.join(WORKBUDDY_SESSION_RELATIVE))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env::temp_dir;

    #[test]
    fn round_trip_file() {
        let path = temp_dir().join(format!(
            "d50-wb-session-{}.json",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        let session = WorkbuddySession {
            cookie: "a=1; b=2".into(),
            chrome_major: Some(131),
        };
        WorkbuddySession::save(&path, &session).unwrap();
        let loaded = WorkbuddySession::load(&path);
        assert_eq!(loaded.cookie, "a=1; b=2");
        assert_eq!(loaded.chrome_major, Some(131));
        WorkbuddySession::clear(&path).unwrap();
    }
}
