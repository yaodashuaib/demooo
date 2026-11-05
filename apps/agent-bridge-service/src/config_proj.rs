//! 配置投影：linked-apps、余额缓存、图标/余额分路径下发。

use crate::device::DeviceWorker;
use bridge_ipc::{ConfigAgentView, ConfigView, DeviceView};
use config_core::{
    BridgeConfig, Locale, ProjectableAgent, SOUND_LIGHT_RELATIVE, WORKBUDDY_SESSION_RELATIVE,
    SoundLightConfig, UsageMetrics, WorkbuddySession, balance_for, fetch_usage,
    icon_relative_path, projectable_agents, usage_last_error, workbuddy_session_configured,
};
use d50_protocol::commands;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;

pub const DEFAULT_BALANCE_REFRESH_SECS: u64 = 180;

#[derive(Clone)]
pub struct CachedBalance {
    metrics: Option<UsageMetrics>,
    balance_ok: bool,
    summary: String,
}

pub struct ConfigState {
    pub agents: Vec<ProjectableAgent>,
    pub agent_views: Vec<ConfigAgentView>,
    pub last_projection_at: Option<u64>,
    pub sound_light: SoundLightConfig,
    pub errors: Vec<String>,
    pub balance_refresh_secs: u64,
    balance_cache: HashMap<String, CachedBalance>,
}

impl Default for ConfigState {
    fn default() -> Self {
        Self {
            agents: Vec::new(),
            agent_views: Vec::new(),
            last_projection_at: None,
            sound_light: SoundLightConfig::protocol_default(),
            errors: Vec::new(),
            balance_refresh_secs: DEFAULT_BALANCE_REFRESH_SECS,
            balance_cache: HashMap::new(),
        }
    }
}

impl ConfigState {
    pub fn load_sound_light(path: &Path) -> SoundLightConfig {
        match fs::read_to_string(path) {
            Ok(text) => SoundLightConfig::parse(&text)
                .unwrap_or_else(|_| SoundLightConfig::protocol_default()),
            Err(_) => SoundLightConfig::protocol_default(),
        }
    }

    pub fn save_sound_light(path: &Path, cfg: &SoundLightConfig) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let json = cfg.to_json().map_err(|e| e.to_string())?;
        fs::write(path, json).map_err(|e| e.to_string())
    }
}

pub struct ProjectionPaths {
    pub linked_apps: PathBuf,
    pub sound_light: PathBuf,
    pub workbuddy_session: PathBuf,
    pub bridge_config: PathBuf,
    pub icons: PathBuf,
}

pub fn default_paths() -> ProjectionPaths {
    let appdata = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    ProjectionPaths {
        linked_apps: appdata.join("Ulanzi").join("linked-apps.json"),
        sound_light: appdata.join(SOUND_LIGHT_RELATIVE),
        workbuddy_session: appdata.join(WORKBUDDY_SESSION_RELATIVE),
        bridge_config: appdata.join(BridgeConfig::RELATIVE),
        icons: icon_dir(),
    }
}

fn icon_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("D50_ICON_DIR") {
        return PathBuf::from(dir);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("icon")
}

pub fn system_locale() -> Locale {
    windows_ui_locale()
        .or_else(|| std::env::var("LANG").ok().map(|t| Locale::from_tag(&t)))
        .unwrap_or(Locale::En)
}

#[cfg(windows)]
fn windows_ui_locale() -> Option<Locale> {
    use std::os::windows::ffi::OsStringExt;
    let mut buf = [0u16; 85];
    unsafe extern "system" {
        fn GetUserDefaultLocaleName(lpLocaleName: *mut u16, cchLocaleName: i32) -> i32;
    }
    let len = unsafe { GetUserDefaultLocaleName(buf.as_mut_ptr(), buf.len() as i32) };
    if len <= 1 {
        return None;
    }
    let tag = std::ffi::OsString::from_wide(&buf[..(len as usize - 1)]);
    Some(Locale::from_tag(&tag.to_string_lossy()))
}

#[cfg(not(windows))]
fn windows_ui_locale() -> Option<Locale> {
    None
}

fn summarize(card: &d50_protocol::AgentBalance) -> String {
    match card.balance_type {
        d50_protocol::BalanceType::DoubleBar | d50_protocol::BalanceType::SingleBar => card
            .bars
            .iter()
            .map(|b| format!("{} {}", b.left_label, b.right_label))
            .collect::<Vec<_>>()
            .join(" | "),
        d50_protocol::BalanceType::Text => card.text.clone().unwrap_or_default(),
        d50_protocol::BalanceType::Value => format!(
            "{} {}",
            card.label.clone().unwrap_or_default(),
            card.value_text.clone().unwrap_or_default()
        ),
    }
}

fn cache_entry(agent: &ProjectableAgent, metrics: Option<UsageMetrics>, locale: Locale) -> CachedBalance {
    let balance_ok = metrics.is_some();
    let card = balance_for(agent, metrics.as_ref(), locale);
    CachedBalance {
        metrics,
        balance_ok,
        summary: summarize(&card),
    }
}

pub fn read_projectable(paths: &ProjectionPaths) -> (Vec<ProjectableAgent>, Vec<String>) {
    let mut errors = Vec::new();
    let json = match fs::read_to_string(&paths.linked_apps) {
        Ok(text) => text,
        Err(err) => {
            errors.push(format!("linked-apps: {err}"));
            return (Vec::new(), errors);
        }
    };
    match projectable_agents(&json) {
        Ok(agents) => (agents, errors),
        Err(err) => {
            errors.push(err.to_string());
            (Vec::new(), errors)
        }
    }
}

fn rebuild_views(state: &mut ConfigState, locale: Locale) {
    let mut views = Vec::new();
    for agent in &state.agents {
        let (ok, summary) = match state.balance_cache.get(&agent.id) {
            Some(c) => (c.balance_ok, c.summary.clone()),
            None => {
                let card = balance_for(agent, None, locale);
                (false, summarize(&card))
            }
        };
        views.push(ConfigAgentView {
            id: agent.id.clone(),
            name: agent.name.clone(),
            agent_index: agent.agent_index,
            balance_ok: ok,
            balance_summary: summary,
        });
    }
    state.agent_views = views;
}

fn encode_balances(
    agents: &[ProjectableAgent],
    cache: &HashMap<String, CachedBalance>,
    locale: Locale,
) -> Result<Vec<u8>, String> {
    let mut balances = Vec::new();
    for agent in agents {
        let metrics = cache.get(&agent.id).and_then(|c| c.metrics.as_ref());
        balances.push(balance_for(agent, metrics, locale));
    }
    commands::set_agent_balance(&balances).map_err(|err| format!("balance encode: {err}"))
}

fn push_balance(state: &mut ConfigState, device: &DeviceWorker, locale: Locale) {
    match encode_balances(&state.agents, &state.balance_cache, locale) {
        Ok(payload) if !payload.is_empty() => {
            eprintln!(
                "device: SET_AGENT_BALANCE agents={} bytes={}",
                state.agents.len(),
                payload.len()
            );
            device.set_agent_balance(payload);
        }
        Ok(_) => {}
        Err(err) => state.errors.push(err),
    }
}

fn push_icons(
    agents: &[ProjectableAgent],
    icon_dir: &Path,
    device: &DeviceWorker,
    errors: &mut Vec<String>,
) {
    let entries = load_icons(agents, icon_dir, errors);
    eprintln!("device: SET_AGENT_INDEX count={}", entries.len());
    device.set_agent_icons(entries);
}

/// 阻塞：全量拉用量。
pub fn fetch_all_balances(
    agents: &[ProjectableAgent],
    locale: Locale,
) -> (HashMap<String, CachedBalance>, Vec<String>) {
    let mut cache = HashMap::new();
    let mut errors = Vec::new();
    for agent in agents {
        let metrics = fetch_usage(&agent.id, locale);
        if let Some(err) = usage_last_error(&agent.id) {
            errors.push(format!("{}: {err}", agent.id));
        }
        cache.insert(agent.id.clone(), cache_entry(agent, metrics, locale));
    }
    (cache, errors)
}

/// 阻塞：只拉单个 id。
pub fn fetch_one_balance(
    agent: &ProjectableAgent,
    locale: Locale,
) -> (CachedBalance, Option<String>) {
    let metrics = fetch_usage(&agent.id, locale);
    let err = usage_last_error(&agent.id).map(|e| format!("{}: {e}", agent.id));
    (cache_entry(agent, metrics, locale), err)
}

/// 定时 / 上线：只刷新余额并下发（禁止图标）。
pub fn apply_balance_only(
    state: &mut ConfigState,
    device: &DeviceWorker,
    agents: Vec<ProjectableAgent>,
    cache: HashMap<String, CachedBalance>,
    errors: Vec<String>,
    online: bool,
    now_ms: u64,
    locale: Locale,
) {
    // 丢掉已解绑 id 的缓存，避免膨胀。
    state.balance_cache.retain(|id, _| agents.iter().any(|a| &a.id == id));
    for (id, entry) in cache {
        state.balance_cache.insert(id, entry);
    }
    state.agents = agents;
    state.errors = errors;
    rebuild_views(state, locale);
    state.last_projection_at = Some(now_ms);
    if online {
        push_balance(state, device, locale);
    } else {
        eprintln!("device offline: skip SET_AGENT_BALANCE (will push when online)");
    }
}

/// 绑定后：更新缓存一项 + 在线时 INDEX + BALANCE。
pub fn apply_after_bind(
    state: &mut ConfigState,
    device: &DeviceWorker,
    paths: &ProjectionPaths,
    agents: Vec<ProjectableAgent>,
    new_id: &str,
    entry: CachedBalance,
    fetch_err: Option<String>,
    online: bool,
    now_ms: u64,
    locale: Locale,
) {
    let mut errors = Vec::new();
    if let Some(err) = fetch_err {
        errors.push(err);
    }
    state.balance_cache.retain(|id, _| agents.iter().any(|a| &a.id == id));
    state.balance_cache.insert(new_id.to_owned(), entry);
    state.agents = agents;
    state.errors = errors;
    rebuild_views(state, locale);
    state.last_projection_at = Some(now_ms);
    if online {
        push_icons(&state.agents, &paths.icons, device, &mut state.errors);
        push_balance(state, device, locale);
    } else {
        eprintln!("device offline: skip INDEX/BALANCE after bind");
    }
}

/// 解绑后：删缓存、在线时 INDEX + 用缓存组 BALANCE（不重拉）。
pub fn apply_after_unbind(
    state: &mut ConfigState,
    device: &DeviceWorker,
    paths: &ProjectionPaths,
    agents: Vec<ProjectableAgent>,
    removed_id: &str,
    online: bool,
    now_ms: u64,
    locale: Locale,
) {
    state.balance_cache.remove(removed_id);
    state.balance_cache.retain(|id, _| agents.iter().any(|a| &a.id == id));
    state.agents = agents;
    state.errors.clear();
    rebuild_views(state, locale);
    state.last_projection_at = Some(now_ms);
    if online {
        push_icons(&state.agents, &paths.icons, device, &mut state.errors);
        push_balance(state, device, locale);
    } else {
        eprintln!("device offline: skip INDEX/BALANCE after unbind");
    }
}

/// 协议 11.9：固定 100×100。
const AGENT_ICON_PX: u32 = 100;

fn validate_agent_png(bytes: &[u8]) -> Result<(u32, u32), String> {
    let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
    let (w, h) = (img.width(), img.height());
    if w != AGENT_ICON_PX || h != AGENT_ICON_PX {
        return Err(format!("need {AGENT_ICON_PX}x{AGENT_ICON_PX}, got {w}x{h}"));
    }
    if bytes.is_empty() || bytes.len() > 65535 {
        return Err(format!("ImageLength out of range: {}", bytes.len()));
    }
    Ok((w, h))
}

fn load_icons(
    agents: &[ProjectableAgent],
    icon_dir: &Path,
    errors: &mut Vec<String>,
) -> Vec<(u8, Vec<u8>)> {
    let mut entries = Vec::new();
    for agent in agents {
        let Some(rel) = icon_relative_path(&agent.id) else {
            errors.push(format!("no icon mapping for {}", agent.id));
            continue;
        };
        let path = icon_dir.join(rel);
        match fs::read(&path) {
            Ok(bytes) if bytes.is_empty() => {
                errors.push(format!("empty icon {path:?}"));
            }
            Ok(bytes) => match validate_agent_png(&bytes) {
                Ok((w, h)) => {
                    eprintln!(
                        "device: icon {} {}x{} {} bytes (raw)",
                        agent.id,
                        w,
                        h,
                        bytes.len()
                    );
                    entries.push((agent.agent_index, bytes));
                }
                Err(err) => errors.push(format!("icon {path:?}: {err}")),
            },
            Err(err) => errors.push(format!("icon {path:?}: {err}")),
        }
    }
    entries
}

pub fn config_view(state: &ConfigState, device: DeviceView) -> ConfigView {
    let sound_light =
        serde_json::to_value(&state.sound_light).unwrap_or_else(|_| serde_json::json!({}));
    ConfigView {
        device,
        agents: state.agent_views.clone(),
        last_projection_at: state.last_projection_at,
        sound_light,
        workbuddy_session_configured: workbuddy_session_configured(),
        balance_refresh_secs: state.balance_refresh_secs,
        errors: state.errors.clone(),
    }
}

pub fn save_workbuddy_session(
    path: &Path,
    cookie: &str,
    chrome_major: Option<u32>,
) -> Result<(), String> {
    let trimmed = cookie.trim();
    if trimmed.is_empty() {
        WorkbuddySession::clear(path).map_err(|e| e.to_string())
    } else {
        WorkbuddySession::save(
            path,
            &WorkbuddySession {
                cookie: trimmed.to_owned(),
                chrome_major,
            },
        )
        .map_err(|e| e.to_string())
    }
}

pub type SharedConfig = Arc<Mutex<ConfigState>>;
