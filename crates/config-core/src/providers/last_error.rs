//! 各 provider 最近一次失败原因（线程局部，投影循环读走）。

use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    static LAST: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
}

pub fn clear(agent: &str) {
    LAST.with(|slot| {
        slot.borrow_mut().remove(agent);
    });
}

pub fn set(agent: &str, msg: impl Into<String>) {
    LAST.with(|slot| {
        slot.borrow_mut().insert(agent.to_owned(), msg.into());
    });
}

pub fn take(agent: &str) -> Option<String> {
    LAST.with(|slot| slot.borrow_mut().remove(agent))
}
