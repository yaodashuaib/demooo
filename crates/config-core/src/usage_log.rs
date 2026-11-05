//! 用量采集运行时日志 → 进程 cwd 下 `logs/usage.log`（可用 `D50_LOG_DIR` 覆盖）。

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

static LOCK: Mutex<()> = Mutex::new(());

fn log_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("D50_LOG_DIR") {
        return PathBuf::from(dir);
    }
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("logs")
}

fn stamp() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f").to_string()
}

/// 追加一行用量诊断日志。失败静默（不能拖垮采集）。
pub fn usage(agent: &str, message: impl AsRef<str>) {
    let dir = log_dir();
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("usage.log");
    let line = format!("{} [{}] {}\n", stamp(), agent, message.as_ref());
    let _guard = LOCK.lock();
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = file.write_all(line.as_bytes());
        let _ = file.flush();
    }
}

pub fn log_path_hint() -> String {
    log_dir().join("usage.log").display().to_string()
}
