//! 审批桥接服务：命名管道进、D50 HID 出。
//!
//! 进程内：审批状态、配置投影、设备工作线程、管道服务器。
//! 会话投影走 `DeviceWorker::sync`（revision 丢弃迟到帧）；图标 / 余额 / 声光走同线程队列上的其它命令，共享 Stop-and-Wait。

pub mod config_proj;
pub mod device;
pub mod state;

use approval_core::{Approval, ApprovalOption, ApprovalRequest, Outcome, SlotView, SubmitOutcome};
use bridge_ipc::{DeviceView, Listener, Request, Response};
use config_core::{BridgeConfig, LinkedAppsError, SoundLightConfig, bind_agent, unbind_agent};
use config_proj::{ConfigState, ProjectionPaths, SharedConfig, default_paths, system_locale};
use d50_hid::{Link, SessionEvent, sim};
use device::DeviceWorker;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{Mutex, Notify, mpsc, oneshot};
use tokio::task::JoinHandle;

pub struct Config {
    pub pipe: String,
    pub simulate: bool,
    pub serial: Option<String>,
    /// 多台候选且都没有序列号时，按枚举序号选择。
    pub device_index: usize,
    /// 无 Report ID 的设备在 Windows hidapi 上通常要补一个 0x00 前缀。
    /// 真机验证前保留开关，免得为了一个字节重新编译。
    pub report_id_prefix: bool,
}

#[derive(Default)]
struct State {
    approval: Approval,
    waiters: HashMap<String, oneshot::Sender<Outcome>>,
    online: bool,
    serial: Option<String>,
    firmware: Option<String>,
    sent_revision: u64,
}

#[derive(Clone)]
struct Ctx {
    state: Arc<Mutex<State>>,
    config: SharedConfig,
    paths: Arc<ProjectionPaths>,
    device: DeviceWorker,
    sim: Option<sim::SimHandle>,
    /// 用量采集在阻塞线程里跑；防止 bind / 定时器 / 上线叠压。
    proj_busy: Arc<std::sync::atomic::AtomicBool>,
    balance_refresh_secs: Arc<AtomicU64>,
    balance_timer_reset: Arc<Notify>,
}

pub struct Bridge {
    pub pipe: String,
    tasks: Vec<JoinHandle<()>>,
}

impl Bridge {
    pub async fn shutdown(self) {
        for task in self.tasks {
            task.abort();
        }
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as u64)
        .unwrap_or(0)
}

pub fn start(config: Config) -> Result<Bridge, String> {
    let listener = Listener::bind(&config.pipe).map_err(|error| error.to_string())?;

    let (link, sim): (Box<dyn Link>, Option<sim::SimHandle>) = if config.simulate {
        let (link, handle) = sim::pair();
        (Box::new(link), Some(handle))
    } else {
        (
            open_hid(
                config.serial.as_deref(),
                config.device_index,
                config.report_id_prefix,
            )?,
            None,
        )
    };

    let (events_tx, mut events_rx) = mpsc::unbounded_channel();
    let reopen = if config.simulate {
        None
    } else {
        let serial = config.serial.clone();
        let device_index = config.device_index;
        let prefix = config.report_id_prefix;
        Some(Box::new(move || {
            open_hid(serial.as_deref(), device_index, prefix)
        }) as device::ReopenFn)
    };
    let device = DeviceWorker::spawn(link, events_tx, reopen);
    let paths = Arc::new(default_paths());
    let bridge_cfg = BridgeConfig::load(&paths.bridge_config);
    let balance_secs = BridgeConfig::clamp_secs(bridge_cfg.balance_refresh_secs);
    let mut config_state = ConfigState::default();
    config_state.sound_light = ConfigState::load_sound_light(&paths.sound_light);
    config_state.balance_refresh_secs = balance_secs;
    // 启动即装载已绑定槽，供 UPSERT_CONVERSATION 引用正确 AgentIndex。
    let (boot_agents, boot_errors) = config_proj::read_projectable(&paths);
    config_state.agents = boot_agents;
    config_state.errors = boot_errors;
    let boot_indices = bound_indices_map(&config_state.agents);
    let mut boot_state = State::default();
    boot_state.approval.set_bound_agent_indices(boot_indices);
    let balance_refresh_secs = Arc::new(AtomicU64::new(balance_secs));
    let balance_timer_reset = Arc::new(Notify::new());
    let ctx = Ctx {
        state: Arc::new(Mutex::new(boot_state)),
        config: Arc::new(Mutex::new(config_state)),
        paths,
        device,
        sim,
        proj_busy: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        balance_refresh_secs,
        balance_timer_reset,
    };

    let device_ctx = ctx.clone();
    let device_task = tokio::spawn(async move {
        while let Some(event) = events_rx.recv().await {
            on_device_event(&device_ctx, event).await;
        }
    });

    let timer_ctx = ctx.clone();
    let timer_task = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_millis(200));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            expire_tick(&timer_ctx).await;
        }
    });

    let proj_ctx = ctx.clone();
    let proj_task = tokio::spawn(async move {
        loop {
            let secs = proj_ctx.balance_refresh_secs.load(Ordering::SeqCst).max(30);
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(secs)) => {
                    kick_balance(&proj_ctx);
                }
                _ = proj_ctx.balance_timer_reset.notified() => {}
            }
        }
    });

    let accept_ctx = ctx.clone();
    let accept_task = tokio::spawn(async move {
        let mut listener = listener;
        loop {
            let connection = listener.accept().await;
            match connection {
                Ok(mut connection) => {
                    let ctx = accept_ctx.clone();
                    tokio::spawn(async move {
                        while let Ok(Some(request)) = connection.next_request().await {
                            let response = handle(&ctx, request).await;
                            if connection.send(&response).await.is_err() {
                                break;
                            }
                        }
                    });
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
            }
        }
    });

    Ok(Bridge {
        pipe: config.pipe,
        tasks: vec![device_task, timer_task, proj_task, accept_task],
    })
}

#[cfg(feature = "hid")]
fn open_hid(
    serial: Option<&str>,
    device_index: usize,
    prefix: bool,
) -> Result<Box<dyn Link>, String> {
    match serial {
        Some(serial) => d50_hid::hid::open(Some(serial), prefix),
        None => d50_hid::hid::open_index(device_index, prefix),
    }
}

#[cfg(not(feature = "hid"))]
fn open_hid(
    _serial: Option<&str>,
    _device_index: usize,
    _prefix: bool,
) -> Result<Box<dyn Link>, String> {
    Err("HID_SUPPORT_DISABLED: 用 --features hid 重新构建，或用 --simulate".into())
}

async fn handle(ctx: &Ctx, request: Request) -> Response {
    match request {
        Request::Hello { protocol, .. } => {
            if protocol != bridge_ipc::PROTOCOL {
                return Response::error(
                    "PROTOCOL_MISMATCH",
                    format!("server speaks {}", bridge_ipc::PROTOCOL),
                );
            }
            Response::Hello {
                protocol: bridge_ipc::PROTOCOL,
                device: device_view(ctx).await,
            }
        }
        Request::ApprovalRequest {
            request_id,
            agent_id,
            session_id,
            tool_name,
            summary,
            cwd,
            options,
            timeout_ms,
            open_app,
        } => {
            approve(
                ctx,
                ApprovalRequest {
                    id: request_id,
                    agent_id,
                    session_id,
                    tool_name,
                    summary,
                    cwd,
                    options: options
                        .into_iter()
                        .map(|option| ApprovalOption {
                            index: option.index,
                            label: option.label,
                            decision: option.decision,
                        })
                        .collect(),
                    deadline_ms: now_ms().saturating_add(timeout_ms),
                    open_app,
                },
                timeout_ms,
            )
            .await
        }
        Request::Notice {
            notice_id,
            agent_id,
            session_id,
            title,
            content,
            link,
            ttl_ms,
        } => {
            let notice = approval_core::Notice {
                id: notice_id,
                agent_id,
                session_id,
                title,
                content,
                link,
                expires_at_ms: now_ms().saturating_add(ttl_ms),
            };
            let now = now_ms();
            let link_for_log = notice.link.clone();
            let id_for_log = notice.id.clone();
            let (outcome, sync) = {
                let mut guard = ctx.state.lock().await;
                let outcome = guard.approval.notice(notice, now);
                (outcome, sync_projection(&mut guard))
            };
            eprintln!(
                "device: notice {id_for_log} shown={} link={link_for_log}",
                matches!(outcome, approval_core::NoticeOutcome::Shown)
            );
            if let Some((revision, slots)) = sync {
                ctx.device.sync(revision, slots);
            }
            Response::Notice {
                shown: matches!(outcome, approval_core::NoticeOutcome::Shown),
            }
        }
        Request::ApprovalStatus { request_id } => {
            let guard = ctx.state.lock().await;
            Response::Status {
                outcome: guard.approval.outcome(&request_id),
            }
        }
        Request::ApprovalCancel { request_id } => {
            let mut guard = ctx.state.lock().await;
            let cancelled = guard.approval.cancel(&request_id);
            let waiter = guard.waiters.remove(&request_id);
            let sync = sync_projection(&mut guard);
            drop(guard);
            if let Some((revision, slots)) = sync {
                ctx.device.sync(revision, slots);
            }
            if let Some(tx) = waiter {
                let _ = tx.send(Outcome::Cancelled);
            }
            if cancelled {
                Response::Ok
            } else {
                Response::error("NOT_PENDING", "request is not pending")
            }
        }
        Request::DeviceStatus => Response::Device {
            device: device_view(ctx).await,
        },
        Request::DeviceSelfTest => self_test(ctx).await,
        Request::DevicePress {
            request_id,
            button_index,
        } => match &ctx.sim {
            Some(handle) => {
                handle.press(&request_id, button_index);
                Response::Ok
            }
            None => Response::error("DEVICE_NOT_SIMULATED", "需要 --simulate 启动"),
        },
        Request::ConfigStatus => {
            let device = device_view(ctx).await;
            let guard = ctx.config.lock().await;
            Response::Config {
                config: config_proj::config_view(&guard, device),
            }
        }
        Request::ConfigBindAgent { id, name } => bind_agent_rpc(ctx, &id, name.as_deref()).await,
        Request::ConfigUnbindAgent { id } => unbind_agent_rpc(ctx, &id).await,
        Request::ConfigSetBalanceInterval { secs } => set_balance_interval(ctx, secs).await,
        Request::ConfigSetSoundLight { config, preview } => {
            set_sound_light(ctx, config, preview).await
        }
        Request::ConfigSetWorkbuddySession {
            cookie,
            chrome_major,
        } => match config_proj::save_workbuddy_session(
            &ctx.paths.workbuddy_session,
            &cookie,
            chrome_major,
        ) {
            Ok(()) => {
                kick_balance(ctx);
                Response::Ok
            }
            Err(err) => Response::error("IO", err),
        },
    }
}

fn linked_apps_error(err: LinkedAppsError) -> Response {
    let code = err.to_string();
    Response::error(&code, code.clone())
}

async fn bind_agent_rpc(ctx: &Ctx, id: &str, name: Option<&str>) -> Response {
    let path = ctx.paths.linked_apps.clone();
    let id_owned = id.to_owned();
    let name_owned = name.map(str::to_owned);
    let bound = match tokio::task::spawn_blocking(move || {
        bind_agent(&path, &id_owned, name_owned.as_deref())
    })
    .await
    {
        Ok(Ok(agent)) => agent,
        Ok(Err(err)) => return linked_apps_error(err),
        Err(err) => return Response::error("INTERNAL", err.to_string()),
    };

    let paths = ctx.paths.clone();
    let locale = system_locale();
    let new_id = bound.id.clone();
    let fetch_agent = bound.clone();
    let (entry, fetch_err) = match tokio::task::spawn_blocking(move || {
        config_proj::fetch_one_balance(&fetch_agent, locale)
    })
    .await
    {
        Ok(v) => v,
        Err(err) => return Response::error("INTERNAL", err.to_string()),
    };

    let (agents, read_errors) = {
        let paths2 = paths.clone();
        match tokio::task::spawn_blocking(move || config_proj::read_projectable(&paths2)).await {
            Ok(v) => v,
            Err(err) => return Response::error("INTERNAL", err.to_string()),
        }
    };

    let online = {
        let guard = ctx.state.lock().await;
        guard.online
    };
    let ts = now_ms();
    {
        let mut cfg = ctx.config.lock().await;
        config_proj::apply_after_bind(
            &mut cfg,
            &ctx.device,
            &paths,
            agents,
            &new_id,
            entry,
            fetch_err,
            online,
            ts,
            locale,
        );
        cfg.errors.extend(read_errors);
    }
    publish_bound_indices(ctx).await;
    let device = device_view(ctx).await;
    let guard = ctx.config.lock().await;
    Response::Config {
        config: config_proj::config_view(&guard, device),
    }
}

async fn unbind_agent_rpc(ctx: &Ctx, id: &str) -> Response {
    let path = ctx.paths.linked_apps.clone();
    let id_owned = id.to_owned();
    match tokio::task::spawn_blocking(move || unbind_agent(&path, &id_owned)).await {
        Ok(Ok(())) => {}
        Ok(Err(err)) => return linked_apps_error(err),
        Err(err) => return Response::error("INTERNAL", err.to_string()),
    }

    let paths = ctx.paths.clone();
    let (agents, _) = {
        let paths2 = paths.clone();
        match tokio::task::spawn_blocking(move || config_proj::read_projectable(&paths2)).await {
            Ok(v) => v,
            Err(err) => return Response::error("INTERNAL", err.to_string()),
        }
    };
    let online = {
        let guard = ctx.state.lock().await;
        guard.online
    };
    let locale = system_locale();
    let ts = now_ms();
    {
        let mut cfg = ctx.config.lock().await;
        config_proj::apply_after_unbind(
            &mut cfg,
            &ctx.device,
            &paths,
            agents,
            id,
            online,
            ts,
            locale,
        );
    }
    publish_bound_indices(ctx).await;
    let device = device_view(ctx).await;
    let guard = ctx.config.lock().await;
    Response::Config {
        config: config_proj::config_view(&guard, device),
    }
}

async fn set_balance_interval(ctx: &Ctx, secs: u64) -> Response {
    let clamped = BridgeConfig::clamp_secs(secs);
    let path = ctx.paths.bridge_config.clone();
    let cfg = BridgeConfig {
        balance_refresh_secs: clamped,
    };
    match tokio::task::spawn_blocking(move || BridgeConfig::save(&path, &cfg)).await {
        Ok(Ok(())) => {}
        Ok(Err(err)) => return linked_apps_error(err),
        Err(err) => return Response::error("INTERNAL", err.to_string()),
    }
    ctx.balance_refresh_secs
        .store(clamped, Ordering::SeqCst);
    {
        let mut guard = ctx.config.lock().await;
        guard.balance_refresh_secs = clamped;
    }
    ctx.balance_timer_reset.notify_waiters();
    let device = device_view(ctx).await;
    let guard = ctx.config.lock().await;
    Response::Config {
        config: config_proj::config_view(&guard, device),
    }
}

/// 全量余额刷新（定时 / 上线 / Cookie）；不下发图标。
fn kick_balance(ctx: &Ctx) {
    if ctx
        .proj_busy
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }
    let ctx = ctx.clone();
    tokio::spawn(async move {
        run_balance_projection(&ctx).await;
        ctx.proj_busy.store(false, Ordering::SeqCst);
    });
}

async fn run_balance_projection(ctx: &Ctx) {
    let online = {
        let guard = ctx.state.lock().await;
        guard.online
    };
    let paths = ctx.paths.clone();
    let locale = system_locale();
    let ts = now_ms();

    let result = tokio::task::spawn_blocking(move || {
        let (agents, mut errors) = config_proj::read_projectable(&paths);
        let (cache, fetch_errors) = config_proj::fetch_all_balances(&agents, locale);
        errors.extend(fetch_errors);
        (agents, cache, errors)
    })
    .await;

    let (agents, cache, errors) = match result {
        Ok(v) => v,
        Err(err) => {
            eprintln!("balance projection join: {err}");
            return;
        }
    };

    {
        let mut cfg = ctx.config.lock().await;
        config_proj::apply_balance_only(
            &mut cfg,
            &ctx.device,
            agents,
            cache,
            errors,
            online,
            ts,
            locale,
        );
    }
    publish_bound_indices(ctx).await;
}

async fn set_sound_light(ctx: &Ctx, value: serde_json::Value, preview: bool) -> Response {
    let mut cfg: SoundLightConfig = match serde_json::from_value(value) {
        Ok(cfg) => cfg,
        Err(err) => return Response::error("INVALID_ARGUMENT", err.to_string()),
    };
    if let Err(err) = cfg.validate() {
        return Response::error("INVALID_ARGUMENT", err.to_string());
    }
    if preview {
        cfg = cfg.with_preview(true, true);
    } else {
        cfg = cfg.with_preview(false, false);
    }
    let payload = match serde_json::to_vec(&cfg) {
        Ok(bytes) if bytes.len() <= 512 => bytes,
        Ok(_) => return Response::error("INVALID_ARGUMENT", "sound light payload > 512"),
        Err(err) => return Response::error("INVALID_ARGUMENT", err.to_string()),
    };
    ctx.device.set_sound_light(payload);
    if !preview {
        if let Err(err) = ConfigState::save_sound_light(&ctx.paths.sound_light, &cfg) {
            return Response::error("IO", err);
        }
        let mut guard = ctx.config.lock().await;
        guard.sound_light = cfg;
    }
    Response::Ok
}

async fn approve(ctx: &Ctx, request: ApprovalRequest, timeout_ms: u64) -> Response {
    if !request.validate() {
        return Response::error(
            "INVALID_ARGUMENT",
            "需要 1–4 个选项，索引在 1–4 之间且不重复；加上 open app 后总数不得超过 4",
        );
    }
    let id = request.id.clone();
    let now = now_ms();

    let (waiter, sync) = {
        let mut guard = ctx.state.lock().await;
        match guard.approval.submit(request, now) {
            SubmitOutcome::Expired => {
                return Response::Approval {
                    outcome: Outcome::Expired,
                };
            }
            SubmitOutcome::Duplicate => {
                // 同一 ID 重试：已经有结果就直接复用，仍在等待就再挂一个等待者。
                if let Some(outcome) = guard.approval.outcome(&id) {
                    return Response::Approval { outcome };
                }
            }
            SubmitOutcome::Shown | SubmitOutcome::Queued => {}
        }
        let (tx, rx) = oneshot::channel();
        guard.waiters.insert(id.clone(), tx);
        (rx, sync_projection(&mut guard))
    };
    if let Some((revision, slots)) = sync {
        ctx.device.sync(revision, slots);
    }

    let waited = tokio::time::timeout(
        Duration::from_millis(timeout_ms.saturating_add(1_500)),
        waiter,
    )
    .await;
    match waited {
        Ok(Ok(outcome)) => Response::Approval { outcome },
        Ok(Err(_)) => Response::error("WAITER_DROPPED", "服务侧等待者被替换"),
        Err(_) => {
            // hook 自己先放弃了：按截止处理，别让这条请求继续占着槽位。
            let mut guard = ctx.state.lock().await;
            guard.waiters.remove(&id);
            let outcome = guard
                .approval
                .expire_id(&id, now_ms())
                .unwrap_or(Outcome::Expired);
            let sync = sync_projection(&mut guard);
            drop(guard);
            if let Some((revision, slots)) = sync {
                ctx.device.sync(revision, slots);
            }
            Response::Approval { outcome }
        }
    }
}

async fn self_test(ctx: &Ctx) -> Response {
    let id = format!("self-test-{}", uuid::Uuid::new_v4());
    let request = ApprovalRequest {
        id,
        agent_id: "self-test".into(),
        session_id: "self-test".into(),
        tool_name: "D50".into(),
        summary: "按任意按钮即可，这次不阻塞任何人".into(),
        cwd: ".".into(),
        options: vec![
            ApprovalOption {
                index: 1,
                label: "同意".into(),
                decision: approval_core::Decision::Allow,
            },
            ApprovalOption {
                index: 2,
                label: "拒绝".into(),
                decision: approval_core::Decision::Deny,
            },
        ],
        deadline_ms: now_ms() + 60_000,
        open_app: None,
    };
    let mut guard = ctx.state.lock().await;
    guard.approval.submit(request, now_ms());
    let sync = sync_projection(&mut guard);
    drop(guard);
    if let Some((revision, slots)) = sync {
        ctx.device.sync(revision, slots);
    }
    Response::Device {
        device: device_view(ctx).await,
    }
}

async fn device_view(ctx: &Ctx) -> DeviceView {
    let guard = ctx.state.lock().await;
    DeviceView {
        online: guard.online,
        serial: guard.serial.clone(),
        firmware: guard.firmware.clone(),
        shown: guard.approval.shown_count(),
        queued: guard.approval.queued_count(),
    }
}

async fn on_device_event(ctx: &Ctx, event: SessionEvent) {
    match event {
        SessionEvent::ProtocolVersion { .. } => {
            let became = {
                let mut guard = ctx.state.lock().await;
                let was = guard.online;
                guard.online = true;
                !was
            };
            if became {
                kick_balance(ctx);
            }
        }
        SessionEvent::DeviceInfo(info) => {
            let became = {
                let mut guard = ctx.state.lock().await;
                let was = guard.online;
                guard.online = true;
                guard.serial = Some(info.serial_number);
                guard.firmware = Some(info.d_version);
                !was
            };
            if became {
                kick_balance(ctx);
            }
        }
        SessionEvent::LinkLost(reason) => {
            eprintln!("device lost: {reason}");
            let (pending, sync) = {
                let mut guard = ctx.state.lock().await;
                guard.online = false;
                let lost = guard.approval.device_lost();
                let mut pending = Vec::new();
                for id in lost {
                    if let Some(tx) = guard.waiters.remove(&id) {
                        pending.push((tx, Outcome::Unavailable));
                    }
                }
                (pending, sync_projection(&mut guard))
            };
            for (tx, outcome) in pending {
                let _ = tx.send(outcome);
            }
            if let Some((revision, slots)) = sync {
                ctx.device.sync(revision, slots);
            }
        }
        SessionEvent::Action(action) => {
            let mut link: Option<String> = None;
            let mut waiter = None;
            let mut sync = None;
            {
                let mut guard = ctx.state.lock().await;
                // 提示会话：唯一的键就是「打开/知道了」，按完这条提示就消失。
                if let Some(target) = guard
                    .approval
                    .notice_link(&action.id)
                    .map(str::to_owned)
                {
                    guard.approval.dismiss_notice(&action.id);
                    link = Some(target);
                    sync = sync_projection(&mut guard);
                } else if action.button_index == 0 {
                    // 0 是「打开会话」的全局动作，不是审批答案。
                    link = guard.approval.open_app_link(&action.id).map(str::to_owned);
                    // 设备按下任何反馈键都会清 NeedsAttention，重新点亮才能继续按。
                    guard.approval.touch();
                    sync = sync_projection(&mut guard);
                    eprintln!("device: open {} (not a decision)", action.id);
                } else {
                    match guard
                        .approval
                        .decide(&action.id, action.button_index, now_ms())
                    {
                        Ok(decision) => {
                            eprintln!(
                                "device: {} button {} -> {decision:?}",
                                action.id, action.button_index
                            );
                        }
                        Err(approval_core::DecideError::OpenApp) => {
                            link = guard.approval.open_app_link(&action.id).map(str::to_owned);
                            guard.approval.touch();
                            sync = sync_projection(&mut guard);
                            eprintln!("device: {} open app", action.id);
                        }
                        Err(error) => {
                            eprintln!(
                                "device: {} button {} ignored ({error:?})",
                                action.id, action.button_index
                            );
                            return;
                        }
                    }
                    if link.is_none() {
                        let outcome = guard.approval.outcome(&action.id);
                        waiter = guard.waiters.remove(&action.id).zip(outcome);
                        sync = sync_projection(&mut guard);
                    }
                }
            }
            if let Some(target) = link {
                if target.is_empty() {
                    eprintln!("device: notice dismissed (no app to open)");
                } else if ctx.sim.is_some() {
                    // 模拟模式没有真实窗口可开，别在测试里弹浏览器。
                    eprintln!("device: open (simulated) {target}");
                } else {
                    // focus/ShellExecute 可能阻塞；放到 blocking 池，并加总超时，
                    // 避免一次卡死的置前拖垮事件循环 / 让 Ctrl+C 像失灵。
                    let target_clone = target.clone();
                    let open = tokio::task::spawn_blocking(move || {
                        if let Some(ok) = bridge_ipc::focus::perform(&target_clone) {
                            eprintln!("device: focus {target_clone} -> {ok}");
                            ok
                        } else {
                            open_link_blocking(&target_clone);
                            true
                        }
                    });
                    match tokio::time::timeout(Duration::from_secs(3), open).await {
                        Ok(Ok(_ok)) => {}
                        Ok(Err(error)) => eprintln!("device: open task join failed: {error}"),
                        Err(_) => eprintln!("device: open timed out (3s) for {target}"),
                    }
                }
            }
            if let Some((tx, outcome)) = waiter {
                let _ = tx.send(outcome);
            }
            if let Some((revision, slots)) = sync {
                ctx.device.sync(revision, slots);
            }
        }
        SessionEvent::Control(_) | SessionEvent::Ignored { .. } => {}
    }
}

async fn expire_tick(ctx: &Ctx) {
    let (pending, sync) = {
        let mut guard = ctx.state.lock().await;
        guard.approval.expire_notices(now_ms());
        let expired = guard.approval.expire(now_ms());
        let mut pending = Vec::new();
        for id in expired {
            if let Some(tx) = guard.waiters.remove(&id) {
                if let Some(outcome) = guard.approval.outcome(&id) {
                    pending.push((tx, outcome));
                }
            }
        }
        (pending, sync_projection(&mut guard))
    };
    for (tx, outcome) in pending {
        let _ = tx.send(outcome);
    }
    if let Some((revision, slots)) = sync {
        ctx.device.sync(revision, slots);
    }
}

/// 投影版本变了才下发；`sent_revision` 保证不会把旧内容写回设备。
fn sync_projection(state: &mut State) -> Option<(u64, Vec<SlotView>)> {
    let revision = state.approval.revision();
    if revision == state.sent_revision {
        return None;
    }
    state.sent_revision = revision;
    Some((revision, state.approval.projection()))
}

fn bound_indices_map(agents: &[config_core::ProjectableAgent]) -> HashMap<String, u8> {
    agents
        .iter()
        .map(|a| (a.id.clone(), a.agent_index))
        .collect()
}

/// 把配置侧绑定槽同步进审批投影（11.11 `AgentIndex` 只引用，不分配）。
async fn publish_bound_indices(ctx: &Ctx) {
    let map = {
        let cfg = ctx.config.lock().await;
        bound_indices_map(&cfg.agents)
    };
    let sync = {
        let mut guard = ctx.state.lock().await;
        guard.approval.set_bound_agent_indices(map);
        sync_projection(&mut guard)
    };
    if let Some((revision, slots)) = sync {
        ctx.device.sync(revision, slots);
    }
}

/// 用系统外壳打开 Agent 的 deeplink。失败只记日志：开不起来不能影响审批。
///
/// 不要 `cmd /c start`：在 Windows Terminal 作默认终端时会闪一下空白
/// `C:\windows\system32\cmd.exe` 窗。直接 `ShellExecuteW` 无控制台。
#[cfg(windows)]
fn open_link_blocking(link: &str) {
    use std::os::windows::ffi::OsStrExt;

    let wide: Vec<u16> = std::ffi::OsStr::new(link)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let open: Vec<u16> = std::ffi::OsStr::new("open")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    // > 32 成功；否则是 SE_ERR_*。不用 windows-sys，避免服务多挂一套依赖。
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn ShellExecuteW(
            hwnd: *mut core::ffi::c_void,
            lp_operation: *const u16,
            lp_file: *const u16,
            lp_parameters: *const u16,
            lp_directory: *const u16,
            n_show_cmd: i32,
        ) -> isize;
    }

    const SW_SHOWNORMAL: i32 = 1;
    let rc = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            open.as_ptr(),
            wide.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if rc as usize > 32 {
        eprintln!("device: opened {link}");
    } else {
        eprintln!("device: open {link} failed (ShellExecuteW={rc})");
    }
}

#[cfg(not(windows))]
fn open_link_blocking(link: &str) {
    eprintln!("device: open {link} unsupported on this platform");
}