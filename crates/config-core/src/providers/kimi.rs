//! Kimi Code：API key / CLI credentials → `coding/v1/usages`（对齐 CodexBar）。

use super::http::{get_json, home_dir, jwt_claim};
use crate::locale::{Locale, labels};
use crate::usage::{UsageMetrics, UsageWindow};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

pub fn fetch(locale: Locale) -> Option<UsageMetrics> {
    use super::set_last_error;
    use crate::usage_log;
    const AGENT: &str = "kimi";

    let token = match resolve_token_logged() {
        Some(t) => t,
        None => return None,
    };
    let mut headers = vec![
        ("Authorization", format!("Bearer {token}")),
        ("Accept", "application/json".into()),
        ("User-Agent", "d50-config/0.1".into()),
    ];
    if let Some(device) = jwt_claim(&token, "device_id") {
        headers.push(("x-msh-device-id", device));
    }
    if let Some(ssid) = jwt_claim(&token, "ssid") {
        headers.push(("x-msh-session-id", ssid));
    }
    if let Some(sub) = jwt_claim(&token, "sub") {
        headers.push(("x-traffic-id", sub));
    }
    let header_refs: Vec<(&str, &str)> = headers
        .iter()
        .map(|(k, v)| (*k, v.as_str()))
        .collect();
    let base = std::env::var("KIMI_CODE_BASE_URL")
        .unwrap_or_else(|_| "https://api.kimi.com".into());
    let url = format!("{}/coding/v1/usages", base.trim_end_matches('/'));
    usage_log::usage(AGENT, format!("GET {url}"));
    let (status, body) = match get_json(&url, &header_refs) {
        Ok(v) => v,
        Err(err) => {
            let msg = format!("HTTP transport error: {err}");
            usage_log::usage(AGENT, &msg);
            set_last_error(AGENT, msg);
            return None;
        }
    };
    usage_log::usage(AGENT, format!("HTTP {status}"));
    if status != 200 {
        set_last_error(AGENT, format!("HTTP {status} from kimi usages"));
        return None;
    }
    match parse_usages(&body, locale) {
        Some(m) => Some(m),
        None => {
            let msg = "parse kimi usages failed";
            usage_log::usage(AGENT, msg);
            set_last_error(AGENT, msg);
            None
        }
    }
}

fn resolve_token_logged() -> Option<String> {
    use super::set_last_error;
    use crate::usage_log;
    const AGENT: &str = "kimi";

    if let Ok(key) = std::env::var("KIMI_CODE_API_KEY") {
        let key = key.trim().to_owned();
        if !key.is_empty() {
            usage_log::usage(AGENT, "token from KIMI_CODE_API_KEY");
            return Some(key);
        }
    }
    let path = match credentials_path() {
        Some(p) => {
            usage_log::usage(AGENT, format!("credentials path={}", p.display()));
            p
        }
        None => {
            let msg = "credentials path unresolved";
            usage_log::usage(AGENT, msg);
            set_last_error(AGENT, msg);
            return None;
        }
    };
    if !path.is_file() {
        let msg = format!("credentials not found: {}", path.display());
        usage_log::usage(AGENT, &msg);
        set_last_error(AGENT, msg);
        return None;
    }
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(err) => {
            let msg = format!("read credentials failed: {err}");
            usage_log::usage(AGENT, &msg);
            set_last_error(AGENT, msg);
            return None;
        }
    };
    match parse_cli_credentials(&text) {
        Some(t) => {
            usage_log::usage(AGENT, format!("token ok len={}", t.len()));
            Some(t)
        }
        None => {
            let msg = "parse kimi credentials failed (need access_token)";
            usage_log::usage(AGENT, msg);
            set_last_error(AGENT, msg);
            None
        }
    }
}

fn credentials_path() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("KIMI_CODE_HOME") {
        return Some(PathBuf::from(home).join("credentials").join("kimi-code.json"));
    }
    Some(
        home_dir()?
            .join(".kimi-code")
            .join("credentials")
            .join("kimi-code.json"),
    )
}

pub(crate) fn parse_cli_credentials(text: &str) -> Option<String> {
    let json: Value = serde_json::from_str(text).ok()?;
    json.get("access_token")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

pub(crate) fn parse_usages(body: &Value, locale: Locale) -> Option<UsageMetrics> {
    // 新 ratio pools：`usages` 数组
    if let Some(pools) = body.get("usages").and_then(|v| v.as_array()) {
        if let Some(metrics) = parse_ratio_pools(pools, locale) {
            return Some(metrics);
        }
    }

    let mut windows = Vec::new();
    // 5 小时：limits[0]
    if let Some(limit) = body
        .get("limits")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
    {
        if let Some(detail) = limit.get("detail") {
            if let Some(w) = remaining_from_detail(detail) {
                windows.push(UsageWindow {
                    left_label: labels::session_remaining(locale).to_owned(),
                    right_label: format!("{w}%"),
                    value: w,
                });
            }
        }
    }
    // 周配额：usage
    if let Some(usage) = body.get("usage") {
        if let Some(w) = remaining_from_detail(usage) {
            windows.push(UsageWindow {
                left_label: labels::week_remaining(locale).to_owned(),
                right_label: format!("{w}%"),
                value: w,
            });
        }
    }
    if windows.is_empty() {
        return None;
    }
    Some(UsageMetrics {
        windows,
        text: None,
        value_label: None,
        value_text: None,
    })
}

fn parse_ratio_pools(pools: &[Value], locale: Locale) -> Option<UsageMetrics> {
    let mut session = None;
    let mut weekly = None;
    for pool in pools {
        let name = pool
            .get("name")
            .or_else(|| pool.get("id"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let remaining = pool
            .get("remaining_ratio")
            .or_else(|| pool.get("remainingRatio"))
            .and_then(|v| v.as_f64())
            .map(|r| (r * 100.0).round().clamp(0.0, 100.0) as u8)
            .or_else(|| remaining_from_detail(pool));
        let Some(value) = remaining else { continue };
        if name.contains("5") || name.contains("hour") || name.contains("session") {
            session = Some(value);
        } else if name.contains("week") || name.contains("7") {
            weekly = Some(value);
        }
    }
    let mut windows = Vec::new();
    if let Some(v) = session {
        windows.push(UsageWindow {
            left_label: labels::session_remaining(locale).to_owned(),
            right_label: format!("{v}%"),
            value: v,
        });
    }
    if let Some(v) = weekly {
        windows.push(UsageWindow {
            left_label: labels::week_remaining(locale).to_owned(),
            right_label: format!("{v}%"),
            value: v,
        });
    }
    if windows.is_empty() {
        None
    } else {
        Some(UsageMetrics {
            windows,
            text: None,
            value_label: None,
            value_text: None,
        })
    }
}

fn remaining_from_detail(detail: &Value) -> Option<u8> {
    let limit = num(detail.get("limit"))?;
    let remaining = num(detail.get("remaining")).or_else(|| {
        let used = num(detail.get("used"))?;
        Some((limit - used).max(0.0))
    })?;
    if limit <= 0.0 {
        return None;
    }
    Some(((remaining / limit) * 100.0).round().clamp(0.0, 100.0) as u8)
}

fn num(value: Option<&Value>) -> Option<f64> {
    let v = value?;
    v.as_f64()
        .or_else(|| v.as_u64().map(|u| u as f64))
        .or_else(|| v.as_str()?.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_legacy_count_shape() {
        let body = serde_json::json!({
            "usage": { "limit": "2048", "used": "214", "remaining": "1834" },
            "limits": [{
                "window": { "duration": 300, "timeUnit": "TIME_UNIT_MINUTE" },
                "detail": { "limit": "200", "used": "40", "remaining": "160" }
            }]
        });
        let metrics = parse_usages(&body, Locale::En).unwrap();
        assert_eq!(metrics.windows.len(), 2);
        assert_eq!(metrics.windows[0].value, 80); // session 160/200
        assert_eq!(metrics.windows[1].value, 90); // weekly ~1834/2048
    }
}
