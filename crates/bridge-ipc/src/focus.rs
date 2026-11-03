//! 把「跑 Agent 的那个窗口」带回前台。
//!
//! ## 宿主分流
//!
//! - **桌面客户端**（进程树含 `ChatGPT.exe` 等）→ `focus://exe/…` 或由 hook 发 deeplink
//! - **CLI 终端** → hook 当场解析 HWND，发 `focus://window/{hwnd}`；按键只 Activate。
//!   解析失败才退回 `focus://pid/…?title=…`（优先活着的 CLI 链，而非短命 hook 祖先）。
//!
//! ## Windows CLI 策略（文档化 Win32 API）
//!
//! - **有控制台的 hook**（Claude CLI 等）：只用 `GetConsoleWindow` / 进程树枚举，
//!   **不** `AttachConsole`，以免拆掉 Agent 控制台。
//! - **无控制台的 hook**（Codex `app-server-daemon`）：可安全 `AttachConsole` 到活着的
//!   CLI（跳过 daemon 自身），以便 ConPTY 下拿到真正的 WT 窗。
//! - 按键置前尽量走无控制台的 `d50-focus`；**服务进程禁止 `AttachConsole`/`FreeConsole`**，
//!   否则会拆掉自己的控制台（Ctrl+C 失灵）。

/// hook 所在宿主：桌面 App 还是 CLI 终端。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostKind {
    /// 桌面客户端（如 ChatGPT.exe / Codex 桌面壳）
    Desktop { exe: String },
    /// 终端里跑的 CLI
    Cli,
    Unknown,
}

/// 根据进程树判断当前 hook 跑在桌面客户端还是 CLI 里。
#[cfg(windows)]
pub fn detect_host() -> HostKind {
    win::detect_host_kind()
}

#[cfg(not(windows))]
pub fn detect_host() -> HostKind {
    HostKind::Unknown
}

/// CLI：hook 当场解析宿主 HWND，发 `focus://window/{hwnd}`。
///
/// `title_hint`（如 `"Claude Code"` / `"Codex"`）用来在同进程树多窗时消歧，
/// 并拒绝 Explorer / 路径型前台窗。解析不到才退回带 `?title=` 的 pid 链。
///
/// Codex 的 hook 常跑在无 UI 的 `app-server-daemon` 里：祖先链没有终端窗，
/// 短命 PID 按键时已死。此时改钉 **活着的 CLI 进程**（非 daemon）或直接解析 HWND。
#[cfg(windows)]
pub fn target(title_hint: Option<&str>) -> Option<String> {
    // hook 触发时就能看见自己的终端窗；把 HWND 写进 link，open app 不再靠标题猜。
    if let Some(hwnd) = resolve_host_hwnd(title_hint).filter(|h| *h != 0) {
        return Some(format!("focus://window/{hwnd}"));
    }

    let ancestors = ancestor_chain();
    let cli = live_agent_cli_pids(title_hint);
    // daemon / 无终端祖先：pid 链改用活着的 CLI，否则按键时只剩 app-server。
    let chain = if !cli.is_empty()
        && (ancestors.is_empty() || chain_lacks_terminal_host(&ancestors))
    {
        cli
    } else if !ancestors.is_empty() {
        ancestors
    } else {
        cli
    };
    if chain.is_empty() {
        return None;
    }
    let joined: Vec<String> = chain.iter().map(u32::to_string).collect();
    let mut link = format!("focus://pid/{}", joined.join(","));
    if let Some(hint) = title_hint.map(str::trim).filter(|s| !s.is_empty()) {
        link.push_str("?title=");
        link.push_str(&url_encode(hint));
    }
    Some(link)
}

/// 桌面客户端：按进程名置前主窗（`focus://exe/ChatGPT.exe`）。
#[cfg(windows)]
pub fn target_exe(exe_name: &str) -> Option<String> {
    let name = exe_name.trim();
    if name.is_empty() {
        return None;
    }
    Some(format!("focus://exe/{}", url_encode(name)))
}

#[cfg(not(windows))]
pub fn target_exe(_exe_name: &str) -> Option<String> {
    None
}

#[cfg(target_os = "macos")]
pub fn target(title_hint: Option<&str>) -> Option<String> {
    let _ = title_hint;
    let app = terminal_app()?;
    Some(format!("focus://app/{}", url_encode(&app)))
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn target(_title_hint: Option<&str>) -> Option<String> {
    None
}

/// 执行一个 `focus://` 目标；不是这个前缀返回 `None`，交给调用方当普通 deeplink。
pub fn perform(target: &str) -> Option<bool> {
    let rest = target.strip_prefix("focus://")?;
    let (kind, value) = rest.split_once('/')?;
    match kind {
        "pid" => {
            let (pids, title) = split_pid_target(value);
            Some(run_helper(&pids, title.as_deref()))
        }
        "exe" => {
            let exe = url_decode(value.split(['?', '#']).next().unwrap_or(value));
            // 与 pid 路径一样优先 helper，避免在服务线程里做 Win32 置前。
            Some(if run_exe_helper(&exe) {
                true
            } else {
                raise_by_exe_name(&exe)
            })
        }
        "app" => Some(focus_app(&url_decode(value))),
        "window" => {
            let (hwnd_str, _query) = value.split_once('?').unwrap_or((value, ""));
            let Ok(hwnd) = hwnd_str
                .split(['#'])
                .next()
                .unwrap_or(hwnd_str)
                .parse::<u64>()
            else {
                return None;
            };
            // HWND 已在 hook 时钉死；不再用 title 否决（用户改标题是常态）。
            Some(if run_window_helper(hwnd) {
                true
            } else {
                raise_window_checked(hwnd, None)
            })
        }
        _ => Some(false),
    }
}

fn split_pid_target(value: &str) -> (String, Option<String>) {
    let (path, query) = value.split_once('?').unwrap_or((value, ""));
    let title = query
        .split('&')
        .find_map(|pair| pair.strip_prefix("title="))
        .map(url_decode)
        .filter(|s| !s.is_empty());
    (path.to_string(), title)
}

#[cfg(windows)]
fn run_helper(pids: &str, title_hint: Option<&str>) -> bool {
    let parsed: Vec<u32> = pids
        .split(',')
        .filter_map(|value| value.trim().parse().ok())
        .collect();
    if parsed.is_empty() {
        return false;
    }

    // 先走无控制台 helper；失败时服务进程内只做 EnumWindows，绝不 AttachConsole。
    if run_focus_helper(pids, title_hint) {
        return true;
    }

    match raise_from_pids_no_attach(&parsed, title_hint) {
        Ok(()) => true,
        Err(error) => {
            eprintln!("focus in-process: {error}");
            false
        }
    }
}

#[cfg(windows)]
fn helper_exe() -> Option<std::path::PathBuf> {
    let current = std::env::current_exe().ok()?;
    Some(current.parent()?.join("d50-focus.exe"))
}

/// helper 最多等这么久；超时杀掉，避免服务卡死 / Ctrl+C 无响应。
const HELPER_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(2_000);

#[cfg(windows)]
fn run_command_with_timeout(mut cmd: std::process::Command) -> bool {
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(error) => {
            eprintln!("focus helper 启动失败: {error}");
            return false;
        }
    };
    let deadline = std::time::Instant::now() + HELPER_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                eprintln!("focus helper 超时已杀掉（{}ms）", HELPER_TIMEOUT.as_millis());
                return false;
            }
            Err(error) => {
                eprintln!("focus helper wait 失败: {error}");
                let _ = child.kill();
                return false;
            }
        }
    }
}

#[cfg(windows)]
fn run_focus_helper(pids: &str, title_hint: Option<&str>) -> bool {
    let Some(helper) = helper_exe() else {
        return false;
    };
    let mut cmd = std::process::Command::new(&helper);
    cmd.args(["--pids", pids]);
    if let Some(hint) = title_hint {
        cmd.args(["--title", hint]);
    }
    run_command_with_timeout(cmd)
}

#[cfg(windows)]
fn run_window_helper(hwnd: u64) -> bool {
    let Some(helper) = helper_exe() else {
        return false;
    };
    let mut cmd = std::process::Command::new(&helper);
    cmd.args(["--hwnd", &hwnd.to_string()]);
    run_command_with_timeout(cmd)
}

#[cfg(windows)]
fn run_exe_helper(exe_name: &str) -> bool {
    let Some(helper) = helper_exe() else {
        return false;
    };
    let mut cmd = std::process::Command::new(&helper);
    cmd.args(["--exe", exe_name]);
    run_command_with_timeout(cmd)
}

/// 供 `d50-focus --exe` 调用：按进程名置前，不走 helper（避免递归）。
#[cfg(windows)]
pub fn raise_exe(exe_name: &str) -> Result<(), String> {
    win::raise_by_exe(exe_name)
}

#[cfg(windows)]
fn raise_by_exe_name(exe_name: &str) -> bool {
    match raise_exe(exe_name) {
        Ok(()) => true,
        Err(error) => {
            eprintln!("focus exe/{exe_name}: {error}");
            false
        }
    }
}

#[cfg(not(windows))]
fn raise_by_exe_name(_exe_name: &str) -> bool {
    false
}

#[cfg(not(windows))]
fn run_helper(_pids: &str, _title_hint: Option<&str>) -> bool {
    false
}

#[cfg(not(windows))]
fn run_window_helper(_hwnd: u64) -> bool {
    false
}

#[cfg(not(windows))]
fn run_exe_helper(_exe_name: &str) -> bool {
    false
}

/// 供 `d50-focus` 调用：按 pid 链解析并置前（helper 内可 AttachConsole）。
#[cfg(windows)]
pub fn raise_from_pids(pids: &[u32], title_hint: Option<&str>) -> Result<(), String> {
    raise_from_pids_inner(pids, title_hint, true)
}

/// 服务进程内回退：禁止 AttachConsole，以免 FreeConsole 拆掉自身控制台。
#[cfg(windows)]
fn raise_from_pids_no_attach(pids: &[u32], title_hint: Option<&str>) -> Result<(), String> {
    raise_from_pids_inner(pids, title_hint, false)
}

#[cfg(windows)]
fn raise_from_pids_inner(
    pids: &[u32],
    title_hint: Option<&str>,
    allow_attach_console: bool,
) -> Result<(), String> {
    if let Some(hwnd) = hwnd_from_pid_chain(pids, title_hint, allow_attach_console) {
        if raise_hwnd(hwnd) {
            return Ok(());
        }
        return Err(format!("SetForegroundWindow 失败 hwnd={hwnd}"));
    }
    Err(format!(
        "没有可用的宿主窗口（pids={pids:?}, title={title_hint:?}）"
    ))
}

/// 供 `d50-focus --hwnd`：按句柄置前，忽略标题。
#[cfg(windows)]
pub fn raise_hwnd_raw(hwnd: u64) -> Result<(), String> {
    if hwnd == 0 {
        return Err("hwnd=0".into());
    }
    if raise_hwnd(hwnd as isize) {
        Ok(())
    } else {
        Err(format!("SetForegroundWindow 失败 hwnd={hwnd}"))
    }
}

#[cfg(windows)]
fn raise_window_checked(window: u64, title_hint: Option<&str>) -> bool {
    if window == 0 {
        return false;
    }
    raise_hwnd_checked(window as isize, title_hint)
}

#[cfg(not(windows))]
fn raise_window_checked(_window: u64, _title_hint: Option<&str>) -> bool {
    false
}

#[cfg(target_os = "macos")]
fn focus_app(app: &str) -> bool {
    std::process::Command::new("open")
        .args(["-a", app])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(not(target_os = "macos"))]
fn focus_app(_app: &str) -> bool {
    false
}

/// 从 `TERM_PROGRAM` 推出终端 App 的名字；macOS 的 `open -a` 认这些名字。
#[cfg(target_os = "macos")]
fn terminal_app() -> Option<String> {
    let program = std::env::var("TERM_PROGRAM").ok()?;
    let name = match program.as_str() {
        "Apple_Terminal" => "Terminal",
        "iTerm.app" => "iTerm",
        "vscode" => "Visual Studio Code",
        "WezTerm" => "WezTerm",
        "Hyper" => "Hyper",
        "WarpTerminal" => "Warp",
        other => other,
    };
    Some(name.to_string())
}

#[cfg(windows)]
mod win {
    use std::collections::{HashMap, HashSet};
    use std::sync::Mutex;

    use windows_sys::core::BOOL;
    use windows_sys::Win32::Foundation::{CloseHandle, HWND, LPARAM, TRUE};
    use windows_sys::Win32::System::Console::{AttachConsole, FreeConsole, GetConsoleWindow};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        AttachThreadInput, GetCurrentProcessId, GetCurrentThreadId, OpenProcess,
        QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY, VK_MENU,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, EnumWindows, GW_OWNER, GetForegroundWindow, GetParent, GetWindow,
        GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, HWND_TOPMOST,
        HWND_NOTOPMOST, IsIconic, IsWindow, IsWindowVisible, SW_RESTORE, SW_SHOW,
        SetForegroundWindow, SetWindowPos, ShowWindow, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
    };

    /// 父进程链（不含自己），最多 8 层；遇到系统进程即停，避免挂上 svchost/services。
    pub fn ancestor_chain() -> Vec<u32> {
        let parents = process_parents();
        let names = process_names();
        let mut chain = Vec::new();
        let mut pid = unsafe { GetCurrentProcessId() };
        for _ in 0..8 {
            let Some(&parent) = parents.get(&pid) else {
                break;
            };
            if parent == 0 || parent == pid || chain.contains(&parent) {
                break;
            }
            if names
                .get(&parent)
                .map(|n| is_system_process(n))
                .unwrap_or(false)
            {
                break;
            }
            chain.push(parent);
            pid = parent;
        }
        chain
    }

    fn is_system_process(exe: &str) -> bool {
        matches!(
            exe.to_ascii_lowercase().as_str(),
            "svchost.exe"
                | "services.exe"
                | "wininit.exe"
                | "winlogon.exe"
                | "csrss.exe"
                | "smss.exe"
                | "lsass.exe"
                | "fontdrvhost.exe"
                | "dwm.exe"
        )
    }

    pub fn looks_like_noise_title(title: &str) -> bool {
        let t = title.trim();
        t.is_empty()
            || t.eq_ignore_ascii_case("DDE Server Window")
            || t.eq_ignore_ascii_case("Windows Terminal")
    }

    /// 品牌 / 待回复标题（相对 cwd / 随手改名）。仅测试断言用。
    #[cfg(test)]
    pub fn is_durable_focus_title(title: &str) -> bool {
        let t = title.to_ascii_lowercase();
        t.contains("claude")
            || t.contains("codex")
            || t.contains("action required")
            || t.contains('✳')
            || t.contains("workbuddy")
    }

    fn process_parents() -> HashMap<u32, u32> {
        let mut parents = HashMap::new();
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snapshot.is_null() {
                return parents;
            }
            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            if Process32FirstW(snapshot, &mut entry) != 0 {
                loop {
                    parents.insert(entry.th32ProcessID, entry.th32ParentProcessID);
                    if Process32NextW(snapshot, &mut entry) == 0 {
                        break;
                    }
                }
            }
            let _ = CloseHandle(snapshot);
        }
        parents
    }

    fn process_names() -> HashMap<u32, String> {
        let mut names = HashMap::new();
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snapshot.is_null() {
                return names;
            }
            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            if Process32FirstW(snapshot, &mut entry) != 0 {
                loop {
                    let name = wchar_to_string(&entry.szExeFile);
                    names.insert(entry.th32ProcessID, name);
                    if Process32NextW(snapshot, &mut entry) == 0 {
                        break;
                    }
                }
            }
            let _ = CloseHandle(snapshot);
        }
        names
    }

    fn wchar_to_string(buf: &[u16]) -> String {
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..len])
    }

    fn is_terminal_host(exe: &str) -> bool {
        matches!(
            exe.to_ascii_lowercase().as_str(),
            "windowsterminal.exe"
                | "wt.exe"
                | "openconsole.exe"
                | "conhost.exe"
                | "cmd.exe"
                | "powershell.exe"
                | "pwsh.exe"
                | "mintty.exe"
                | "conemu.exe"
                | "conemu64.exe"
                | "alacritty.exe"
                | "wezterm-gui.exe"
                | "wezterm.exe"
                | "windowsterminal"
        )
    }

    /// Codex 桌面壳（ChatGPT.exe）。Claude 桌面与 CLI 同名 `claude.exe`，必须看路径。
    fn is_chatgpt_desktop_name(exe: &str) -> bool {
        let n = exe.to_ascii_lowercase();
        n == "chatgpt.exe" || n == "chatgpt"
    }

    /// WorkBuddy / CodeBuddy 桌面客户端（安装包名 `WorkBuddy.exe`）。
    pub fn is_workbuddy_desktop_name(exe: &str) -> bool {
        let n = exe.to_ascii_lowercase();
        matches!(
            n.as_str(),
            "workbuddy.exe" | "workbuddy" | "codebuddy.exe" | "codebuddy"
        )
    }

    /// 商店版 Claude 桌面：`...\WindowsApps\Claude_*\app\Claude.exe`。
    /// 排除 npm CLI 与桌面内嵌的 `Claude-3p\claude-code\...\claude.exe`。
    pub fn is_claude_desktop_path(path: &str) -> bool {
        let p = path.to_ascii_lowercase().replace('/', "\\");
        p.contains("\\windowsapps\\claude") && p.ends_with("\\claude.exe")
    }

    fn desktop_host_exe_for_pid(pid: u32, exe: &str) -> Option<String> {
        if is_chatgpt_desktop_name(exe) {
            return Some("ChatGPT.exe".into());
        }
        if is_workbuddy_desktop_name(exe) {
            return Some("WorkBuddy.exe".into());
        }
        let n = exe.to_ascii_lowercase();
        if n == "claude.exe" || n == "claude" {
            if process_image_path(pid).is_some_and(|p| is_claude_desktop_path(&p)) {
                return Some("Claude.exe".into());
            }
        }
        None
    }

    /// 只对可能有控制台的进程 AttachConsole；对 ChatGPT.exe 等 GUI 调无意义且可能拖死。
    fn may_attach_console(exe: &str) -> bool {
        let n = exe.to_ascii_lowercase();
        is_terminal_host(&n)
            || n == "node.exe"
            || n == "claude.exe"
            || n == "codex.exe"
            || n == "python.exe"
    }

    pub fn detect_host_kind() -> crate::focus::HostKind {
        let names = process_names();
        let self_pid = unsafe { GetCurrentProcessId() };
        if let Some(name) = names.get(&self_pid) {
            if let Some(exe) = desktop_host_exe_for_pid(self_pid, name) {
                return crate::focus::HostKind::Desktop { exe };
            }
        }
        let mut saw_terminal = false;
        for pid in ancestor_chain() {
            let Some(name) = names.get(&pid) else {
                continue;
            };
            if let Some(exe) = desktop_host_exe_for_pid(pid, name) {
                return crate::focus::HostKind::Desktop { exe };
            }
            if is_terminal_host(name) {
                saw_terminal = true;
            }
        }
        if saw_terminal {
            crate::focus::HostKind::Cli
        } else {
            crate::focus::HostKind::Unknown
        }
    }

    pub fn raise_by_exe(exe_name: &str) -> Result<(), String> {
        let want = exe_name.to_ascii_lowercase();
        let want = want.strip_suffix(".exe").unwrap_or(&want);
        let names = process_names();
        let mut pids: Vec<u32> = names
            .iter()
            .filter(|(pid, name)| {
                let n = name.to_ascii_lowercase();
                let stem = n.strip_suffix(".exe").unwrap_or(&n);
                if !(stem == want || n == format!("{want}.exe")) {
                    return false;
                }
                // Claude.exe 同名 CLI/内嵌 runtime：只抬商店桌面实例。
                if want == "claude" {
                    return process_image_path(**pid).is_some_and(|p| is_claude_desktop_path(&p));
                }
                true
            })
            .map(|(pid, _)| *pid)
            .collect();
        if pids.is_empty() {
            return Err(format!("没有运行中的进程 {exe_name}"));
        }
        pids.sort_unstable();
        // 优先有可见主窗的实例。
        if let Some(hwnd) = pick_window_for_pids(&pids, None) {
            if raise_hwnd(hwnd) {
                return Ok(());
            }
            return Err(format!("SetForegroundWindow 失败 exe={exe_name} hwnd={hwnd}"));
        }
        // 允许 cloak/无严格可见性：再扫一遍顶层有标题的窗。
        let windows = windows_for_pids(&pids);
        if let Some((hwnd, _)) = windows.into_iter().next() {
            if raise_hwnd(hwnd) {
                return Ok(());
            }
            return Err(format!("SetForegroundWindow 失败 exe={exe_name} hwnd={hwnd}"));
        }
        Err(format!("进程 {exe_name} 没有可激活的顶层窗"))
    }

    fn title_matches(title: &str, hint: &str) -> bool {
        title
            .to_ascii_lowercase()
            .contains(&hint.to_ascii_lowercase())
    }

    /// hook 进程内解析。
    ///
    /// 有控制台时**不** AttachConsole（会拆掉 Agent 控制台）。
    /// 无控制台时（Codex app-server-daemon 跑 hook）可以安全 AttachConsole 到真正的 CLI。
    pub fn resolve_host_hwnd(title_hint: Option<&str>) -> Option<isize> {
        let self_pid = unsafe { GetCurrentProcessId() };
        let chain = ancestor_chain();
        let names = process_names();
        let mut tree: HashSet<u32> = chain.iter().copied().collect();
        tree.insert(self_pid);
        // 「从文件夹打开终端」会把 explorer 挂进祖先链；绝不能把文件夹窗当宿主。
        let tree = exclude_explorer_pids(&tree, &names);

        // 1) 当前进程控制台：本会话最准；优先于前台窗，避免误抓 Explorer / 隔壁 Agent。
        let console = unsafe { GetConsoleWindow() };
        if let Some(hwnd) = elevate_console_hwnd(console, title_hint) {
            return Some(hwnd);
        }

        // 2) 前台窗：必须属于本进程树（已剔 explorer）；拒绝路径 / 外品牌。
        let fg = unsafe { GetForegroundWindow() };
        if !fg.is_null() {
            let mut pid = 0u32;
            unsafe {
                GetWindowThreadProcessId(fg, &mut pid);
            }
            if pid != 0 && tree.contains(&pid) && is_top_level_visible(fg) {
                let title = window_title(fg);
                if foreground_title_acceptable(&title, title_hint) {
                    return Some(fg as isize);
                }
            }
        }

        // 3) 终端宿主进程上的顶层窗（Windows Terminal / cmd / …）。
        let host_pids = terminal_host_pids_from_chain(&chain);
        if let Some(hwnd) = pick_window_for_pids(&host_pids, title_hint) {
            return Some(hwnd);
        }
        if let Some(hwnd) = pick_session_window(&host_pids, title_hint) {
            return Some(hwnd);
        }
        let mut self_and_chain = chain.clone();
        self_and_chain.push(self_pid);
        if let Some(hwnd) = pick_session_window(&self_and_chain, title_hint) {
            return Some(hwnd);
        }

        // 4) 无控制台 + 祖先无终端（Codex daemon hook）：AttachConsole 到活着的 CLI。
        if console.is_null() {
            let cli = live_agent_cli_pids(title_hint);
            if let Some(hwnd) = hwnd_from_pid_chain(&cli, title_hint, true) {
                return Some(hwnd);
            }
        }

        // 5) 最后：全机终端宿主，按品牌 / 中性标题（拒外品牌）。
        let all_hosts = all_terminal_host_pids();
        pick_session_window(&all_hosts, title_hint)
            .or_else(|| pick_best_agent_window(&all_hosts, title_hint))
    }

    /// 前台窗标题能否作为本 hook 的宿主。
    /// 拒绝路径 / 噪声 / 外品牌；本品牌或中性自定义标题（`aa66621321`）可通过。
    pub fn foreground_title_acceptable(title: &str, title_hint: Option<&str>) -> bool {
        if looks_like_filesystem_path(title) || looks_like_noise_title(title) {
            return false;
        }
        match title_hint.map(str::trim).filter(|s| !s.is_empty()) {
            Some(hint) => {
                title_matches(title, hint) || !is_foreign_agent_title(title, Some(hint))
            }
            None => true,
        }
    }

    /// 标题是否明显属于「别的」Agent。中性自定义标题（无品牌字样）返回 false。
    pub fn is_foreign_agent_title(title: &str, title_hint: Option<&str>) -> bool {
        let t = title.to_ascii_lowercase();
        let hint = title_hint
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_default();
        let is_claude = t.contains("claude") || t.contains('✳');
        let is_codex = t.contains("codex");
        let is_workbuddy = t.contains("workbuddy");
        if hint.contains("claude") {
            return is_codex || is_workbuddy;
        }
        if hint.contains("codex") || hint.contains("chatgpt") {
            return is_claude || is_workbuddy;
        }
        if hint.contains("workbuddy") {
            return is_claude || is_codex;
        }
        false
    }

    pub fn chain_lacks_terminal_host(chain: &[u32]) -> bool {
        terminal_host_pids_from_chain(chain).is_empty()
    }

    fn process_image_path(pid: u32) -> Option<String> {
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return None;
            }
            let mut buf = vec![0u16; 1024];
            let mut size = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut size);
            let _ = CloseHandle(handle);
            if ok == 0 || size == 0 {
                return None;
            }
            Some(String::from_utf16_lossy(&buf[..size as usize]))
        }
    }

    pub fn is_codex_daemon_path(path: &str) -> bool {
        let p = path.to_ascii_lowercase().replace('/', "\\");
        p.contains("app-server-daemon")
            || p.contains("\\codex\\packages\\app-server")
            || p.contains("managed-daemon")
    }

    /// 活着的、带控制台的 Agent CLI（排除 Codex app-server-daemon）。
    /// 顺序：shell → node → agent，方便 AttachConsole。
    pub fn live_agent_cli_pids(title_hint: Option<&str>) -> Vec<u32> {
        let hint = title_hint
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_default();
        let want_codex = hint.is_empty()
            || hint.contains("codex")
            || hint.contains("chatgpt");
        let want_claude = hint.is_empty() || hint.contains("claude");
        let names = process_names();
        let parents = process_parents();
        let mut agents: Vec<u32> = Vec::new();

        for (&pid, name) in &names {
            let n = name.to_ascii_lowercase();
            if want_codex && n == "codex.exe" {
                match process_image_path(pid) {
                    Some(path) if is_codex_daemon_path(&path) => continue,
                    Some(_) => agents.push(pid),
                    None => {}
                }
            }
            if want_claude && n == "claude.exe" {
                agents.push(pid);
            }
        }

        let mut out: Vec<u32> = Vec::new();
        for &agent in &agents {
            // grandparent shell (cmd ← node ← codex)
            if let Some(&parent) = parents.get(&agent) {
                if let Some(&gp) = parents.get(&parent) {
                    if names
                        .get(&gp)
                        .map(|n| is_terminal_host(n))
                        .unwrap_or(false)
                        && !out.contains(&gp)
                    {
                        out.push(gp);
                    }
                }
                if names
                    .get(&parent)
                    .map(|n| {
                        is_terminal_host(n) || n.eq_ignore_ascii_case("node.exe")
                    })
                    .unwrap_or(false)
                    && !out.contains(&parent)
                {
                    out.push(parent);
                }
            }
            if !out.contains(&agent) {
                out.push(agent);
            }
        }
        out
    }

    fn is_codex_daemon_pid(pid: u32) -> bool {
        process_image_path(pid).is_some_and(|p| is_codex_daemon_path(&p))
    }

    fn exclude_explorer_pids(pids: &HashSet<u32>, names: &HashMap<u32, String>) -> HashSet<u32> {
        pids.iter()
            .copied()
            .filter(|pid| {
                names
                    .get(pid)
                    .map(|n| !n.eq_ignore_ascii_case("explorer.exe"))
                    .unwrap_or(true)
            })
            .collect()
    }

    fn terminal_host_pids_from_chain(chain: &[u32]) -> Vec<u32> {
        let names = process_names();
        let parents = process_parents();
        let mut host_pids: Vec<u32> = chain
            .iter()
            .copied()
            .filter(|pid| names.get(pid).map(|n| is_terminal_host(n)).unwrap_or(false))
            .collect();
        for &pid in chain {
            let mut walk = pid;
            for _ in 0..4 {
                let Some(parent) = parents.get(&walk).copied() else {
                    break;
                };
                if parent == 0 || parent == walk {
                    break;
                }
                if names
                    .get(&parent)
                    .map(|n| is_terminal_host(n))
                    .unwrap_or(false)
                    && !host_pids.contains(&parent)
                {
                    host_pids.push(parent);
                }
                walk = parent;
            }
        }
        host_pids
    }

    /// 系统里所有终端宿主进程（父链断裂时仍能靠标题找回 WT 窗）。
    fn all_terminal_host_pids() -> Vec<u32> {
        process_names()
            .into_iter()
            .filter(|(_, name)| is_terminal_host(name))
            .map(|(pid, _)| pid)
            .collect()
    }

    // Codex 等输入时标题常为 `[ ! ] Action Required | <cwd>`，不是品牌名 "Codex"。
    const ATTENTION_TITLE: &str = "Action Required";

    /// 无控制台 helper：先 AttachConsole（正规接口），再标题枚举兜底。
    ///
    /// 重要：指定了品牌 `title_hint`（如 Codex）时，**禁止**回退到其它品牌
    /// （Claude Code），否则会误激活隔壁 Agent 的终端窗。
    pub fn hwnd_from_pid_chain(
        pids: &[u32],
        title_hint: Option<&str>,
        allow_attach_console: bool,
    ) -> Option<isize> {
        let host_pids = terminal_host_pids_from_chain(pids);
        let all_hosts = all_terminal_host_pids();
        // explorer 出现在「从文件夹打开终端」的祖先链里，绝不能当候选窗。
        let safe_pids = exclude_explorer(pids);

        // 1) AttachConsole → GetConsoleWindow → 提升到 WT owner（会话级，最准确）
        //    仅 helper / 无控制台 hook 允许；跳过 app-server-daemon 与 WT 本体（易挂错）。
        if allow_attach_console {
            let mut attach_candidates: Vec<u32> = safe_pids
                .iter()
                .copied()
                .filter(|pid| !is_codex_daemon_pid(*pid))
                .collect();
            // daemon 祖先链常已空心：补上活着的 CLI。
            for pid in live_agent_cli_pids(title_hint) {
                if !attach_candidates.contains(&pid) {
                    attach_candidates.push(pid);
                }
            }
            // shell/node/cli 优先，WindowsTerminal 放最后。
            attach_candidates.sort_by_key(|pid| attach_priority(*pid));
            let names = process_names();
            for pid in attach_candidates {
                let allow = names
                    .get(&pid)
                    .map(|n| may_attach_console(n))
                    .unwrap_or(false);
                if !allow {
                    continue;
                }
                unsafe {
                    let _ = FreeConsole();
                    if AttachConsole(pid) != 0 {
                        let window = GetConsoleWindow();
                        let _ = FreeConsole();
                        if let Some(hwnd) = elevate_console_hwnd(window, title_hint) {
                            return Some(hwnd);
                        }
                    }
                }
            }
        }

        // 2) 标题消歧：只用「调用方给的 hint」+ 通用的 Action Required。
        let mut hints: Vec<&str> = Vec::new();
        if let Some(hint) = title_hint.map(str::trim).filter(|s| !s.is_empty()) {
            hints.push(hint);
        }
        hints.push(ATTENTION_TITLE);

        for hint in &hints {
            if let Some(hwnd) = pick_window_for_pids(&host_pids, Some(hint)) {
                return Some(hwnd);
            }
            if let Some(hwnd) = pick_window_for_pids(&safe_pids, Some(hint)) {
                return Some(hwnd);
            }
            if title_hint.is_some() {
                if let Some(hwnd) = pick_window_for_pids(&all_hosts, Some(hint)) {
                    return Some(hwnd);
                }
            }
        }

        // 3) 标题已变：会话内回退；daemon 空心链则扫全机终端（拒外品牌）。
        if let Some(hwnd) = pick_session_window(&host_pids, title_hint) {
            return Some(hwnd);
        }
        if let Some(hwnd) = pick_session_window(&safe_pids, title_hint) {
            return Some(hwnd);
        }
        if title_hint.is_some() || host_pids.is_empty() {
            if let Some(hwnd) = pick_session_window(&all_hosts, title_hint) {
                return Some(hwnd);
            }
            if let Some(hwnd) = pick_best_agent_window(&all_hosts, title_hint) {
                return Some(hwnd);
            }
        } else if let Some(hwnd) = pick_best_agent_window(&host_pids, title_hint) {
            return Some(hwnd);
        }

        None
    }

    /// AttachConsole 优先级：越小越先试。shell > node/cli > WT。
    fn attach_priority(pid: u32) -> u8 {
        let names = process_names();
        let Some(name) = names.get(&pid).map(|n| n.to_ascii_lowercase()) else {
            return 50;
        };
        if name == "cmd.exe"
            || name == "powershell.exe"
            || name == "pwsh.exe"
            || name == "conhost.exe"
        {
            0
        } else if name == "node.exe" || name == "claude.exe" {
            1
        } else if name == "codex.exe" {
            2
        } else if name.contains("windowsterminal") || name == "wt.exe" || name == "openconsole.exe"
        {
            9
        } else {
            5
        }
    }

    /// 本会话弱回退：拒路径 / 外品牌；允中性自定义标题；多中性窗时跟前台。
    fn pick_session_window(pids: &[u32], title_hint: Option<&str>) -> Option<isize> {
        let windows = windows_for_pids(pids);
        if windows.is_empty() {
            return None;
        }
        if windows.len() == 1 {
            let (_, ref title) = windows[0];
            if looks_like_filesystem_path(title) || is_foreign_agent_title(title, title_hint) {
                return None;
            }
            return Some(windows[0].0);
        }
        let usable: Vec<(isize, String)> = windows
            .into_iter()
            .filter(|(_, title)| {
                !looks_like_filesystem_path(title)
                    && !looks_like_noise_title(title)
                    && !is_foreign_agent_title(title, title_hint)
            })
            .collect();
        if usable.is_empty() {
            return None;
        }
        if usable.len() == 1 {
            return Some(usable[0].0);
        }
        if let Some(hint) = title_hint.map(str::trim).filter(|s| !s.is_empty()) {
            let hint_l = hint.to_ascii_lowercase();
            if let Some((hwnd, _)) = usable
                .iter()
                .find(|(_, title)| title.to_ascii_lowercase().contains(&hint_l))
            {
                return Some(*hwnd);
            }
        }
        for key in brand_fallback_keys(title_hint) {
            if let Some((hwnd, _)) = usable
                .iter()
                .find(|(_, title)| title.to_ascii_lowercase().contains(key))
            {
                return Some(*hwnd);
            }
        }
        let fg = unsafe { GetForegroundWindow() } as isize;
        if let Some((hwnd, _)) = usable.iter().find(|(h, _)| *h == fg) {
            return Some(*hwnd);
        }
        None
    }

    /// 多窗消歧的标题关键词顺序：本品牌优先，再 Action Required，最后才扫其它品牌。
    pub fn brand_fallback_keys(title_hint: Option<&str>) -> Vec<&'static str> {
        let hint = title_hint.map(|s| s.to_ascii_lowercase()).unwrap_or_default();
        if hint.contains("claude") {
            vec!["✳ claude", "claude code", "claude", "action required"]
        } else if hint.contains("codex") || hint.contains("chatgpt") {
            vec!["codex", "action required"]
        } else if hint.contains("workbuddy") {
            vec!["workbuddy", "action required"]
        } else {
            // 无品牌：Action Required 优先，再按出现顺序扫，避免总偏向 Claude。
            vec!["action required", "codex", "✳ claude", "claude code"]
        }
    }

    fn exclude_explorer(pids: &[u32]) -> Vec<u32> {
        let names = process_names();
        pids.iter()
            .copied()
            .filter(|pid| {
                names
                    .get(pid)
                    .map(|n| !n.eq_ignore_ascii_case("explorer.exe"))
                    .unwrap_or(true)
            })
            .collect()
    }

    pub fn looks_like_filesystem_path(title: &str) -> bool {
        let t = title.trim();
        // `C:\...` / `D:/...` / `\\server\share`
        if t.len() >= 3 {
            let bytes = t.as_bytes();
            if bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && (bytes[2] == b'\\' || bytes[2] == b'/')
            {
                return true;
            }
        }
        t.starts_with("\\\\") || t.contains(".cargo\\bin") || t.contains(".cargo/bin")
    }

    fn pick_best_agent_window(host_pids: &[u32], title_hint: Option<&str>) -> Option<isize> {
        let hint = title_hint
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_default();
        let windows = windows_for_pids(host_pids);
        let mut scored: Vec<(isize, i32)> = windows
            .into_iter()
            .filter(|(_, title)| {
                !looks_like_filesystem_path(title)
                    && !is_foreign_agent_title(title, title_hint)
            })
            .map(|(h, title)| {
                let t = title.to_ascii_lowercase();
                let score = if !hint.is_empty() && t.contains(&hint) {
                    0
                } else if t.contains("action required") {
                    1
                } else if hint.contains("codex") || hint.contains("chatgpt") {
                    if t.contains("codex") {
                        0
                    } else {
                        5 // 中性自定义标题，如 aa66621321
                    }
                } else if hint.contains("claude") {
                    if t.contains("claude code") || t.contains('✳') || t.contains("claude") {
                        0
                    } else {
                        5
                    }
                } else if t.contains("codex") {
                    2
                } else if t.contains("claude code") || t.contains('✳') || t.contains("claude") {
                    3
                } else {
                    5
                };
                (h, score)
            })
            .filter(|(_, score)| *score < 10)
            .collect();
        scored.sort_by_key(|(_, s)| *s);
        scored.first().map(|(h, _)| *h)
    }

    /// `GetConsoleWindow` 的结果常被 WT cloak：无标题、有 owner。
    /// AttachConsole 命中的就是本会话控制台——**始终**提升到 owner/parent 框架窗。
    /// 品牌标题只用于 EnumWindows 消歧，不能用来否决 AttachConsole 结果
    /// （Codex CLI 窗口标题经常是目录名 `aa`，不是 "Codex"）。
    fn elevate_console_hwnd(console: HWND, _title_hint: Option<&str>) -> Option<isize> {
        if console.is_null() || unsafe { IsWindow(console) } == 0 {
            return None;
        }
        let frame = unsafe {
            let owner = GetWindow(console, GW_OWNER);
            if !owner.is_null() && IsWindow(owner) != 0 {
                owner
            } else {
                let parent = GetParent(console);
                if !parent.is_null() && IsWindow(parent) != 0 {
                    parent
                } else {
                    console
                }
            }
        };
        Some(frame as isize)
    }

    fn is_top_level_visible(hwnd: HWND) -> bool {
        unsafe {
            if IsWindow(hwnd) == 0 || IsWindowVisible(hwnd) == 0 {
                return false;
            }
            // 只要有主窗体标题的顶层窗；跳过 owner 非空的工具窗。
            if !GetWindow(hwnd, GW_OWNER).is_null() {
                return false;
            }
            GetWindowTextLengthW(hwnd) > 0
        }
    }

    /// 含 DWM cloak 的顶层窗（WT 非活动窗常见 IsWindowVisible=0 但仍有标题）。
    fn is_top_level_frame(hwnd: HWND) -> bool {
        unsafe {
            if IsWindow(hwnd) == 0 {
                return false;
            }
            if !GetWindow(hwnd, GW_OWNER).is_null() {
                return false;
            }
            if !GetParent(hwnd).is_null() {
                return false;
            }
            GetWindowTextLengthW(hwnd) > 0
        }
    }

    fn window_title(hwnd: HWND) -> String {
        unsafe {
            let len = GetWindowTextLengthW(hwnd);
            if len <= 0 {
                return String::new();
            }
            let mut buf = vec![0u16; (len + 1) as usize];
            let copied = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
            if copied <= 0 {
                return String::new();
            }
            String::from_utf16_lossy(&buf[..copied as usize])
        }
    }

    struct EnumState {
        pids: HashSet<u32>,
        found: Vec<(isize, String)>,
    }

    static ENUM_STATE: Mutex<Option<EnumState>> = Mutex::new(None);

    unsafe extern "system" fn enum_callback(hwnd: HWND, _lparam: LPARAM) -> BOOL {
        unsafe {
            if let Ok(mut guard) = ENUM_STATE.lock() {
                if let Some(state) = guard.as_mut() {
                    // 标题搜索需要含 cloaked 窗；仅可见性过滤会漏掉 ✳ Claude Code。
                    if !is_top_level_frame(hwnd) {
                        return TRUE;
                    }
                    let mut pid = 0u32;
                    GetWindowThreadProcessId(hwnd, &mut pid);
                    if state.pids.contains(&pid) {
                        state.found.push((hwnd as isize, window_title(hwnd)));
                    }
                }
            }
            TRUE
        }
    }

    fn windows_for_pids(pids: &[u32]) -> Vec<(isize, String)> {
        if pids.is_empty() {
            return Vec::new();
        }
        let state = EnumState {
            pids: pids.iter().copied().collect(),
            found: Vec::new(),
        };
        {
            let mut guard = ENUM_STATE.lock().expect("enum state");
            *guard = Some(state);
        }
        unsafe {
            let _ = EnumWindows(Some(enum_callback), 0);
        }
        let mut guard = ENUM_STATE.lock().expect("enum state");
        guard.take().map(|s| s.found).unwrap_or_default()
    }

    fn pick_window_for_pids(pids: &[u32], title_hint: Option<&str>) -> Option<isize> {
        let windows = windows_for_pids(pids);
        if windows.is_empty() {
            return None;
        }

        if let Some(hint) = title_hint.map(str::trim).filter(|s| !s.is_empty()) {
            let hint_l = hint.to_ascii_lowercase();
            let mut matched: Vec<(isize, String, i32)> = windows
                .iter()
                .filter(|(_, title)| title.to_ascii_lowercase().contains(&hint_l))
                .map(|(h, title)| {
                    let t = title.to_ascii_lowercase();
                    let score = if t == hint_l {
                        0
                    } else if t.ends_with(&hint_l) || t.starts_with(&hint_l) {
                        1
                    } else {
                        2
                    };
                    (*h, title.clone(), score)
                })
                .collect();
            if matched.is_empty() {
                // 有提示却匹配不到：宁可失败，也不要激活隔壁文件夹/cmd 窗。
                return None;
            }
            matched.sort_by_key(|(_, _, score)| *score);
            return Some(matched[0].0);
        }

        if windows.len() == 1 {
            return Some(windows[0].0);
        }

        let fg = unsafe { GetForegroundWindow() } as isize;
        if let Some((hwnd, _)) = windows.iter().find(|(h, _)| *h == fg) {
            return Some(*hwnd);
        }

        // 无标题提示时的弱兜底（仅 Agent 常见标题，避免路径型标题；不偏向 Claude）。
        for key in brand_fallback_keys(None) {
            if let Some((hwnd, _)) = windows
                .iter()
                .find(|(_, title)| title.to_ascii_lowercase().contains(key))
            {
                return Some(*hwnd);
            }
        }

        None
    }

    pub fn raise_hwnd(hwnd: isize) -> bool {
        raise_hwnd_checked(hwnd, None)
    }

    pub fn raise_hwnd_checked(hwnd: isize, title_hint: Option<&str>) -> bool {
        if hwnd == 0 {
            return false;
        }
        let hwnd = hwnd as HWND;
        unsafe {
            if IsWindow(hwnd) == 0 {
                return false;
            }
            if let Some(hint) = title_hint.map(str::trim).filter(|s| !s.is_empty()) {
                let title = window_title(hwnd);
                // cloak 窗标题可能暂时读到空；有提示时空标题不直接否决。
                if !title.is_empty() && !title_matches(&title, hint) {
                    return false;
                }
            }

            ShowWindow(hwnd, SW_SHOW);
            if IsIconic(hwnd) != 0 {
                ShowWindow(hwnd, SW_RESTORE);
            }
            let _ = BringWindowToTop(hwnd);
            // 短暂 TOPMOST 有助于从其它前台应用抢到 Z 序（仍受前台锁约束）。
            let _ = SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
            );
            let _ = SetWindowPos(
                hwnd,
                HWND_NOTOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
            );

            let mut target_pid = 0u32;
            let target_tid = GetWindowThreadProcessId(hwnd, &mut target_pid);
            let fore = GetForegroundWindow();
            let mut fore_pid = 0u32;
            let fore_tid = GetWindowThreadProcessId(fore, &mut fore_pid);
            let current_tid = GetCurrentThreadId();

            let mut attached_fore = false;
            let mut attached_target = false;
            if fore_tid != 0 && fore_tid != current_tid {
                attached_fore = AttachThreadInput(current_tid, fore_tid, TRUE) != 0;
            }
            if target_tid != 0 && target_tid != current_tid && target_tid != fore_tid {
                attached_target = AttachThreadInput(current_tid, target_tid, TRUE) != 0;
            }

            tap_alt();
            let mut raised = SetForegroundWindow(hwnd) != 0;
            if !raised {
                tap_alt();
                raised = SetForegroundWindow(hwnd) != 0;
            }

            if attached_target {
                let _ = AttachThreadInput(current_tid, target_tid, 0);
            }
            if attached_fore {
                let _ = AttachThreadInput(current_tid, fore_tid, 0);
            }

            // 以实际前台为准；cloak 解除后标题可读。
            if GetForegroundWindow() == hwnd {
                return true;
            }
            raised
        }
    }

    fn tap_alt() {
        unsafe {
            let mut down: INPUT = std::mem::zeroed();
            down.r#type = INPUT_KEYBOARD;
            down.Anonymous.ki = KEYBDINPUT {
                wVk: VK_MENU as VIRTUAL_KEY,
                wScan: 0,
                dwFlags: 0,
                time: 0,
                dwExtraInfo: 0,
            };
            let mut up = down;
            up.Anonymous.ki.dwFlags = KEYEVENTF_KEYUP;
            let mut inputs = [down, up];
            let _ = SendInput(
                inputs.len() as u32,
                inputs.as_mut_ptr(),
                std::mem::size_of::<INPUT>() as i32,
            );
        }
    }
}

#[cfg(windows)]
use win::{
    ancestor_chain, chain_lacks_terminal_host, hwnd_from_pid_chain, live_agent_cli_pids,
    raise_hwnd, raise_hwnd_checked, resolve_host_hwnd,
};

#[cfg(any(windows, target_os = "macos"))]
fn url_encode(text: &str) -> String {
    text.chars()
        .map(|ch| match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => ch.to_string(),
            ' ' => "%20".to_string(),
            other => format!("%{:02X}", other as u32 & 0xFF),
        })
        .collect()
}

fn url_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or("");
            if let Ok(value) = u8::from_str_radix(hex, 16) {
                out.push(value);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perform_ignores_plain_deeplinks() {
        assert_eq!(perform("claude://code/continue?session=x"), None);
        assert_eq!(perform("codex://threads/x"), None);
    }

    #[test]
    fn window_target_is_still_understood() {
        assert_eq!(perform("focus://window/0"), Some(false));
        assert_eq!(perform("focus://window/not-a-number"), None);
        assert_eq!(perform("focus://nope/1"), Some(false));
    }

    #[test]
    fn app_target_round_trips() {
        assert_eq!(perform("focus://app/iTerm"), Some(false));
        assert_eq!(perform("focus://app/Visual%20Studio%20Code"), Some(false));
    }

    #[test]
    fn pid_target_parses_title_query() {
        let (pids, title) = split_pid_target("1,2,3?title=Claude%20Code");
        assert_eq!(pids, "1,2,3");
        assert_eq!(title.as_deref(), Some("Claude Code"));
    }

    #[cfg(windows)]
    #[test]
    fn durable_titles_keep_brands_not_cwd_aliases() {
        assert!(win::is_durable_focus_title("✳ Claude Code"));
        assert!(win::is_durable_focus_title("[ ! ] Action Required | aa"));
        assert!(win::is_durable_focus_title("Codex"));
        assert!(!win::is_durable_focus_title("aa1111"));
        assert!(!win::is_durable_focus_title("aa"));
        assert!(!win::is_durable_focus_title("workplace - Cursor"));
    }

    /// 回归：拒 Explorer 路径与跨品牌；Codex 自定义标题 `aa66621321` 必须能过。
    #[cfg(windows)]
    #[test]
    fn foreground_rejects_explorer_path_and_cross_brand() {
        assert!(win::looks_like_filesystem_path(r"C:\Users\aa\.codex"));
        assert!(win::looks_like_filesystem_path(r"D:/workplace/demo"));
        assert!(!win::looks_like_filesystem_path("✳ Claude Code"));
        assert!(!win::looks_like_filesystem_path("aa66621321"));

        assert!(!win::foreground_title_acceptable(
            r"C:\Users\aa\.codex",
            Some("Claude Code")
        ));
        assert!(win::is_foreign_agent_title("✳ Claude Code", Some("Codex")));
        assert!(!win::is_foreign_agent_title("aa66621321", Some("Codex")));
        assert!(!win::foreground_title_acceptable(
            "✳ Claude Code",
            Some("Codex")
        ));
        assert!(win::foreground_title_acceptable(
            "✳ Claude Code",
            Some("Claude Code")
        ));
        assert!(win::foreground_title_acceptable("Codex", Some("Codex")));
        assert!(win::foreground_title_acceptable("aa66621321", Some("Codex")));
        assert!(!win::foreground_title_acceptable(r"C:\Users\aa\.codex", None));
    }

    #[cfg(windows)]
    #[test]
    fn codex_daemon_path_detected() {
        assert!(win::is_codex_daemon_path(
            r"C:\Users\aa\.codex\packages\app-server-daemon\releases\0.166.0\bin\codex.exe"
        ));
        assert!(!win::is_codex_daemon_path(
            r"C:\Users\aa\AppData\Roaming\npm\node_modules\@openai\codex\vendor\codex.exe"
        ));
    }

    #[cfg(windows)]
    #[test]
    fn claude_desktop_path_not_confused_with_cli() {
        assert!(win::is_claude_desktop_path(
            r"C:\Program Files\WindowsApps\Claude_4.4212.0.0_x64__pzs8sxrjxfjjc\app\Claude.exe"
        ));
        assert!(!win::is_claude_desktop_path(
            r"C:\Users\aa\AppData\Roaming\npm\node_modules\@anthropic-ai\claude-code\bin\claude.exe"
        ));
        assert!(!win::is_claude_desktop_path(
            r"C:\Users\aa\AppData\Local\Claude-3p\claude-code\2.1.309\9078b9c1d466\claude.exe"
        ));
    }

    #[cfg(windows)]
    #[test]
    fn workbuddy_desktop_name_recognized() {
        assert!(win::is_workbuddy_desktop_name("WorkBuddy.exe"));
        assert!(win::is_workbuddy_desktop_name("workbuddy"));
        assert!(win::is_workbuddy_desktop_name("CodeBuddy.exe"));
        assert!(!win::is_workbuddy_desktop_name("Claude.exe"));
        assert!(!win::is_workbuddy_desktop_name("ChatGPT.exe"));
    }

    /// 回归：Codex 消歧不能先扫到 Claude。
    #[cfg(windows)]
    #[test]
    fn brand_fallback_prefers_own_agent() {
        let codex = win::brand_fallback_keys(Some("Codex"));
        assert_eq!(codex.first().copied(), Some("codex"));
        assert!(!codex.iter().any(|k| k.contains("claude")));

        let claude = win::brand_fallback_keys(Some("Claude Code"));
        assert!(claude.first().copied().unwrap_or("").contains("claude"));

        let none = win::brand_fallback_keys(None);
        assert_eq!(none.first().copied(), Some("action required"));
    }

    #[test]
    fn pid_fallback_link_keeps_title_query() {
        let (pids, title) = split_pid_target("38376,6676?title=Codex");
        assert_eq!(pids, "38376,6676");
        assert_eq!(title.as_deref(), Some("Codex"));
    }
}
