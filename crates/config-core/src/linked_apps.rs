//! Linked Apps → 可投影 Agent；`agentIndex` 由绑定分配并持久化。

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use thiserror::Error;

/// 本 demo 可投影的 Agent id；其它 `aiAgent` 忽略。
pub const ALLOWLIST: &[&str] = &["claude_code", "kimi", "chatgpt", "workbuddy"];
pub const MAX_AGENTS: usize = 6;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LinkedAppsError {
    #[error("LINKED_APPS_JSON")]
    Json,
    #[error("LINKED_APPS_IO")]
    Io,
    #[error("LINKED_APPS_UNKNOWN_ID")]
    UnknownId,
    #[error("LINKED_APPS_FULL")]
    Full,
    #[error("LINKED_APPS_NOT_BOUND")]
    NotBound,
    #[error("LINKED_APPS_ALREADY_BOUND")]
    AlreadyBound,
    #[error("LINKED_APPS_INDEX_CONFLICT")]
    IndexConflict,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectableAgent {
    pub id: String,
    pub name: String,
    pub agent_index: u8,
}

#[derive(Debug, Deserialize)]
struct FileView {
    #[serde(default)]
    apps: Vec<AppView>,
}

#[derive(Debug, Deserialize)]
struct AppView {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    category: String,
    #[serde(default)]
    status: String,
    #[serde(default, rename = "agentIndex")]
    agent_index: Option<u8>,
}

/// 从 `linked-apps.json` 选出可投影 Agent：configured ∩ allowlist ∩ 合法持久化 `agentIndex`。
/// 缺 `agentIndex` 或冲突的条目跳过（不静默重排）。
pub fn projectable_agents(json: &str) -> Result<Vec<ProjectableAgent>, LinkedAppsError> {
    let file: FileView = serde_json::from_str(json).map_err(|_| LinkedAppsError::Json)?;
    let mut out: Vec<ProjectableAgent> = Vec::new();
    let mut used = HashSet::new();
    for app in file.apps {
        if app.category != "aiAgent" {
            continue;
        }
        if app.status != "configured" {
            continue;
        }
        if !ALLOWLIST.iter().any(|id| *id == app.id) {
            continue;
        }
        if out.iter().any(|a: &ProjectableAgent| a.id == app.id) {
            continue;
        }
        let Some(agent_index) = app.agent_index else {
            continue;
        };
        if agent_index > 5 || used.contains(&agent_index) {
            continue;
        }
        if out.len() >= MAX_AGENTS {
            break;
        }
        used.insert(agent_index);
        let id = app.id;
        let name = if app.name.is_empty() {
            id.clone()
        } else {
            app.name
        };
        out.push(ProjectableAgent {
            id,
            name,
            agent_index,
        });
    }
    out.sort_by_key(|a| a.agent_index);
    Ok(out)
}

pub fn is_allowlisted(id: &str) -> bool {
    ALLOWLIST.iter().any(|a| *a == id)
}

/// 把 hook / IPC 传来的 `agent_id` 归一成 allowlist id（`claude_code` 等）。
/// 未识别的返回 `None`（审批投影不得发明槽位）。
pub fn canonicalize_agent_id(agent_id: &str) -> Option<&'static str> {
    match agent_id {
        "claude_code" | "claude-code" | "claude" => Some("claude_code"),
        "chatgpt" | "codex" => Some("chatgpt"),
        "kimi" | "kimi-cli" | "kimi_cli" => Some("kimi"),
        "workbuddy" => Some("workbuddy"),
        _ => None,
    }
}

/// 在已绑定集合里查 `AgentIndex`；未绑定或未知 id 返回 `None`。
pub fn bound_agent_index(agents: &[ProjectableAgent], agent_id: &str) -> Option<u8> {
    let canonical = canonicalize_agent_id(agent_id)?;
    agents
        .iter()
        .find(|a| a.id == canonical)
        .map(|a| a.agent_index)
}

pub fn default_agent_name(id: &str) -> String {
    match id {
        "claude_code" => "Claude Code".into(),
        "kimi" => "Kimi Code CLI".into(),
        "chatgpt" => "ChatGPT".into(),
        "workbuddy" => "WorkBuddy".into(),
        other => other.to_owned(),
    }
}

/// 仓库内图标相对路径（相对 `icon/`）。须为协议 11.9：100×100、8bit RGB/RGBA PNG。
pub fn icon_relative_path(id: &str) -> Option<&'static str> {
    match id {
        "claude_code" => Some("cc.png"),
        "kimi" => Some("kimi.png"),
        "chatgpt" => Some("chatgpt.png"),
        "workbuddy" => Some("workbuddy.png"),
        _ => None,
    }
}

fn load_root(path: &Path) -> Result<Value, LinkedAppsError> {
    if !path.exists() {
        return Ok(json!({ "schemaVersion": 1, "apps": [] }));
    }
    let text = fs::read_to_string(path).map_err(|_| LinkedAppsError::Io)?;
    serde_json::from_str(&text).map_err(|_| LinkedAppsError::Json)
}

fn save_root(path: &Path, root: &Value) -> Result<(), LinkedAppsError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| LinkedAppsError::Io)?;
    }
    let text = serde_json::to_string_pretty(root).map_err(|_| LinkedAppsError::Json)?;
    fs::write(path, text).map_err(|_| LinkedAppsError::Io)
}

fn apps_array_mut(root: &mut Value) -> Result<&mut Vec<Value>, LinkedAppsError> {
    if !root.is_object() {
        *root = json!({ "schemaVersion": 1, "apps": [] });
    }
    let obj = root.as_object_mut().ok_or(LinkedAppsError::Json)?;
    if !obj.contains_key("apps") {
        obj.insert("apps".into(), json!([]));
    }
    obj.get_mut("apps")
        .and_then(|v| v.as_array_mut())
        .ok_or(LinkedAppsError::Json)
}

fn occupied_indices(apps: &[Value]) -> HashSet<u8> {
    let mut used = HashSet::new();
    for app in apps {
        let category = app.get("category").and_then(|v| v.as_str()).unwrap_or("");
        let status = app.get("status").and_then(|v| v.as_str()).unwrap_or("");
        if category != "aiAgent" || status != "configured" {
            continue;
        }
        let id = app.get("id").and_then(|v| v.as_str()).unwrap_or("");
        if !is_allowlisted(id) {
            continue;
        }
        if let Some(idx) = app.get("agentIndex").and_then(|v| v.as_u64()) {
            if idx <= 5 {
                used.insert(idx as u8);
            }
        }
    }
    used
}

fn allocate_index(used: &HashSet<u8>) -> Option<u8> {
    (0u8..6).find(|i| !used.contains(i))
}

/// 绑定 Agent：写成 configured，分配最小空闲 `agentIndex`（已绑定则 AlreadyBound）。
pub fn bind_agent(
    path: &Path,
    id: &str,
    name: Option<&str>,
) -> Result<ProjectableAgent, LinkedAppsError> {
    if !is_allowlisted(id) {
        return Err(LinkedAppsError::UnknownId);
    }
    let mut root = load_root(path)?;
    let apps = apps_array_mut(&mut root)?;
    let used = occupied_indices(apps);
    if apps.iter().any(|a| {
        a.get("id").and_then(|v| v.as_str()) == Some(id)
            && a.get("category").and_then(|v| v.as_str()) == Some("aiAgent")
            && a.get("status").and_then(|v| v.as_str()) == Some("configured")
            && a.get("agentIndex").and_then(|v| v.as_u64()).is_some()
    }) {
        return Err(LinkedAppsError::AlreadyBound);
    }

    let agent_index = allocate_index(&used).ok_or(LinkedAppsError::Full)?;
    let display = name
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| default_agent_name(id));

    if let Some(app) = apps.iter_mut().find(|a| {
        a.get("id").and_then(|v| v.as_str()) == Some(id)
            && a.get("category").and_then(|v| v.as_str()) == Some("aiAgent")
    }) {
        let obj = app.as_object_mut().ok_or(LinkedAppsError::Json)?;
        obj.insert("name".into(), json!(display));
        obj.insert("status".into(), json!("configured"));
        obj.insert("category".into(), json!("aiAgent"));
        obj.insert("agentIndex".into(), json!(agent_index));
        if !obj.contains_key("icon") {
            obj.insert("icon".into(), json!(""));
        }
        if !obj.contains_key("config") {
            obj.insert("config".into(), json!({}));
        }
    } else {
        apps.push(json!({
            "id": id,
            "name": display,
            "category": "aiAgent",
            "status": "configured",
            "agentIndex": agent_index,
            "icon": "",
            "config": { "kind": "agent", "credentialRef": null }
        }));
    }

    save_root(path, &root)?;
    Ok(ProjectableAgent {
        id: id.to_owned(),
        name: display,
        agent_index,
    })
}

/// 解绑：status → unbound，清除 `agentIndex`（不重排其它槽）。
pub fn unbind_agent(path: &Path, id: &str) -> Result<(), LinkedAppsError> {
    if !is_allowlisted(id) {
        return Err(LinkedAppsError::UnknownId);
    }
    let mut root = load_root(path)?;
    let apps = apps_array_mut(&mut root)?;
    let mut found = false;
    for app in apps.iter_mut() {
        if app.get("id").and_then(|v| v.as_str()) != Some(id) {
            continue;
        }
        if app.get("category").and_then(|v| v.as_str()) != Some("aiAgent") {
            continue;
        }
        found = true;
        let obj = app.as_object_mut().ok_or(LinkedAppsError::Json)?;
        obj.insert("status".into(), json!("unbound"));
        obj.remove("agentIndex");
    }
    if !found {
        return Err(LinkedAppsError::NotBound);
    }
    save_root(path, &root)?;
    Ok(())
}

/// bridge 侧余额刷新间隔草稿。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeConfig {
    #[serde(default = "default_balance_refresh_secs")]
    pub balance_refresh_secs: u64,
}

fn default_balance_refresh_secs() -> u64 {
    180
}

impl Default for BridgeConfig {
    fn default() -> Self {
        Self {
            balance_refresh_secs: default_balance_refresh_secs(),
        }
    }
}

impl BridgeConfig {
    pub const RELATIVE: &'static str = r"Ulanzi\D50\d50-bridge.json";

    pub fn clamp_secs(secs: u64) -> u64 {
        secs.clamp(30, 3600)
    }

    pub fn load(path: &Path) -> Self {
        match fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(path: &Path, cfg: &Self) -> Result<(), LinkedAppsError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|_| LinkedAppsError::Io)?;
        }
        let mut copy = cfg.clone();
        copy.balance_refresh_secs = Self::clamp_secs(copy.balance_refresh_secs);
        let text = serde_json::to_string_pretty(&copy).map_err(|_| LinkedAppsError::Json)?;
        fs::write(path, text).map_err(|_| LinkedAppsError::Io)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env::temp_dir;

    #[test]
    fn skips_without_agent_index() {
        let json = r#"{
          "apps": [
            {"id":"kimi","name":"Kimi","category":"aiAgent","status":"configured","icon":"","config":{}}
          ]
        }"#;
        assert!(projectable_agents(json).unwrap().is_empty());
    }

    #[test]
    fn uses_persisted_indices_sorted() {
        let json = r#"{
          "apps": [
            {"id":"chatgpt","name":"ChatGPT","category":"aiAgent","status":"configured","agentIndex":2},
            {"id":"kimi","name":"Kimi","category":"aiAgent","status":"configured","agentIndex":0}
          ]
        }"#;
        let agents = projectable_agents(json).unwrap();
        assert_eq!(
            agents
                .iter()
                .map(|a| (a.id.as_str(), a.agent_index))
                .collect::<Vec<_>>(),
            vec![("kimi", 0), ("chatgpt", 2)]
        );
    }

    #[test]
    fn skips_conflicting_indices() {
        let json = r#"{
          "apps": [
            {"id":"kimi","name":"Kimi","category":"aiAgent","status":"configured","agentIndex":1},
            {"id":"chatgpt","name":"ChatGPT","category":"aiAgent","status":"configured","agentIndex":1}
          ]
        }"#;
        let agents = projectable_agents(json).unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].id, "kimi");
    }

    #[test]
    fn bind_allocates_lowest_free_slot() {
        let path = temp_dir().join(format!("d50-bind-{}.json", std::process::id()));
        let _ = fs::remove_file(&path);
        let a = bind_agent(&path, "kimi", None).unwrap();
        assert_eq!(a.agent_index, 0);
        let b = bind_agent(&path, "chatgpt", None).unwrap();
        assert_eq!(b.agent_index, 1);
        assert_eq!(bind_agent(&path, "kimi", None), Err(LinkedAppsError::AlreadyBound));
        unbind_agent(&path, "kimi").unwrap();
        let agents = projectable_agents(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].id, "chatgpt");
        assert_eq!(agents[0].agent_index, 1);
        let c = bind_agent(&path, "workbuddy", None).unwrap();
        assert_eq!(c.agent_index, 0);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn chatgpt_maps_to_chatgpt_icon_file() {
        assert_eq!(icon_relative_path("chatgpt"), Some("chatgpt.png"));
    }

    #[test]
    fn claude_code_maps_to_cc_icon_file() {
        assert_eq!(icon_relative_path("claude_code"), Some("cc.png"));
    }

    #[test]
    fn empty_when_none_configured() {
        let json = r#"{"apps":[{"id":"claude_code","name":"C","category":"aiAgent","status":"error","icon":"","config":{}}]}"#;
        assert!(projectable_agents(json).unwrap().is_empty());
    }

    #[test]
    fn canonicalize_hook_aliases_to_allowlist() {
        assert_eq!(canonicalize_agent_id("claude-code"), Some("claude_code"));
        assert_eq!(canonicalize_agent_id("codex"), Some("chatgpt"));
        assert_eq!(canonicalize_agent_id("kimi-cli"), Some("kimi"));
        assert_eq!(canonicalize_agent_id("unknown"), None);
    }

    #[test]
    fn bound_agent_index_follows_persisted_slots() {
        let agents = vec![
            ProjectableAgent {
                id: "chatgpt".into(),
                name: "ChatGPT".into(),
                agent_index: 0,
            },
            ProjectableAgent {
                id: "claude_code".into(),
                name: "Claude Code".into(),
                agent_index: 1,
            },
        ];
        assert_eq!(bound_agent_index(&agents, "claude-code"), Some(1));
        assert_eq!(bound_agent_index(&agents, "codex"), Some(0));
        assert_eq!(bound_agent_index(&agents, "kimi"), None);
    }
}
