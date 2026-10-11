//! App-wide popups: permission requests, questions and plans from any session, and
//! agent CLI updates. They stack in the corner so the person can answer a session
//! without opening it.

use super::*;

/// How often installed agents are checked for a newer release.
const UPDATE_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

/// A newer release of an installed agent CLI.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AgentUpdate {
    pub provider: &'static str,
    pub current: String,
    pub latest: String,
}

/// An agent update while it runs, and briefly after it succeeds.
pub(crate) struct UpdateRun {
    pub started: std::time::Instant,
    /// The newest line the updater printed.
    pub line: String,
    /// A percentage the updater printed itself, when it prints one.
    pub percent: Option<f32>,
    /// How far the updater's own messages say it got (downloading, installing, done).
    pub phase: f32,
    pub finished: Option<std::time::Instant>,
}

/// How long the success badge stays before the card hides itself.
pub(crate) const UPDATE_DONE_SHOWN: Duration = Duration::from_millis(2600);

impl UpdateRun {
    /// Fraction done for the bar: the updater's own percentage when it prints one, else
    /// the furthest phase it reported, eased forward by elapsed time so it never stalls.
    pub(crate) fn progress(&self, now: std::time::Instant) -> f32 {
        if self.finished.is_some() {
            return 1.;
        }
        if let Some(p) = self.percent {
            return (p / 100.).clamp(0.02, 0.98);
        }
        let t = now.duration_since(self.started).as_secs_f32();
        let timed = 0.85 * (1. - (-t / 25.).exp());
        timed.max(self.phase).clamp(0.03, 0.97)
    }

    fn take_line(&mut self, line: &str) {
        if let Some(p) = percent_of(line) {
            self.percent = Some(p);
        }
        if let Some(phase) = phase_of(line) {
            self.phase = self.phase.max(phase);
        }
        self.line = line.chars().take(160).collect();
    }
}

/// A percentage printed in a progress line, like "Downloading 42%" or "[####] 42.5 %".
fn percent_of(line: &str) -> Option<f32> {
    let at = line.rfind('%')?;
    let digits: String = line[..at]
        .trim_end()
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    digits
        .parse::<f32>()
        .ok()
        .filter(|p| (0. ..=100.).contains(p))
}

/// How far along an updater is, from the words in its output.
fn phase_of(line: &str) -> Option<f32> {
    let l = line.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| l.contains(w));
    if has(&[
        "added ",
        "changed ",
        "updated",
        "successfully",
        "up to date",
        "complete",
    ]) {
        Some(0.95)
    } else if has(&["verif", "linking", "postinstall", "reify"]) {
        Some(0.8)
    } else if has(&["extract", "unpack", "installing"]) {
        Some(0.6)
    } else if has(&["download", "fetch", "get 200", "get 304"]) {
        Some(0.35)
    } else {
        None
    }
}

/// A line of updater output without colour codes or spinner characters.
fn clean_line(raw: &str) -> String {
    let mut out = String::new();
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip an escape sequence: ESC [ ... final letter, or ESC and one character.
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            } else {
                chars.next();
            }
        } else if !c.is_control() && !('\u{2800}'..='\u{28ff}').contains(&c) {
            out.push(c);
        }
    }
    out.trim().to_string()
}

/// Run an updater, sending each line it prints as `<key>-line` and the result as `<key>`.
fn run_streaming(key: String, mut command: std::process::Command, tx: Sender<Event>) {
    use std::io::Read;
    use std::process::Stdio;
    command
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    std::thread::spawn(move || {
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                let _ = tx.send(Event::Output(
                    key,
                    Err(format!(
                        "could not start {:?}: {error}",
                        command.get_program()
                    )),
                ));
                return;
            }
        };
        let tail = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let mut readers = Vec::new();
        let streams: Vec<Box<dyn Read + Send>> = [
            child
                .stdout
                .take()
                .map(|s| Box::new(s) as Box<dyn Read + Send>),
            child
                .stderr
                .take()
                .map(|s| Box::new(s) as Box<dyn Read + Send>),
        ]
        .into_iter()
        .flatten()
        .collect();
        for mut stream in streams {
            let (tx, tail, line_key) = (tx.clone(), tail.clone(), format!("{key}-line"));
            readers.push(std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                let mut pending = Vec::new();
                let emit = |bytes: &[u8]| {
                    let line = clean_line(&String::from_utf8_lossy(bytes));
                    if line.is_empty() {
                        return;
                    }
                    if let Ok(mut tail) = tail.lock() {
                        tail.push(line.clone());
                        let extra = tail.len().saturating_sub(30);
                        tail.drain(..extra);
                    }
                    let _ = tx.send(Event::Output(line_key.clone(), Ok(line)));
                };
                while let Ok(n) = stream.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    for &b in &buf[..n] {
                        // Progress bars redraw with \r, so treat it as a line end too.
                        if b == b'\n' || b == b'\r' {
                            emit(&pending);
                            pending.clear();
                        } else {
                            pending.push(b);
                        }
                    }
                }
                emit(&pending);
            }));
        }
        for reader in readers {
            let _ = reader.join();
        }
        let ok = child.wait().is_ok_and(|status| status.success());
        let text = tail.lock().map(|t| t.join("\n")).unwrap_or_default();
        let _ = tx.send(Event::Output(key, if ok { Ok(text) } else { Err(text) }));
    });
}

/// The green "Updated" pill shown when an update finishes.
fn update_done_badge(n: usize) -> AnyElement {
    let pill = div()
        .flex()
        .items_center()
        .gap_1()
        .px_2()
        .h(px(24.))
        .rounded_full()
        .bg(rgb(settings::mix(c_green(), c_raised(), 0.18)))
        .border_1()
        .border_color(rgb(settings::mix(c_green(), c_border(), 0.5)))
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(c_green()))
        .child(icon(IconName::Check, 12., c_green()))
        .child("Updated");
    pill.animated(
        ("up-done", n),
        Animation::new(Duration::from_millis(420)).with_easing(ease_out_quint()),
        |el, d| el.opacity(d).ml(px(8. * (1. - d))),
    )
}

/// Something a session is waiting on the person for.
pub(crate) enum Waiting {
    Denied(Vec<String>),
    Question(store::Question),
    Plan,
}

/// The npm package each agent CLI ships as, for looking up its latest release.
pub(crate) fn npm_package(provider: &str) -> Option<&'static str> {
    Some(match provider {
        "claude" => "@anthropic-ai/claude-code",
        "codex" => "@openai/codex",
        "opencode" => "opencode-ai",
        "gemini" => "@google/gemini-cli",
        "copilot" => "@github/copilot",
        "amp" => "@sourcegraph/amp",
        "qwen" => "@qwen-code/qwen-code",
        _ => return None,
    })
}

/// The command that updates an agent: its own updater when it has one, else npm.
pub(crate) fn update_command(provider: &str, bin: &str) -> Option<Vec<String>> {
    let own = |args: &[&str]| {
        std::iter::once(bin.to_string())
            .chain(args.iter().map(|a| a.to_string()))
            .collect()
    };
    Some(match provider {
        "claude" => own(&["update"]),
        "opencode" => own(&["upgrade"]),
        "cursor" => own(&["update"]),
        _ => {
            let package = npm_package(provider)?;
            // `--loglevel http` makes npm print each download as it happens, so the
            // update card has live progress to show.
            vec![
                "npm".into(),
                "install".into(),
                "-g".into(),
                format!("{package}@latest"),
                "--loglevel".into(),
                "http".into(),
            ]
        }
    })
}

/// The numeric parts of a version such as "v2.1.296 (Claude Code)" or "codex-cli 0.50.1".
fn version_parts(v: &str) -> Vec<u64> {
    let start = v.find(|c: char| c.is_ascii_digit()).unwrap_or(v.len());
    v[start..]
        .split(|c: char| !c.is_ascii_digit() && c != '.')
        .next()
        .unwrap_or("")
        .split('.')
        .filter_map(|p| p.parse().ok())
        .collect()
}

/// Whether `latest` is a newer release than `current`.
pub(crate) fn is_newer(latest: &str, current: &str) -> bool {
    let (l, c) = (version_parts(latest), version_parts(current));
    !l.is_empty() && !c.is_empty() && l > c
}

/// What the latest message of a session asks of the person, if anything.
pub(crate) fn waiting_on(session: &Session) -> Option<(usize, Waiting)> {
    let index = session.messages.len().checked_sub(1)?;
    let m = &session.messages[index];
    if m.role != Role::Agent {
        return None;
    }
    if !m.denied.is_empty() {
        Some((index, Waiting::Denied(m.denied.clone())))
    } else if let Some(q) = m.questions.first().filter(|q| !q.options.is_empty()) {
        Some((index, Waiting::Question(q.clone())))
    } else if !m.plan.trim().is_empty() {
        Some((index, Waiting::Plan))
    } else {
        None
    }
}

/// Join names as "A", "A and B", "A, B and C".
pub(crate) fn and_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

impl RunoApp {
    /// Look up the latest release of every installed agent, in the background, once
    /// their installed versions are known.
    pub(crate) fn check_updates(&mut self) {
        self.update_checked = Some(std::time::Instant::now());
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            // Discovery runs at start; give it a moment to read installed versions.
            let start = std::time::Instant::now();
            let ready = || {
                agent::PROVIDERS
                    .iter()
                    .all(|p| !agent::installed(p.bin) || !caps::get(p.id).version.is_empty())
            };
            while !ready() && start.elapsed() < Duration::from_secs(30) {
                std::thread::sleep(Duration::from_millis(500));
            }
            let mut found = Vec::new();
            for p in agent::PROVIDERS.iter() {
                let current = caps::get(p.id).version;
                let Some(package) = npm_package(p.id) else {
                    continue;
                };
                if current.is_empty() {
                    continue;
                }
                let out = std::process::Command::new("curl")
                    .args([
                        "-sL",
                        "-m",
                        "8",
                        &format!("https://registry.npmjs.org/{package}/latest"),
                    ])
                    .output();
                let latest = out
                    .ok()
                    .and_then(|o| serde_json::from_slice::<serde_json::Value>(&o.stdout).ok())
                    .and_then(|v| v["version"].as_str().map(str::to_string))
                    .unwrap_or_default();
                if is_newer(&latest, &current) {
                    found.push(format!("{}\t{current}\t{latest}", p.id));
                }
            }
            let _ = tx.send(Event::Output("updates".into(), Ok(found.join("\n"))));
        });
    }

    /// Re-check for updates now and then while the app stays open.
    pub(crate) fn maybe_check_updates(&mut self) {
        if self
            .update_checked
            .is_none_or(|at| at.elapsed() > UPDATE_EVERY)
        {
            self.check_updates();
        }
    }

    pub(crate) fn run_update(&mut self, provider: &'static str, cx: &mut Context<Self>) {
        let p = agent::provider(provider);
        let configured = cx.global::<AppSettings>().agent(provider).binary;
        let bin = if configured.trim().is_empty() {
            p.bin.to_string()
        } else {
            configured.trim().to_string()
        };
        let Some(args) = update_command(provider, &bin) else {
            return;
        };
        let mut cmd = std::process::Command::new(&args[0]);
        cmd.args(&args[1..]);
        self.update_runs.insert(
            provider,
            UpdateRun {
                started: std::time::Instant::now(),
                line: format!("Starting {}…", args.join(" ")),
                percent: None,
                phase: 0.,
                finished: None,
            },
        );
        run_streaming(format!("update:{provider}"), cmd, self.tx.clone());
        cx.notify();
    }

    /// Hide update cards whose success badge has been shown long enough.
    pub(crate) fn prune_update_runs(&mut self) -> bool {
        let done: Vec<&'static str> = self
            .update_runs
            .iter()
            .filter(|(_, run)| {
                run.finished
                    .is_some_and(|at| at.elapsed() > UPDATE_DONE_SHOWN)
            })
            .map(|(id, _)| *id)
            .collect();
        for id in &done {
            self.update_runs.remove(id);
            self.updates.retain(|u| u.provider != *id);
        }
        !done.is_empty()
    }

    /// Results of update checks and update runs.
    pub(crate) fn handle_alert_job(&mut self, key: &str, result: Result<String, String>) -> bool {
        if let Some(provider) = key.strip_prefix("install:") {
            self.finish_install(provider, result);
            return true;
        }
        if key == "updates" {
            let text = result.unwrap_or_default();
            self.updates = text
                .lines()
                .filter_map(|line| {
                    let mut parts = line.split('\t');
                    let id = parts.next()?;
                    let provider = agent::PROVIDERS.iter().find(|p| p.id == id)?.id;
                    Some(AgentUpdate {
                        provider,
                        current: parts.next()?.to_string(),
                        latest: parts.next()?.to_string(),
                    })
                })
                .collect();
            return true;
        }
        if let Some(provider) = key
            .strip_prefix("update:")
            .and_then(|k| k.strip_suffix("-line"))
        {
            if let (Some(run), Ok(line)) = (self.update_runs.get_mut(provider), result) {
                run.take_line(&line);
            }
            return true;
        }
        let Some(provider) = key.strip_prefix("update:") else {
            return false;
        };
        let Some(provider) = agent::PROVIDERS
            .iter()
            .find(|p| p.id == provider)
            .map(|p| p.id)
        else {
            return true;
        };
        let name = agent::provider(provider).name;
        match result {
            Ok(_) => {
                let latest = self
                    .updates
                    .iter()
                    .find(|u| u.provider == provider)
                    .map(|u| u.latest.clone())
                    .unwrap_or_default();
                // The card turns into a success badge, then hides itself.
                if let Some(run) = self.update_runs.get_mut(provider) {
                    run.finished = Some(std::time::Instant::now());
                }
                self.state.log(0, format!("Updated {name} to {latest}"));
                caps::refresh();
            }
            Err(error) => {
                self.update_runs.remove(provider);
                let last = error
                    .lines()
                    .rev()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("")
                    .to_string();
                self.notice = Some(format!("Could not update {name}: {last}"));
            }
        }
        true
    }

    /// Send a reply to any session, not just the one on screen.
    pub(crate) fn answer_session(
        &mut self,
        id: u64,
        prompt: String,
        allow: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        if self.handles.contains_key(&Target::Session(id)) || self.demo_guard(cx) {
            return;
        }
        let Some(provider) = self.state.session(id).map(|s| s.provider.clone()) else {
            return;
        };
        let mut opts = cx.global::<AppSettings>().agent(&provider);
        if !allow.is_empty() {
            let mut tools: Vec<String> = opts
                .allowed_tools
                .split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect();
            tools.extend(allow);
            opts.allowed_tools = tools.join(",");
        }
        self.start_turn(id, prompt, Vec::new(), &opts);
    }

    /// Sessions waiting on the person, except the one already on screen.
    fn waiting_sessions(&self) -> Vec<(u64, usize, Waiting)> {
        let on_screen = (self.view == View::Session)
            .then_some(self.selected)
            .flatten();
        self.state
            .sessions
            .iter()
            .filter(|s| !s.archived && Some(s.id) != on_screen)
            .filter(|s| !self.handles.contains_key(&Target::Session(s.id)))
            .filter_map(|s| waiting_on(s).map(|(i, w)| (s.id, i, w)))
            .filter(|(id, i, _)| !self.alerts_hidden.contains(&format!("s:{id}:{i}")))
            .collect()
    }

    /// The popups in the corner of the window.
    pub(crate) fn alerts_view(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.demo || self.view == View::Settings || self.tour.is_some() {
            return None;
        }
        let mut cards: Vec<AnyElement> = Vec::new();
        for (n, (id, index, waiting)) in self.waiting_sessions().into_iter().enumerate() {
            let Some(session) = self.state.session(id) else {
                continue;
            };
            let provider = agent::provider(&session.provider);
            let key = format!("s:{id}:{index}");
            let (accent, glyph, title, body) = match &waiting {
                Waiting::Denied(tools) => (
                    c_amber(),
                    IconName::ShieldAlert,
                    "Permission needed",
                    format!(
                        "{} wants to use {} and is paused until you allow it.",
                        provider.name,
                        and_list(tools)
                    ),
                ),
                Waiting::Question(q) => (
                    c_blue(),
                    IconName::CircleQuestionMark,
                    "Question",
                    q.question.clone(),
                ),
                Waiting::Plan => (
                    c_blue(),
                    IconName::ListChecks,
                    "Plan ready for review",
                    format!(
                        "{} proposed a plan and waits for your go-ahead.",
                        provider.name
                    ),
                ),
            };
            let mut actions = div().flex().flex_wrap().gap_2();
            match waiting {
                Waiting::Denied(tools) => {
                    let (once, always) = (tools.clone(), tools);
                    actions = actions
                        .child(
                            self.button(("al-once", n), "Allow once", false)
                                .h(px(26.))
                                .child(icon(IconName::Check, 12., c_secondary()))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.answer_session(
                                        id,
                                        format!(
                                            "You now have permission to use {}. Continue.",
                                            once.join(", ")
                                        ),
                                        once.clone(),
                                        cx,
                                    );
                                    cx.notify();
                                })),
                        )
                        .child(
                            self.button(("al-always", n), "Always allow", true)
                                .h(px(26.))
                                .child(icon(IconName::ShieldCheck, 12., c_canvas()))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(provider) =
                                        this.state.session(id).map(|s| s.provider.clone())
                                    {
                                        this.allow_tools_for(&provider, &always, cx);
                                    }
                                    this.answer_session(
                                        id,
                                        format!(
                                            "You now have permission to use {}. Continue.",
                                            always.join(", ")
                                        ),
                                        Vec::new(),
                                        cx,
                                    );
                                    cx.notify();
                                })),
                        );
                }
                Waiting::Question(q) => {
                    for (oi, option) in q.options.iter().take(4).enumerate() {
                        let answer = format!("My answer: {} {option}", q.question);
                        actions = actions.child(
                            self.chip(("al-opt", n * 10 + oi), option.clone(), false)
                                .h(px(26.))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.answer_session(id, answer.clone(), Vec::new(), cx);
                                    cx.notify();
                                })),
                        );
                    }
                }
                Waiting::Plan => {
                    actions = actions.child(
                        self.button(("al-plan", n), "Approve and build", true)
                            .h(px(26.))
                            .child(icon(IconName::Play, 12., c_canvas()))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(s) = this.state.session_mut(id) {
                                    s.opts.insert("mode".into(), "edit".into());
                                }
                                this.answer_session(
                                    id,
                                    "The plan is approved. Go ahead and implement it.".into(),
                                    Vec::new(),
                                    cx,
                                );
                                cx.notify();
                            })),
                    );
                }
            }
            cards.push(self.alert_card(
                SharedString::from(format!("alert-{key}")),
                accent,
                glyph,
                title,
                Some(session.title.clone()),
                Some(self.provider_badge(provider, 18.)),
                body,
                actions,
                key,
                Some(id),
                cx,
            ));
        }
        for (n, update) in self.updates.iter().enumerate() {
            let key = format!("u:{}:{}", update.provider, update.latest);
            let run = self.update_runs.get(update.provider);
            let done = run.is_some_and(|r| r.finished.is_some());
            if self.alerts_hidden.contains(&key) && !done {
                continue;
            }
            let provider = agent::provider(update.provider);
            let id = update.provider;
            let latest = update.latest.trim_start_matches('v').to_string();
            let current = version_parts(&update.current)
                .iter()
                .map(u64::to_string)
                .collect::<Vec<_>>()
                .join(".");
            let (body, actions) = match run {
                Some(_) if done => (
                    format!("{} is now on version {latest}.", provider.name),
                    div().flex().child(update_done_badge(n)),
                ),
                Some(run) => (
                    format!("Updating from {current} to {latest}."),
                    self.update_progress_view(n, run),
                ),
                None => (
                    format!("Version {latest} is out. You have {current}."),
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            self.button(("up-now", n), "Update now", true)
                                .h(px(26.))
                                .child(icon(IconName::Download, 12., c_canvas()))
                                .on_click(
                                    cx.listener(move |this, _, _, cx| this.run_update(id, cx)),
                                ),
                        )
                        .child({
                            let later = key.clone();
                            self.button(("up-later", n), "Later", false)
                                .h(px(26.))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.alerts_hidden.insert(later.clone());
                                    cx.notify();
                                }))
                        }),
                ),
            };
            let card = self.alert_card(
                SharedString::from(format!("alert-{key}")),
                c_green(),
                if done {
                    IconName::CircleCheck
                } else {
                    IconName::Download
                },
                if done {
                    "Update complete"
                } else if run.is_some() {
                    "Updating"
                } else {
                    "Update available"
                },
                Some(provider.name.to_string()),
                Some(self.provider_badge(provider, 18.)),
                body,
                actions,
                key.clone(),
                None,
                cx,
            );
            cards.push(if done {
                // Hold the badge, then fade out just before the card is removed.
                let total = UPDATE_DONE_SHOWN.as_secs_f32();
                div().child(card).animated(
                    SharedString::from(format!("up-out-{key}")),
                    Animation::new(UPDATE_DONE_SHOWN),
                    move |el, d| {
                        let fade_from = 1. - 0.5 / total;
                        el.opacity(if d < fade_from {
                            1.
                        } else {
                            1. - (d - fade_from) / (1. - fade_from)
                        })
                    },
                )
            } else {
                card
            });
        }
        if cards.is_empty() {
            return None;
        }
        let total = cards.len();
        let shown = if self.alerts_expanded { total } else { 3 };
        let mut stack = div()
            .absolute()
            .right_4()
            .bottom(px(36.))
            .w(px(380.))
            .max_w(relative(0.9))
            .v_flex()
            .gap_2()
            .occlude();
        if total > shown || self.alerts_expanded && total > 3 {
            let more = total.saturating_sub(shown);
            stack = stack.child(
                div()
                    .id("alerts-more")
                    .self_end()
                    .px_3()
                    .py_1()
                    .rounded_full()
                    .border_1()
                    .border_color(rgb(c_border()))
                    .bg(rgb(c_raised()))
                    .shadow_md()
                    .cursor_pointer()
                    .text_xs()
                    .text_color(rgb(c_secondary()))
                    .hover(|el| el.bg(rgb(c_subtle())))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.alerts_expanded = !this.alerts_expanded;
                        cx.notify();
                    }))
                    .child(if self.alerts_expanded {
                        "Show fewer".to_string()
                    } else {
                        format!("{more} more waiting")
                    }),
            );
        }
        let mut list = div().v_flex().gap_2();
        for card in cards.into_iter().take(shown) {
            list = list.child(card);
        }
        stack = stack.child(
            div()
                .id("alerts-scroll")
                .max_h(px(640.))
                .overflow_y_scroll()
                .child(list),
        );
        Some(stack.into_any_element())
    }

    /// The live progress of an update: a bar, what the updater is doing, and the time.
    fn update_progress_view(&self, n: usize, run: &UpdateRun) -> Div {
        let now = std::time::Instant::now();
        let fraction = run.progress(now);
        let secs = now.duration_since(run.started).as_secs();
        let elapsed = if secs < 60 {
            format!("{secs}s")
        } else {
            format!("{}m {:02}s", secs / 60, secs % 60)
        };
        let figure = match run.percent {
            Some(p) => format!("{p:.0}% · {elapsed}"),
            None => elapsed,
        };
        let bar = div()
            .relative()
            .h(px(5.))
            .w_full()
            .rounded_full()
            .overflow_hidden()
            .bg(rgb(settings::mix(c_green(), c_border(), 0.2)))
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .bottom_0()
                    .w(relative(fraction))
                    .rounded_full()
                    .bg(rgb(c_green())),
            )
            .child(
                // A light sweep across the filled part, so the bar always reads as alive.
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .w(relative(0.25))
                    .bg(hsla(0., 0., 1., 0.35))
                    .animated(
                        ("up-sweep", n),
                        Animation::new(Duration::from_millis(1400)).repeat(),
                        move |el, d| el.left(relative(d * (fraction + 0.25) - 0.25)),
                    ),
            );
        div().v_flex().gap(px(6.)).w_full().child(bar).child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .min_w_0()
                .child(icon(IconName::LoaderCircle, 11., c_green()).animated(
                    ("up-spin", n),
                    Animation::new(Duration::from_millis(900)).repeat(),
                    |i, d| i.rotate(percentage(d)),
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_family(self.mono_font.clone())
                        .text_size(px(11.))
                        .text_color(rgb(c_muted()))
                        .child(run.line.clone()),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(px(11.))
                        .text_color(rgb(c_secondary()))
                        .child(figure),
                ),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn alert_card(
        &self,
        id: SharedString,
        accent: u32,
        glyph: IconName,
        title: &'static str,
        subject: Option<String>,
        badge: Option<Div>,
        body: String,
        actions: Div,
        key: String,
        open: Option<u64>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        slide_in(
            div()
                .id(id.clone())
                .relative()
                .v_flex()
                .gap(px(6.))
                .p_3()
                .rounded_xl()
                .border_1()
                .border_color(rgb(settings::mix(accent, c_border(), 0.45)))
                .bg(float(c_raised()))
                .shadow_xl()
                .overflow_hidden()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(icon(glyph, 14., accent))
                        .child(
                            div()
                                .text_size(px(12.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(title),
                        )
                        .child(div().flex_1())
                        .child(
                            self.ghost_button_sized(
                                SharedString::from(format!("{id}-x")),
                                IconName::X,
                                "Hide",
                                22.,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.alerts_hidden.insert(key.clone());
                                    cx.notify();
                                },
                            )),
                        ),
                )
                .when_some(subject, |el, subject| {
                    el.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .min_w_0()
                            .children(badge)
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(subject),
                            )
                            .when_some(open, |el, sid| {
                                el.child(
                                    div()
                                        .id(SharedString::from(format!("{id}-open")))
                                        .flex_none()
                                        .flex()
                                        .items_center()
                                        .gap_1()
                                        .px_1()
                                        .rounded_sm()
                                        .cursor_pointer()
                                        .text_xs()
                                        .text_color(rgb(c_secondary()))
                                        .hover(|el| {
                                            el.bg(rgb(c_subtle())).text_color(rgb(c_text()))
                                        })
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.select(sid);
                                            this.view = View::Session;
                                            cx.notify();
                                        }))
                                        .child("Open")
                                        .child(icon(IconName::ArrowRight, 11., c_secondary())),
                                )
                            }),
                    )
                })
                .child(
                    div()
                        .text_size(px(12.5))
                        .text_color(rgb(c_secondary()))
                        .child(body),
                )
                .child(actions),
            id,
            260,
        )
        .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn versions_compare_and_updates_have_commands() {
        assert!(is_newer("2.1.300", "2.1.296 (Claude Code)"));
        assert!(is_newer("0.51.0", "codex-cli 0.50.1"));
        assert!(!is_newer("2.1.296", "v2.1.296"));
        assert!(!is_newer("1.0.0", "1.2.0"));
        assert!(!is_newer("", "1.0.0"));
        assert_eq!(
            update_command("claude", "claude").unwrap(),
            vec!["claude", "update"]
        );
        assert_eq!(
            update_command("codex", "codex").unwrap(),
            vec![
                "npm",
                "install",
                "-g",
                "@openai/codex@latest",
                "--loglevel",
                "http"
            ]
        );
        assert!(update_command("grok", "grok").is_none());
        assert_eq!(percent_of("Downloading 2.1.296 66%"), Some(66.));
        assert_eq!(percent_of("[####    ] 42.5 %"), Some(42.5));
        assert_eq!(percent_of("no number here %"), None);
        assert_eq!(
            clean_line("\x1b[32mDownloading\x1b[0m 12% \u{280b}"),
            "Downloading 12%"
        );
        assert_eq!(
            phase_of("npm http fetch GET 200 https://x 41ms"),
            Some(0.35)
        );
        assert_eq!(phase_of("changed 1 package in 7s"), Some(0.95));
        let start = std::time::Instant::now();
        let mut run = UpdateRun {
            started: start,
            line: String::new(),
            percent: None,
            phase: 0.,
            finished: None,
        };
        let early = run.progress(start);
        let later = run.progress(start + Duration::from_secs(30));
        assert!(early < later && later < 0.97, "time eases the bar forward");
        run.take_line("Verifying checksum...");
        assert!(run.progress(start) >= 0.8, "a later phase moves the bar");
        run.take_line("Downloading 50%");
        assert_eq!(run.progress(start), 0.5, "a printed percentage wins");
        run.finished = Some(start);
        assert_eq!(run.progress(start), 1.);
        assert_eq!(
            and_list(&["A".into(), "B".into(), "C".into()]),
            "A, B and C"
        );
    }

    #[test]
    fn sessions_waiting_on_the_person() {
        let mut s = Session {
            messages: vec![Message::new(Role::Agent, "done")],
            ..Session::default()
        };
        assert!(waiting_on(&s).is_none());
        s.messages[0].denied = vec!["Bash".into()];
        assert!(matches!(waiting_on(&s), Some((0, Waiting::Denied(_)))));
        s.messages.push(Message::new(Role::User, "ok"));
        assert!(waiting_on(&s).is_none());
    }
}
