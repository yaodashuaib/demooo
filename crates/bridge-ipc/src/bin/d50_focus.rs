//! 无窗口的辅助进程：把某个宿主终端或桌面客户端窗口带到前台。
//!
//! 只在按键时由服务拉起。自身不带控制台，可安全 `AttachConsole`；若目标在
//! Windows Terminal / ConPTY 下没有控制台 HWND，再按进程树用文档化的
//! `EnumWindows` + `GetWindowThreadProcessId` 定位可见顶层窗。
//!
//! 用法：
//! - `d50-focus --hwnd 12345678`
//! - `d50-focus --pids p1,p2[,…] [--title 标题子串]`
//! - `d50-focus --exe ChatGPT.exe`
//! - `d50-focus focus://window/…` / `focus://pid/…` / `focus://exe/…`

// GUI subsystem：被服务 spawn 时不要闪控制台 / Windows Terminal 空窗。
#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    #[cfg(windows)]
    {
        let args: Vec<String> = std::env::args().skip(1).collect();

        if let Some(hwnd) = flag_value(&args, "--hwnd") {
            match hwnd.parse::<u64>() {
                Ok(hwnd) => match bridge_ipc::focus::raise_hwnd_raw(hwnd) {
                    Ok(()) => {
                        eprintln!("d50-focus: ok hwnd={hwnd}");
                        std::process::exit(0);
                    }
                    Err(error) => {
                        eprintln!("d50-focus: {error}");
                        std::process::exit(2);
                    }
                },
                Err(_) => {
                    eprintln!("d50-focus: 无效 hwnd={hwnd}");
                    std::process::exit(1);
                }
            }
        }

        if let Some(exe) = flag_value(&args, "--exe") {
            match bridge_ipc::focus::raise_exe(&exe) {
                Ok(()) => {
                    eprintln!("d50-focus: ok exe={exe}");
                    std::process::exit(0);
                }
                Err(error) => {
                    eprintln!("d50-focus: {error}");
                    std::process::exit(2);
                }
            }
        }

        if let Some(uri) = args.iter().find(|a| a.starts_with("focus://")) {
            let rest = uri.strip_prefix("focus://").unwrap_or(uri);
            if let Some(exe) = rest.strip_prefix("exe/") {
                let exe = exe.split(['?', '#']).next().unwrap_or(exe);
                match bridge_ipc::focus::raise_exe(exe) {
                    Ok(()) => {
                        eprintln!("d50-focus: ok {uri}");
                        std::process::exit(0);
                    }
                    Err(error) => {
                        eprintln!("d50-focus: {error}");
                        std::process::exit(2);
                    }
                }
            }
            if let Some(hwnd_part) = rest.strip_prefix("window/") {
                let hwnd_str = hwnd_part.split(['?', '#']).next().unwrap_or(hwnd_part);
                match hwnd_str.parse::<u64>() {
                    Ok(hwnd) => match bridge_ipc::focus::raise_hwnd_raw(hwnd) {
                        Ok(()) => {
                            eprintln!("d50-focus: ok {uri}");
                            std::process::exit(0);
                        }
                        Err(error) => {
                            eprintln!("d50-focus: {error}");
                            std::process::exit(2);
                        }
                    },
                    Err(_) => {
                        eprintln!("d50-focus: 无效 {uri}");
                        std::process::exit(1);
                    }
                }
            }
            if let Some(pid_part) = rest.strip_prefix("pid/") {
                let (path, query) = pid_part.split_once('?').unwrap_or((pid_part, ""));
                let title = query
                    .split('&')
                    .find_map(|pair| pair.strip_prefix("title="))
                    .map(|s| {
                        // 简易 percent-decode 交给 raise；原样传可能已编码，helper 用明文 --title 更稳
                        s.replace("%20", " ")
                    });
                let pids: Vec<u32> = path
                    .split(',')
                    .filter_map(|value| value.trim().parse().ok())
                    .collect();
                match bridge_ipc::focus::raise_from_pids(&pids, title.as_deref()) {
                    Ok(()) => {
                        eprintln!("d50-focus: ok {uri}");
                        std::process::exit(0);
                    }
                    Err(error) => {
                        eprintln!("d50-focus: {error}");
                        std::process::exit(2);
                    }
                }
            }
            eprintln!("d50-focus: unsupported {uri}");
            std::process::exit(1);
        }

        let chain = flag_value(&args, "--pids").unwrap_or_default();
        let title = flag_value(&args, "--title");
        let pids: Vec<u32> = chain
            .split(',')
            .filter_map(|value| value.trim().parse().ok())
            .collect();

        if pids.is_empty() {
            eprintln!(
                "d50-focus: 用法:\n  d50-focus --hwnd HWND\n  d50-focus --pids p1,p2[,…] [--title 标题子串]\n  d50-focus --exe ChatGPT.exe\n  d50-focus focus://…"
            );
            std::process::exit(1);
        }

        match bridge_ipc::focus::raise_from_pids(&pids, title.as_deref()) {
            Ok(()) => {
                eprintln!("d50-focus: ok pids={pids:?} title={title:?}");
                std::process::exit(0);
            }
            Err(error) => {
                eprintln!("d50-focus: {error}");
                std::process::exit(2);
            }
        }
    }

    #[cfg(not(windows))]
    {
        eprintln!("d50-focus: 仅 Windows 可用");
        std::process::exit(1);
    }
}

fn flag_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|index| args.get(index + 1))
        .cloned()
}
