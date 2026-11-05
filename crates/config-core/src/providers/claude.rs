//! Claude Code：读 `~/.claude/.credentials.json`，调 OAuth usage（对齐 CodexBar）。

use super::http::{get_json, home_dir};
use crate::locale::{Locale, labels};
use crate::usage::{UsageMetrics, UsageWindow};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

pub fn fetch(locale: Locale) -> Option<UsageMetrics> {
    use super::set_last_error;
    use crate::usage_log;
    const AGENT: &str = "claude_code";

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
    let token = match parse_credentials_json(&text) {
        Some(t) => {
            usage_log::usage(AGENT, format!("token ok len={}", t.len()));
            t
        }
        None => {
            let msg = "parse credentials failed (need claudeAiOauth.accessToken)";
            usage_log::usage(AGENT, msg);
            set_last_error(AGENT, msg);
            return None;
        }
    };
    let headers = [
        ("Authorization", format!("Bearer {token}")),
        ("Accept", "application/json".into()),
        ("Content-Type", "application/json".into()),
        ("anthropic-beta", "oauth-2025-04-20".into()),
        ("User-Agent", "claude-cli/2.1.0 (external, cli)".into()),
    ];
    let header_refs: Vec<(&str, &str)> = headers
        .iter()
        .map(|(k, v)| (*k, v.as_str()))
        .collect();
    let url = "https://api.anthropic.com/api/oauth/usage?cedar_ember=1";
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
    usage_log::usage(AGENT, format!("HTTP {status}"));
    if status != 200 {
        set_last_error(AGENT, format!("HTTP {status} from anthropic oauth usage"));
        return None;
    }
    match parse_oauth_usage(&body, locale) {
        Some(m) => Some(m),
        None => {
            let msg = "parse oauth usage failed";
            usage_log::usage(AGENT, msg);
            set_last_error(AGENT, msg);
            None
        }
    }
}

fn credentials_path() -> Option<PathBuf> {
    Some(home_dir()?.join(".claude").join(".credentials.json"))
}

pub(crate) fn parse_credentials_json(text: &str) -> Option<String> {
    let json: Value = serde_json::from_str(text).ok()?;
    let oauth = json
        .get("claudeAiOauth")
        .or_else(|| json.get("claude_ai_oauth"))
        .unwrap_or(&json);
    oauth
        .get("accessToken")
        .or_else(|| oauth.get("access_token"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

pub(crate) fn parse_oauth_usage(body: &Value, locale: Locale) -> Option<UsageMetrics> {
    let mut windows = Vec::new();
    if let Some(w) = remaining_from_window(body.get("five_hour")) {
        windows.push(UsageWindow {
            left_label: labels::session_remaining(locale).to_owned(),
            right_label: format!("{w}%"),
            value: w,
        });
    }
    if let Some(w) = remaining_from_window(body.get("seven_day")) {
        windows.push(UsageWindow {
            left_label: labels::week_remaining(locale).to_owned(),
            right_label: format!("{w}%"),
            value: w,
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

fn remaining_from_window(value: Option<&Value>) -> Option<u8> {
    let window = value?;
    let util = window.get("utilization")?;
    let used_f = util
        .as_f64()
        .or_else(|| util.as_u64().map(|u| u as f64))?;
    let used = used_f.round().clamp(0.0, 100.0) as u8;
    Some(100u8.saturating_sub(used))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_credentials() {
        let token = parse_credentials_json(
            r#"{"claudeAiOauth":{"accessToken":"at-123","refreshToken":"rt"}}"#,
        )
        .unwrap();
        assert_eq!(token, "at-123");
    }

    #[test]
    fn parses_five_and_seven_day() {
        let body = serde_json::json!({
            "five_hour": { "utilization": 20.0, "resets_at": "x" },
            "seven_day": { "utilization": 55.5, "resets_at": "y" }
        });
        let metrics = parse_oauth_usage(&body, Locale::ZhHans).unwrap();
        assert_eq!(metrics.windows[0].value, 80);
        assert_eq!(metrics.windows[1].value, 44);
        assert_eq!(metrics.windows[0].left_label, "5小时剩余");
    }
}
