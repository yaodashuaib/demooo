//! 审批状态机：请求表、会话槽分配、投影与截止。
//!
//! 本模块不碰 IO、不碰设备、不认识任何 Agent 的 JSON。它是唯一的事实来源，
//! 设备上显示什么由 [`Approval::projection`] 随时算出来，不从设备回读。
//!
//! 时间一律由调用方传入（epoch 毫秒），因此所有边界都能在测试里精确命中。

use d50_protocol::clip_utf8;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// D50 一次最多显示 6 个会话槽；第 7 个必须排队，直接下发会挤掉最老的一条。
pub const MAX_SHOWN: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Allow,
    Deny,
    /// 「总是允许」：只有 Claude Code 的 `updatedPermissions` 能表达，Codex 不支持，
    /// 所以只会在 Claude 的三选项请求里出现。
    AllowAlways,
}

impl Decision {
    /// 服务与设备只需要知道这一票是不是「放行」；`AllowAlways` 也是放行，
    /// 差别在 hook 端要不要附加 `updatedPermissions`。
    pub fn allows(self) -> bool {
        matches!(self, Decision::Allow | Decision::AllowAlways)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalOption {
    pub index: u8,
    pub label: String,
    pub decision: Decision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub id: String,
    pub agent_id: String,
    pub session_id: String,
    pub tool_name: String,
    pub summary: String,
    pub cwd: String,
    pub options: Vec<ApprovalOption>,
    pub deadline_ms: u64,
    /// 「打开 App」按钮要跳的 deeplink；`None` 表示这个 Agent 没有可跳的会话页。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_app: Option<String>,
}

impl ApprovalRequest {
    /// 结构自检：没有选项、选项索引越界或重复的请求不允许进入状态机。
    pub fn validate(&self) -> bool {
        if self.id.is_empty() || self.id.len() > 64 || self.options.is_empty() {
            return false;
        }
        if self.options.len() > 4 {
            return false;
        }
        let mut seen = [false; 5];
        for option in &self.options {
            if !(1..=4).contains(&option.index) || seen[usize::from(option.index)] {
                return false;
            }
            seen[usize::from(option.index)] = true;
        }
        // 追加的「打开 App」也要占一个键位；加上它超过 4 个就放不下。
        if let Some(index) = self.open_index() {
            if index > 4 || seen[usize::from(index)] {
                return false;
            }
        }
        true
    }

    /// 「打开 App」按钮的索引：排在所有选项之后。没有链接就没有这个按钮。
    pub fn open_index(&self) -> Option<u8> {
        self.open_app.as_ref()?;
        let max = self.options.iter().map(|option| option.index).max()?;
        Some(max + 1)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestState {
    Queued,
    Shown,
    Decided {
        decision: Decision,
        option_index: u8,
        decided_at_ms: u64,
    },
    Expired,
    Cancelled,
    /// 设备掉线导致的中断：没有人做过决定，不能当成拒绝。
    Unavailable,
}

/// 服务对 hook 的唯一答复。`Expired`/`Unavailable`/`Cancelled` 都没有 decision。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Outcome {
    Decided {
        decision: Decision,
        option_index: u8,
        decided_at_ms: u64,
    },
    Expired,
    Unavailable,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubmitOutcome {
    Shown,
    Queued,
    /// 请求到达时已经过了自己的截止时刻；直接拒绝，不占槽位。
    Expired,
    /// 同一个 ID 已经在本进程里出现过；调用方应改为读取既有状态。
    Duplicate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecideError {
    Unknown,
    /// 请求还在排队，设备上没有它的按钮，按键不可能是给它的。
    NotShown,
    UnknownOption,
    /// 按的是「打开 App」：不是裁决，调用方应该去开链接。
    OpenApp,
    AlreadyDecided,
    Expired,
}

/// 设备投影里的一条会话。字段已经按协议上限截断。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotView {
    pub id: String,
    pub title: String,
    pub path: String,
    pub content: String,
    pub agent_index: u8,
    pub needs_attention: u8,
    pub buttons: Vec<(u8, String)>,
}

struct Entry {
    seq: u64,
    request: ApprovalRequest,
    state: RequestState,
}

/// 一条「有事情待你看」的提示：不阻塞任何 hook，只有一个「打开 App」动作。
///
/// 用于 Agent 需要人在自己的界面里回答的场合（`AskUserQuestion`、
/// `request_user_input`、`request_permissions`）：设备负不了这个责，但可以提醒。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notice {
    pub id: String,
    pub agent_id: String,
    pub session_id: String,
    pub title: String,
    pub content: String,
    /// 打开 Agent 会话页的 deeplink。
    pub link: String,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoticeOutcome {
    Shown,
    /// 六个槽位都被占着：提示是尽力而为的，丢一条不影响任何人的等待。
    Dropped,
    Duplicate,
}

#[derive(Default)]
pub struct Approval {
    entries: BTreeMap<String, Entry>,
    notices: BTreeMap<String, (u64, Notice)>,
    next_seq: u64,
    revision: u64,
    /// allowlist id（如 `claude_code`）→ 配置上下文持久化的 `AgentIndex`。
    /// 由 bridge 在绑定集合变化时写入；审批只引用，不分配。
    bound_agent_indices: HashMap<String, u8>,
}

impl Approval {
    pub fn new() -> Self {
        Self::default()
    }

    /// 投影版本；每次可能改变设备内容时递增。
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// 用配置侧已绑定槽位替换查找表；内容变化时抬升 revision，使在途会话换图标。
    pub fn set_bound_agent_indices(&mut self, indices: HashMap<String, u8>) {
        if self.bound_agent_indices == indices {
            return;
        }
        self.bound_agent_indices = indices;
        self.revision += 1;
    }

    pub fn submit(&mut self, request: ApprovalRequest, now_ms: u64) -> SubmitOutcome {
        if self.entries.contains_key(&request.id) {
            return SubmitOutcome::Duplicate;
        }
        if request.deadline_ms <= now_ms {
            return SubmitOutcome::Expired;
        }
        let state = if self.shown_count() < MAX_SHOWN {
            RequestState::Shown
        } else {
            RequestState::Queued
        };
        let shown = matches!(state, RequestState::Shown);
        let seq = self.next_seq;
        self.next_seq += 1;
        self.entries.insert(
            request.id.clone(),
            Entry {
                seq,
                request,
                state,
            },
        );
        self.revision += 1;
        if shown {
            SubmitOutcome::Shown
        } else {
            SubmitOutcome::Queued
        }
    }

    /// 设备按键只对「正在显示」的请求有效；排队中的请求按不到。
    pub fn decide(
        &mut self,
        id: &str,
        button_index: u8,
        now_ms: u64,
    ) -> Result<Decision, DecideError> {
        let entry = self.entries.get(id).ok_or(DecideError::Unknown)?;
        match &entry.state {
            RequestState::Decided { .. } => return Err(DecideError::AlreadyDecided),
            RequestState::Expired => return Err(DecideError::Expired),
            RequestState::Queued | RequestState::Cancelled | RequestState::Unavailable => {
                return Err(DecideError::NotShown);
            }
            RequestState::Shown => {}
        }
        // 「打开 App」不是裁决：既不消费请求，也不改状态。
        if Some(button_index) == entry.request.open_index() {
            return Err(DecideError::OpenApp);
        }
        let option = entry
            .request
            .options
            .iter()
            .find(|option| option.index == button_index)
            .cloned()
            .ok_or(DecideError::UnknownOption)?;
        if let Some(entry) = self.entries.get_mut(id) {
            entry.state = RequestState::Decided {
                decision: option.decision,
                option_index: option.index,
                decided_at_ms: now_ms,
            };
        }
        self.revision += 1;
        self.promote();
        Ok(option.decision)
    }

    /// 到期处理：显示中的和排队中的都会过期。返回本次新过期的 ID。
    pub fn expire(&mut self, now_ms: u64) -> Vec<String> {
        let expired: Vec<String> = self
            .entries
            .iter()
            .filter(|(_, entry)| {
                matches!(entry.state, RequestState::Shown | RequestState::Queued)
                    && entry.request.deadline_ms <= now_ms
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in &expired {
            if let Some(entry) = self.entries.get_mut(id) {
                entry.state = RequestState::Expired;
            }
        }
        if !expired.is_empty() {
            self.revision += 1;
            self.promote();
        }
        expired
    }

    pub fn cancel(&mut self, id: &str) -> bool {
        match self.entries.get_mut(id) {
            Some(entry) if matches!(entry.state, RequestState::Shown | RequestState::Queued) => {
                entry.state = RequestState::Cancelled;
                self.revision += 1;
                self.promote();
                true
            }
            _ => false,
        }
    }

    /// 设备掉线：显示中的请求全部转 `unavailable`，排队中的继续等设备回来。
    pub fn device_lost(&mut self) -> Vec<String> {
        let lost: Vec<String> = self
            .entries
            .iter()
            .filter(|(_, entry)| matches!(entry.state, RequestState::Shown))
            .map(|(id, _)| id.clone())
            .collect();
        for id in &lost {
            if let Some(entry) = self.entries.get_mut(id) {
                entry.state = RequestState::Unavailable;
            }
        }
        if !lost.is_empty() {
            self.revision += 1;
        }
        lost
    }

    pub fn outcome(&self, id: &str) -> Option<Outcome> {
        let entry = self.entries.get(id)?;
        Some(match &entry.state {
            RequestState::Decided {
                decision,
                option_index,
                decided_at_ms,
            } => Outcome::Decided {
                decision: *decision,
                option_index: *option_index,
                decided_at_ms: *decided_at_ms,
            },
            RequestState::Expired => Outcome::Expired,
            RequestState::Cancelled => Outcome::Cancelled,
            RequestState::Unavailable => Outcome::Unavailable,
            RequestState::Queued | RequestState::Shown => return None,
        })
    }

    /// 只让某一条过期：hook 自己的等待先到点时用，不牵连同批次的其它请求。
    pub fn expire_id(&mut self, id: &str, now_ms: u64) -> Option<Outcome> {
        let changed = match self.entries.get_mut(id) {
            Some(entry)
                if matches!(entry.state, RequestState::Shown | RequestState::Queued)
                    && entry.request.deadline_ms <= now_ms =>
            {
                entry.state = RequestState::Expired;
                true
            }
            _ => false,
        };
        if changed {
            self.revision += 1;
            self.promote();
        }
        self.outcome(id)
    }

    /// 终态请求可以清理；保持表有界。
    pub fn forget(&mut self, id: &str) {
        if matches!(
            self.entries.get(id).map(|entry| &entry.state),
            Some(RequestState::Expired | RequestState::Cancelled | RequestState::Unavailable)
        ) {
            self.entries.remove(id);
        }
    }

    pub fn shown_count(&self) -> usize {
        self.entries
            .values()
            .filter(|entry| matches!(entry.state, RequestState::Shown))
            .count()
            + self.notices.len()
    }

    pub fn queued_count(&self) -> usize {
        self.entries
            .values()
            .filter(|entry| matches!(entry.state, RequestState::Queued))
            .count()
    }

    /// 插一条提示。槽位满就丢——它不阻塞任何人，丢掉只损失一次提醒。
    pub fn notice(&mut self, notice: Notice, now_ms: u64) -> NoticeOutcome {
        if self.notices.contains_key(&notice.id) {
            return NoticeOutcome::Duplicate;
        }
        if notice.expires_at_ms <= now_ms || self.shown_count() >= MAX_SHOWN {
            return NoticeOutcome::Dropped;
        }
        let seq = self.next_seq;
        self.next_seq += 1;
        self.notices.insert(notice.id.clone(), (seq, notice));
        self.revision += 1;
        NoticeOutcome::Shown
    }

    pub fn notice_link(&self, id: &str) -> Option<&str> {
        self.notices.get(id).map(|(_, notice)| notice.link.as_str())
    }

    /// 设备按键会清掉会话的 `NeedsAttention`。按的是「打开 App」时请求还挂着，
/// 必须重新点亮一次，否则那颗键就再也按不动了。
    pub fn touch(&mut self) {
        self.revision += 1;
    }

    /// 审批请求自己的「打开 App」链接。
    pub fn open_app_link(&self, id: &str) -> Option<&str> {
        self.entries
            .get(id)
            .and_then(|entry| entry.request.open_app.as_deref())
    }

    pub fn dismiss_notice(&mut self, id: &str) -> bool {
        if self.notices.remove(id).is_some() {
            self.revision += 1;
            true
        } else {
            false
        }
    }

    /// 过期提示。返回本次清掉的 ID。
    pub fn expire_notices(&mut self, now_ms: u64) -> Vec<String> {
        let expired: Vec<String> = self
            .notices
            .iter()
            .filter(|(_, (_, notice))| notice.expires_at_ms <= now_ms)
            .map(|(id, _)| id.clone())
            .collect();
        for id in &expired {
            self.notices.remove(id);
        }
        if !expired.is_empty() {
            self.revision += 1;
        }
        expired
    }

    /// 设备显示顺序：提示在最前（它们最需要人动手），其后是审批；各自新会话在前。
    pub fn projection(&self) -> Vec<SlotView> {
        let bound = &self.bound_agent_indices;
        let mut slots: Vec<(u64, SlotView)> = Vec::new();
        let mut notices: Vec<&(u64, Notice)> = self.notices.values().collect();
        notices.sort_by(|a, b| b.0.cmp(&a.0));
        for (seq, notice) in notices {
            slots.push((*seq, notice_slot(notice, bound)));
        }
        let mut shown: Vec<&Entry> = self
            .entries
            .values()
            .filter(|entry| matches!(entry.state, RequestState::Shown))
            .collect();
        shown.sort_by(|a, b| b.seq.cmp(&a.seq));
        for entry in shown {
            slots.push((entry.seq, slot(entry, bound)));
        }
        slots.truncate(MAX_SHOWN);
        slots.into_iter().map(|(_, slot)| slot).collect()
    }

    /// 释放槽位：按提交顺序把排队的请求提升为显示中。
    fn promote(&mut self) {
        let mut queued: Vec<(u64, String)> = self
            .entries
            .iter()
            .filter(|(_, entry)| matches!(entry.state, RequestState::Queued))
            .map(|(id, entry)| (entry.seq, id.clone()))
            .collect();
        queued.sort();
        for (_, id) in queued {
            if self.shown_count() >= MAX_SHOWN {
                break;
            }
            if let Some(entry) = self.entries.get_mut(&id) {
                entry.state = RequestState::Shown;
            }
        }
    }
}

/// hook `agent_id` → allowlist id；与 `config_core::canonicalize_agent_id` 对齐。
pub fn canonical_agent_id(agent_id: &str) -> Option<&'static str> {
    match agent_id {
        "claude_code" | "claude-code" | "claude" => Some("claude_code"),
        "chatgpt" | "codex" => Some("chatgpt"),
        "kimi" | "kimi-cli" | "kimi_cli" => Some("kimi"),
        "workbuddy" => Some("workbuddy"),
        _ => None,
    }
}

/// 从绑定表解析 `AgentIndex`；未绑定回落 0（设备仍显示槽 0 图标，避免非法索引）。
pub fn agent_index(agent_id: &str, bound: &HashMap<String, u8>) -> u8 {
    canonical_agent_id(agent_id)
        .and_then(|id| bound.get(id).copied())
        .unwrap_or(0)
}

fn slot(entry: &Entry, bound: &HashMap<String, u8>) -> SlotView {
    let request = &entry.request;
    let mut buttons: Vec<(u8, String)> = request
        .options
        .iter()
        .map(|option| (option.index, clip_utf8(&option.label, 32)))
        .collect();
    // 「打开 App」排在所有裁决按钮之后：它不是选项，按了不产生决定。
    if let Some(index) = request.open_index() {
        buttons.push((index, OPEN_APP_LABEL.to_string()));
    }
    SlotView {
        id: request.id.clone(),
        title: clip_utf8(&format!("{} · {}", request.agent_id, request.tool_name), 64),
        path: clip_utf8(&request.cwd, 64),
        content: clip_utf8(request.summary.lines().next().unwrap_or(""), 64),
        agent_index: agent_index(&request.agent_id, bound),
        needs_attention: 1,
        buttons,
    }
}

/// 提示只有一个动作：打开 App。没有 App 可开时退化成「知道了」。
fn notice_slot(notice: &Notice, bound: &HashMap<String, u8>) -> SlotView {
    let action = if notice.link.is_empty() {
        // CLI 里没有可拉起的客户端：给个诚实的「知道了」，别写 open app。
        (1, DISMISS_LABEL.to_string())
    } else {
        (1, OPEN_APP_LABEL.to_string())
    };
    SlotView {
        id: notice.id.clone(),
        title: clip_utf8(&notice.title, 64),
        path: String::new(),
        content: clip_utf8(&notice.content, 64),
        agent_index: agent_index(&notice.agent_id, bound),
        needs_attention: 1,
        buttons: vec![action],
    }
}

/// 「打开 App」按钮的文案。设备上只显示这一行，英文比中文省字节。
pub const OPEN_APP_LABEL: &str = "open app";
/// 没有 App 可开时的按钮文案。
pub const DISMISS_LABEL: &str = "dismiss";

#[cfg(test)]
mod tests {
    use super::*;

    fn request(id: &str, deadline_ms: u64) -> ApprovalRequest {
        ApprovalRequest {
            id: id.into(),
            agent_id: "claude-code".into(),
            session_id: "s1".into(),
            tool_name: "Bash".into(),
            summary: "rm -rf build".into(),
            cwd: "D:/work".into(),
            options: vec![
                ApprovalOption {
                    index: 1,
                    label: "同意".into(),
                    decision: Decision::Allow,
                },
                ApprovalOption {
                    index: 2,
                    label: "拒绝".into(),
                    decision: Decision::Deny,
                },
            ],
            deadline_ms,
            open_app: None,
        }
    }

    #[test]
    fn sixth_shows_seventh_queues() {
        let mut approval = Approval::new();
        for index in 0..6 {
            assert_eq!(
                approval.submit(request(&format!("r{index}"), 10_000), 0),
                SubmitOutcome::Shown
            );
        }
        assert_eq!(
            approval.submit(request("r6", 10_000), 0),
            SubmitOutcome::Queued
        );
        assert_eq!(approval.shown_count(), 6);
        assert_eq!(approval.queued_count(), 1);
        assert_eq!(approval.outcome("r6"), None);
    }

    #[test]
    fn deciding_frees_a_slot_and_promotes_fifo() {
        let mut approval = Approval::new();
        for index in 0..7 {
            approval.submit(request(&format!("r{index}"), 10_000), index as u64);
        }
        assert_eq!(approval.shown_count(), 6);
        approval.decide("r0", 1, 100).unwrap();
        assert_eq!(approval.shown_count(), 6);
        assert!(approval.projection().iter().any(|slot| slot.id == "r6"));
    }

    #[test]
    fn queued_request_cannot_be_decided() {
        let mut approval = Approval::new();
        for index in 0..7 {
            approval.submit(request(&format!("r{index}"), 10_000), index as u64);
        }
        assert_eq!(approval.decide("r6", 1, 100), Err(DecideError::NotShown));
    }

    #[test]
    fn decision_is_recorded_once() {
        let mut approval = Approval::new();
        approval.submit(request("r0", 10_000), 0);
        assert_eq!(approval.decide("r0", 2, 50), Ok(Decision::Deny));
        assert_eq!(
            approval.decide("r0", 1, 60),
            Err(DecideError::AlreadyDecided)
        );
        assert_eq!(
            approval.outcome("r0"),
            Some(Outcome::Decided {
                decision: Decision::Deny,
                option_index: 2,
                decided_at_ms: 50
            })
        );
    }

    #[test]
    fn unknown_button_does_not_change_state() {
        let mut approval = Approval::new();
        approval.submit(request("r0", 10_000), 0);
        assert_eq!(
            approval.decide("r0", 3, 10),
            Err(DecideError::UnknownOption)
        );
        assert_eq!(
            approval.decide("r0", 0, 10),
            Err(DecideError::UnknownOption)
        );
        assert_eq!(approval.outcome("r0"), None);
    }

    #[test]
    fn expiry_covers_shown_and_queued() {
        let mut approval = Approval::new();
        approval.submit(request("r0", 100), 0);
        for index in 1..7 {
            approval.submit(request(&format!("r{index}"), 100), 0);
        }
        assert_eq!(approval.expire(99), Vec::<String>::new());
        let expired = approval.expire(100);
        assert_eq!(expired.len(), 7);
        assert_eq!(approval.outcome("r0"), Some(Outcome::Expired));
        assert_eq!(approval.outcome("r6"), Some(Outcome::Expired));
        assert_eq!(approval.shown_count(), 0);
    }

    #[test]
    fn expiry_after_decision_is_a_no_op() {
        let mut approval = Approval::new();
        approval.submit(request("r0", 100), 0);
        approval.decide("r0", 1, 50).unwrap();
        assert!(approval.expire(200).is_empty());
        assert!(matches!(
            approval.outcome("r0"),
            Some(Outcome::Decided { .. })
        ));
    }

    #[test]
    fn device_lost_only_hits_shown() {
        let mut approval = Approval::new();
        for index in 0..7 {
            approval.submit(request(&format!("r{index}"), 10_000), 0);
        }
        let lost = approval.device_lost();
        assert_eq!(lost.len(), 6);
        assert_eq!(approval.outcome("r0"), Some(Outcome::Unavailable));
        assert_eq!(approval.outcome("r6"), None);
        assert_eq!(approval.queued_count(), 1);
    }

    #[test]
    fn projection_is_newest_first_and_clipped() {
        let mut approval = Approval::new();
        let mut long = request("r0", 10_000);
        long.summary = "x".repeat(200);
        approval.submit(long, 0);
        approval.submit(request("r1", 10_000), 1);
        let slots = approval.projection();
        assert_eq!(slots[0].id, "r1");
        assert_eq!(slots[1].content.len(), 64);
        assert!(slots[1].content.ends_with('…'));
    }

    #[test]
    fn duplicate_id_is_not_replaced() {
        let mut approval = Approval::new();
        approval.submit(request("r0", 10_000), 0);
        let mut other = request("r0", 10_000);
        other.summary = "different".into();
        assert_eq!(approval.submit(other, 1), SubmitOutcome::Duplicate);
        assert_eq!(approval.projection()[0].content, "rm -rf build");
    }

    #[test]
    fn cancel_frees_slot() {
        let mut approval = Approval::new();
        for index in 0..7 {
            approval.submit(request(&format!("r{index}"), 10_000), 0);
        }
        assert!(approval.cancel("r0"));
        assert_eq!(approval.outcome("r0"), Some(Outcome::Cancelled));
        assert!(approval.projection().iter().any(|slot| slot.id == "r6"));
    }

    #[test]
    fn already_expired_submission_is_refused() {
        let mut approval = Approval::new();
        assert_eq!(
            approval.submit(request("r0", 100), 100),
            SubmitOutcome::Expired
        );
        assert_eq!(approval.shown_count(), 0);
    }

    #[test]
    fn invalid_request_shape_is_detectable() {
        let mut bad = request("r0", 1);
        bad.options.clear();
        assert!(!bad.validate());
        let mut dup = request("r0", 1);
        dup.options[1].index = 1;
        assert!(!dup.validate());
    }

    #[test]
    fn open_app_takes_the_next_button_slot() {
        let mut with_link = request("r0", 10_000);
        with_link.open_app = Some("codex://threads/s1".into());
        assert_eq!(with_link.open_index(), Some(3));
        assert!(with_link.validate());

        let mut three = request("r0", 10_000);
        three.options.push(ApprovalOption {
            index: 3,
            label: "总是允许".into(),
            decision: Decision::AllowAlways,
        });
        three.open_app = Some("codex://threads/s1".into());
        assert_eq!(three.open_index(), Some(4));
        assert!(three.validate());

        // 四个选项 + 打开 = 五个键位，设备放不下。
        let mut four = three.clone();
        four.options.push(ApprovalOption {
            index: 4,
            label: "第四项".into(),
            decision: Decision::Allow,
        });
        assert!(!four.validate());
    }

    #[test]
    fn open_app_button_is_not_a_decision() {
        let mut approval = Approval::new();
        let mut with_link = request("r0", 10_000);
        with_link.open_app = Some("codex://threads/s1".into());
        approval.submit(with_link, 0);

        assert_eq!(approval.decide("r0", 3, 10), Err(DecideError::OpenApp));
        assert_eq!(approval.outcome("r0"), None);
        // 打开之后请求还在，仍然可以裁决。
        assert_eq!(approval.decide("r0", 1, 20), Ok(Decision::Allow));
    }

    #[test]
    fn open_app_shows_as_a_button_named_open_app() {
        let mut approval = Approval::new();
        let mut with_link = request("r0", 10_000);
        with_link.open_app = Some("claude://code/continue?session=local_s1".into());
        approval.submit(with_link, 0);
        let slots = approval.projection();
        assert_eq!(slots[0].buttons.last().unwrap().0, 3);
        assert_eq!(slots[0].buttons.last().unwrap().1, OPEN_APP_LABEL);
    }

    #[test]
    fn projection_uses_bound_agent_index_not_hardcoded_slot() {
        // 与真机一致：chatgpt 占 0，claude_code 占 1；hook 仍传 claude-code。
        let mut approval = Approval::new();
        approval.set_bound_agent_indices(HashMap::from([
            ("chatgpt".into(), 0),
            ("claude_code".into(), 1),
        ]));
        approval.submit(request("r-claude", 10_000), 0);
        let slots = approval.projection();
        assert_eq!(slots[0].agent_index, 1);
        let mut codex = request("r-codex", 10_000);
        codex.agent_id = "codex".into();
        approval.submit(codex, 0);
        let slots = approval.projection();
        let codex_slot = slots.iter().find(|s| s.id == "r-codex").unwrap();
        assert_eq!(codex_slot.agent_index, 0);
    }

    #[test]
    fn notice_shows_one_open_button_and_can_be_dismissed() {
        let mut approval = Approval::new();
        let notice = Notice {
            id: "n0".into(),
            agent_id: "claude-code".into(),
            session_id: "s1".into(),
            title: "claude-code · 待回复".into(),
            content: "有 3 个问题待您回复…".into(),
            link: "claude://code/continue?session=local_s1".into(),
            expires_at_ms: 10_000,
        };
        assert_eq!(approval.notice(notice, 0), NoticeOutcome::Shown);
        let slots = approval.projection();
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].buttons, vec![(1, OPEN_APP_LABEL.to_string())]);
        assert_eq!(
            approval.notice_link("n0"),
            Some("claude://code/continue?session=local_s1")
        );
        assert!(approval.dismiss_notice("n0"));
        assert!(approval.projection().is_empty());
    }

    #[test]
    fn notices_expire_and_free_their_slot() {
        let mut approval = Approval::new();
        for index in 0..6 {
            approval.submit(request(&format!("r{index}"), 10_000), 0);
        }
        let notice = Notice {
            id: "n0".into(),
            agent_id: "codex".into(),
            session_id: "s1".into(),
            title: "codex · 待回复".into(),
            content: "有 1 个问题待您回复…".into(),
            link: "codex://threads/s1".into(),
            expires_at_ms: 500,
        };
        // 槽位满了：提示被丢掉，不影响正在等待的 6 条审批。
        assert_eq!(approval.notice(notice.clone(), 0), NoticeOutcome::Dropped);
        approval.decide("r0", 1, 10).unwrap();
        assert_eq!(approval.notice(notice, 20), NoticeOutcome::Shown);
        assert_eq!(approval.projection().len(), 6);
        assert_eq!(approval.expire_notices(499), Vec::<String>::new());
        assert_eq!(approval.expire_notices(500), vec!["n0".to_string()]);
        assert_eq!(approval.projection().len(), 5);
    }
}
