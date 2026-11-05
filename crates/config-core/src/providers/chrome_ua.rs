//! 探测本机 Chrome 主版本，供 WorkBuddy Cookie/UA 绑定。

use std::fs;
use std::path::{Path, PathBuf};

/// 候选主版本：已保存 / 环境 / 本机探测（各带 major-1），最多 4 个，避免串行拖死。
pub fn chrome_majors(preferred: Option<u32>) -> Vec<u32> {
    let mut out = Vec::new();
    let push = |list: &mut Vec<u32>, v: u32| {
        if v > 0 && !list.contains(&v) && list.len() < 4 {
            list.push(v);
        }
    };
    if let Some(v) = preferred {
        push(&mut out, v);
        push(&mut out, v.saturating_sub(1));
    }
    if let Some(v) = std::env::var("D50_CHROME_MAJOR").ok().and_then(|s| s.parse().ok()) {
        push(&mut out, v);
        push(&mut out, v.saturating_sub(1));
    }
    if let Some(v) = detect_installed_chrome_major() {
        push(&mut out, v);
        push(&mut out, v.saturating_sub(1));
    }
    // 探测失败时的兜底，仍限制总数
    for v in [156u32, 155, 131] {
        push(&mut out, v);
    }
    out
}

pub fn user_agent(major: u32) -> String {
    format!(
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{major}.0.0.0 Safari/537.36"
    )
}

pub fn detect_installed_chrome_major() -> Option<u32> {
    let candidates = [
        PathBuf::from(r"C:\Program Files\Google\Chrome\Application"),
        PathBuf::from(r"C:\Program Files (x86)\Google\Chrome\Application"),
        std::env::var_os("LOCALAPPDATA").map(|p| {
            PathBuf::from(p)
                .join("Google")
                .join("Chrome")
                .join("Application")
        })?,
    ];
    let mut best = None;
    for dir in candidates {
        if let Some(v) = scan_chrome_dir(&dir) {
            best = Some(best.map_or(v, |b: u32| b.max(v)));
        }
    }
    best
}

fn scan_chrome_dir(dir: &Path) -> Option<u32> {
    let entries = fs::read_dir(dir).ok()?;
    let mut best = None;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        // 版本目录如 156.0.8078.74
        if let Some(major) = name.split('.').next().and_then(|s| s.parse::<u32>().ok()) {
            if major >= 50 {
                best = Some(best.map_or(major, |b: u32| b.max(major)));
            }
        }
    }
    // 也可从 chrome.exe 旁最近版本目录推断；上面已扫子目录
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn majors_include_preferred_and_fallback() {
        let list = chrome_majors(Some(156));
        assert_eq!(list[0], 156);
        assert!(list.contains(&155));
    }
}
