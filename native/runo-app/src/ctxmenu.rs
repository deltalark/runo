//! Right-click menus for sessions, tabs, projects, files, messages and the terminal.

use super::*;
use std::path::PathBuf;

/// What was right-clicked.
#[derive(Clone, PartialEq)]
pub(crate) enum CtxTarget {
    Session(u64),
    Tab(u64),
    Project(u64),
    /// A changed file in the Changes panel (path relative to the session folder).
    Change(String),
    /// A file in the Files tab (path relative to the session folder).
    File(String),
    Message(usize),
    Terminal,
    /// The Playbooks button in a session's header, for its project.
    Playbooks(u64),
}

#[derive(Clone)]
enum Act {
    Open(u64),
    Split(u64),
    Rename(u64),
    Pin(u64),
    Unread(u64),
    Archive(u64),
    Delete(u64),
    Shell(u64),
    CloseTab(u64),
    CloseOthers(u64),
    NewIn(u64),
    EditProject(u64),
    /// Run a playbook (id) in a project (id).
    Playbook(u64, u64),
    ManagePlaybooks,
    Toggle(u64),
    Diff(String),
    Preview(String),
    Edit(PathBuf),
    Mention(String),
    Discard(String),
    Copy(String, &'static str),
    Reveal(PathBuf),
    Quote(String),
    TermCopy,
    TermPaste,
    TermClear,
    TermClose,
}

struct Item {
    label: String,
    glyph: IconName,
    act: Act,
    danger: bool,
}

fn item(label: impl Into<String>, glyph: IconName, act: Act) -> Option<Item> {
    Some(Item {
        label: label.into(),
        glyph,
        act,
        danger: false,
    })
}

fn danger(label: impl Into<String>, glyph: IconName, act: Act) -> Option<Item> {
    Some(Item {
        label: label.into(),
        glyph,
        act,
        danger: true,
    })
}

impl RunoApp {
    /// A handler that opens the right-click menu for `target` at the pointer.
    pub(crate) fn on_context(
        &self,
        target: CtxTarget,
        cx: &mut Context<Self>,
    ) -> impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static {
        let this = cx.entity().downgrade();
        move |ev: &MouseDownEvent, _: &mut Window, cx: &mut App| {
            let (pos, target) = (ev.position, target.clone());
            let _ = this.update(cx, |this, cx| {
                this.ctx_menu = Some((pos, target));
                cx.notify();
            });
            cx.stop_propagation();
        }
    }

    /// Menu entries for the target; `None` entries are separators.
    fn ctx_items(&self, target: &CtxTarget) -> Vec<Option<Item>> {
        let session_dir = |id: u64| {
            self.state
                .session(id)
                .map(|s| PathBuf::from(&s.dir))
                .unwrap_or_default()
        };
        let current_dir = self
            .current()
            .map(|s| PathBuf::from(&s.dir))
            .unwrap_or_default();
        match target {
            CtxTarget::Session(id) | CtxTarget::Tab(id) => {
                let id = *id;
                let Some(s) = self.state.session(id) else {
                    return Vec::new();
                };
                let mut v = Vec::new();
                if let CtxTarget::Tab(_) = target {
                    v.push(item("Close tab", IconName::X, Act::CloseTab(id)));
                    if self.tabs.len() > 1 {
                        v.push(item(
                            "Close other tabs",
                            IconName::Layers,
                            Act::CloseOthers(id),
                        ));
                    }
                    v.push(None);
                } else {
                    v.push(item("Open", IconName::ArrowRight, Act::Open(id)));
                }
                if self.selected.is_some_and(|sel| sel != id) {
                    v.push(item(
                        "Open beside current",
                        IconName::Columns2,
                        Act::Split(id),
                    ));
                }
                v.push(item(
                    "Open terminal here",
                    IconName::SquareTerminal,
                    Act::Shell(id),
                ));
                v.push(None);
                v.push(item("Rename…", IconName::Pencil, Act::Rename(id)));
                v.push(item(
                    if s.pinned { "Unpin" } else { "Pin to top" },
                    if s.pinned {
                        IconName::PinOff
                    } else {
                        IconName::Pin
                    },
                    Act::Pin(id),
                ));
                v.push(item(
                    if s.unread {
                        "Mark as read"
                    } else {
                        "Mark as unread"
                    },
                    if s.unread {
                        IconName::MailOpen
                    } else {
                        IconName::Mail
                    },
                    Act::Unread(id),
                ));
                v.push(None);
                if !s.branch.is_empty() {
                    v.push(item(
                        "Copy branch name",
                        IconName::GitBranch,
                        Act::Copy(s.branch.clone(), "branch name"),
                    ));
                }
                v.push(item(
                    "Copy folder path",
                    IconName::Copy,
                    Act::Copy(s.dir.clone(), "folder path"),
                ));
                v.push(item(
                    "Reveal in file manager",
                    IconName::FolderOpen,
                    Act::Reveal(session_dir(id)),
                ));
                v.push(None);
                v.push(item(
                    if s.archived { "Restore" } else { "Archive" },
                    IconName::Archive,
                    Act::Archive(id),
                ));
                v.push(danger("Delete…", IconName::Trash, Act::Delete(id)));
                v
            }
            CtxTarget::Project(pid) => {
                let Some(p) = self.state.projects.iter().find(|p| p.id == *pid) else {
                    return Vec::new();
                };
                vec![
                    item("New session here", IconName::Plus, Act::NewIn(*pid)),
                    item(
                        if self.expanded.contains(pid) {
                            "Collapse"
                        } else {
                            "Expand"
                        },
                        IconName::ChevronsUpDown,
                        Act::Toggle(*pid),
                    ),
                    item("Edit project…", IconName::Palette, Act::EditProject(*pid)),
                ]
                .into_iter()
                .chain(
                    // Playbooks that can run here: any-project ones and this project's own.
                    self.state
                        .automations
                        .iter()
                        .filter(|a| a.project_id == 0 || a.project_id == *pid)
                        .take(8)
                        .enumerate()
                        .flat_map(|(i, a)| {
                            let first = (i == 0).then_some(None);
                            first.into_iter().chain(std::iter::once(item(
                                a.name.clone(),
                                IconName::Play,
                                Act::Playbook(a.id, *pid),
                            )))
                        }),
                )
                .chain([
                    None,
                    item(
                        "Copy path",
                        IconName::Copy,
                        Act::Copy(p.path.clone(), "project path"),
                    ),
                    item(
                        "Reveal in file manager",
                        IconName::FolderOpen,
                        Act::Reveal(PathBuf::from(&p.path)),
                    ),
                ])
                .collect()
            }
            CtxTarget::Playbooks(pid) => self
                .state
                .automations
                .iter()
                .filter(|a| a.project_id == 0 || a.project_id == *pid)
                .take(12)
                .map(|a| item(a.name.clone(), IconName::Play, Act::Playbook(a.id, *pid)))
                .chain([
                    None,
                    item(
                        "Manage playbooks…",
                        IconName::BookOpen,
                        Act::ManagePlaybooks,
                    ),
                ])
                .collect(),
            CtxTarget::Change(path) | CtxTarget::File(path) => {
                let full = current_dir.join(path);
                let mut v = Vec::new();
                if let CtxTarget::Change(_) = target {
                    v.push(item(
                        "Show diff",
                        IconName::GitCompare,
                        Act::Diff(path.clone()),
                    ));
                }
                v.push(item("Preview", IconName::Eye, Act::Preview(path.clone())));
                v.push(item("Edit", IconName::Pencil, Act::Edit(full.clone())));
                v.push(item(
                    "Mention in prompt",
                    IconName::AtSign,
                    Act::Mention(path.clone()),
                ));
                v.push(None);
                v.push(item(
                    "Copy relative path",
                    IconName::Copy,
                    Act::Copy(path.clone(), "path"),
                ));
                v.push(item(
                    "Copy full path",
                    IconName::Copy,
                    Act::Copy(full.to_string_lossy().into_owned(), "full path"),
                ));
                v.push(item(
                    "Reveal in file manager",
                    IconName::FolderOpen,
                    Act::Reveal(full),
                ));
                if let CtxTarget::Change(_) = target {
                    v.push(None);
                    v.push(danger(
                        "Discard changes…",
                        IconName::Undo2,
                        Act::Discard(path.clone()),
                    ));
                }
                v
            }
            CtxTarget::Message(index) => {
                let Some(m) = self.current().and_then(|s| s.messages.get(*index)) else {
                    return Vec::new();
                };
                let mut v = vec![item(
                    "Copy text",
                    IconName::Copy,
                    Act::Copy(m.text.clone(), "message"),
                )];
                if !m.plan.is_empty() {
                    v.push(item(
                        "Copy plan",
                        IconName::ListChecks,
                        Act::Copy(m.plan.clone(), "plan"),
                    ));
                }
                v.push(item(
                    "Quote in reply",
                    IconName::Quote,
                    Act::Quote(m.text.clone()),
                ));
                v
            }
            CtxTarget::Terminal => vec![
                item("Copy screen", IconName::Copy, Act::TermCopy),
                item("Paste", IconName::Clipboard, Act::TermPaste),
                item("Clear", IconName::Eraser, Act::TermClear),
                None,
                item(
                    "New shell",
                    IconName::Plus,
                    Act::Shell(self.selected.unwrap_or(0)),
                ),
                danger("Close shell", IconName::X, Act::TermClose),
            ],
        }
    }

    fn run_ctx(&mut self, act: Act, window: &mut Window, cx: &mut Context<Self>) {
        self.ctx_menu = None;
        match act {
            Act::Open(id) => self.select(id),
            Act::Split(id) => {
                self.split = Some(id);
                self.view = View::Session;
                self.split_scroll.scroll_to_bottom();
            }
            Act::Rename(id) => {
                self.select(id);
                self.start_rename(window, cx);
            }
            Act::Pin(id) => self.toggle_pin(id),
            Act::Unread(id) => {
                if let Some(s) = self.state.session_mut(id) {
                    s.unread = !s.unread;
                }
                self.save();
            }
            Act::Archive(id) => self.toggle_archive(id),
            Act::Delete(id) => {
                self.select(id);
                self.confirm_delete = Some(id);
            }
            Act::Shell(id) => {
                if self.selected != Some(id) {
                    self.select(id);
                }
                self.show_panel = true;
                self.new_shell(cx);
            }
            Act::CloseTab(id) => self.close_tab(id),
            Act::CloseOthers(id) => {
                self.tabs.retain(|t| *t == id);
                if self.selected != Some(id) {
                    self.select(id);
                }
                self.save();
            }
            Act::EditProject(pid) => self.edit_project(pid),
            Act::Playbook(id, pid) => self.run_playbook(id, Some(pid), cx),
            Act::ManagePlaybooks => self.view = View::Automations,
            Act::NewIn(pid) => {
                self.new_project = Some(pid);
                self.open_new_session();
            }
            Act::Toggle(pid) => {
                if let Some(pos) = self.expanded.iter().position(|p| *p == pid) {
                    self.expanded.remove(pos);
                } else {
                    self.expanded.push(pid);
                }
            }
            Act::Diff(path) => {
                self.right_tab = RightTab::Changes;
                if let Some(dir) = self.current().map(|s| s.dir.clone()) {
                    if !self.demo {
                        self.diff = self.load_diff(&dir, &path);
                    }
                }
                self.selected_file = Some(path);
            }
            Act::Preview(path) => self.open_preview(&path),
            Act::Edit(path) => self.edit_path(&path),
            Act::Mention(path) => {
                let text = self.prompt.read(cx).value().to_string();
                let sep = if text.is_empty() || text.ends_with(' ') {
                    ""
                } else {
                    " "
                };
                let new = format!("{text}{sep}@{path} ");
                self.prompt.update(cx, |p, cx| {
                    p.set_value(new, window, cx);
                    p.focus(window, cx);
                });
            }
            Act::Discard(path) => {
                self.right_tab = RightTab::Changes;
                self.confirm_discard = Some(path);
            }
            Act::Copy(text, what) => {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                self.info = Some((format!("Copied the {what}."), std::time::Instant::now()));
            }
            Act::Reveal(path) => cx.reveal_path(&path),
            Act::Quote(text) => {
                let quoted: String = text
                    .trim()
                    .lines()
                    .take(12)
                    .map(|l| format!("> {l}\n"))
                    .collect();
                let current = self.prompt.read(cx).value().to_string();
                let new = format!("{quoted}\n{current}");
                self.prompt.update(cx, |p, cx| {
                    p.set_value(new, window, cx);
                    p.focus(window, cx);
                });
            }
            Act::TermCopy => {
                if let Some(shell) = self.active_shell() {
                    cx.write_to_clipboard(ClipboardItem::new_string(shell.text()));
                    self.info = Some((
                        "Copied the terminal screen.".into(),
                        std::time::Instant::now(),
                    ));
                }
            }
            Act::TermPaste => {
                if let (Some(shell), Some(text)) = (
                    self.active_shell(),
                    cx.read_from_clipboard().and_then(|c| c.text()),
                ) {
                    shell.paste(&text);
                }
                window.focus(&self.term_focus, cx);
            }
            Act::TermClear => {
                if let Some(shell) = self.active_shell() {
                    shell.write(b"\x0c");
                }
                window.focus(&self.term_focus, cx);
            }
            Act::TermClose => {
                if let Some(id) = self.shell_tab {
                    self.close_shell(id, cx);
                }
            }
        }
        cx.notify();
    }

    /// The open right-click menu, drawn at the pointer and kept inside the window.
    pub(crate) fn ctx_menu_view(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (pos, target) = self.ctx_menu.clone()?;
        let items = self.ctx_items(&target);
        if items.is_empty() {
            return None;
        }
        let mut list = div().v_flex().p_1();
        for (i, entry) in items.into_iter().enumerate() {
            let Some(entry) = entry else {
                list = list.child(div().my_1().mx_2().h(px(1.)).bg(rgb(c_border())));
                continue;
            };
            let color = if entry.danger { c_red() } else { c_text() };
            let act = entry.act.clone();
            list =
                list.child(
                    div()
                        .id(("ctx-item", i))
                        .h(px(28.))
                        .px_2()
                        .flex()
                        .items_center()
                        .gap_2()
                        .rounded_md()
                        .cursor_pointer()
                        .text_size(px(13.))
                        .text_color(rgb(color))
                        .hover(|el| {
                            el.bg(rgb(if entry.danger {
                                settings::mix(c_red(), c_raised(), 0.14)
                            } else {
                                c_active()
                            }))
                        })
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.run_ctx(act.clone(), window, cx)
                        }))
                        .child(icon(
                            entry.glyph,
                            14.,
                            if entry.danger { c_red() } else { c_secondary() },
                        ))
                        .child(entry.label),
                );
        }
        let menu = div()
            .id("ctx-menu")
            .min_w(px(210.))
            .rounded_lg()
            .border_1()
            .border_color(float_edge())
            .bg(float(c_raised()))
            .shadow_lg()
            .occlude()
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.ctx_menu = None;
                cx.notify();
            }))
            .child(list);
        Some(
            deferred(
                anchored()
                    .position(pos)
                    .snap_to_window_with_margin(px(8.))
                    .child(rise(
                        menu,
                        SharedString::from(format!(
                            "ctx-{:.0}-{:.0}",
                            f32::from(pos.x),
                            f32::from(pos.y)
                        )),
                        160,
                    )),
            )
            .with_priority(2)
            .into_any_element(),
        )
    }
}
