//! WorkBuddy：网站 Cookie → billing summary（对齐 CodexBar）。
//!
//! 凭据优先级：
//! 1. 环境变量 `D50_WORKBUDDY_COOKIE` / `WORKBUDDY_COOKIE`（调试覆盖）
//! 2. `%AppData%\Roaming\Ulanzi\D50\workbuddy-session.json`（推荐，由配置客户端写入）
//!
//! Cookie 与 Chrome User-Agent 大版本绑定；错误 UA 会 401。采集时自动探测本机 Chrome 并多版本重试。

use super::chrome_ua::{chrome_majors, user_agent};
use super::http::{SHORT_TIMEOUT, post_json_timeout};
use crate::locale::Locale;
use crate::usage::{UsageMetrics, UsageWindow};
use crate::workbuddy_session::WorkbuddySession;
use serde_json::Value;

thread_local! {
    static LAST_ERROR: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

pub fn take_last_error() -> Option<String> {
    LAST_ERROR.with(|slot| slot.borrow_mut().take())
}

fn set_last_error(msg: impl Into<String>) {
    LAST_ERROR.with(|slot| *slot.borrow_mut() = Some(msg.into()));
}

pub fn fetch(locale: Locale) -> Option<UsageMetrics> {
    use crate::usage_log;
    const AGENT: &str = "workbuddy";

    LAST_ERROR.with(|slot| *slot.borrow_mut() = None);
    let session = match resolve_session() {
        Some(s) => {
            usage_log::usage(
                AGENT,
                format!(
                    "session ok cookie_len={} chrome_major={:?}",
                    s.cookie.len(),
                    s.chrome_major
                ),
            );
            s
        }
        None => {
            let msg = "未配置 Cookie 会话";
            usage_log::usage(AGENT, msg);
            set_last_error(msg);
            super::set_last_error(AGENT, format!("workbuddy: {msg}"));
            return None;
        }
    };
    let cookie = session.cookie.trim();
    let majors = chrome_majors(session.chrome_major);
    usage_log::usage(AGENT, format!("chrome majors={majors:?}"));
    let mut last_status = 0u16;
    for major in majors {
        let ua = user_agent(major);
        let headers = [
            ("Accept", "application/json"),
            ("Content-Type", "application/json"),
            ("Origin", "https://www.workbuddy.cn"),
            ("Referer", "https://www.workbuddy.cn/profile/plans-usage"),
            ("Cookie", cookie),
            ("User-Agent", ua.as_str()),
        ];
        usage_log::usage(AGENT, format!("POST billing summary UA Chrome/{major}"));
        let Ok((status, body)) = post_json_timeout(
            "https://www.workbuddy.cn/billing/meter/get-user-resource-summary",
            &headers,
            &serde_json::json!({}),
            SHORT_TIMEOUT,
        ) else {
            usage_log::usage(AGENT, format!("transport fail Chrome/{major}"));
            continue;
        };
        last_status = status;
        usage_log::usage(AGENT, format!("HTTP {status} Chrome/{major}"));
        if status == 401 {
            continue;
        }
        if status != 200 {
            let msg = format!("HTTP {status}");
            set_last_error(format!("workbuddy: {msg}"));
            super::set_last_error(AGENT, format!("workbuddy: {msg}"));
            return None;
        }
        if session.chrome_major != Some(major) {
            if let Some(path) = WorkbuddySession::default_path() {
                let _ = WorkbuddySession::save(
                    &path,
                    &WorkbuddySession {
                        cookie: cookie.to_owned(),
                        chrome_major: Some(major),
                    },
                );
            }
        }
        return match parse_summary(&body, locale) {
            Some(m) => Some(m),
            None => {
                let msg = "响应无法解析";
                usage_log::usage(AGENT, msg);
                set_last_error(format!("workbuddy: {msg}"));
                super::set_last_error(AGENT, format!("workbuddy: {msg}"));
                None
            }
        };
    }
    let msg = format!(
        "Cookie/UA 未通过（最后 HTTP {last_status}）。请用当前 Chrome 重新复制 Cookie，或设置 D50_CHROME_MAJOR"
    );
    usage_log::usage(AGENT, &msg);
    set_last_error(format!("workbuddy: {msg}"));
    super::set_last_error(AGENT, format!("workbuddy: {msg}"));
    None
}

fn resolve_session() -> Option<WorkbuddySession> {
    for key in ["D50_WORKBUDDY_COOKIE", "WORKBUDDY_COOKIE"] {
        if let Ok(value) = std::env::var(key) {
            let value = value.trim().to_owned();
            if !value.is_empty() {
                return Some(WorkbuddySession {
                    cookie: value,
                    chrome_major: None,
                });
            }
        }
    }
    let path = WorkbuddySession::default_path()?;
    let session = WorkbuddySession::load(&path);
    session.is_configured().then_some(session)
}

pub fn session_configured() -> bool {
    resolve_session().is_some_and(|s| s.is_configured())
}

pub(crate) fn parse_summary(body: &Value, locale: Locale) -> Option<UsageMetrics> {
    if body.get("code").and_then(|c| c.as_i64()) != Some(0) {
        return None;
    }
    let data = body.get("data")?;
    let packages = data.get("Packages")?.as_array()?;
    let mut total = 0.0;
    let mut remaining = 0.0;
    for pkg in packages {
        if pkg.get("CapacityUnit").and_then(|v| v.as_str()) != Some("credits") {
            continue;
        }
        total += amount(pkg.get("CycleTotalCapacity")).unwrap_or(0.0);
        remaining += amount(pkg.get("CycleRemainCapacity")).unwrap_or(0.0);
    }
    let plan = data
        .get("SubscriptionPackageName")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if total > 0.0 {
        let pct = ((remaining / total) * 100.0).round().clamp(0.0, 100.0) as u8;
        let right = match locale {
            Locale::ZhHans => format!("{remaining:.0} / {total:.0}"),
            Locale::En => format!("{remaining:.0}/{total:.0}"),
        };
        return Some(UsageMetrics {
            windows: vec![UsageWindow {
                left_label: match locale {
                    Locale::ZhHans => "可用额度".into(),
                    Locale::En => "Credits".into(),
                },
                right_label: right,
                value: pct,
            }],
            text: if plan.is_empty() {
                None
            } else {
                Some(match locale {
                    Locale::ZhHans => format!("当前套餐：{plan}"),
                    Locale::En => format!("Plan: {plan}"),
                })
            },
            value_label: None,
            value_text: None,
        });
    }

    Some(UsageMetrics {
        windows: Vec::new(),
        text: Some(match locale {
            Locale::ZhHans => format!(
                "当前套餐：{}\n剩余额度：{remaining:.0}",
                if plan.is_empty() { "—" } else { plan }
            ),
            Locale::En => format!(
                "Plan: {}\nCredits left: {remaining:.0}",
                if plan.is_empty() { "-" } else { plan }
            ),
        }),
        value_label: None,
        value_text: None,
    })
}

fn amount(value: Option<&Value>) -> Option<f64> {
    let v = value?;
    v.as_f64()
        .or_else(|| v.as_u64().map(|u| u as f64))
        .or_else(|| v.as_str()?.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_credits_meter() {
        let body = serde_json::json!({
            "code": 0,
            "data": {
                "SubscriptionPackageName": "体验版",
                "Packages": [{
                    "CapacityUnit": "credits",
                    "CycleTotalCapacity": "500",
                    "CycleRemainCapacity": "450"
                }]
            }
        });
        let metrics = parse_summary(&body, Locale::ZhHans).unwrap();
        assert_eq!(metrics.windows.len(), 1);
        assert_eq!(metrics.windows[0].value, 90);
        assert!(metrics.text.as_ref().unwrap().contains("体验版"));
    }
}
