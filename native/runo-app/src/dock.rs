//! The count on Runo's Dock icon, the red bubble other apps show: everything that needs
//! the person (`RunoApp::attention`). The corner mark counts only the assistant's own
//! unread answers.

use std::sync::Mutex;

/// What the Dock shows now, so only a change reaches AppKit; `None` until the first call.
static SHOWN: Mutex<Option<Option<String>>> = Mutex::new(None);

/// The bubble's text for `n`: nothing at 0, "99+" past 99.
pub(crate) fn label(n: usize) -> Option<String> {
    match n {
        0 => None,
        1..=99 => Some(n.to_string()),
        _ => Some("99+".into()),
    }
}

/// Show `n` on the Dock icon, or nothing at 0. Call it on the main thread.
pub(crate) fn show(n: usize) {
    let want = label(n);
    let mut shown = SHOWN.lock().unwrap_or_else(|e| e.into_inner());
    if shown.as_ref() != Some(&want) {
        platform::set(want.as_deref());
        *shown = Some(want);
    }
}

/// Set the count again, when the Dock icon comes back from the menu bar.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn reapply() {
    let shown = SHOWN.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(label) = shown.as_ref() {
        platform::set(label.as_deref());
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    use objc2_foundation::NSString;

    pub(super) fn set(label: Option<&str>) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let tile = NSApplication::sharedApplication(mtm).dockTile();
        let label = label.map(NSString::from_str);
        tile.setBadgeLabel(label.as_deref());
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    pub(super) fn set(_: Option<&str>) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels() {
        assert_eq!(label(0), None);
        assert_eq!(label(1).as_deref(), Some("1"));
        assert_eq!(label(99).as_deref(), Some("99"));
        assert_eq!(label(100).as_deref(), Some("99+"));
        assert_eq!(label(5000).as_deref(), Some("99+"));
    }
}
