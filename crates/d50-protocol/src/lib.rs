//! D50 应用层协议：帧、CRC16、多帧与命令 Payload。
//!
//! 本 crate 只做字节，不碰 HID、不碰时间、不碰进程。所有长度、序号与校验
//! 都在这里检查，上层拿到的要么是已校验的 `Frame`，要么是明确的错误。
//! 协议来源：`doc/D50与PC的应用层通信协议.md`（V0.1）。

pub mod commands;
pub mod crc;
pub mod frame;

pub use commands::{
    AgentBalance, BalanceBar, BalanceType, Button, ControlEvent, Conversation, ConversationAction,
    ConversationList, DeviceInfo, HostType, ReturnType, clip_utf8, command, return_type,
    set_agent_balance, set_agent_index, set_sound_light_config,
};
pub use crc::crc16;
pub use frame::{
    Frame, FrameType, MAX_FRAME_LEN, MAX_PAYLOAD_LEN, MIN_FRAME_LEN, ProtocolError, Reassembler,
    SYNC, Scanner, decode, encode, split_payload,
};
