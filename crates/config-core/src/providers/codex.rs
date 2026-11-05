//! Codex / ChatGPT：读 `~/.codex/auth.json`，调 `wham/usage`（对齐 CodexBar）。

use super::http::{get_json, home_dir, jwt_claim};
use super::{set_last_error};
use crate::locale::{Locale, labels};
use crate::usage::{UsageMetrics, UsageWindow};
use crate::usage_log;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

const AGENT: &str = "chatgpt";

pub fn fetch(locale: Locale) -> Option<UsageMetrics> {
    let path = match auth_path() {
        Some(p) => {
            usage_log::usage(AGENT, format!("auth path={}", p.display()));
            p
        }
        None => {
            let msg = "auth path unresolved (USERPROFILE/HOME/CODEX_HOME missing)";
            usage_log::usage(AGENT, msg);
            set_last_error(AGENT, msg);
            return None;
        }
    };
    if !path.is_file() {
        let msg = format!("auth.json not found: {}", path.display());
        usage_log::usage(AGENT, &msg);
        set_last_error(AGENT, msg);
        return None;
    }
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(err) => {
            let msg = format!("read auth.json failed: {err}");
            usage_log::usage(AGENT, &msg);
            set_last_error(AGENT, msg);
            return None;
        }
    };
    let creds = match parse_auth_json(&text) {
        Some(c) => {
            usage_log::usage(
                AGENT,
                format!(
                    "auth ok access_token_len={} account_id={}",
                    c.access_token.len(),
                    c.account_id.as_deref().unwrap_or("(none)")
                ),
            );
            c
        }
        None => {
            let msg = "parse auth.json failed (need tokens.access_token)";
            usage_log::usage(AGENT, msg);
            set_last_error(AGENT, msg);
            return None;
        }
    };

    let mut headers = vec![
        ("Authorization", format!("Bearer {}", creds.access_token)),
        ("User-Agent", "d50-config/0.1".into()),
        ("Accept", "application/json".into()),
    ];
    if let Some(account) = &creds.account_id {
        headers.push(("ChatGPT-Account-Id", account.clone()));
    }
    let header_refs: Vec<(&str, &str)> = headers
        .iter()
        .map(|(k, v)| (*k, v.as_str()))
        .collect();
    let url = "https://chatgpt.com/backend-api/wham/usage";
    usage_log::usage(AGENT, format!("GET {url}"));
    let (status, body) = match get_json(url, &header_refs) {
        Ok(v) => v,
        Err(err) => {
            let msg = format!("HTTP transport error: {err}");
            usage_log::usage(AGENT, &msg);
            set_last_error(AGENT, msg);
            return None;
        }
    };
    usage_log::usage(
        AGENT,
        format!(
            "HTTP {status}; body_keys={}",
            body.as_object()
                .map(|o| o.keys().cloned().collect::<Vec<_>>().join(","))
                .unwrap_or_else(|| "(not-object)".into())
        ),
    );
    if status != 200 {
        let msg = format!("HTTP {status} from wham/usage");
        set_last_error(AGENT, msg);
        return None;
    }
    match parse_wham_usage(&body, locale) {
        Some(m) => Some(m),
        None => {
            let preview = serde_json::to_string(&body)
                .unwrap_or_default()
                .chars()
                .take(400)
                .collect::<String>();
            let msg = "parse wham/usage failed (no primary/secondary used_percent)";
            usage_log::usage(AGENT, format!("{msg}; body_preview={preview}"));
            set_last_error(AGENT, msg);
            None
        }
    }
}

pub(crate) struct CodexAuth {
    access_token: String,
    account_id: Option<String>,
}

fn auth_path() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("CODEX_HOME") {
        return Some(PathBuf::from(home).join("auth.json"));
    }
    Some(home_dir()?.join(".codex").join("auth.json"))
}

pub(crate) fn parse_auth_json(text: &str) -> Option<CodexAuth> {
    let json: Value = serde_json::from_str(text).ok()?;
    let tokens = json.get("tokens")?;
    let access = tokens
        .get("access_token")
        .or_else(|| tokens.get("accessToken"))?
        .as_str()?
        .to_owned();
    if access.is_empty() {
        return None;
    }
    let account_id = tokens
        .get("account_id")
        .or_else(|| tokens.get("accountId"))
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .filter(|s| !s.is_empty())
        .or_else(|| jwt_claim(&access, "chatgpt_account_id"));
    Some(CodexAuth {
        access_token: access,
        account_id,
    })
}

pub(crate) fn parse_wham_usage(body: &Value, locale: Locale) -> Option<UsageMetrics> {
    let rate = body
        .get("rate_limit")
        .or_else(|| body.get("rateLimit"))
        .unwrap_or(body);
    let primary = window_from(rate.get("primary_window").or_else(|| rate.get("primaryWindow")));
    let secondary = window_from(
        rate.get("secondary_window")
            .or_else(|| rate.get("secondaryWindow")),
    );
    let mut windows = Vec::new();
    if let Some((used, _)) = primary {
        windows.push(UsageWindow {
            left_label: labels::session_remaining(locale).to_owned(),
            right_label: format!("{}%", 100u8.saturating_sub(used)),
            value: 100u8.saturating_sub(used),
        });
    }
    if let Some((used, _)) = secondary {
        windows.push(UsageWindow {
            left_label: labels::week_remaining(locale).to_owned(),
            right_label: format!("{}%", 100u8.saturating_sub(used)),
            value: 100u8.saturating_sub(used),
        });
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

fn window_from(value: Option<&Value>) -> Option<(u8, Option<i64>)> {
    let obj = value?;
    let used = obj
        .get("used_percent")
        .or_else(|| obj.get("usedPercent"))?
        .as_u64()
        .or_else(|| {
            obj.get("used_percent")
                .or_else(|| obj.get("usedPercent"))?
                .as_f64()
                .map(|f| f.round() as u64)
        })?
        .min(100) as u8;
    let reset = obj
        .get("reset_at")
        .or_else(|| obj.get("resetAt"))
        .and_then(|v| v.as_i64());
    Some((used, reset))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_auth_tokens() {
        let auth = parse_auth_json(
            r#"{"tokens":{"access_token":"tok","refresh_token":"r","account_id":"acc"}}"#,
        )
        .unwrap();
        assert_eq!(auth.access_token, "tok");
        assert_eq!(auth.account_id.as_deref(), Some("acc"));
    }

    #[test]
    fn parses_double_bar_from_wham() {
        let body = serde_json::json!({
            "rate_limit": {
                "primary_window": { "used_percent": 24, "reset_at": 1 },
                "secondary_window": { "used_percent": 58, "reset_at": 2 }
            }
        });
        let metrics = parse_wham_usage(&body, Locale::En).unwrap();
        assert_eq!(metrics.windows.len(), 2);
        assert_eq!(metrics.windows[0].value, 76);
        assert_eq!(metrics.windows[1].value, 42);
    }
}
