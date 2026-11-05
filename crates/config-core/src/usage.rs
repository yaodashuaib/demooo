use crate::linked_apps::ProjectableAgent;
use crate::locale::{Locale, labels};
use d50_protocol::{AgentBalance, BalanceBar, BalanceType};

/// 对齐 CodexBar 的一档配额窗（会话 / 周等）。
#[derive(Debug, Clone, PartialEq)]
pub struct UsageWindow {
    pub left_label: String,
    pub right_label: String,
    /// 0–100 剩余比例。
    pub value: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UsageMetrics {
    pub windows: Vec<UsageWindow>,
    pub text: Option<String>,
    pub value_label: Option<String>,
    pub value_text: Option<String>,
}

/// 按指标形态选用固件展示 Type。
pub fn balance_for(
    agent: &ProjectableAgent,
    metrics: Option<&UsageMetrics>,
    locale: Locale,
) -> AgentBalance {
    let Some(metrics) = metrics else {
        return AgentBalance {
            name: clip_name(&agent.name),
            agent_index: agent.agent_index,
            balance_type: BalanceType::Text,
            bars: Vec::new(),
            text: Some(labels::no_data(locale).to_owned()),
            label: None,
            value_text: None,
        };
    };

    if metrics.windows.len() >= 2 {
        return AgentBalance {
            name: clip_name(&agent.name),
            agent_index: agent.agent_index,
            balance_type: BalanceType::DoubleBar,
            bars: metrics
                .windows
                .iter()
                .take(2)
                .map(|w| BalanceBar {
                    left_label: clip_label(&w.left_label),
                    right_label: clip_label(&w.right_label),
                    value: w.value.min(100),
                })
                .collect(),
            text: None,
            label: None,
            value_text: None,
        };
    }

    if metrics.windows.len() == 1 {
        let w = &metrics.windows[0];
        return AgentBalance {
            name: clip_name(&agent.name),
            agent_index: agent.agent_index,
            balance_type: BalanceType::SingleBar,
            bars: vec![BalanceBar {
                left_label: clip_label(&w.left_label),
                right_label: clip_label(&w.right_label),
                value: w.value.min(100),
            }],
            text: None,
            label: None,
            value_text: None,
        };
    }

    if let (Some(label), Some(value_text)) = (&metrics.value_label, &metrics.value_text) {
        return AgentBalance {
            name: clip_name(&agent.name),
            agent_index: agent.agent_index,
            balance_type: BalanceType::Value,
            bars: Vec::new(),
            text: None,
            label: Some(clip_label(label)),
            value_text: Some(value_text.chars().filter(|c| c.is_ascii_digit()).take(16).collect()),
        };
    }

    if let Some(text) = &metrics.text {
        return AgentBalance {
            name: clip_name(&agent.name),
            agent_index: agent.agent_index,
            balance_type: BalanceType::Text,
            bars: Vec::new(),
            text: Some(clip_text(text)),
            label: None,
            value_text: None,
        };
    }

    balance_for(agent, None, locale)
}

/// 骨架阶段用的固定用量（双进度条），语义对齐 CodexBar 常见两档。
pub fn mock_usage(locale: Locale) -> UsageMetrics {
    UsageMetrics {
        windows: vec![
            UsageWindow {
                left_label: labels::session_remaining(locale).to_owned(),
                right_label: "80%".into(),
                value: 80,
            },
            UsageWindow {
                left_label: labels::week_remaining(locale).to_owned(),
                right_label: "42%".into(),
                value: 42,
            },
        ],
        text: None,
        value_label: None,
        value_text: None,
    }
}

fn clip_name(s: &str) -> String {
    d50_protocol::clip_utf8(s, 32)
}

fn clip_label(s: &str) -> String {
    let flat: String = s.chars().filter(|c| *c != '\n' && *c != '\r').collect();
    d50_protocol::clip_utf8(&flat, 32)
}

fn clip_text(s: &str) -> String {
    d50_protocol::clip_utf8(s, 256)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent() -> ProjectableAgent {
        ProjectableAgent {
            id: "kimi".into(),
            name: "Kimi".into(),
            agent_index: 0,
        }
    }

    #[test]
    fn missing_metrics_become_text_placeholder() {
        let card = balance_for(&agent(), None, Locale::En);
        assert_eq!(card.balance_type, BalanceType::Text);
        assert_eq!(card.text.as_deref(), Some("No data"));
    }

    #[test]
    fn two_windows_become_double_bar() {
        let card = balance_for(&agent(), Some(&mock_usage(Locale::ZhHans)), Locale::ZhHans);
        assert_eq!(card.balance_type, BalanceType::DoubleBar);
        assert_eq!(card.bars.len(), 2);
        assert_eq!(card.bars[0].left_label, "5小时剩余");
    }
}
