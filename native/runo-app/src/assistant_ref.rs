//! What the Runo assistant knows of Runo besides the guide, read from the app's own tables
//! each time it is asked: the version, the menus, every command and shortcut, and every
//! page of Settings with what each setting is now. None of it is written by hand, so it
//! changes with the app and can't fall behind it.

use super::*;
use crate::settings_ui::{describe_page, every_page};

/// A page of the reference: one page of Settings, found the way guide articles are.
pub(crate) struct RefPage {
    pub title: String,
    pub body: String,
}

/// The operating system, as people call it.
fn os() -> &'static str {
    if cfg!(target_os = "macos") {
        "macOS"
    } else if cfg!(target_os = "windows") {
        "Windows"
    } else {
        "Linux"
    }
}

/// The menu bar's menus with each item and its keys, read from the menus Runo installs.
fn menus(cx: &App) -> String {
    let mut out = String::new();
    for menu in app_menus() {
        let items: Vec<String> = menu
            .items
            .iter()
            .filter_map(|item| match item {
                MenuItem::Action { name, action, .. } => {
                    let keys = keymap::label_for(cx, action.as_ref());
                    Some(if keys.is_empty() {
                        name.to_string()
                    } else {
                        format!("{name} ({keys})")
                    })
                }
                _ => None,
            })
            .collect();
        out.push_str(&format!("- {}: {}\n", menu.name, items.join(", ")));
    }
    out
}

/// Every command in Quick open, with its id for `ACTION:` lines and its keys.
fn commands(cx: &App) -> String {
    let mut out = String::new();
    for c in commands::COMMANDS {
        let keys = c.keys(cx);
        out.push_str(&format!(
            "- {} (id {}){}; guide article {}\n",
            c.label,
            c.id,
            if keys.is_empty() {
                String::new()
            } else {
                format!(", {keys}")
            },
            c.guide
        ));
    }
    out
}

/// Every shortcut that can be changed, with its keys now and what it does.
fn shortcuts(cx: &App) -> String {
    let mut out = String::new();
    for s in keymap::SHORTCUTS.iter() {
        let keys = keymap::label(cx, s.id);
        out.push_str(&format!(
            "- {}: {} {}\n",
            s.title,
            if keys.is_empty() { "no keys" } else { &keys },
            s.desc
        ));
    }
    out
}

/// Settings' tabs and pages, each with its sections, so the agent knows where any
/// setting lives before it reads the page itself.
fn settings_map(cx: &App) -> String {
    let st = cx.global::<AppSettings>();
    let mut out = String::new();
    for (tab, page) in every_page() {
        if matches!(page, SettingsPage::Agent(_)) {
            continue;
        }
        let place = if tab == page.label() {
            tab.to_string()
        } else {
            format!("{tab} → {}", page.label())
        };
        let sections = crate::settings_ui::section_titles(page, st);
        out.push_str(&format!("- {place}: {}\n", sections.join(", ")));
    }
    out.push_str("- Agents → Providers lists each agent CLI; each has its own page of settings.\n");
    out
}

/// What the assistant always knows of Runo itself: which Runo this is, its menus,
/// commands, shortcuts and the map of Settings.
pub(crate) fn overview(cx: &App) -> String {
    let st = cx.global::<AppSettings>();
    let mut out = format!("This is Runo {} on {}.\n", env!("CARGO_PKG_VERSION"), os());
    if cfg!(target_os = "macos") && st.menu_bar && st.run_in_background {
        out.push_str(
            "Keep running in the menu bar is on: closing the window and the Quit Runo menu \
             item hide Runo to the menu bar and agents keep working; Quit Runo in the menu \
             bar panel, or the quit command, quits for real.\n",
        );
    }
    out.push_str("\nThe menu bar's menus (items with their keys):\n");
    out.push_str(&menus(cx));
    out.push_str("\nCommands in Quick open (every one can also be run for the person):\n");
    out.push_str(&commands(cx));
    out.push_str("\nShortcuts, as set now (changed in Settings → Keys → Keybindings):\n");
    out.push_str(&shortcuts(cx));
    out.push_str("\nSettings: its tabs and pages, with the sections on each:\n");
    out.push_str(&settings_map(cx));
    out
}

/// Every page of Settings written out, each agent's included.
pub(crate) fn pages(data: &Discovered, cx: &App) -> Vec<RefPage> {
    every_page()
        .into_iter()
        .map(|(_, page)| RefPage {
            title: crate::settings_ui::place(page),
            body: describe_page(page, data, cx),
        })
        .collect()
}
