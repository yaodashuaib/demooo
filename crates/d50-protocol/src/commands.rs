//! 命令号与 Payload 编解码。
//!
//! 这里只放 demo 用得到的命令：握手、心跳、时间、页面、图标、会话与设备上报。
//! 未用到的命令不定义，避免出现「看起来支持但没人验证过」的接口。

use crate::frame::ProtocolError;
use serde::{Deserialize, Serialize};

pub mod command {
    pub const GET_PROTOCOL_VERSION: u8 = 0x00;
    pub const GET_DEVICE_INFO: u8 = 0x01;
    pub const HEARTBEAT: u8 = 0x02;
    pub const TIME_CONFIG: u8 = 0x03;
    pub const SET_PAGE_CONFIG: u8 = 0x10;
    pub const SET_AGENT_INDEX: u8 = 0x20;
    pub const SET_CONVERSATION_LIST: u8 = 0x21;
    pub const UPSERT_CONVERSATION: u8 = 0x22;
    pub const CONTROL_EVENT: u8 = 0x40;
    pub const CONVERSATION_ACTION: u8 = 0x41;
}

/// 心跳里的上位机类型；D50 同一时间只允许一种上位机连接。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostType {
    Studio = 0x00,
    Server = 0x01,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceInfo {
    #[serde(rename = "SerialNumber")]
    pub serial_number: String,
    #[serde(rename = "Dversion")]
    pub d_version: String,
    #[serde(rename = "DeviceType")]
    pub device_type: String,
    #[serde(rename = "HardwareVersion")]
    pub hardware_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Button {
    #[serde(rename = "Index")]
    pub index: u8,
    #[serde(rename = "Name")]
    pub name: String,
}

/// 设备上的一个会话槽内容。审批就是它的一个投影。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Conversation {
    #[serde(rename = "ID")]
    pub id: String,
    #[serde(rename = "Title")]
    pub title: String,
    #[serde(rename = "Path")]
    pub path: String,
    #[serde(rename = "Content")]
    pub content: String,
    #[serde(rename = "AgentIndex")]
    pub agent_index: u8,
    #[serde(rename = "NeedsAttention")]
    pub needs_attention: u8,
    #[serde(rename = "Buttons")]
    pub buttons: Vec<Button>,
}

impl Conversation {
    /// 在发往设备前自检，避免一次非法投影把整条链路写成半截状态。
    pub fn validate(&self) -> Result<(), ProtocolError> {
        let text_ok = |v: &str, min: usize| {
            let len = v.len();
            len >= min && len <= 64 && !v.contains('\0')
        };
        if !text_ok(&self.id, 1)
            || !text_ok(&self.title, 0)
            || !text_ok(&self.path, 0)
            || !text_ok(&self.content, 0)
        {
            return Err(ProtocolError::Parameter);
        }
        if self.agent_index > 5 || self.needs_attention > 1 || self.buttons.len() > 4 {
            return Err(ProtocolError::Parameter);
        }
        let mut seen = [false; 5];
        for button in &self.buttons {
            if !(1..=4).contains(&button.index) || seen[usize::from(button.index)] {
                return Err(ProtocolError::Parameter);
            }
            if button.name.len() > 32 {
                return Err(ProtocolError::Parameter);
            }
            seen[usize::from(button.index)] = true;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationList {
    #[serde(rename = "Conversations")]
    pub conversations: Vec<Conversation>,
}

/// 设备按下的反馈按钮；`button_index = 0` 是「打开会话」的全局动作，不是审批答案。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationAction {
    #[serde(rename = "ID")]
    pub id: String,
    #[serde(rename = "ButtonIndex")]
    pub button_index: u8,
}

impl ConversationAction {
    pub fn decode(payload: &[u8]) -> Result<Self, ProtocolError> {
        let value: Self = serde_json::from_slice(payload).map_err(|_| ProtocolError::Frame)?;
        if value.id.is_empty() || value.id.len() > 64 || value.button_index > 4 {
            return Err(ProtocolError::Frame);
        }
        Ok(value)
    }
}

/// `CONTROL_EVENT` 的三种上报形态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlEvent {
    Key { index: u8, action: u8 },
    Stick { x: i8, y: i8 },
    Direction { direction: u8, action: u8 },
}

impl ControlEvent {
    pub fn decode(payload: &[u8]) -> Result<Self, ProtocolError> {
        match payload {
            [1, index, action] if *index <= 4 && *action <= 1 => Ok(Self::Key {
                index: *index,
                action: *action,
            }),
            [2, x, y] => Ok(Self::Stick {
                x: *x as i8,
                y: *y as i8,
            }),
            [3, direction, action] if *direction <= 3 && *action <= 1 => Ok(Self::Direction {
                direction: *direction,
                action: *action,
            }),
            _ => Err(ProtocolError::Frame),
        }
    }
}

pub fn heartbeat(host: HostType) -> Vec<u8> {
    vec![host as u8]
}

/// 命令的返回类型，决定发送方要不要等、等什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReturnType {
    None,
    Ack,
    Response,
}

/// 只覆盖 demo 用到的命令；未登记的命令按 ACK 处理，等不到就超时失败，
/// 不会静默当成成功。
pub fn return_type(id: u8) -> ReturnType {
    match id {
        command::HEARTBEAT => ReturnType::None,
        command::GET_PROTOCOL_VERSION | command::GET_DEVICE_INFO => ReturnType::Response,
        _ => ReturnType::Ack,
    }
}

/// 7 字节本地时间，不带时区。年按 uint16 小端。
pub fn time_config(year: u16, month: u8, day: u8, hour: u8, minute: u8, second: u8) -> Vec<u8> {
    let [lo, hi] = year.to_le_bytes();
    vec![lo, hi, month, day, hour, minute, second]
}

pub fn set_page_config(pages: &[&str]) -> Result<Vec<u8>, ProtocolError> {
    #[derive(Serialize)]
    struct Page<'a> {
        #[serde(rename = "PageID")]
        page_id: &'a str,
        #[serde(rename = "Name")]
        name: &'a str,
    }
    let names = [
        ("tasks", "会话队列"),
        ("balance", "余额"),
        ("clock", "时钟"),
        ("schedule_1", "日程"),
        ("schedule_2", "日程2"),
        ("schedule_3", "日程3"),
        ("game", "游戏"),
        ("test", "硬件测试"),
    ];
    let mut list = Vec::new();
    for id in pages {
        let id: &str = id;
        let name = names
            .iter()
            .find(|(known, _)| *known == id)
            .map(|(_, name)| *name)
            .ok_or(ProtocolError::Parameter)?;
        list.push(Page { page_id: id, name });
    }
    if list.is_empty() || list.len() > 8 {
        return Err(ProtocolError::Parameter);
    }
    serde_json::to_vec(&serde_json::json!({ "Pages": list })).map_err(|_| ProtocolError::Parameter)
}

pub fn set_conversation_list(list: &[Conversation]) -> Result<Vec<u8>, ProtocolError> {
    if list.len() > 6 {
        return Err(ProtocolError::Parameter);
    }
    for conversation in list {
        conversation.validate()?;
    }
    serde_json::to_vec(&ConversationList {
        conversations: list.to_vec(),
    })
    .map_err(|_| ProtocolError::Parameter)
}

pub fn upsert_conversation(conversation: &Conversation) -> Result<Vec<u8>, ProtocolError> {
    conversation.validate()?;
    serde_json::to_vec(conversation).map_err(|_| ProtocolError::Parameter)
}

/// 截断到不超过 `max_bytes` 个 UTF-8 字节，并在真的截断时留一个 `…`。
///
/// 设备字段按 UTF-8 字节计数，直接按字节切会把中文切成半个字符，设备上
/// 会显示成乱码或整个字段被丢弃。
pub fn clip_utf8(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_owned();
    }
    let budget = max_bytes.saturating_sub('…'.len_utf8());
    let mut end = 0;
    for (index, ch) in text.char_indices() {
        let next = index + ch.len_utf8();
        if next > budget {
            break;
        }
        end = next;
    }
    let mut out = String::with_capacity(end + '…'.len_utf8());
    out.push_str(&text[..end]);
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_config_matches_documented_example() {
        // 文档示例：2026-08-11 15:30:45 -> EA 07 08 0B 0F 1E 2D
        assert_eq!(
            time_config(2026, 8, 11, 15, 30, 45),
            vec![0xEA, 0x07, 0x08, 0x0B, 0x0F, 0x1E, 0x2D]
        );
    }

    #[test]
    fn heartbeat_carries_host_type() {
        assert_eq!(heartbeat(HostType::Studio), vec![0x00]);
        assert_eq!(heartbeat(HostType::Server), vec![0x01]);
    }

    #[test]
    fn conversation_json_uses_documented_field_names() {
        let conversation = Conversation {
            id: "conv-1".into(),
            title: "Claude Code · Bash".into(),
            path: "D:/work".into(),
            content: "rm -rf build".into(),
            agent_index: 0,
            needs_attention: 1,
            buttons: vec![
                Button {
                    index: 1,
                    name: "同意".into(),
                },
                Button {
                    index: 2,
                    name: "拒绝".into(),
                },
            ],
        };
        let json = upsert_conversation(&conversation).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&json).unwrap();
        assert_eq!(value["ID"], "conv-1");
        assert_eq!(value["NeedsAttention"], 1);
        assert_eq!(value["Buttons"][0]["Index"], 1);
    }

    #[test]
    fn conversation_rejects_duplicate_button_index() {
        let mut conversation = sample();
        conversation.buttons = vec![
            Button {
                index: 1,
                name: "a".into(),
            },
            Button {
                index: 1,
                name: "b".into(),
            },
        ];
        assert_eq!(conversation.validate(), Err(ProtocolError::Parameter));
    }

    #[test]
    fn conversation_rejects_zero_button_index() {
        let mut conversation = sample();
        conversation.buttons = vec![Button {
            index: 0,
            name: "全局".into(),
        }];
        assert_eq!(conversation.validate(), Err(ProtocolError::Parameter));
    }

    #[test]
    fn conversation_list_is_capped_at_six() {
        assert_eq!(
            set_conversation_list(&vec![sample(); 7]),
            Err(ProtocolError::Parameter)
        );
    }

    #[test]
    fn conversation_action_decodes_and_validates() {
        let action = ConversationAction::decode(br#"{"ID":"conv-1","ButtonIndex":2}"#).unwrap();
        assert_eq!(action.button_index, 2);
        assert_eq!(
            ConversationAction::decode(br#"{"ID":"conv-1","ButtonIndex":5}"#),
            Err(ProtocolError::Frame)
        );
        assert_eq!(
            ConversationAction::decode(br#"{"ID":"","ButtonIndex":1}"#),
            Err(ProtocolError::Frame)
        );
        assert_eq!(
            ConversationAction::decode(br#"{"ID":"x","ButtonIndex":1,"Extra":1}"#),
            Err(ProtocolError::Frame)
        );
    }

    #[test]
    fn control_event_decodes_three_shapes() {
        assert_eq!(
            ControlEvent::decode(&[1, 3, 0]),
            Ok(ControlEvent::Key {
                index: 3,
                action: 0
            })
        );
        assert_eq!(
            ControlEvent::decode(&[2, 0x9C, 0x64]),
            Ok(ControlEvent::Stick { x: -100, y: 100 })
        );
        assert_eq!(ControlEvent::decode(&[3, 2, 1]).is_ok(), true);
        assert!(ControlEvent::decode(&[0, 0, 0]).is_err());
        assert!(ControlEvent::decode(&[1, 5, 0]).is_err());
    }

    #[test]
    fn clip_keeps_utf8_boundaries() {
        let text = "批准这个危险的命令";
        let clipped = clip_utf8(text, 10);
        assert!(clipped.len() <= 10);
        assert!(clipped.ends_with('…'));
        assert_eq!(clipped, "批准…");
    }

    #[test]
    fn clip_leaves_short_text_alone() {
        assert_eq!(clip_utf8("ok", 64), "ok");
    }

    #[test]
    fn clip_handles_exact_boundary() {
        assert_eq!(clip_utf8("abcd", 4), "abcd");
    }

    fn sample() -> Conversation {
        Conversation {
            id: "conv-1".into(),
            title: "t".into(),
            path: "p".into(),
            content: "c".into(),
            agent_index: 0,
            needs_attention: 1,
            buttons: Vec::new(),
        }
    }
}
