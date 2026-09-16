//! 跨平台主修饰键: Windows/Linux = Ctrl, macOS = Command.

use gpui::{Action, KeyBinding, Modifiers};

#[inline]
#[allow(dead_code)]
pub fn is_primary_mod(m: &Modifiers) -> bool {
    m.secondary() || m.control
}

pub fn primary_mod() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘"
    } else {
        "Ctrl+"
    }
}

pub fn with_mod(name: &str, key: &str) -> String {
    format!("{name} ({}{key})", primary_mod())
}

pub fn bind_primary<A: Action + Clone>(
    rest: &str,
    action: A,
    context: Option<&str>,
) -> [KeyBinding; 2] {
    [
        KeyBinding::new(&format!("secondary-{rest}"), action.clone(), context),
        KeyBinding::new(&format!("ctrl-{rest}"), action, context),
    ]
}
