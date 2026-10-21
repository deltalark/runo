//! Make it reviewable: before a branch goes out, an agent writes a reviewer's guide
//! (what changed, the order to read it in, where the risk is, how it was tested) and can
//! split oversized work into a stack of small branches, each built on the one before,
//! opened as pull requests in order. The original branch is never changed.

use super::*;
use branches::git_term;

/// Lines a review can take comfortably, and the size past which reviews stall.
const SMALL: usize = 200;
const LARGE: usize = 400;

/// One part of a planned split.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SplitPart {
    pub title: String,
    pub why: String,
    pub files: Vec<String>,
}

/// Read an agent's split plan: JSON with parts in order, each naming its files. Files it
/// left out go in the last part, files it made up are dropped, and empty parts go.
fn parse_split(text: &str, known: &[String]) -> Option<Vec<SplitPart>> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    let v: serde_json::Value = serde_json::from_str(text.get(start..=end)?).ok()?;
    let mut seen: Vec<String> = Vec::new();
    let mut parts: Vec<SplitPart> = Vec::new();
    for p in v["parts"].as_array()? {
        let mut files = Vec::new();
        for f in p["files"].as_array().into_iter().flatten() {
            let Some(f) = f.as_str() else { continue };
            if known.iter().any(|k| k == f) && !seen.iter().any(|s| s == f) {
                seen.push(f.to_string());
                files.push(f.to_string());
            }
        }
        if !files.is_empty() {
            parts.push(SplitPart {
                title: p["title"].as_str().unwrap_or("").trim().to_string(),
                why: p["why"].as_str().unwrap_or("").trim().to_string(),
                files,
            });
        }
    }
    let left: Vec<String> = known
        .iter()
        .filter(|k| !seen.contains(k))
        .cloned()
        .collect();
    if let Some(last) = parts.last_mut() {
        last.files.extend(left);
    }
    for (i, p) in parts.iter_mut().enumerate() {
        if p.title.is_empty() {
            p.title = format!("Part {}", i + 1);
        }
    }
    (parts.len() > 1).then_some(parts)
}

fn clean_guide(text: &str) -> String {
    let t = text.trim();
    let t = t
        .strip_prefix("```markdown")
        .or_else(|| t.strip_prefix("```"))
        .map_or(t, |r| r.trim_end().strip_suffix("```").unwrap_or(r));
    t.trim().to_string()
}

impl RunoApp {
    /// Ask the default agent something about a branch; the answer comes back by `key`.
    fn ask_about_branch(&mut self, key: String, prompt: String, cx: &mut Context<Self>) {
        let Some(dir) = self.git_place_dir() else {
            return;
        };
        let st = cx.global::<AppSettings>();
        let provider = st.default_provider.clone();
        let configured = st.agent(&provider).binary;
        let command = agent::ask_command(&provider, &configured, &prompt);
        agent::oneshot(key, command, &dir, self.tx.clone());
    }

    fn write_guide(&mut self, branch: String, cx: &mut Context<Self>) {
        let Some(dir) = self.git_place_dir() else {
            return;
        };
        let base = git::base_ref(&dir, &git::default_branch(&dir));
        let patch = git::branch_patch(&dir, &branch, &base, 24_000);
        // An agent session on this branch may have had its work checked after each turn.
        let checks = self
            .state
            .sessions
            .iter()
            .filter(|s| s.branch == branch)
            .find_map(|s| s.proof.as_ref())
            .map(crate::proof::tested_note)
            .filter(|note| !note.is_empty())
            .map(|note| format!("{note}\n\n"))
            .unwrap_or_default();
        let prompt = format!(
            "Write a reviewer's guide for this branch, in Markdown, under 250 words, with \
             exactly these headings:\n## What changed\n(two to four plain sentences)\n\
             ## Read in this order\n(a numbered list of files, one line each on what to look \
             for)\n## Where the risk is\n(short bullets)\n## How it was tested\n(the checks \
             Runo ran, when given below, and what the commits and diff show; say plainly if \
             nothing shows tests)\nReply with only the guide, no preamble.\n\n{checks}{patch}"
        );
        self.ask_about_branch(format!("guide:{branch}"), prompt, cx);
        self.git_guides.insert(branch, None);
    }

    fn plan_split(&mut self, branch: String, files: &[git::Change], cx: &mut Context<Self>) {
        let Some(dir) = self.git_place_dir() else {
            return;
        };
        let base = git::base_ref(&dir, &git::default_branch(&dir));
        let patch = git::branch_patch(&dir, &branch, &base, 24_000);
        let list: Vec<String> = files
            .iter()
            .map(|f| format!("- {} (+{} −{})", f.path, f.added, f.removed))
            .collect();
        let prompt = format!(
            "Split this branch's changes into a stack of small pull requests that build on \
             each other, so each is easy to review: about {SMALL} changed lines or fewer each \
             where possible, in an order where each part makes sense on its own (foundations \
             first, then what uses them). Every file goes in exactly one part. Reply with only \
             JSON like {{\"parts\": [{{\"title\": \"short PR title\", \"why\": \"one sentence \
             for the reviewer\", \"files\": [\"path\"]}}]}}.\n\nFiles:\n{}\n\n{patch}",
            list.join("\n")
        );
        self.ask_about_branch(format!("split:{branch}"), prompt, cx);
        self.git_splits.insert(branch, None);
    }

    /// The agent's guide or split plan for a branch, or why there isn't one.
    pub(crate) fn handle_reviewable(&mut self, key: &str, result: Result<String, String>) {
        if let Some(branch) = key.strip_prefix("guide:") {
            match result.map(|t| clean_guide(&t)) {
                Ok(text) if !text.is_empty() => {
                    self.git_guides.insert(branch.to_string(), Some(text));
                }
                other => {
                    self.git_guides.remove(branch);
                    let why = other.err().unwrap_or_default();
                    let first = why
                        .lines()
                        .next()
                        .unwrap_or("it came back empty")
                        .to_string();
                    self.fail("git", format!("Couldn't write the guide: {first}"));
                }
            }
        } else if let Some(branch) = key.strip_prefix("split:") {
            let known: Vec<String> = self
                .git_branch_detail
                .as_ref()
                .filter(|d| d.name == branch)
                .map(|d| d.files.iter().map(|f| f.path.clone()).collect())
                .unwrap_or_default();
            match result.ok().and_then(|t| parse_split(&t, &known)) {
                Some(parts) => {
                    self.git_splits.insert(branch.to_string(), Some(parts));
                }
                None => {
                    self.git_splits.remove(branch);
                    self.fail(
                        "git",
                        "The agent didn't find a good way to split it. It may read best as one."
                            .into(),
                    );
                }
            }
        }
    }

    /// Make the planned stack of branches.
    fn make_stack(&mut self, branch: String, parts: Vec<SplitPart>) {
        self.git_step("Making the branches…", move |dir| {
            let base = git::base_ref(dir, &git::default_branch(dir));
            let names: Vec<String> = (1..=parts.len())
                .map(|i| format!("{branch}-part-{i}"))
                .collect();
            let undo =
                git::branches_point(dir, &names.iter().map(String::as_str).collect::<Vec<_>>());
            let plan: Vec<(String, Vec<String>)> = parts
                .iter()
                .map(|p| (p.title.clone(), p.files.clone()))
                .collect();
            git::stack_parts(dir, &base, &branch, &branch, &plan)?;
            Ok(Some((
                format!(
                    "Made {} branches, {}-part-1 to {}-part-{}. {branch} is unchanged.",
                    names.len(),
                    branch,
                    branch,
                    names.len()
                ),
                Some(undo),
            )))
        });
    }

    /// Send the stack and open its pull requests, each onto the one below.
    pub(crate) fn open_stack(&mut self, branch: String, parts: Vec<SplitPart>) {
        if !self.belt_clear(&branch, explain::Fix::OpenStackAnyway(branch.clone())) {
            return;
        }
        let remote = self
            .git_repo
            .as_ref()
            .map(|r| r.state.remote.clone())
            .unwrap_or_default();
        if remote.is_empty() {
            self.fail("git", "This folder isn't connected to GitHub yet.".into());
            return;
        }
        self.git_step("Opening pull requests…", move |dir| {
            let base = git::default_branch(dir);
            let n = parts.len();
            let mut below = base.clone();
            for (i, part) in parts.iter().enumerate() {
                let name = format!("{branch}-part-{}", i + 1);
                git::push_branch_over(dir, &remote, &name)?;
                let body = format!(
                    "{}\n\nPart {} of {n} of {branch}, stacked: each builds on the one before, so review and merge them in order.",
                    part.why,
                    i + 1
                );
                git::open_pull_request(dir, &name, &below, &part.title, &body)?;
                below = name;
            }
            Ok(Some((format!("Opened {n} pull requests, stacked in order."), None)))
        });
    }

    /// The review kit for a branch, in place of its page.
    pub(crate) fn kit_page(&self, row: &git::BranchRow, cx: &mut Context<Self>) -> AnyElement {
        let branch = row.name.clone();
        let detail = self.git_branch_detail.as_ref().filter(|d| d.name == branch);
        let files: Vec<git::Change> = detail.map(|d| d.files.clone()).unwrap_or_default();
        let (added, removed) = files
            .iter()
            .fold((0, 0), |a, f| (a.0 + f.added, a.1 + f.removed));
        let total = added + removed;
        let (verdict, color, advice) = if total <= SMALL {
            (
                "Easy to review",
                c_green(),
                "A reviewer's guide is all it needs.",
            )
        } else if total <= LARGE {
            (
                "On the large side",
                c_amber(),
                "A guide helps; splitting it would help more.",
            )
        } else {
            (
                "Too big for one review",
                c_red(),
                "Reviews this size wait many times longer and get skimmed. Split it into a stack.",
            )
        };
        let card = |title: &'static str, term: &'static str, body: Div| {
            div()
                .mt(px(18.))
                .p(px(16.))
                .v_flex()
                .gap(px(8.))
                .rounded_lg()
                .bg(pane(c_sheet()))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .text_size(px(14.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(title),
                        )
                        .when(!term.is_empty(), |el| el.child(git_term(term))),
                )
                .child(body)
        };
        let button = |id: &'static str, label: String, primary: bool| {
            div()
                .id(id)
                .flex_none()
                .self_start()
                .h(px(30.))
                .px_3()
                .flex()
                .items_center()
                .gap_2()
                .rounded_md()
                .cursor_pointer()
                .text_size(px(12.5))
                .when(primary, |el| {
                    el.bg(rgb(c_blue()))
                        .text_color(rgb(0xffffff))
                        .font_weight(FontWeight::SEMIBOLD)
                        .hover(|el| el.opacity(0.9))
                })
                .when(!primary, |el| {
                    el.border_1()
                        .border_color(divider())
                        .hover(|el| el.bg(tint(c_subtle())))
                })
                .child(label)
        };
        // The guide.
        let guide_body = match self.git_guides.get(&branch) {
            Some(Some(text)) => {
                let copy = text.clone();
                let again = branch.clone();
                div()
                    .v_flex()
                    .gap_2()
                    .child(
                        div()
                            .p(px(12.))
                            .rounded_md()
                            .bg(glass_fill(c_canvas()))
                            .text_size(px(12.5))
                            .line_height(px(19.))
                            .child(TextView::markdown("git-guide", text.clone()).selectable(true)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(button("git-guide-copy", "Copy".into(), false).on_click(
                                cx.listener(move |_, _, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()));
                                }),
                            ))
                            .child(button("git-guide-again", "Write it again".into(), false).on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.write_guide(again.clone(), cx);
                                    cx.notify();
                                }),
                            ))
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .text_color(rgb(c_muted()))
                                    .child("Opening a pull request from Changes starts its description with this."),
                            ),
                    )
            }
            Some(None) => div()
                .text_size(px(12.5))
                .text_color(rgb(c_muted()))
                .child("The agent is reading the branch…"),
            None => {
                let b = branch.clone();
                div()
                    .v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(12.5))
                            .text_color(rgb(c_secondary()))
                            .child("What changed, the order to read it in, where the risk is, and how it was tested. Reviewers start in the right place and finish sooner."),
                    )
                    .child(button("git-guide-write", "Write the guide".into(), true).on_click(
                        cx.listener(move |this, _, _, cx| {
                            this.write_guide(b.clone(), cx);
                            cx.notify();
                        }),
                    ))
            }
        };
        // The split.
        let split_body = match self.git_splits.get(&branch) {
            Some(Some(parts)) => {
                let made = self
                    .git_branches
                    .as_ref()
                    .is_some_and(|b| b.rows.iter().any(|r| r.name == format!("{branch}-part-1")));
                let mut list = div().v_flex().gap(px(6.));
                for (i, part) in parts.iter().enumerate() {
                    let lines: usize = part
                        .files
                        .iter()
                        .filter_map(|p| files.iter().find(|f| f.path == *p))
                        .map(|f| f.added + f.removed)
                        .sum();
                    list = list.child(
                        div()
                            .p(px(10.))
                            .flex()
                            .gap(px(10.))
                            .rounded_md()
                            .bg(glass_fill(c_canvas()))
                            .child(
                                div()
                                    .size(px(22.))
                                    .flex_none()
                                    .rounded_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .bg(tint(c_subtle()))
                                    .text_size(px(11.5))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child((i + 1).to_string()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .v_flex()
                                    .gap(px(2.))
                                    .child(
                                        div()
                                            .flex()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .truncate()
                                                    .text_size(px(13.))
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .child(part.title.clone()),
                                            )
                                            .child(
                                                div()
                                                    .text_size(px(11.5))
                                                    .text_color(rgb(if lines > SMALL {
                                                        c_amber()
                                                    } else {
                                                        c_muted()
                                                    }))
                                                    .child(format!("{lines} lines")),
                                            ),
                                    )
                                    .when(!part.why.is_empty(), |el| {
                                        el.child(
                                            div()
                                                .text_size(px(12.))
                                                .text_color(rgb(c_secondary()))
                                                .child(part.why.clone()),
                                        )
                                    })
                                    .child(
                                        div()
                                            .text_size(px(11.5))
                                            .text_color(rgb(c_muted()))
                                            .child(part.files.join(", ")),
                                    ),
                            ),
                    );
                }
                let (b1, p1, b2, p2, b3) = (
                    branch.clone(),
                    parts.clone(),
                    branch.clone(),
                    parts.clone(),
                    branch.clone(),
                );
                div().v_flex().gap_2().child(list).child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .when(!made, |el| {
                            el.child(
                                button(
                                    "git-split-make",
                                    format!("Make {} branches", parts.len()),
                                    true,
                                )
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.make_stack(b1.clone(), p1.clone());
                                        cx.notify();
                                    },
                                )),
                            )
                        })
                        .when(made, |el| {
                            el.child(
                                button(
                                    "git-split-open",
                                    format!("Send and open {} pull requests", parts.len()),
                                    true,
                                )
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.open_stack(b2.clone(), p2.clone());
                                        cx.notify();
                                    },
                                )),
                            )
                        })
                        .child(
                            button("git-split-again", "Plan again".into(), false).on_click(
                                cx.listener(move |this, _, _, cx| {
                                    let files = this
                                        .git_branch_detail
                                        .as_ref()
                                        .map(|d| d.files.clone())
                                        .unwrap_or_default();
                                    this.plan_split(b3.clone(), &files, cx);
                                    cx.notify();
                                }),
                            ),
                        )
                        .child(div().text_size(px(11.5)).text_color(rgb(c_muted())).child(
                            if made {
                                "Each pull request goes onto the one below it."
                            } else {
                                "New branches, each built on the one before. Yours stays as it is."
                            },
                        )),
                )
            }
            Some(None) => div()
                .text_size(px(12.5))
                .text_color(rgb(c_muted()))
                .child("The agent is planning the parts…"),
            None => {
                let b = branch.clone();
                let fs = files.clone();
                div()
                    .v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(12.5))
                            .text_color(rgb(c_secondary()))
                            .child(format!("Small pull requests that build on each other, about {SMALL} lines each, reviewed and merged in order.")),
                    )
                    .when(files.len() > 1, |el| {
                        el.child(
                            button("git-split-plan", "Plan a split".into(), total > LARGE)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.plan_split(b.clone(), &fs, cx);
                                    cx.notify();
                                })),
                        )
                    })
                    .when(files.len() <= 1, |el| {
                        el.child(
                            div()
                                .text_size(px(12.))
                                .text_color(rgb(c_muted()))
                                .child("It changes one file, so there's nothing to split."),
                        )
                    })
            }
        };
        let back = branch.clone();
        div()
            .id("git-kit")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .px(px(24.))
            .py(px(20.))
            .v_flex()
            .child(
                div()
                    .id("git-kit-back")
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_size(px(12.))
                    .text_color(rgb(c_muted()))
                    .cursor_pointer()
                    .hover(|el| el.text_color(rgb(c_text())))
                    .child(icon(IconName::ChevronLeft, 12., c_muted()))
                    .child(format!("Back to {back}"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.git_kit = None;
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .mt_2()
                    .text_size(px(18.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(format!("Make {branch} easy to review")),
            )
            .child(
                div()
                    .mt_3()
                    .p(px(12.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .rounded_lg()
                    .border_1()
                    .border_color(rgba((color << 8) | 0x66))
                    .bg(rgba((color << 8) | 0x14))
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(color))
                            .child(verdict),
                    )
                    .child(
                        div()
                            .text_size(px(12.5))
                            .text_color(rgb(c_secondary()))
                            .child(format!(
                                "{} file{}, {total} lines (+{added} −{removed}). {advice}",
                                files.len(),
                                if files.len() == 1 { "" } else { "s" }
                            )),
                    ),
            )
            .child(card("Reviewer's guide", "", guide_body))
            .child(card(
                "Split into smaller pull requests",
                "stacked PRs",
                split_body,
            ))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{clean_guide, parse_split};

    #[test]
    fn split_plans_keep_every_file_once() {
        let known: Vec<String> = ["a.rs", "b.rs", "c.rs"].map(String::from).to_vec();
        let text = "Here: {\"parts\": [{\"title\": \"Base\", \"why\": \"w\", \"files\": [\"a.rs\", \"x.rs\"]}, {\"title\": \"\", \"files\": [\"b.rs\", \"a.rs\"]}]}";
        let parts = parse_split(text, &known).unwrap();
        assert_eq!(parts[0].files, vec!["a.rs"]);
        assert_eq!(parts[1].files, vec!["b.rs", "c.rs"]);
        assert_eq!(parts[1].title, "Part 2");
        assert!(parse_split("{\"parts\": [{\"files\": [\"a.rs\"]}]}", &known).is_none());
        assert_eq!(
            clean_guide("```markdown\n## What changed\nx\n```"),
            "## What changed\nx"
        );
    }
}
