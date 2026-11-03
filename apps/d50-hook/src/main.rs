use clap::Parser;
use std::io::Read;
use std::time::Duration;

/// 被 Agent 的 PermissionRequest / 相关 hook 调用。
///
/// - **可门控**（Claude / Codex / WorkBuddy）：阻塞等设备裁决，再按方言回填。
/// - **仅通知**（Kimi：`fireAndForget` 的 PermissionRequest）：设备只发 open app 提示，
///   立刻 `{}` 返回；裁决仍在 Kimi 自己的界面（ADR-0005）。
///
/// 铁律：拿不到决策（超时、掉线、服务没起）就输出 `{}` 并 exit 0，
/// 让 Agent 回落到它自己的终端询问。绝不在这条路径上自动放行。
#[derive(Debug, Parser)]
#[command(name = "d50-hook", version, about)]
struct Args {
    #[arg(long, default_value = bridge_ipc::DEFAULT_PIPE)]
    pipe: String,
    /// 决定输出的方言；省略时按 agent 推断。
    #[arg(long)]
    dialect: Option<String>,
    #[arg(long, default_value = "claude-code")]
    agent: String,
    #[arg(long)]
    event: Option<String>,
    /// 比 Agent 自己的 hook 超时少几秒，留出回落的余量。
    #[arg(long)]
    timeout_ms: Option<u64>,
    /// 「有 N 个问题待回复」这类提示在设备上留多久。
    #[arg(long, default_value = "300000")]
    notice_ttl_ms: u64,
    /// 强制不显示「open app」：CLI 环境下没有可拉起的客户端时用。
    #[arg(long)]
    no_open_app: bool,
    /// 重试同一次请求时复用它，服务会返回同一个答案。
    #[arg(long)]
    request_id: Option<String>,
    /// 只打印将要发送的请求，不连接服务。
    #[arg(long)]
    dry_run: bool,
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut args = Args::parse();
    // WorkBuddy HookExecutor 日志里只写 `cmd=…d50-hook.exe`，常丢掉 settings 的 `args`。
    // 默认仍是 claude-code；若进程树已是 WorkBuddy / ChatGPT 桌面壳，按宿主纠正品牌与 deeplink。
    if let Some(inferred) = infer_agent_from_host(&args.agent) {
        eprintln!(
            "d50-hook: --agent={} 与宿主不符，改为 {inferred}",
            args.agent
        );
        args.agent = inferred;
    }
    let payload = read_stdin_json();
    let dialect = args
        .dialect
        .clone()
        .unwrap_or_else(|| infer_dialect(&args.agent));
    let timeout_ms = args
        .timeout_ms
        .unwrap_or_else(|| default_timeout_ms(&dialect));
    let request_id = args
        .request_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let tool_name = first_string(&payload, &["tool_name", "toolName", "name"])
        .unwrap_or_else(|| "unknown".into());
    let session_id = first_string(&payload, &["session_id", "sessionId", "conversation_id"])
        .unwrap_or_else(|| "default".into());
    // CLI 里跑的 Agent 通常没有注册 app 的 URL scheme（`claude://` 属于桌面客户端），
    // 打不开就别给按钮，免得按下去没反应。
    let link = resolve_open_link(&args.agent, &session_id, !args.no_open_app);
    eprintln!(
        "d50-hook: agent={} tool={} event={:?} open_app={:?}",
        args.agent,
        tool_name,
        args.event,
        link
    );
    if link.is_none() && (is_question_tool(&tool_name) || is_notice_only_agent(&args.agent)) {
        eprintln!("d50-hook: 没有可用的 open app 链接，提示只做提醒");
    }

    let event = args.event.as_deref().unwrap_or("");

    // Kimi：PermissionRequest 是 fireAndForget，不读 hook 决定（ADR-0005）。
    // 设备只推「open app」提醒；request_permissions / 选择题等同理。绝不阻塞等按键。
    if is_notice_only_agent(&args.agent) {
        if event.eq_ignore_ascii_case("PermissionRequest")
            || is_question_tool(&tool_name)
            || event.is_empty()
        {
            eprintln!(
                "d50-hook: {} 仅通知（fireAndForget / 对话类），不门控 tool={tool_name}",
                args.agent
            );
            notify(
                &args,
                &payload,
                &tool_name,
                &session_id,
                &link,
                &request_id,
            )
            .await;
        } else if event.eq_ignore_ascii_case("PreToolUse") {
            eprintln!("d50-hook: {} PreToolUse 不阻塞（tool={tool_name}）", args.agent);
        }
        println!("{{}}");
        return;
    }

    // 需要人回到 Agent 自己的界面里回答的请求：设备只做提醒，不阻塞。
    //
    // Claude：同一工具先 PreToolUse、再 PermissionRequest；提示只在 PreToolUse 发一次。
    // WorkBuddy 桌面：AskUserQuestion 在 `waiting_for_permission` 用原生 UI，**不跑**
    // PermissionRequest hook；人答完才 TOOL_STARTED → PreToolUse。弹层出现时靠 hook
    // 推不出 D50；PreToolUse 仍推一次（标题/deeplink 走 workbuddy），至少链路可验。
    if is_question_tool(&tool_name) {
        if event.eq_ignore_ascii_case("PermissionRequest") {
            eprintln!(
                "d50-hook: {tool_name} 已在 PreToolUse 提醒过，PermissionRequest 不再推送设备"
            );
            println!("{{}}");
            return;
        }
        if is_workbuddy_agent(&args.agent) && event.eq_ignore_ascii_case("PreToolUse") {
            eprintln!(
                "d50-hook: WorkBuddy 选题 UI 早于 PreToolUse；此时推送仅为补提醒（已答完也可能闪一次）"
            );
        }
        notify(
            &args,
            &payload,
            &tool_name,
            &session_id,
            &link,
            &request_id,
        )
        .await;
        println!("{{}}");
        return;
    }

    // 计划确认这类「选一个」也交回终端，设备上给同意/拒绝只会误导。
    if is_dialog_tool(&tool_name) {
        eprintln!("d50-hook: {tool_name} 是对话确认，不是权限裁决，交回终端");
        println!("{{}}");
        return;
    }

    // PreToolUse 不能阻塞等设备（ADR-0006）：只允许 deny，没有「批准」语义。
    // 误挂到 PreToolUse 时必须立刻放行，否则 Codex 会 Conversation interrupted。
    if event.eq_ignore_ascii_case("PreToolUse") {
        eprintln!("d50-hook: PreToolUse 不阻塞裁决，交回 Agent（tool={tool_name}）");
        println!("{{}}");
        return;
    }

    let suggestions = permission_suggestions(&payload);
    let options = approval_options(&args.agent, &suggestions);
    // 三选项时，按钮字太短，规则得写在内容行里。
    let summary = match rule_summary(&suggestions) {
        Some(rules) if options.len() > 2 => rules,
        _ => summarize(&payload, &tool_name),
    };
    let cwd = first_string(&payload, &["cwd"])
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|path| path.display().to_string())
        })
        .unwrap_or_default();

    let request = bridge_ipc::Request::ApprovalRequest {
        request_id,
        agent_id: args.agent.clone(),
        session_id,
        tool_name,
        summary,
        cwd,
        options,
        timeout_ms,
        open_app: link,
    };

    if args.dry_run {
        println!("{}", serde_json::to_string_pretty(&request).unwrap());
        return;
    }

    let response = bridge_ipc::call(
        &args.pipe,
        &request,
        Duration::from_millis(timeout_ms.saturating_add(3_000)),
    )
    .await;

    let output = match response {
        Ok(bridge_ipc::Response::Approval { outcome }) => {
            render_with(&dialect, &outcome, suggestions.as_ref())
        }
        Ok(bridge_ipc::Response::Error { code, message }) => {
            eprintln!("d50-hook: {code}: {message}");
            render_with(&dialect, &approval_core::Outcome::Unavailable, None)
        }
        Ok(other) => {
            eprintln!("d50-hook: unexpected response: {other:?}");
            "{}".to_string()
        }
        Err(error) => {
            eprintln!("d50-hook: {error}");
            "{}".to_string()
        }
    };
    // hook 的退出码不能让 Agent 报错；没有决策时就是一次空输出。
    println!("{output}");
}

/// 发一条非阻塞提示；设备上只有一个「open app」。
async fn notify(
    args: &Args,
    payload: &serde_json::Value,
    tool_name: &str,
    session_id: &str,
    link: &Option<String>,
    request_id: &str,
) {
    let count = question_count(payload);
    let content = notice_content(tool_name, count);
    let title = format!("{} · 待回复", agent_label(&args.agent));
    let Some(link) = link.clone() else {
        // 没有 App 可开：仍然提醒一次，内容里告诉人去哪儿。
        let request = bridge_ipc::Request::Notice {
            notice_id: format!("notice-{request_id}"),
            agent_id: args.agent.clone(),
            session_id: session_id.to_string(),
            title,
            content: format!("{content}（终端）"),
            link: String::new(),
            ttl_ms: args.notice_ttl_ms,
        };
        if args.dry_run {
            println!("{}", serde_json::to_string_pretty(&request).unwrap());
            return;
        }
        if let Err(error) =
            bridge_ipc::call(&args.pipe, &request, Duration::from_millis(3_000)).await
        {
            eprintln!("d50-hook: 提示没有送到设备（{error}）");
        }
        return;
    };
    let request = bridge_ipc::Request::Notice {
        notice_id: format!("notice-{request_id}"),
        agent_id: args.agent.clone(),
        session_id: session_id.to_string(),
        title,
        content,
        link,
        ttl_ms: args.notice_ttl_ms,
    };
    if args.dry_run {
        println!("{}", serde_json::to_string_pretty(&request).unwrap());
        return;
    }
    match bridge_ipc::call(
        &args.pipe,
        &request,
        Duration::from_millis(3_000),
    )
    .await
    {
        Ok(_) => {}
        Err(error) => eprintln!("d50-hook: 提示没有送到设备（{error}）"),
    }
}

fn read_stdin_json() -> serde_json::Value {
    let mut body = String::new();
    let _ = std::io::stdin().read_to_string(&mut body);
    serde_json::from_str(&body).unwrap_or(serde_json::Value::Null)
}

fn first_string(payload: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| payload.get(*key))
        .and_then(|value| value.as_str())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// 设备上只放得下一行摘要，优先取命令本身，其次取整个入参。
fn summarize(payload: &serde_json::Value, tool_name: &str) -> String {
    if let Some(input) = payload
        .get("tool_input")
        .or_else(|| payload.get("toolInput"))
    {
        for key in ["command", "file_path", "path", "pattern", "url"] {
            if let Some(value) = input.get(key).and_then(|value| value.as_str()) {
                return value.to_string();
            }
        }
        return input.to_string();
    }
    tool_name.to_string()
}

/// 需要人当面作答的对话工具：设备不接管，一律回落终端。
///
/// 这些工具同样会触发 `PermissionRequest`（调用工具本身需要许可），
/// 但它们的产物是「一个问题」而不是「一个动作」，按下同意只是允许提问，
/// 屏幕上还会留一个设备回答不了的选择菜单。
const DIALOG_TOOLS: &[&str] = &["ExitPlanMode"];

fn is_dialog_tool(tool_name: &str) -> bool {
    DIALOG_TOOLS
        .iter()
        .any(|name| name.eq_ignore_ascii_case(tool_name))
}

/// 只能在 Agent 自己的界面里回答的请求。
///
/// - `AskUserQuestion` / `request_user_input`：选择题，设备上没有选项可按下。
/// - `request_permissions`：沙箱扩权请求，钩子层只能拒绝、不能批准（ADR-0006）。
///
/// 对它们设备只做提醒，按「open app」跳回会话页由人自己处理。
const QUESTION_TOOLS: &[&str] = &["AskUserQuestion", "request_user_input", "request_permissions"];

fn is_question_tool(tool_name: &str) -> bool {
    QUESTION_TOOLS
        .iter()
        .any(|name| name.eq_ignore_ascii_case(tool_name))
}

/// 每个 Agent 打开自己会话页的协议；会话 ID 原样带入。
fn deeplink(agent: &str, session_id: &str) -> Option<String> {
    if session_id.is_empty() || session_id == "default" {
        return None;
    }
    match agent {
        "claude-code" | "claude_code" | "claude" => Some(format!(
            "claude://code/continue?session=local_{session_id}"
        )),
        "codex" | "chatgpt" => Some(format!("codex://threads/{session_id}")),
        "workbuddy" | "codebuddy" => Some(format!("workbuddy://chat/{session_id}")),
        _ => None,
    }
}

/// settings 的 `args` 被丢掉、只剩默认 `--agent claude-code` 时，用桌面宿主纠正。
fn infer_agent_from_host(agent: &str) -> Option<String> {
    let looks_default = matches!(
        agent,
        "claude-code" | "claude_code" | "claude" | ""
    );
    if !looks_default {
        return None;
    }
    match bridge_ipc::focus::detect_host() {
        bridge_ipc::focus::HostKind::Desktop { exe } => {
            let n = exe.to_ascii_lowercase();
            if n.starts_with("workbuddy") || n.starts_with("codebuddy") {
                Some("workbuddy".into())
            } else if n.starts_with("chatgpt") {
                Some("codex".into())
            } else {
                None
            }
        }
        _ => None,
    }
}

fn is_workbuddy_agent(agent: &str) -> bool {
    matches!(agent, "workbuddy" | "codebuddy")
}

/// 决定 open app 打开哪里。
///
/// - **桌面客户端**（祖先为 `WorkBuddy.exe` / 商店版 `Claude.exe` / `ChatGPT.exe`）：
///   deeplink 优先（`workbuddy://` / `claude://` / `codex://`）；否则 `focus://exe/…`。
/// - **CLI 终端**：`focus://window/…` 或 `focus://pid/…?title=…` 置前终端窗。
/// - **Unknown**：先试 CLI focus；再试已注册的 deeplink（不颠倒：避免桌面被误判时
///   用错 HWND；桌面识别靠路径，见 `is_claude_desktop_path`）。
fn resolve_open_link(agent: &str, session_id: &str, enabled: bool) -> Option<String> {
    if !enabled {
        return None;
    }
    match bridge_ipc::focus::detect_host() {
        bridge_ipc::focus::HostKind::Desktop { exe } => {
            eprintln!("d50-hook: host=desktop exe={exe}");
            if let Some(link) = deeplink(agent, session_id) {
                let scheme = link.split("://").next().unwrap_or_default();
                if scheme_registered(scheme) {
                    return Some(link);
                }
                eprintln!("d50-hook: scheme {scheme}:// 未注册，回退 focus://exe");
            }
            // scheme 未注册时仍尽量拉起客户端主窗。
            bridge_ipc::focus::target_exe(&exe)
                .or_else(|| bridge_ipc::focus::target_exe("WorkBuddy.exe"))
                .or_else(|| bridge_ipc::focus::target_exe("Claude.exe"))
                .or_else(|| bridge_ipc::focus::target_exe("ChatGPT.exe"))
        }
        bridge_ipc::focus::HostKind::Cli => {
            eprintln!("d50-hook: host=cli");
            bridge_ipc::focus::target(Some(agent_label(agent)))
        }
        bridge_ipc::focus::HostKind::Unknown => {
            eprintln!("d50-hook: host=unknown, try cli focus then deeplink");
            if let Some(focus) = bridge_ipc::focus::target(Some(agent_label(agent))) {
                return Some(focus);
            }
            let link = deeplink(agent, session_id)?;
            let scheme = link.split("://").next().unwrap_or_default();
            if scheme_registered(scheme) {
                Some(link)
            } else {
                None
            }
        }
    }
}

#[cfg(windows)]
fn scheme_registered(scheme: &str) -> bool {
    std::process::Command::new("reg")
        .args(["query", &format!(r"HKEY_CLASSES_ROOT\{scheme}")])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(not(windows))]
fn scheme_registered(_scheme: &str) -> bool {
    false
}

fn agent_label(agent: &str) -> &str {
    match agent {
        "claude-code" | "claude_code" | "claude" => "Claude Code",
        "codex" | "chatgpt" => "Codex",
        "workbuddy" | "codebuddy" => "WorkBuddy",
        "kimi-cli" | "kimi" | "kimi-code" | "kimi_code" => "Kimi",
        other => other,
    }
}

/// PermissionRequest 不读 hook 返回值的 Agent：设备只做 open app 提醒（ADR-0005）。
fn is_notice_only_agent(agent: &str) -> bool {
    matches!(
        agent,
        "kimi-cli" | "kimi" | "kimi-code" | "kimi_code"
    )
}

fn infer_dialect(agent: &str) -> String {
    match agent {
        // WorkBuddy / CodeBuddy 的 PermissionRequest 与 Claude 同形：
        // hookSpecificOutput.decision.behavior（官方 Hooks 文档与 Claude 对齐）。
        "claude-code" | "claude_code" | "claude" | "workbuddy" | "codebuddy" => {
            "claude".into()
        }
        "codex" | "chatgpt" => "codex".into(),
        _ => "simple".into(),
    }
}

fn default_timeout_ms(dialect: &str) -> u64 {
    match dialect {
        "codex" => 590_000,
        _ => 55_000,
    }
}

/// 对话类请求带几个问题；没有 `questions` 数组时按 1 个算。
fn question_count(payload: &serde_json::Value) -> usize {
    payload
        .get("tool_input")
        .or_else(|| payload.get("toolInput"))
        .and_then(|input| input.get("questions"))
        .and_then(|questions| questions.as_array())
        .map(|questions| questions.len())
        .unwrap_or(0)
}

fn notice_content(tool_name: &str, count: usize) -> String {
    match count {
        0 if tool_name.eq_ignore_ascii_case("request_permissions") => {
            "有 1 个权限请求待您确认…".to_string()
        }
        0 if is_question_tool(tool_name) => "请在 App 中确认…".to_string(),
        // Kimi fireAndForget 的普通工具权限：也是「回 App 自己点」。
        0 => "有权限请求待您在 App 中确认…".to_string(),
        1 => "有 1 个问题待您回复…".to_string(),
        count => format!("有 {count} 个问题待您回复…"),
    }
}

/// Claude Code 会把「总是允许」要装的规则放在 `permission_suggestions` 里，
/// 结构与 `updatedPermissions` 相同，可以原样回填。Codex 没有这个字段。
fn permission_suggestions(payload: &serde_json::Value) -> Option<serde_json::Value> {
    payload
        .get("permission_suggestions")
        .or_else(|| payload.get("permissionSuggestions"))
        .filter(|value| value.as_array().is_some_and(|items| !items.is_empty()))
        .cloned()
}

fn approval_options(
    agent: &str,
    suggestions: &Option<serde_json::Value>,
) -> Vec<bridge_ipc::RequestOption> {
    let mut options = vec![
        bridge_ipc::RequestOption {
            index: 1,
            label: "Agree".into(),
            decision: approval_core::Decision::Allow,
        },
        bridge_ipc::RequestOption {
            index: 2,
            label: "Ignore".into(),
            decision: approval_core::Decision::Deny,
        },
    ];
    // 只有 Claude 能表达「总是允许」；Codex 会因此 fail closed（ADR-0006）。
    // WorkBuddy 尚未确认 updatedPermissions，有建议时也只给两键，避免半门控。
    if suggestions.is_some() && matches!(agent, "claude-code" | "claude_code" | "claude") {
        options.push(bridge_ipc::RequestOption {
            index: 3,
            label: "Always".into(),
            decision: approval_core::Decision::AllowAlways,
        });
    }
    options
}

/// 把建议规则压成一行，供设备的内容行显示，例如 `Bash(git push:*)`。
fn rule_summary(suggestions: &Option<serde_json::Value>) -> Option<String> {
    let items = suggestions.as_ref()?.as_array()?;
    let mut parts = Vec::new();
    for item in items {
        let rules = item.get("rules")?.as_array()?;
        for rule in rules {
            let tool = rule.get("toolName").and_then(|value| value.as_str())?;
            match rule
                .get("ruleContent")
                .or_else(|| rule.get("rule_content"))
                .and_then(|value| value.as_str())
            {
                Some(content) => parts.push(format!("{tool}({content})")),
                None => parts.push(tool.to_string()),
            }
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

#[cfg(test)]
fn render(dialect: &str, outcome: &approval_core::Outcome) -> String {
    render_with(dialect, outcome, None)
}

/// `suggestions` 是 Claude 的 `permission_suggestions`：只有「总是允许」用得上。
fn render_with(
    dialect: &str,
    outcome: &approval_core::Outcome,
    suggestions: Option<&serde_json::Value>,
) -> String {
    let decision = match outcome {
        approval_core::Outcome::Decided { decision, .. } => *decision,
        // 没有人做过决定：不给答案，交给 Agent 自己问。
        _ => return "{}".into(),
    };
    match dialect {
        // Claude Code 与 Codex 的 PermissionRequest 决定结构相同：
        // `hookSpecificOutput.decision.behavior ∈ {allow, deny}`（取自各自的
        // schema：Codex 的 PermissionRequestDecisionWire 与 BehaviorWire）。
        // Codex 另规定 `updatedInput`、`interrupt: true` 会 fail closed，
        // 所以这里只发 behavior，一个保留字段都不带。
        "claude" | "codex" => match (decision, dialect, suggestions) {
            (
                approval_core::Decision::AllowAlways,
                "claude",
                Some(rules),
            ) => serde_json::json!({
                "hookSpecificOutput": {
                    "hookEventName": "PermissionRequest",
                    "decision": {
                        "behavior": "allow",
                        "updatedPermissions": rules
                    }
                }
            })
            .to_string(),
            // Codex：`updatedPermissions` 会让整条决定作废，降级成本次放行。
            _ => {
                let behavior = if decision.allows() { "allow" } else { "deny" };
                serde_json::json!({
                    "hookSpecificOutput": {
                        "hookEventName": "PermissionRequest",
                        "decision": { "behavior": behavior }
                    }
                })
                .to_string()
            }
        },
        "simple" => serde_json::json!({
            "decision": if decision.allows() { "allow" } else { "deny" }
        })
        .to_string(),
        _ => "{}".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approval_core::{Decision, Outcome};

    #[test]
    fn allow_renders_claude_shape() {
        let text = render(
            "claude",
            &Outcome::Decided {
                decision: Decision::Allow,
                option_index: 1,
                decided_at_ms: 1,
            },
        );
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["hookSpecificOutput"]["decision"]["behavior"], "allow");
    }

    #[test]
    fn deny_renders_deny() {
        let text = render(
            "simple",
            &Outcome::Decided {
                decision: Decision::Deny,
                option_index: 2,
                decided_at_ms: 1,
            },
        );
        assert_eq!(text, r#"{"decision":"deny"}"#);
    }

    #[test]
    fn no_decision_renders_fallback() {
        for outcome in [Outcome::Expired, Outcome::Unavailable, Outcome::Cancelled] {
            assert_eq!(render("claude", &outcome), "{}");
            assert_eq!(render("simple", &outcome), "{}");
            assert_eq!(render("codex", &outcome), "{}");
        }
    }

    #[test]
    fn codex_uses_the_same_decision_shape_as_claude() {
        let outcome = Outcome::Decided {
            decision: Decision::Allow,
            option_index: 1,
            decided_at_ms: 1,
        };
        assert_eq!(render("codex", &outcome), render("claude", &outcome));
        let value: serde_json::Value = serde_json::from_str(&render("codex", &outcome)).unwrap();
        assert_eq!(value["hookSpecificOutput"]["decision"]["behavior"], "allow");
        // Codex 对保留字段 fail closed，一个都不能带。
        let decision = &value["hookSpecificOutput"]["decision"];
        assert!(decision.get("updatedInput").is_none());
        assert!(decision.get("interrupt").is_none());
    }

    #[test]
    fn summary_prefers_command() {
        let payload = serde_json::json!({
            "tool_name": "Bash",
            "tool_input": { "command": "rm -rf build", "description": "clean" }
        });
        assert_eq!(summarize(&payload, "Bash"), "rm -rf build");
    }

    #[test]
    fn dialog_tools_are_left_to_the_terminal() {
        assert!(is_dialog_tool("exitplanmode"));
        // 普通的动作类工具必须走设备门控。
        assert!(!is_dialog_tool("Bash"));
        assert!(!is_dialog_tool("Write"));
        assert!(!is_dialog_tool("mcp__memory__create_entities"));
    }

    #[test]
    fn question_tools_are_notice_not_decision() {
        for tool in ["AskUserQuestion", "request_user_input", "request_permissions"] {
            assert!(is_question_tool(tool), "{tool} 应该走提示而不是裁决");
        }
        assert!(!is_question_tool("Bash"));
        assert!(!is_question_tool("apply_patch"));
    }

    #[test]
    fn deeplinks_match_each_agent() {
        assert_eq!(
            deeplink("claude-code", "abc").as_deref(),
            Some("claude://code/continue?session=local_abc")
        );
        assert_eq!(
            deeplink("codex", "abc").as_deref(),
            Some("codex://threads/abc")
        );
        assert_eq!(
            deeplink("workbuddy", "abc").as_deref(),
            Some("workbuddy://chat/abc")
        );
        assert_eq!(
            deeplink("codebuddy", "abc").as_deref(),
            Some("workbuddy://chat/abc")
        );
        // 没有会话 ID 或未知 Agent 时不显示「open app」。
        assert_eq!(deeplink("codex", "default"), None);
        assert_eq!(deeplink("kimi-cli", "abc"), None);
    }

    #[test]
    fn workbuddy_skips_late_pretooluse_notice_for_questions() {
        assert!(is_workbuddy_agent("workbuddy"));
        assert!(is_workbuddy_agent("codebuddy"));
        assert!(!is_workbuddy_agent("claude-code"));
    }

    #[test]
    fn notice_counts_the_questions() {
        let payload = serde_json::json!({
            "tool_name": "AskUserQuestion",
            "tool_input": { "questions": [{ "q": "a" }, { "q": "b" }, { "q": "c" }] }
        });
        assert_eq!(question_count(&payload), 3);
        assert_eq!(notice_content("AskUserQuestion", 3), "有 3 个问题待您回复…");
        assert_eq!(notice_content("AskUserQuestion", 1), "有 1 个问题待您回复…");
        assert_eq!(
            notice_content("request_permissions", 0),
            "有 1 个权限请求待您确认…"
        );
    }

    #[test]
    fn claude_gets_three_options_only_with_suggestions() {
        let suggestions = Some(serde_json::json!([{
            "type": "addRules",
            "behavior": "allow",
            "destination": "session",
            "rules": [{ "toolName": "Bash", "ruleContent": "git push:*" }]
        }]));
        let options = approval_options("claude-code", &suggestions);
        assert_eq!(options.len(), 3);
        assert_eq!(options[2].label, "Always");
        assert_eq!(options[2].decision, approval_core::Decision::AllowAlways);
        assert_eq!(
            rule_summary(&suggestions).as_deref(),
            Some("Bash(git push:*)")
        );

        // Codex 永远只有两个：updatedPermissions 会让整条决定作废。
        assert_eq!(approval_options("codex", &suggestions).len(), 2);
        // Claude 没有建议规则时也不要第三颗键。
        assert_eq!(approval_options("claude-code", &None).len(), 2);
    }

    #[test]
    fn always_allow_sends_updated_permissions_only_to_claude() {
        let suggestions = serde_json::json!([{
            "type": "addRules",
            "behavior": "allow",
            "destination": "session",
            "rules": [{ "toolName": "Bash", "ruleContent": "git push:*" }]
        }]);
        let outcome = Outcome::Decided {
            decision: Decision::AllowAlways,
            option_index: 3,
            decided_at_ms: 1,
        };
        let claude = render_with("claude", &outcome, Some(&suggestions));
        let value: serde_json::Value = serde_json::from_str(&claude).unwrap();
        assert_eq!(
            value["hookSpecificOutput"]["decision"]["updatedPermissions"][0]["rules"][0]["toolName"],
            "Bash"
        );

        // Codex 上不能带这个字段，降级成普通 allow。
        let codex = render_with("codex", &outcome, Some(&suggestions));
        let value: serde_json::Value = serde_json::from_str(&codex).unwrap();
        assert_eq!(value["hookSpecificOutput"]["decision"]["behavior"], "allow");
        assert!(
            value["hookSpecificOutput"]["decision"]
                .get("updatedPermissions")
                .is_none()
        );
    }

    #[test]
    fn summary_falls_back_to_tool_name() {
        let payload = serde_json::json!({ "tool_name": "Bash" });
        assert_eq!(summarize(&payload, "Bash"), "Bash");
    }

    #[test]
    fn dialect_inference_matches_agents() {
        assert_eq!(infer_dialect("claude-code"), "claude");
        assert_eq!(infer_dialect("codex"), "codex");
        assert_eq!(infer_dialect("workbuddy"), "claude");
        assert_eq!(infer_dialect("codebuddy"), "claude");
        assert_eq!(infer_dialect("kimi-cli"), "simple");
    }

    #[test]
    fn kimi_is_notice_only_workbuddy_can_gate() {
        assert!(is_notice_only_agent("kimi-cli"));
        assert!(is_notice_only_agent("kimi"));
        assert!(!is_notice_only_agent("workbuddy"));
        assert!(!is_notice_only_agent("claude-code"));
        assert!(!is_notice_only_agent("codex"));
    }

    #[test]
    fn workbuddy_gets_two_approval_options() {
        let suggestions = Some(serde_json::json!([{
            "type": "addRules",
            "rules": [{ "toolName": "Bash" }]
        }]));
        assert_eq!(approval_options("workbuddy", &suggestions).len(), 2);
        assert_eq!(approval_options("workbuddy", &None).len(), 2);
    }

    #[test]
    fn kimi_permission_notice_copy() {
        assert_eq!(
            notice_content("Bash", 0),
            "有权限请求待您在 App 中确认…"
        );
        assert_eq!(
            notice_content("request_permissions", 0),
            "有 1 个权限请求待您确认…"
        );
    }
}
