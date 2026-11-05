//! 用量采集：对齐 CodexBar 各 provider 的本地凭据与 HTTP 源。

mod chrome_ua;
mod claude;
mod codex;
mod http;
mod kimi;
mod last_error;
pub(crate) mod workbuddy;

use crate::locale::Locale;
use crate::usage::UsageMetrics;
use crate::usage_log;

/// 按 Agent `id` 拉取用量。失败返回 `None`（上层显示「暂无数据」）。
/// 设 `D50_SKIP_USAGE_NETWORK=1` 时跳过外网（测试用）。
pub fn fetch_usage(agent_id: &str, locale: Locale) -> Option<UsageMetrics> {
    last_error::clear(agent_id);
    if std::env::var_os("D50_SKIP_USAGE_NETWORK").is_some() {
        let msg = "skipped: D50_SKIP_USAGE_NETWORK is set";
        usage_log::usage(agent_id, msg);
        last_error::set(agent_id, msg);
        return None;
    }
    usage_log::usage(agent_id, "fetch begin");
    let result = match agent_id {
        "chatgpt" => codex::fetch(locale),
        "claude_code" => claude::fetch(locale),
        "kimi" => kimi::fetch(locale),
        "workbuddy" => workbuddy::fetch(locale),
        _ => {
            let msg = format!("unknown agent id `{agent_id}`");
            usage_log::usage(agent_id, &msg);
            last_error::set(agent_id, msg);
            None
        }
    };
    if let Some(m) = &result {
        usage_log::usage(
            agent_id,
            format!(
                "fetch ok windows={} text_len={}",
                m.windows.len(),
                m.text.as_ref().map(|t| t.len()).unwrap_or(0)
            ),
        );
    } else {
        usage_log::usage(agent_id, "fetch failed (see earlier lines / last_error)");
    }
    result
}

/// 读取并清除某 Agent 最近一次用量失败原因（投影写入 `errors`）。
pub fn usage_last_error(agent_id: &str) -> Option<String> {
    last_error::take(agent_id)
}

/// WorkBuddy 最近一次失败原因（兼容旧调用）。
pub fn workbuddy_last_error() -> Option<String> {
    workbuddy::take_last_error().or_else(|| last_error::take("workbuddy"))
}

pub(crate) use last_error::set as set_last_error;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_id_returns_none() {
        assert!(fetch_usage("unknown", Locale::En).is_none());
    }
}
