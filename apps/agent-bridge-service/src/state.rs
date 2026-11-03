//! 投影与设备 DTO 之间的转换，保持在一处，避免两套字段拼装。

use approval_core::SlotView;
use d50_protocol::{Button, Conversation};

pub fn to_conversation(slot: &SlotView) -> Conversation {
    Conversation {
        id: slot.id.clone(),
        title: slot.title.clone(),
        path: slot.path.clone(),
        content: slot.content.clone(),
        agent_index: slot.agent_index,
        needs_attention: slot.needs_attention,
        buttons: slot
            .buttons
            .iter()
            .map(|(index, name)| Button {
                index: *index,
                name: name.clone(),
            })
            .collect(),
    }
}
