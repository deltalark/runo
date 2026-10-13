//! Desktop notifications. They go through the system's own notification center for
//! Runo where it has one (a bundled Mac app, Windows, Linux desktops), so clicking one
//! brings Runo forward on the session it is about. A Mac build run outside its app
//! bundle falls back to AppleScript, whose notifications can't open anything.

use gpui_kit::{App, SharedString, SystemNotification};
use std::sync::atomic::{AtomicBool, Ordering};

/// The app's identity, as its bundle names it; Windows shows notifications under it.
const APP_ID: &str = "dev.runo.app";

/// Whether clicks are handled yet. On a Mac this first touch of the notification center
/// may ask for permission, so it waits for the first notification.
static LISTENING: AtomicBool = AtomicBool::new(false);

/// Name Runo to the system, early, before any window opens.
pub(crate) fn install(cx: &mut App) {
    cx.set_app_identity(APP_ID, "Runo");
}

/// Open the session a clicked notification is about.
fn listen(cx: &App) {
    if LISTENING.swap(true, Ordering::Relaxed) {
        return;
    }
    cx.on_system_notification_response(|response, cx| {
        cx.activate(true);
        if let Some(id) = session_of(&response.tag) {
            crate::open_session_from_notification(id, cx);
        }
    });
}

/// Show a notification about session `session`, when notifications are on.
pub(crate) fn send(cx: &App, session: Option<u64>, title: &str, body: &str) {
    if !crate::settings::notify_on() {
        return;
    }
    if !native(cx) {
        crate::settings::notify(title, body);
        return;
    }
    listen(cx);
    cx.show_system_notification(SystemNotification {
        tag: SharedString::from(tag(session)),
        title: SharedString::from(title.to_string()),
        body: SharedString::from(body.to_string()),
        actions: Vec::new(),
    });
}

/// Whether the system notification center takes Runo's notifications. On a Mac it needs
/// the app bundle.
fn native(cx: &App) -> bool {
    if cfg!(target_os = "macos") {
        cx.app_path().is_ok()
    } else {
        true
    }
}

/// A tag per session, so a newer notification replaces the older one about it.
fn tag(session: Option<u64>) -> String {
    match session {
        Some(id) => format!("session-{id}"),
        None => format!(
            "runo-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0)
        ),
    }
}

fn session_of(tag: &str) -> Option<u64> {
    tag.strip_prefix("session-")?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_name_the_session() {
        assert_eq!(session_of(&tag(Some(42))), Some(42));
        assert_eq!(session_of(&tag(None)), None);
        assert_eq!(session_of("session-x"), None);
    }
}
