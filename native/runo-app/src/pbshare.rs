//! Sharing playbooks: export one as a file, install from a link, repository or file,
//! update from where it came from, and ask for a playbook's inputs before it runs.

use super::*;
use crate::pbfile::{self, Fetched, Found};
use crate::playbooks::{look, permission_summary};

/// The Install dialog on the Playbooks page.
pub(crate) struct PlaybookInstall {
    /// Reading the source in the background.
    pub busy: bool,
    pub error: Option<String>,
    /// The source that was read, and what it held.
    pub source: String,
    pub fetched: Option<Fetched>,
    /// Indexes into `fetched.found` to install.
    pub chosen: Vec<usize>,
}

/// The form asking for a playbook's inputs before it runs.
pub(crate) struct PlaybookRun {
    pub id: u64,
    pub in_project: Option<u64>,
    /// Picked from `/` in a message: the values ride with the chip instead of starting a
    /// session.
    pub chip: bool,
    pub fields: Vec<(pbfile::Input, Entity<InputState>)>,
}

/// How an installed playbook relates to one already here.
enum Status {
    New,
    Same,
    Update(String),
    Replace,
}

/// What an update would change in `a`, in short phrases such as "may now run commands"
/// or "prompt: 3 lines added, 1 removed". Empty when nothing a playbook file holds differs.
fn changes(a: &store::Automation, file: &pbfile::PlaybookFile) -> Vec<String> {
    let mut out = Vec::new();
    let title = |key: &str| {
        pbfile::PERMISSIONS
            .iter()
            .find(|p| p.0 == key)
            .map(|p| p.1.to_lowercase())
            .unwrap_or_else(|| key.to_string())
    };
    // No list means it may do anything.
    let all: Vec<String> = pbfile::PERMISSIONS
        .iter()
        .map(|p| p.0.to_string())
        .collect();
    let before = if a.permissions.is_empty() {
        &all
    } else {
        &a.permissions
    };
    let after = if file.permissions.is_empty() {
        &all
    } else {
        &file.permissions
    };
    for key in after.iter().filter(|k| !before.contains(k)) {
        out.push(format!("may now {}", title(key)));
    }
    for key in before.iter().filter(|k| !after.contains(k)) {
        out.push(format!("no longer may {}", title(key)));
    }
    if a.prompt.trim() != file.prompt.trim() {
        let old: Vec<&str> = a.prompt.lines().collect();
        let new: Vec<&str> = file.prompt.lines().collect();
        let added = new.iter().filter(|l| !old.contains(l)).count();
        let removed = old.iter().filter(|l| !new.contains(l)).count();
        out.push(match (added, removed) {
            (0, 0) => "prompt reworded".to_string(),
            (a, 0) => format!("prompt: {a} lines added"),
            (0, r) => format!("prompt: {r} lines removed"),
            (a, r) => format!("prompt: {a} lines added, {r} removed"),
        });
    }
    if a.description.trim() != file.description.trim() {
        out.push("new description".into());
    }
    if a.inputs != file.inputs {
        out.push("inputs changed".into());
    }
    out
}

impl RunoApp {
    /// The playbook already installed for `found`: same source and file, else same name.
    fn installed_match(&self, source: &str, found: &Found) -> Option<&store::Automation> {
        self.state
            .automations
            .iter()
            .find(|a| !a.source.is_empty() && a.source == source && a.origin == found.origin)
            .or_else(|| {
                self.state
                    .automations
                    .iter()
                    .find(|a| a.name.eq_ignore_ascii_case(&found.file.name))
            })
    }

    fn install_status(&self, source: &str, found: &Found) -> Status {
        match self.installed_match(source, found) {
            None => Status::New,
            Some(a) if a.source != source || a.origin != found.origin => Status::Replace,
            Some(a) if pbfile::newer(&found.file.version, &a.version) => {
                Status::Update(found.file.version.clone())
            }
            Some(a) if changes(a, &found.file).is_empty() => Status::Same,
            Some(_) => Status::Update(found.file.version.clone()),
        }
    }

    /// Whether an update starts ticked: not when you've edited your copy here.
    fn ticks(&self, source: &str, found: &Found) -> bool {
        match self.install_status(source, found) {
            Status::New => true,
            Status::Update(_) => !self
                .installed_match(source, found)
                .is_some_and(|a| a.edited_here()),
            _ => false,
        }
    }

    pub(crate) fn open_playbook_install(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pb_install = Some(PlaybookInstall {
            busy: false,
            error: None,
            source: String::new(),
            fetched: None,
            chosen: Vec::new(),
        });
        self.pb_source.update(cx, |i, cx| {
            i.set_value("", window, cx);
            i.focus(window, cx);
        });
    }

    /// Read the source box (or `source`) in the background and list what it holds.
    pub(crate) fn fetch_playbooks(&mut self, source: Option<String>, cx: &mut Context<Self>) {
        let source = source.unwrap_or_else(|| self.pb_source.read(cx).value().trim().to_string());
        let Some(install) = self.pb_install.as_mut() else {
            return;
        };
        install.busy = true;
        install.error = None;
        install.fetched = None;
        install.source = source.clone();
        let task = cx
            .background_executor()
            .spawn(async move { pbfile::fetch(&source) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                let statuses: Option<Vec<bool>> = result.as_ref().ok().map(|fetched| {
                    let source = this
                        .pb_install
                        .as_ref()
                        .map(|i| i.source.clone())
                        .unwrap_or_default();
                    fetched
                        .found
                        .iter()
                        // Only what's new or an update starts ticked; same-named
                        // playbooks of your own are left alone unless you tick them.
                        .map(|f| this.ticks(&source, f))
                        .collect()
                });
                if let Some(install) = this.pb_install.as_mut() {
                    install.busy = false;
                    match result {
                        Ok(fetched) => {
                            install.chosen = statuses
                                .unwrap_or_default()
                                .iter()
                                .enumerate()
                                .filter(|(_, pick)| **pick)
                                .map(|(i, _)| i)
                                .collect();
                            install.fetched = Some(fetched);
                        }
                        Err(e) => {
                            crate::applog::warn(
                                "playbook",
                                format!(
                                    "could not fetch shared playbooks: {}",
                                    crate::applog::gist(&e)
                                ),
                            );
                            install.error = Some(e)
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Install from playbook text on the clipboard, as shared in a chat.
    pub(crate) fn paste_playbook(&mut self, cx: &mut Context<Self>) {
        let text = cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .unwrap_or_default();
        let Some(install) = self.pb_install.as_mut() else {
            return;
        };
        install.busy = false;
        install.fetched = None;
        install.chosen.clear();
        match pbfile::parse(&text) {
            Ok(file) => {
                let found = Found {
                    file,
                    origin: String::new(),
                };
                install.source = String::new();
                let tick = self.ticks("", &found);
                let install = self.pb_install.as_mut().expect("checked above");
                if tick {
                    install.chosen.push(0);
                }
                install.error = None;
                install.fetched = Some(Fetched {
                    found: vec![found],
                    skipped: Vec::new(),
                });
            }
            Err(e) => {
                install.error = Some(if text.trim().is_empty() {
                    "The clipboard is empty. Copy a playbook first.".into()
                } else {
                    format!("The clipboard doesn't hold a playbook. {e}")
                })
            }
        }
    }

    /// Choose a playbook file in a dialog and read it.
    pub(crate) fn browse_playbook_file(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: true,
            multiple: false,
            prompt: Some("Install".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                if let Some(path) = paths.first() {
                    let path = path.to_string_lossy().into_owned();
                    let _ = this.update(cx, |this, cx| {
                        this.fetch_playbooks(Some(path), cx);
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    /// Install the chosen playbooks, updating ones already here in place.
    pub(crate) fn install_playbooks(&mut self, cx: &mut Context<Self>) {
        let Some(install) = self.pb_install.take() else {
            return;
        };
        let Some(fetched) = install.fetched else {
            return;
        };
        let default_provider = cx.global::<AppSettings>().default_provider.clone();
        let mut added = 0;
        let mut updated = 0;
        for i in install.chosen {
            let Some(found) = fetched.found.get(i) else {
                continue;
            };
            let file = &found.file;
            let provider = agent::PROVIDERS
                .iter()
                .find(|p| p.id == file.agent)
                .map(|p| p.id.to_string())
                .unwrap_or_else(|| default_provider.clone());
            let existing = self.installed_match(&install.source, found).map(|a| a.id);
            let fill = |a: &mut store::Automation| {
                a.name = file.name.clone();
                a.description = file.description.clone();
                a.prompt = file.prompt.clone();
                a.author = file.author.clone();
                a.version = file.version.clone();
                a.permissions = file.permissions.clone();
                a.inputs = file.inputs.clone();
                a.source = install.source.clone();
                a.origin = found.origin.clone();
                if !file.agent.is_empty() {
                    a.provider = provider.clone();
                }
                a.installed = a.fingerprint();
            };
            match existing.and_then(|id| self.state.automations.iter_mut().find(|a| a.id == id)) {
                Some(a) => {
                    fill(a);
                    updated += 1;
                }
                None => {
                    let mut a = store::Automation {
                        id: self.state.next_id(),
                        provider: provider.clone(),
                        enabled: true,
                        ..Default::default()
                    };
                    fill(&mut a);
                    self.state.automations.push(a);
                    added += 1;
                }
            }
        }
        self.save();
        let say = |n: usize, what: &str| match n {
            0 => None,
            1 => Some(format!("1 playbook {what}")),
            n => Some(format!("{n} playbooks {what}")),
        };
        let parts: Vec<String> = [say(added, "installed"), say(updated, "updated")]
            .into_iter()
            .flatten()
            .collect();
        if !parts.is_empty() {
            self.info = Some((format!("{}.", parts.join(", ")), std::time::Instant::now()));
        }
        cx.notify();
    }

    /// Re-read where a playbook came from and offer what changed.
    pub(crate) fn check_playbook_update(
        &mut self,
        id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(source) = self
            .state
            .automations
            .iter()
            .find(|a| a.id == id)
            .map(|a| a.source.clone())
            .filter(|s| !s.is_empty())
        else {
            return;
        };
        self.open_playbook_install(window, cx);
        self.pb_source
            .update(cx, |i, cx| i.set_value(source.clone(), window, cx));
        self.fetch_playbooks(Some(source), cx);
    }

    fn playbook_text(&self, id: u64) -> Option<(String, String)> {
        let auto = self.state.automations.iter().find(|a| a.id == id)?;
        let file = auto.to_file();
        Some((pbfile::file_name(&file.name), pbfile::render(&file)))
    }

    /// Put the playbook on the clipboard as file text, to paste into a chat or a repo.
    pub(crate) fn copy_playbook(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some((_, text)) = self.playbook_text(id) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            self.info = Some((
                "Copied as a playbook file. Paste it into a chat, a gist or a repo.".into(),
                std::time::Instant::now(),
            ));
        }
    }

    /// Save the playbook as a `.playbook.md` file.
    pub(crate) fn export_playbook(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some((name, text)) = self.playbook_text(id) else {
            return;
        };
        let dir = std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let rx = cx.prompt_for_new_path(&dir, Some(&name));
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(path))) = rx.await {
                let result = std::fs::write(&path, text);
                let _ = this.update(cx, |this, cx| {
                    match result {
                        Ok(()) => {
                            this.info = Some((
                                format!(
                                    "Exported to {}. Share it, or add it to a Git repo others install from.",
                                    path.display()
                                ),
                                std::time::Instant::now(),
                            ))
                        }
                        Err(e) => this.fail("playbook", format!("Couldn't export it: {e}")),
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Ask for a playbook's inputs before running it, or before it rides on a message.
    pub(crate) fn ask_playbook_inputs(&mut self, id: u64, in_project: Option<u64>, chip: bool) {
        self.pb_run = Some(PlaybookRun {
            id,
            in_project,
            chip,
            fields: Vec::new(),
        });
    }

    /// Make the input boxes for the run form; called from render, which has the window.
    pub(crate) fn prepare_playbook_run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(run) = self.pb_run.as_ref() else {
            return;
        };
        if !run.fields.is_empty() {
            return;
        }
        let Some(auto) = self.state.automations.iter().find(|a| a.id == run.id) else {
            self.pb_run = None;
            return;
        };
        let fields: Vec<(pbfile::Input, Entity<InputState>)> = auto
            .run_inputs()
            .into_iter()
            .map(|input| {
                let state = cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder(if input.default.is_empty() {
                            input.title()
                        } else {
                            input.default.clone()
                        })
                        .default_value(input.default.clone())
                });
                (input, state)
            })
            .collect();
        if let Some((_, first)) = fields.first() {
            first.update(cx, |i, cx| i.focus(window, cx));
        }
        if let Some(run) = self.pb_run.as_mut() {
            run.fields = fields;
        }
    }

    pub(crate) fn confirm_playbook_run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(run) = self.pb_run.take() else {
            return;
        };
        let values: Vec<(String, String)> = run
            .fields
            .iter()
            .map(|(input, state)| (input.key.clone(), state.read(cx).value().trim().to_string()))
            .collect();
        if run.chip {
            self.picked_playbook = Some(run.id);
            self.picked_values = values;
            self.prompt.update(cx, |p, cx| p.focus(window, cx));
        } else {
            self.pb_values = Some((run.id, values));
            self.run_playbook(run.id, run.in_project, cx);
        }
        cx.notify();
    }

    /// The dialog asking for a playbook's inputs.
    pub(crate) fn playbook_run_form(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let run = self.pb_run.as_ref()?;
        let auto = self.state.automations.iter().find(|a| a.id == run.id)?;
        let (glyph, color) = look(&auto.name, &auto.prompt);
        let mut fields = div().v_flex().gap_3();
        for (input, state) in &run.fields {
            fields = fields.child(
                div()
                    .v_flex()
                    .gap_1p5()
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(input.title()),
                    )
                    .child(Input::new(state).bordered(true).h(px(34.))),
            );
        }
        let warning =
            crate::workspace::limits_warning(&auto.provider, &auto.permissions).map(|w| {
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(12.))
                    .text_color(rgb(c_amber()))
                    .child(icon(IconName::ShieldAlert, 13., c_amber()))
                    .child(w)
            });
        let limits = (!auto.permissions.is_empty()).then(|| {
            let text = match permission_summary(&auto.permissions) {
                None => "Read only: it can look through the project but not change it.".into(),
                Some(text) => format!("It may: {}.", text.to_lowercase()),
            };
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_size(px(12.))
                .text_color(rgb(c_muted()))
                .child(icon(IconName::ShieldCheck, 13., c_muted()))
                .child(text)
        });
        let card = div()
            .id("pb-run")
            .w(px(460.))
            .max_w(relative(0.94))
            .v_flex()
            .rounded(px(crate::shell::ISLAND_R))
            .border_1()
            .border_color(float_edge())
            .bg(float(c_sheet()))
            .shadow_2xl()
            .overflow_hidden()
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .px_5()
                    .py_4()
                    .flex()
                    .items_center()
                    .gap_3()
                    .border_b_1()
                    .border_color(divider())
                    .child(Self::tile(glyph, color, 34.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .v_flex()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(15.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(format!("Run {}", auto.name)),
                            )
                            .child(div().text_xs().text_color(rgb(c_muted())).child(
                                if run.fields.is_empty() {
                                    "Check what it may do, then run it."
                                } else {
                                    "Fill in what this run should use."
                                },
                            )),
                    ),
            )
            .child(
                div()
                    .px_5()
                    .py_4()
                    .v_flex()
                    .gap_4()
                    .child(fields)
                    .children(limits)
                    .children(warning),
            )
            .child(
                div()
                    .px_5()
                    .py_3()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .border_t_1()
                    .border_color(divider())
                    .child(
                        self.button("pb-run-cancel", "Cancel", false)
                            .h(px(30.))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.pb_run = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        self.button(
                            "pb-run-go",
                            if run.chip { "Add to message" } else { "Run" },
                            true,
                        )
                        .h(px(30.))
                        .child(icon(IconName::Play, 11., c_canvas()))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.confirm_playbook_run(window, cx);
                        })),
                    ),
            );
        Some(self.modal("pb-run-backdrop", card, cx, |this| this.pb_run = None))
    }

    /// The Install dialog.
    pub(crate) fn playbook_install_modal(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let install = self.pb_install.as_ref()?;
        let mut body = div().v_flex().gap_4();
        body = body.child(
            div()
                .v_flex()
                .gap_1p5()
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(FontWeight::MEDIUM)
                        .child("Where from"),
                )
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(Input::new(&self.pb_source).bordered(true).h(px(34.))),
                        )
                        .child(
                            self.button("pb-src-paste", "Paste", false)
                                .h(px(34.))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.paste_playbook(cx);
                                    cx.notify();
                                })),
                        )
                        .child(
                            self.button("pb-src-browse", "Choose…", false)
                                .h(px(34.))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.browse_playbook_file(cx);
                                })),
                        )
                        .child(
                            self.button(
                                "pb-src-read",
                                if install.busy { "Reading…" } else { "Find playbooks" },
                                true,
                            )
                            .h(px(34.))
                            .on_click(cx.listener(|this, _, _, cx| {
                                if !this.pb_install.as_ref().is_some_and(|i| i.busy) {
                                    this.fetch_playbooks(None, cx);
                                }
                                cx.notify();
                            })),
                        ),
                )
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(rgb(c_muted()))
                        .child(format!(
                            "A Git repository (any {} files in it), a link to one file, a file or folder on this computer, or Paste one shared in a chat.",
                            pbfile::SUFFIX
                        )),
                ),
        );
        if let Some(error) = &install.error {
            body = body.child(
                div()
                    .px_3()
                    .py_2()
                    .rounded_lg()
                    .bg(tint(settings::mix(c_red(), c_canvas(), 0.1)))
                    .text_size(px(12.5))
                    .text_color(rgb(c_red()))
                    .child(error.clone()),
            );
        }
        if install.busy {
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(12.5))
                    .text_color(rgb(c_muted()))
                    .child(icon(IconName::LoaderCircle, 14., c_muted()))
                    .child("Reading playbooks…"),
            );
        }
        if let Some(fetched) = &install.fetched {
            let mut list = div().v_flex().gap_2();
            for (i, found) in fetched.found.iter().enumerate() {
                list = list.child(self.found_row(i, found, install, cx));
            }
            body = body.child(
                div()
                    .v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(match fetched.found.len() {
                                1 => "Found 1 playbook".to_string(),
                                n => format!("Found {n} playbooks"),
                            }),
                    )
                    .child(list),
            );
            if !fetched.skipped.is_empty() {
                let names: Vec<String> = fetched
                    .skipped
                    .iter()
                    .map(|(name, why)| format!("{name}: {why}"))
                    .collect();
                body = body.child(
                    div()
                        .text_size(px(11.5))
                        .text_color(rgb(c_amber()))
                        .child(format!("Skipped {}", names.join("; "))),
                );
            }
        }
        let count = install.chosen.len();
        let card = div()
            .id("pb-install")
            .w(px(620.))
            .max_w(relative(0.94))
            .max_h(relative(0.9))
            .v_flex()
            .rounded(px(crate::shell::ISLAND_R))
            .border_1()
            .border_color(float_edge())
            .bg(float(c_sheet()))
            .shadow_2xl()
            .overflow_hidden()
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .px_6()
                    .py_4()
                    .flex()
                    .items_center()
                    .gap_3()
                    .border_b_1()
                    .border_color(divider())
                    .child(Self::tile(IconName::Download, c_blue(), 40.))
                    .child(
                        div()
                            .flex_1()
                            .v_flex()
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Install playbooks"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(c_muted()))
                                    .child("From a teammate's repo, a shared link or a file. You'll see what each one may do first."),
                            ),
                    )
                    .child(
                        self.ghost_button_sized("pb-install-close", IconName::X, "Close", 28.)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.pb_install = None;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .id("pb-install-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_6()
                    .py_5()
                    .child(body),
            )
            .child(
                div()
                    .px_6()
                    .py_3()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap_2()
                    .border_t_1()
                    .border_color(divider())
                    .child(
                        self.button("pb-install-cancel", "Cancel", false)
                            .h(px(32.))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.pb_install = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        self.button(
                            "pb-install-go",
                            match count {
                                0 => "Install".to_string(),
                                1 => "Install 1 playbook".to_string(),
                                n => format!("Install {n} playbooks"),
                            },
                            true,
                        )
                        .h(px(32.))
                        .when(count == 0, |el| el.opacity(0.5))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if count > 0 {
                                this.install_playbooks(cx);
                            }
                        })),
                    ),
            );
        Some(self.modal("pb-install-backdrop", card, cx, |this| {
            this.pb_install = None
        }))
    }

    /// One playbook found while installing: a checkbox, what it is, and what it may do.
    fn found_row(
        &self,
        i: usize,
        found: &Found,
        install: &PlaybookInstall,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let file = &found.file;
        let on = install.chosen.contains(&i);
        let (glyph, color) = look(&file.name, &file.prompt);
        let status = self.install_status(&install.source, found);
        let (tag, tag_color) = match &status {
            Status::New => ("New".to_string(), c_green()),
            Status::Same => ("Installed".to_string(), c_muted()),
            Status::Update(v) if v.is_empty() => ("Update".to_string(), c_blue()),
            Status::Update(v) => (
                format!("Update to v{}", v.trim_start_matches('v')),
                c_blue(),
            ),
            Status::Replace => ("Replaces yours".to_string(), c_amber()),
        };
        let byline = [
            (!file.author.is_empty()).then(|| format!("by {}", file.author)),
            (!file.version.is_empty())
                .then(|| format!("v{}", file.version.trim_start_matches('v'))),
            Some(found.origin.clone()).filter(|o| !o.is_empty() && !o.starts_with("http")),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
        let mut pills = div().flex().flex_wrap().gap_1p5();
        for key in &file.permissions {
            if let Some((_, title, _)) = pbfile::PERMISSIONS.iter().find(|p| p.0 == key) {
                let risky = key != "read";
                pills = pills.child(self.pill(
                    Some(if risky {
                        IconName::ShieldAlert
                    } else {
                        IconName::Eye
                    }),
                    title.to_string(),
                    if risky { c_amber() } else { c_green() },
                ));
            }
        }
        let mine = self.installed_match(&install.source, found);
        let changed = match (&status, mine) {
            (Status::Update(_), Some(a)) => changes(a, file),
            _ => Vec::new(),
        };
        let edited = matches!(status, Status::Update(_)) && mine.is_some_and(|a| a.edited_here());
        let inputs = pbfile::inputs_of(&file.inputs, &file.prompt);
        if !inputs.is_empty() {
            let names: Vec<String> = inputs.iter().map(|i| i.title()).collect();
            pills = pills.child(self.pill(
                Some(IconName::SquarePen),
                format!("Asks for {}", names.join(", ")),
                c_muted(),
            ));
        }
        div()
            .id(("pb-found", i))
            .p_3()
            .flex()
            .items_start()
            .gap_3()
            .rounded_xl()
            .border_1()
            .border_color(rgb(if on { c_blue() } else { c_border() }))
            .when(on, |el| el.bg(tint(c_selected())))
            .cursor_pointer()
            .hover(|el| el.bg(tint(c_subtle())))
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(install) = this.pb_install.as_mut() {
                    if install.chosen.contains(&i) {
                        install.chosen.retain(|c| *c != i);
                    } else {
                        install.chosen.push(i);
                    }
                }
                cx.notify();
            }))
            .child(icon(
                if on {
                    IconName::SquareCheck
                } else {
                    IconName::Square
                },
                15.,
                if on { c_blue() } else { c_muted() },
            ))
            .child(Self::tile(glyph, color, 32.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .v_flex()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(13.5))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(file.name.clone()),
                            )
                            .child(self.pill(None, tag, tag_color)),
                    )
                    .when(!byline.is_empty(), |el| {
                        el.child(
                            div()
                                .truncate()
                                .text_size(px(11.5))
                                .text_color(rgb(c_muted()))
                                .child(byline),
                        )
                    })
                    .when(!file.description.is_empty(), |el| {
                        el.child(
                            div()
                                .text_size(px(12.5))
                                .text_color(rgb(c_secondary()))
                                .child(file.description.clone()),
                        )
                    })
                    .child(pills)
                    .when(!changed.is_empty(), |el| {
                        el.child(
                            div()
                                .text_size(px(12.))
                                .text_color(rgb(c_secondary()))
                                .child(format!("Changes: {}", changed.join("; "))),
                        )
                    })
                    .when(edited, |el| {
                        el.child(div().text_size(px(12.)).text_color(rgb(c_amber())).child(
                            "You've edited this playbook here. Updating replaces your edits.",
                        ))
                    }),
            )
            .into_any_element()
    }

    /// A centered dialog over a dimmed backdrop; clicking outside runs `close`.
    fn modal(
        &self,
        id: &'static str,
        card: Stateful<Div>,
        cx: &mut Context<Self>,
        close: fn(&mut Self),
    ) -> AnyElement {
        let backdrop = if is_dark() {
            hsla(0., 0., 0., 0.55)
        } else {
            hsla(0., 0., 0., 0.3)
        };
        fade(
            div()
                .id(id)
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(backdrop)
                .occlude()
                .on_click(cx.listener(move |this, _, _, cx| {
                    close(this);
                    cx.notify();
                }))
                .child(rise(card, SharedString::from(format!("{id}-rise")), 260)),
            SharedString::from(format!("{id}-fade")),
            180,
        )
        .into_any_element()
    }

    fn tile(glyph: IconName, color: u32, size: f32) -> Div {
        Self::playbook_tile(glyph, color, size)
    }
}

#[cfg(test)]
mod tests {
    use super::changes;
    use crate::store;

    #[test]
    fn an_update_says_what_it_changes() {
        let a = store::Automation {
            prompt: "Check the build.\nReport back.".into(),
            permissions: vec!["read".into()],
            ..Default::default()
        };
        let mut file = a.to_file();
        assert!(changes(&a, &file).is_empty());
        file.permissions = vec!["read".into(), "shell".into()];
        file.prompt = "Check the build.\nFix what fails.\nReport back.".into();
        assert_eq!(
            changes(&a, &file),
            ["may now run commands", "prompt: 1 lines added"]
        );
        file.permissions.clear();
        assert!(changes(&a, &file).contains(&"may now edit files".to_string()));
    }

    #[test]
    fn edits_made_here_are_told_from_the_installed_copy() {
        let mut a = store::Automation {
            name: "Tidy".into(),
            prompt: "Tidy up.".into(),
            ..Default::default()
        };
        assert!(!a.edited_here(), "nothing to compare with");
        a.installed = a.fingerprint();
        a.version = "2".into();
        assert!(!a.edited_here(), "the version alone isn't an edit");
        a.prompt = "Tidy up, then test.".into();
        assert!(a.edited_here());
    }
}
