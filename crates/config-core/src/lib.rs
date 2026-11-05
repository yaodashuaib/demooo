//! D50 设备配置域：Linked App → 可投影 Agent、余额卡片、声光草稿。
//!
//! 不碰 HID / 管道；时间与系统语言由调用方注入，便于单测。

mod linked_apps;
mod locale;
mod providers;
mod sound_light;
mod usage;
mod usage_log;
mod workbuddy_session;

pub use linked_apps::{
    ALLOWLIST, BridgeConfig, LinkedAppsError, MAX_AGENTS, ProjectableAgent, bind_agent,
    bound_agent_index, canonicalize_agent_id, default_agent_name, icon_relative_path,
    is_allowlisted, projectable_agents, unbind_agent,
};
pub use locale::{Locale, labels};
pub use providers::fetch_usage;
pub use providers::usage_last_error;
pub use providers::workbuddy::session_configured as workbuddy_session_configured;
pub use providers::workbuddy_last_error;
pub use usage_log::log_path_hint;
pub use sound_light::{SOUND_LIGHT_RELATIVE, SoundLightConfig, SoundLightError};
pub use usage::{UsageMetrics, UsageWindow, balance_for, mock_usage};
pub use workbuddy_session::{
    WORKBUDDY_SESSION_RELATIVE, WorkbuddySession, WorkbuddySessionError,
};
