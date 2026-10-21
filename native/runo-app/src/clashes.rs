//! The clash page: when bringing updates in or merging stops because both sides changed
//! the same lines in the project folder, each spot shows both versions side by side to
//! keep one, the other or both, or an agent writes the file with both sides' intent.
//! Nothing is saved until Finish, and calling it off puts everything back.

use super::*;
use git::{Clash, ClashFile, Stretch, Undo};

/// The key for a whole-file pick, where a file has no lines to pick from or the agent's
/// version is used.
const WHOLE: usize = usize::MAX;

/// What to keep at one spot, or for a whole file.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Pick {
    Yours,
    Theirs,
    Both,
    /// The agent's version of the whole file.
    Agent,
}

/// An agent's whole-file answer to a clash, cleaned of any code fence around it.
fn clean_answer(text: &str) -> String {
    let t = text.trim();
    let t = t.strip_prefix("```").map_or(t, |rest| {
        let rest = rest.split_once('\n').map_or("", |(_, r)| r);
        rest.trim_end().strip_suffix("```").unwrap_or(rest)
    });
    let mut out = t.trim_end_matches('\n').to_string();
    out.push('\n');
    out
}

fn spots(n: usize) -> String {
    match n {
        1 => "1 spot".into(),
        n => format!("{n} spots"),
    }
}

/// Code lines in a clash card, tinted when `on`.
fn code(text: &str, color: u32, on: bool) -> Div {
    let mut block = div()
        .v_flex()
        .py(px(6.))
        .font_family("JetBrains Mono")
        .text_size(px(12.))
        .when(on, |el| el.bg(rgba((color << 8) | 0x1c)))
        .when(!on, |el| el.opacity(0.45));
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        block = block.child(
            div()
                .px_3()
                .h(px(20.))
                .text_color(rgb(c_muted()))
                .child("(nothing here)"),
        );
    }
    for line in lines {
        block = block.child(
            div()
                .px_3()
                .min_h(px(20.))
                .child(line.replace('\t', "    ")),
        );
    }
    block
}

impl RunoApp {
    fn pick_of(&self, path: &str, spot: usize) -> Option<Pick> {
        self.git_picks.get(&(path.to_string(), spot)).copied()
    }

    /// The file as the picks so far would leave it, once every spot has one.
    fn settled_text(&self, file: &ClashFile, yours_is_ours: bool) -> Option<String> {
        if self.pick_of(&file.path, WHOLE) == Some(Pick::Agent) {
            return self.git_agent_fix.get(&file.path).cloned().flatten();
        }
        let mut out = String::new();
        let mut spot = 0;
        for stretch in file.stretches.as_ref()? {
            match stretch {
                Stretch::Same(text) => out.push_str(text),
                Stretch::Spot { ours, theirs } => {
                    let (yours, incoming) = if yours_is_ours {
                        (ours, theirs)
                    } else {
                        (theirs, ours)
                    };
                    match self.pick_of(&file.path, spot)? {
                        Pick::Yours => out.push_str(yours),
                        Pick::Theirs => out.push_str(incoming),
                        Pick::Both | Pick::Agent => {
                            out.push_str(yours);
                            out.push_str(incoming);
                        }
                    }
                    spot += 1;
                }
            }
        }
        Some(out)
    }

    /// Spots in a file still without a pick.
    fn spots_left(&self, file: &ClashFile) -> usize {
        if self.pick_of(&file.path, WHOLE).is_some() {
            return 0;
        }
        match &file.stretches {
            None => 1,
            Some(_) => (0..file.spots())
                .filter(|i| self.pick_of(&file.path, *i).is_none())
                .count(),
        }
    }

    /// Settle every file as picked and carry on with the rebase or merge.
    fn finish_clash(&mut self, clash: &Clash) {
        let mut settled: Vec<(String, Result<String, bool>)> = Vec::new();
        let mut left = 0;
        for file in &clash.files {
            left += self.spots_left(file);
            if file.stretches.is_none() && self.pick_of(&file.path, WHOLE) != Some(Pick::Agent) {
                if let Some(pick) = self.pick_of(&file.path, WHOLE) {
                    let ours = (pick == Pick::Yours) == clash.yours_is_ours;
                    settled.push((file.path.clone(), Err(ours)));
                }
            } else if let Some(text) = self.settled_text(file, clash.yours_is_ours) {
                settled.push((file.path.clone(), Ok(text)));
            }
        }
        if left > 0 {
            self.fail(
                "git",
                format!("{} still need a pick before you can finish.", spots(left)),
            );
            return;
        }
        let (yours, theirs, kind) = (clash.yours.clone(), clash.theirs.clone(), clash.kind);
        self.git_picks.clear();
        self.git_agent_fix.clear();
        self.git_clash_file = None;
        self.git_step("Finishing…", move |dir| {
            let before = git::orig_head(dir);
            for (path, how) in &settled {
                match how {
                    Ok(text) => git::settle(dir, path, text)?,
                    Err(ours) => git::settle_side(dir, path, *ours)?,
                }
            }
            match git::continue_update(dir)? {
                git::Update::Conflicts(files) => Ok(Some((
                    format!(
                        "That commit is done. The next one clashes too, in {}. Pick again.",
                        files.join(", ")
                    ),
                    None,
                ))),
                _ => {
                    let note = if kind == "rebase" {
                        format!("Brought {theirs} into {yours}, keeping what you picked.")
                    } else {
                        format!("Merged {theirs} into {yours}, keeping what you picked.")
                    };
                    Ok(Some((
                        note,
                        before.map(|head| Undo::Folder {
                            branch: yours.clone(),
                            head,
                        }),
                    )))
                }
            }
        });
    }

    /// Ask the default agent to write a clashing file with both sides' intent.
    fn agent_settle(&mut self, clash: &Clash, file: &ClashFile, cx: &mut Context<Self>) {
        let Some(dir) = self.git_repo.as_ref().map(|r| r.dir.clone()) else {
            return;
        };
        let (first, second) = if clash.yours_is_ours {
            (
                format!("the person's own work on {}", clash.yours),
                format!("the incoming changes from {}", clash.theirs),
            )
        } else {
            (
                format!("the incoming changes from {}", clash.theirs),
                format!("the person's own work on {}", clash.yours),
            )
        };
        let prompt = format!(
            "This file has clashing changes, marked with git's <<<<<<<, ======= and >>>>>>> \
             lines. In each spot the first side is {first} and the second is {second}. \
             Write the whole file with every spot resolved so that what each side meant to do \
             is kept where both make sense together. Reply with only the file's full \
             contents: no markers, no code fences, no explanation.\n\nFile: {}\n\n{}",
            file.path, file.text
        );
        let st = cx.global::<AppSettings>();
        let provider = st.default_provider.clone();
        let configured = st.agent(&provider).binary;
        let command = agent::ask_command(&provider, &configured, &prompt);
        agent::oneshot(
            format!("resolve:{}", file.path),
            command,
            &dir,
            self.tx.clone(),
        );
        self.git_agent_fix.insert(file.path.clone(), None);
    }

    /// An agent's version of a clashing file, or why there isn't one.
    pub(crate) fn handle_resolve(&mut self, key: &str, result: Result<String, String>) {
        let Some(path) = key.strip_prefix("resolve:") else {
            return;
        };
        match result.map(|t| clean_answer(&t)) {
            Ok(text) if !text.trim().is_empty() && !text.contains("<<<<<<<") => {
                self.git_agent_fix.insert(path.to_string(), Some(text));
            }
            Ok(_) => {
                self.git_agent_fix.remove(path);
                self.fail(
                    "git",
                    "The agent's answer still had clashes in it. Pick by hand instead.".into(),
                );
            }
            Err(error) => {
                self.git_agent_fix.remove(path);
                let first = error.lines().next().unwrap_or("").to_string();
                self.fail("git", format!("The agent couldn't sort it out: {first}"));
            }
        }
    }

    /// The clash page, in place of the Git tabs while the folder has a clash.
    pub(crate) fn clash_view(&self, clash: &Clash, cx: &mut Context<Self>) -> AnyElement {
        let picked = self
            .git_clash_file
            .as_ref()
            .and_then(|p| clash.files.iter().find(|f| f.path == *p))
            .or_else(|| clash.files.first());
        let agent_name = agent::provider(&cx.global::<AppSettings>().default_provider).name;
        // The list of files.
        let mut list = div().v_flex().gap(px(2.));
        let label = |text: String| {
            div()
                .px_3()
                .pt(px(14.))
                .pb(px(6.))
                .text_size(px(10.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(c_muted()))
                .child(text)
        };
        if !clash.files.is_empty() {
            list = list.child(label(format!(
                "SAME LINES CHANGED  ·  {} FILE{}",
                clash.files.len(),
                if clash.files.len() == 1 { "" } else { "S" }
            )));
        }
        for (i, file) in clash.files.iter().enumerate() {
            let on = picked.is_some_and(|p| p.path == file.path);
            let left = self.spots_left(file);
            let (name, _) = file
                .path
                .rsplit_once('/')
                .map_or((file.path.as_str(), ""), |(d, n)| (n, d));
            let sub = if self.pick_of(&file.path, WHOLE) == Some(Pick::Agent) {
                format!("Using {agent_name}'s version")
            } else if left == 0 {
                "Ready".to_string()
            } else if file.stretches.is_none() {
                "Pick a side".to_string()
            } else {
                format!("{} left", spots(left))
            };
            let path = file.path.clone();
            list = list.child(
                div()
                    .id(("clash-file", i))
                    .relative()
                    .ml(px(10.))
                    .mr_1()
                    .px_2()
                    .py(px(7.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .cursor_pointer()
                    .when(on, |el| {
                        el.child(shell::row_fuse(format!("clash-fuse-{i}"), 4.))
                    })
                    .when(!on, |el| el.hover(|el| el.bg(tint(c_subtle()))))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.git_clash_file = Some(path.clone());
                        cx.notify();
                    }))
                    .child(
                        div()
                            .size(px(28.))
                            .flex_none()
                            .rounded_md()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(tint(c_subtle()))
                            .child(if left == 0 {
                                icon(IconName::Check, 13., c_green())
                            } else {
                                icon(IconName::TriangleAlert, 13., c_amber())
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .v_flex()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(name.to_string()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(11.5))
                                    .text_color(rgb(c_muted()))
                                    .child(sub),
                            ),
                    ),
            );
        }
        if !clash.merged.is_empty() {
            list = list.child(label(format!(
                "CAME TOGETHER ON THEIR OWN  ·  {}",
                clash.merged.len()
            )));
            for path in &clash.merged {
                list = list.child(
                    div()
                        .ml(px(10.))
                        .mr_1()
                        .px_2()
                        .h(px(30.))
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(12.5))
                        .text_color(rgb(c_secondary()))
                        .child(icon(IconName::File, 12., c_muted()))
                        .child(div().truncate().child(path.clone())),
                );
            }
        }
        let side = div()
            .w(px(280.))
            .flex_none()
            .h_full()
            .v_flex()
            .bg(pane(c_sheet()))
            .child(
                div()
                    .id("clash-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .pb(px(12.))
                    .child(list),
            );
        let headline = if clash.kind == "rebase" {
            format!("Bringing {} into {}", clash.theirs, clash.yours)
        } else {
            format!("Merging {} into {}", clash.theirs, clash.yours)
        };
        let n_files = clash.files.len();
        let sub = if n_files == 0 {
            "Every clash has a pick. Finish to carry on.".to_string()
        } else {
            format!(
                "Both sides changed the same lines in {} file{}. Pick what to keep. Nothing is saved until you finish.",
                n_files,
                if n_files == 1 { "" } else { "s" }
            )
        };
        let asking = picked.is_some_and(|f| {
            self.git_agent_fix
                .get(&f.path)
                .is_some_and(|fix| fix.is_none())
        });
        let mut banner = div()
            .p(px(14.))
            .flex()
            .items_center()
            .gap(px(12.))
            .rounded_lg()
            .border_1()
            .border_color(rgba((c_amber() << 8) | 0x77))
            .bg(rgba((c_amber() << 8) | 0x14))
            .child(
                div()
                    .size(px(32.))
                    .flex_none()
                    .rounded_md()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(rgba((c_amber() << 8) | 0x2a))
                    .child(icon(IconName::GitMerge, 15., c_amber())),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .v_flex()
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(px(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(headline),
                    )
                    .child(
                        div()
                            .text_size(px(12.5))
                            .text_color(rgb(c_secondary()))
                            .child(sub),
                    ),
            );
        if let Some(file) = picked.filter(|_| !asking) {
            let (clash_c, file_c) = (clash.clone(), file.clone());
            banner = banner.child(
                div()
                    .id("clash-agent")
                    .flex_none()
                    .h(px(30.))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .cursor_pointer()
                    .bg(rgb(c_blue()))
                    .text_color(rgb(0xffffff))
                    .text_size(px(12.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .hover(|el| el.opacity(0.9))
                    .child(icon(IconName::Sparkles, 12., 0xffffff))
                    .child(format!("Let {agent_name} sort it out"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.agent_settle(&clash_c, &file_c, cx);
                        cx.notify();
                    })),
            );
        }
        let body = picked.map(|file| self.clash_file_view(clash, file, agent_name, cx));
        let left: usize = clash.files.iter().map(|f| self.spots_left(f)).sum();
        let files_left = clash
            .files
            .iter()
            .filter(|f| self.spots_left(f) > 0)
            .count();
        let finish = clash.clone();
        let main = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .v_flex()
            .bg(glass_fill(c_canvas()))
            .child(
                div()
                    .id("clash-page")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(20.))
                    .py(px(18.))
                    .v_flex()
                    .gap(px(16.))
                    .child(banner)
                    .children(body),
            )
            .child(
                div()
                    .h(px(56.))
                    .flex_none()
                    .px(px(20.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .border_t_1()
                    .border_color(divider())
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(12.5))
                            .text_color(rgb(c_muted()))
                            .child(if left == 0 {
                                "Everything has a pick.".to_string()
                            } else {
                                format!(
                                    "{} left in {} file{}",
                                    spots(left),
                                    files_left,
                                    if files_left == 1 { "" } else { "s" }
                                )
                            }),
                    )
                    .child(
                        div()
                            .id("clash-call-off")
                            .h(px(32.))
                            .px_3()
                            .flex()
                            .items_center()
                            .rounded_md()
                            .border_1()
                            .border_color(divider())
                            .cursor_pointer()
                            .text_size(px(12.5))
                            .hover(|el| el.bg(tint(c_subtle())))
                            .child("Call it off and put everything back")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.git_picks.clear();
                                this.git_agent_fix.clear();
                                this.git_clash_file = None;
                                this.git_step("Calling it off…", |dir| {
                                    git::abort_update(dir)?;
                                    Ok(Some((
                                        "Called it off. Everything is back as it was.".into(),
                                        None,
                                    )))
                                });
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .id("clash-finish")
                            .h(px(32.))
                            .px_4()
                            .flex()
                            .items_center()
                            .rounded_md()
                            .cursor_pointer()
                            .bg(rgb(c_blue()))
                            .text_color(rgb(0xffffff))
                            .text_size(px(12.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .when(left > 0, |el| el.opacity(0.5))
                            .hover(|el| el.opacity(0.9))
                            .child("Finish")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.finish_clash(&finish);
                                cx.notify();
                            })),
                    ),
            );
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .child(side)
            .child(main)
            .into_any_element()
    }

    fn clash_file_view(
        &self,
        clash: &Clash,
        file: &ClashFile,
        agent_name: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let path = file.path.clone();
        let mut out = div().v_flex().gap(px(14.)).child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_size(px(14.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(file.path.clone()),
                )
                .when(file.spots() > 0, |el| {
                    el.child(
                        div()
                            .text_size(px(12.5))
                            .text_color(rgb(c_muted()))
                            .child(format!("·  {}", spots(file.spots()))),
                    )
                }),
        );
        // What the agent wrote, to use or set aside.
        match self.git_agent_fix.get(&file.path) {
            Some(None) => {
                out = out.child(
                    div()
                        .p_3()
                        .rounded_lg()
                        .border_1()
                        .border_color(divider())
                        .text_size(px(12.5))
                        .text_color(rgb(c_muted()))
                        .child(format!("{agent_name} is reading both sides…")),
                );
            }
            Some(Some(text)) => {
                let using = self.pick_of(&file.path, WHOLE) == Some(Pick::Agent);
                let (use_path, drop_path) = (path.clone(), path.clone());
                out = out.child(
                    div()
                        .rounded_lg()
                        .border_1()
                        .border_color(rgba((c_blue() << 8) | 0x66))
                        .overflow_hidden()
                        .child(
                            div()
                                .h(px(40.))
                                .px_3()
                                .flex()
                                .items_center()
                                .gap_2()
                                .bg(rgba((c_blue() << 8) | 0x14))
                                .child(icon(IconName::Sparkles, 12., c_blue()))
                                .child(
                                    div()
                                        .flex_1()
                                        .text_size(px(12.5))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(format!("{agent_name}'s version of the whole file")),
                                )
                                .when(!using, |el| {
                                    el.child(
                                        div()
                                            .id("clash-agent-use")
                                            .h(px(26.))
                                            .px_3()
                                            .flex()
                                            .items_center()
                                            .rounded_md()
                                            .cursor_pointer()
                                            .bg(rgb(c_blue()))
                                            .text_color(rgb(0xffffff))
                                            .text_size(px(12.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child("Use it")
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.git_picks
                                                    .insert((use_path.clone(), WHOLE), Pick::Agent);
                                                cx.notify();
                                            })),
                                    )
                                })
                                .child(
                                    div()
                                        .id("clash-agent-drop")
                                        .h(px(26.))
                                        .px_3()
                                        .flex()
                                        .items_center()
                                        .rounded_md()
                                        .border_1()
                                        .border_color(divider())
                                        .cursor_pointer()
                                        .text_size(px(12.))
                                        .hover(|el| el.bg(tint(c_subtle())))
                                        .child("Pick by hand")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.git_agent_fix.remove(&drop_path);
                                            this.git_picks.remove(&(drop_path.clone(), WHOLE));
                                            cx.notify();
                                        })),
                                ),
                        )
                        .child(
                            div()
                                .id("clash-agent-text")
                                .max_h(px(360.))
                                .overflow_y_scroll()
                                .child(code(text, c_blue(), true)),
                        ),
                );
                if using {
                    return out.into_any_element();
                }
            }
            None => {}
        }
        let button = |id: SharedString, label: &'static str, on: bool, pick: Pick, spot: usize| {
            let path = path.clone();
            div()
                .id(id)
                .h(px(30.))
                .px_3()
                .flex()
                .items_center()
                .rounded_md()
                .cursor_pointer()
                .text_size(px(12.5))
                .when(on, |el| {
                    el.bg(rgb(c_blue()))
                        .text_color(rgb(0xffffff))
                        .font_weight(FontWeight::SEMIBOLD)
                })
                .when(!on, |el| {
                    el.border_1()
                        .border_color(divider())
                        .hover(|el| el.bg(tint(c_subtle())))
                })
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.git_picks.insert((path.clone(), spot), pick);
                    cx.notify();
                }))
        };
        let Some(stretches) = &file.stretches else {
            // No lines to pick from: one side's whole file wins.
            let pick = self.pick_of(&file.path, WHOLE);
            return out
                .child(
                    div()
                        .text_size(px(12.5))
                        .text_color(rgb(c_secondary()))
                        .child("This file can't be compared line by line: it isn't text, or one side deleted it. Keep one side's version."),
                )
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(button(
                            "clash-whole-yours".into(),
                            "Keep yours",
                            pick == Some(Pick::Yours),
                            Pick::Yours,
                            WHOLE,
                        ))
                        .child(button(
                            "clash-whole-theirs".into(),
                            "Keep theirs",
                            pick == Some(Pick::Theirs),
                            Pick::Theirs,
                            WHOLE,
                        )),
                )
                .into_any_element();
        };
        let mut spot = 0;
        for stretch in stretches {
            let Stretch::Spot { ours, theirs } = stretch else {
                continue;
            };
            let (yours, incoming) = if clash.yours_is_ours {
                (ours, theirs)
            } else {
                (theirs, ours)
            };
            let pick = self.pick_of(&file.path, spot);
            let keep_yours = matches!(pick, None | Some(Pick::Yours | Pick::Both));
            let keep_theirs = matches!(pick, None | Some(Pick::Theirs | Pick::Both));
            let column = |who: &'static str, name: String, text: &str, on: bool| {
                div()
                    .flex_1()
                    .min_w_0()
                    .v_flex()
                    .child(
                        div()
                            .h(px(34.))
                            .px_3()
                            .flex()
                            .items_center()
                            .gap_2()
                            .bg(tint(c_subtle()))
                            .text_size(px(12.5))
                            .child(div().font_weight(FontWeight::SEMIBOLD).child(who))
                            .child(
                                div()
                                    .truncate()
                                    .text_color(rgb(c_muted()))
                                    .child(format!("·  {name}")),
                            ),
                    )
                    .child(code(text, c_add(), on))
            };
            out = out.child(
                div()
                    .v_flex()
                    .gap(px(8.))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(rgb(c_muted()))
                            .child(format!("SPOT {} OF {}", spot + 1, file.spots())),
                    )
                    .child(
                        div()
                            .flex()
                            .rounded_lg()
                            .border_1()
                            .border_color(divider())
                            .overflow_hidden()
                            .child(column("Yours", clash.yours.clone(), yours, keep_yours))
                            .child(div().w(px(1.)).bg(divider()))
                            .child(column(
                                "Theirs",
                                clash.theirs.clone(),
                                incoming,
                                keep_theirs,
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(button(
                                format!("clash-yours-{spot}").into(),
                                "Keep yours",
                                pick == Some(Pick::Yours),
                                Pick::Yours,
                                spot,
                            ))
                            .child(button(
                                format!("clash-theirs-{spot}").into(),
                                "Keep theirs",
                                pick == Some(Pick::Theirs),
                                Pick::Theirs,
                                spot,
                            ))
                            .child(button(
                                format!("clash-both-{spot}").into(),
                                "Keep both",
                                pick == Some(Pick::Both),
                                Pick::Both,
                                spot,
                            )),
                    ),
            );
            spot += 1;
        }
        out.into_any_element()
    }
}
