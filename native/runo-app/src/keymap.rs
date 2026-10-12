//! Runo's global shortcuts, which can be changed in Settings > Keybindings.

use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A shortcut that can be rebound.
pub(crate) struct Shortcut {
    pub id: &'static str,
    pub title: &'static str,
    pub desc: &'static str,
    /// Keys in GPUI's form; `secondary` is ⌘ on macOS and Ctrl elsewhere.
    pub default: &'static [&'static str],
}

pub(crate) const SHORTCUTS: [Shortcut; 8] = [
    Shortcut {
        id: "quick-open",
        title: "Quick open",
        desc: "Jump to any session, file or command.",
        default: &["secondary-k", "secondary-p"],
    },
    Shortcut {
        id: "new-session",
        title: "New session",
        desc: "Open the New session form.",
        default: &["secondary-n"],
    },
    Shortcut {
        id: "settings",
        title: "Settings",
        desc: "Open this screen.",
        default: &["secondary-,"],
    },
    Shortcut {
        id: "panel",
        title: "Changes panel",
        desc: "Show or hide the Changes, Files and Terminal panel.",
        default: &["secondary-b"],
    },
    Shortcut {
        id: "sidebar",
        title: "Sidebar",
        desc: "Show or hide the sessions sidebar.",
        default: &["secondary-\\"],
    },
    Shortcut {
        id: "stop",
        title: "Stop agent",
        desc: "Stop the agent running in the selected session.",
        default: &["secondary-."],
    },
    Shortcut {
        id: "toggle-view",
        title: "Chat or CLI",
        desc: "Switch the session between Chat and the agent's own CLI.",
        default: &["secondary-shift-m"],
    },
    Shortcut {
        id: "save-file",
        title: "Save file",
        desc: "Save the file open in Runo's editor.",
        default: &["secondary-s"],
    },
];

/// Shortcuts the text boxes and the system already use.
const RESERVED: [(&str, &str); 7] = [
    ("secondary-c", "copy"),
    ("secondary-v", "paste"),
    ("secondary-x", "cut"),
    ("secondary-a", "select all"),
    ("secondary-z", "undo"),
    ("secondary-shift-z", "redo"),
    ("secondary-q", "quit"),
];

/// How many key bindings came before Runo's (gpui-kit's own), so Runo's can be replaced.
static BASE: AtomicUsize = AtomicUsize::new(usize::MAX);

fn binding(id: &str, keys: &str) -> Option<KeyBinding> {
    let context = Some("Runo");
    Some(match id {
        "quick-open" => KeyBinding::new(keys, QuickOpen, context),
        "new-session" => KeyBinding::new(keys, NewSession, context),
        "settings" => KeyBinding::new(keys, OpenSettings, context),
        "panel" => KeyBinding::new(keys, TogglePanel, context),
        "sidebar" => KeyBinding::new(keys, ToggleSidebar, context),
        "stop" => KeyBinding::new(keys, StopAgent, context),
        "toggle-view" => KeyBinding::new(keys, ToggleView, context),
        "save-file" => KeyBinding::new(keys, SaveFile, context),
        _ => return None,
    })
}

/// The keys a shortcut uses now: the person's choice, or the defaults.
pub(crate) fn keys_for(custom: &HashMap<String, String>, id: &str) -> Vec<String> {
    match custom.get(id).filter(|k| !k.is_empty()) {
        Some(keys) => vec![keys.clone()],
        None => SHORTCUTS
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.default.iter().map(|k| k.to_string()).collect())
            .unwrap_or_default(),
    }
}

/// Keys in one spelling per platform, so `secondary-k` and `cmd-k` compare equal on a Mac.
pub(crate) fn normalize(keys: &str) -> String {
    let platform = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    let mut parts: Vec<String> = keys
        .split('-')
        .map(|p| match p {
            "secondary" => platform.to_string(),
            "super" | "win" if cfg!(target_os = "macos") => "cmd".into(),
            "control" => "ctrl".into(),
            "option" => "alt".into(),
            other => other.to_lowercase(),
        })
        .collect();
    // `ctrl--` is the minus key: keep empty parts as the key.
    let key = parts.pop().unwrap_or_default();
    let order = ["ctrl", "alt", "shift", "cmd", "super", "fn"];
    parts.sort_by_key(|p| order.iter().position(|o| o == p).unwrap_or(9));
    parts.dedup();
    parts.push(key);
    parts.join("-")
}

/// What a keystroke is called in a binding, or None for one that can't be a shortcut.
pub(crate) fn from_keystroke(k: &Keystroke) -> Option<String> {
    let key = k.key.to_lowercase();
    if key.is_empty()
        || matches!(
            key.as_str(),
            "shift" | "control" | "alt" | "platform" | "function" | "capslock"
        )
    {
        return None;
    }
    let function_key = key.len() > 1 && key.starts_with('f') && key[1..].parse::<u8>().is_ok();
    let m = &k.modifiers;
    if !(m.control || m.alt || m.platform || function_key) {
        return None;
    }
    let mut parts = Vec::new();
    if m.control {
        parts.push("ctrl");
    }
    if m.alt {
        parts.push("alt");
    }
    if m.shift {
        parts.push("shift");
    }
    if m.platform {
        parts.push(if cfg!(target_os = "macos") {
            "cmd"
        } else {
            "super"
        });
    }
    let mut out = parts.join("-");
    if !out.is_empty() {
        out.push('-');
    }
    out.push_str(&key);
    Some(out)
}

/// Keys as people read them: "⌘ ⇧ K" on a Mac, "Ctrl Shift K" elsewhere.
pub(crate) fn display(keys: &str) -> Vec<String> {
    let mac = cfg!(target_os = "macos");
    let normalized = normalize(keys);
    let mut parts: Vec<&str> = normalized.split('-').collect();
    let key = parts.pop().unwrap_or("");
    let mut out: Vec<String> = parts
        .into_iter()
        .map(|p| {
            match (p, mac) {
                ("cmd", true) => "⌘",
                ("ctrl", true) => "⌃",
                ("alt", true) => "⌥",
                ("shift", true) => "⇧",
                ("ctrl", false) => "Ctrl",
                ("alt", false) => "Alt",
                ("shift", false) => "Shift",
                ("super", false) => "Super",
                (other, _) => other,
            }
            .to_string()
        })
        .collect();
    out.push(match key {
        "" => "-".into(),
        "enter" => "Enter".into(),
        "escape" => "Esc".into(),
        "space" => "Space".into(),
        "tab" => "Tab".into(),
        "backspace" => "Backspace".into(),
        "delete" => "Delete".into(),
        "up" => "↑".into(),
        "down" => "↓".into(),
        "left" => "←".into(),
        "right" => "→".into(),
        k if k.len() > 1 && k.starts_with('f') => k.to_uppercase(),
        k => k.to_uppercase(),
    });
    out
}

/// A shortcut as one short label, such as "⌘K" or "Ctrl+K", for hints.
pub(crate) fn label(cx: &App, id: &str) -> String {
    let keys = keys_for(&cx.global::<AppSettings>().keybindings, id);
    let Some(first) = keys.first() else {
        return String::new();
    };
    let parts = display(first);
    if cfg!(target_os = "macos") {
        parts.concat()
    } else {
        parts.join("+")
    }
}

/// Why `keys` can't be used for shortcut `id`: another shortcut or a reserved key has it.
pub(crate) fn conflict(custom: &HashMap<String, String>, id: &str, keys: &str) -> Option<String> {
    let wanted = normalize(keys);
    if let Some((_, what)) = RESERVED.iter().find(|(k, _)| normalize(k) == wanted) {
        return Some(format!("{} is {what}.", display(keys).join(" ")));
    }
    SHORTCUTS
        .iter()
        .filter(|s| s.id != id)
        .find(|s| {
            keys_for(custom, s.id)
                .iter()
                .any(|k| normalize(k) == wanted)
        })
        .map(|s| format!("{} is already {}.", display(keys).join(" "), s.title))
}

/// Put Runo's shortcuts in the keymap, replacing the ones there.
pub(crate) fn install(cx: &mut App) {
    let custom = cx.global::<AppSettings>().keybindings.clone();
    {
        let keymap = cx.key_bindings();
        let mut keymap = keymap.borrow_mut();
        let total = keymap.bindings().len();
        let base = match BASE.load(Ordering::Relaxed) {
            usize::MAX => {
                BASE.store(total, Ordering::Relaxed);
                total
            }
            base => base.min(total),
        };
        let kept: Vec<KeyBinding> = keymap.bindings().take(base).cloned().collect();
        keymap.clear();
        keymap.add_bindings(kept);
    }
    let mut bindings = Vec::new();
    let mut used = Vec::new();
    for s in SHORTCUTS.iter() {
        for keys in keys_for(&custom, s.id) {
            if let Some(b) = binding(s.id, &keys) {
                bindings.push(b);
                used.push(keys);
            }
        }
    }
    bindings.push(KeyBinding::new("escape", Dismiss, Some("Runo")));
    // Inside a shell, keys belong to the shell: Ctrl-P is history, Escape is vi.
    // Keys typed into the Browser tab's page go to the page.
    bindings.push(KeyBinding::new("escape", NoAction {}, Some("RunoBrowser")));
    bindings.push(KeyBinding::new("escape", NoAction {}, Some("RunoTerminal")));
    for keys in used {
        let n = normalize(&keys);
        if n.split('-').any(|p| p == "ctrl") {
            bindings.push(KeyBinding::new(&keys, NoAction {}, Some("RunoTerminal")));
        }
    }
    cx.bind_keys(bindings);
}

impl RunoApp {
    /// A key pressed while recording a shortcut: use it, refuse it, or stop on Escape.
    pub(crate) fn capture_key(&mut self, keystroke: &Keystroke, cx: &mut Context<Self>) {
        let Some(id) = self.key_capture else {
            return;
        };
        if keystroke.key == "escape" && !keystroke.modifiers.modified() {
            self.key_capture = None;
            self.key_note = None;
            cx.notify();
            return;
        }
        let Some(keys) = from_keystroke(keystroke) else {
            if !matches!(
                keystroke.key.as_str(),
                "shift" | "control" | "alt" | "platform" | "function"
            ) {
                self.key_note = Some(format!(
                    "Use {} or another modifier with the key.",
                    if cfg!(target_os = "macos") {
                        "⌘, ⌃ or ⌥"
                    } else {
                        "Ctrl or Alt"
                    }
                ));
                cx.notify();
            }
            return;
        };
        let custom = cx.global::<AppSettings>().keybindings.clone();
        if let Some(why) = conflict(&custom, id, &keys) {
            self.key_note = Some(format!("{why} Press another shortcut."));
            cx.notify();
            return;
        }
        let default = SHORTCUTS
            .iter()
            .find(|s| s.id == id)
            .and_then(|s| s.default.first())
            .is_some_and(|d| normalize(d) == normalize(&keys));
        settings::update(cx, move |s| {
            if default {
                s.keybindings.remove(id);
            } else {
                s.keybindings.insert(id.to_string(), keys);
            }
        });
        install(cx);
        self.key_capture = None;
        self.key_note = None;
        cx.notify();
    }

    pub(crate) fn reset_shortcut(&mut self, id: Option<&'static str>, cx: &mut Context<Self>) {
        settings::update(cx, move |s| match id {
            Some(id) => {
                s.keybindings.remove(id);
            }
            None => s.keybindings.clear(),
        });
        install(cx);
        self.key_capture = None;
        self.key_note = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn ks(key: &str, control: bool, alt: bool, shift: bool, platform: bool) -> Keystroke {
        Keystroke {
            modifiers: Modifiers {
                control,
                alt,
                shift,
                platform,
                function: false,
            },
            key: key.into(),
            key_char: None,
        }
    }

    #[test]
    fn keystrokes_need_a_modifier() {
        assert_eq!(
            from_keystroke(&ks("k", true, false, true, false)).as_deref(),
            Some("ctrl-shift-k")
        );
        assert_eq!(from_keystroke(&ks("k", false, false, false, false)), None);
        assert_eq!(
            from_keystroke(&ks("f5", false, false, false, false)).as_deref(),
            Some("f5")
        );
        assert_eq!(
            from_keystroke(&ks("shift", false, false, true, false)),
            None
        );
    }

    #[test]
    fn same_keys_compare_equal() {
        let platform = if cfg!(target_os = "macos") {
            "cmd"
        } else {
            "ctrl"
        };
        assert_eq!(
            normalize("secondary-shift-k"),
            format!("{platform}-shift-k")
        );
        assert_eq!(normalize("shift-ctrl-k"), "ctrl-shift-k");
    }

    #[test]
    fn conflicts_name_the_other_shortcut() {
        let mut custom = HashMap::new();
        let why = conflict(&custom, "new-session", "secondary-k").unwrap();
        assert!(why.contains("Quick open"), "{why}");
        assert!(conflict(&custom, "new-session", "secondary-c")
            .unwrap()
            .contains("copy"));
        assert!(conflict(&custom, "new-session", "secondary-shift-j").is_none());
        // Once Quick open moves, its old keys are free.
        custom.insert("quick-open".into(), "secondary-shift-o".into());
        assert!(conflict(&custom, "new-session", "secondary-k").is_none());
        assert!(conflict(&custom, "quick-open", "secondary-shift-o").is_none());
    }

    #[test]
    fn every_shortcut_binds_its_defaults() {
        for s in SHORTCUTS.iter() {
            for keys in s.default {
                assert!(binding(s.id, keys).is_some(), "{}", s.id);
            }
        }
        assert_eq!(display("secondary-k").len(), 2);
    }
}
