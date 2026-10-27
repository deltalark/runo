//! A sign-in run started from inside Runo: Runo spawns the CLI's own login command,
//! streams its output into a popup card, polls the local sign-in check in the background,
//! and ends on the real success or failure line the CLI wrote. The person keeps the
//! terminal to themselves while this happens, and the run can be cancelled.
//!
//! For a CLI with no `login` subcommand (Claude Code, Copilot, Qwen, Droid, Gemini on
//! first run), the fallback is the old flow: open the CLI in a terminal panel (or a new
//! terminal window), and let the person sign in there. The sign-in check then catches up
//! when the terminal exits.

use super::*;
use std::time::Instant;

/// What a running sign-in is doing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SignInStatus {
    /// The login command is running.
    Running,
    /// Runo has verified the agent is signed in.
    Signed,
    /// The command stopped without signing in; the string is the CLI's own last error line.
    Failed(String),
    /// The person pressed Cancel.
    Cancelled,
}

/// A sign-in in flight (or just finished) for an agent CLI on this machine.
pub(crate) struct SignInRun {
    pub started: Instant,
    /// The command Runo spawned, as the person would type it.
    pub command: String,
    /// The CLI's output so far, newest at the end. The card shows the tail.
    pub lines: Vec<String>,
    /// A URL the CLI printed for the person to open, if it printed one.
    pub url: Option<String>,
    /// A "Signed in as …" value the CLI printed, if it printed one.
    pub signed_in_as: Option<String>,
    pub status: SignInStatus,
    pub finished: Option<Instant>,
}

impl SignInRun {
    fn new(command: String) -> Self {
        SignInRun {
            started: Instant::now(),
            command,
            lines: Vec::new(),
            url: None,
            signed_in_as: None,
            status: SignInStatus::Running,
            finished: None,
        }
    }

    pub(crate) fn running(&self) -> bool {
        matches!(self.status, SignInStatus::Running)
    }

    pub(crate) fn signed(&self) -> bool {
        matches!(self.status, SignInStatus::Signed)
    }

    pub(crate) fn failed(&self) -> Option<&str> {
        if let SignInStatus::Failed(e) = &self.status {
            Some(e.as_str())
        } else {
            None
        }
    }

    pub(crate) fn cancelled(&self) -> bool {
        matches!(self.status, SignInStatus::Cancelled)
    }

    fn add_line(&mut self, line: String) {
        if self.url.is_none() {
            if let Some(url) = url_in(&line) {
                self.url = Some(url);
            }
        }
        if self.signed_in_as.is_none() {
            if let Some(name) = signed_in_as(&line) {
                self.signed_in_as = Some(name);
            }
        }
        self.lines.push(line);
        let extra = self.lines.len().saturating_sub(KEEP);
        if extra > 0 {
            self.lines.drain(..extra);
        }
    }
}

/// How many of the last lines the card keeps for the person to read on failure.
const KEEP: usize = 60;

/// How often Runo re-checks the sign-in while the login command runs.
const POLL_EVERY: Duration = Duration::from_millis(2500);

/// A sign-in that keeps running without a success signal is stopped after this. The CLI's
/// own command is usually a few seconds; a browser round-trip may take a minute.
const LIMIT: Duration = Duration::from_secs(5 * 60);

/// Key the login run itself uses when it reports lines and its result.
pub(crate) fn run_key(provider: &str) -> String {
    format!("sign-in:{provider}")
}

/// Key the background sign-in poller uses.
fn check_key(provider: &str) -> String {
    format!("sign-in-check:{provider}")
}

/// The real success line a CLI prints, as "Signed in as …" or "Logged in as …". Returns
/// what comes after the "as " up to the end of a sensible token, trimmed; None if the line
/// is not one of these. Covers the lines seen in codex, grok, amp, opencode and cursor
/// output and makes a best effort at others.
fn signed_in_as(line: &str) -> Option<String> {
    let clean = alerts::clean_line(line);
    let lower = clean.to_lowercase();
    let heads = [
        "signed in as ",
        "signed in as: ",
        "logged in as ",
        "logged in as: ",
        "logged in to ",
        "you are logged in as ",
        "authenticated as ",
        "successfully authenticated as ",
        "login successful as ",
        "login successful - signed in as ",
    ];
    for head in heads {
        if let Some(at) = lower.find(head) {
            let rest = &clean[at + head.len()..];
            // The name ends at a line break or a trailing comment such as "(key …)";
            // dots inside the name are kept so email addresses stay whole.
            let end = rest
                .find(|c: char| c == '\n' || c == '\r' || c == '(' || c == '[')
                .unwrap_or(rest.len());
            // Strip at most one sentence-ending dot; dots inside the token stay.
            let token = rest[..end].trim();
            let token = token.strip_suffix('.').unwrap_or(token);
            let name = token.trim_end_matches(['!', ',', ';']).trim();
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }
    None
}

/// A URL printed in a sign-in line, if there is one.
fn url_in(line: &str) -> Option<String> {
    let clean = alerts::clean_line(line);
    let start = clean.find("http://").or_else(|| clean.find("https://"))?;
    let end = clean[start..]
        .find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == '>' || c == '<')
        .map(|n| start + n)
        .unwrap_or(clean.len());
    let url = clean[start..end].trim_end_matches(['.', ',', ')', ']']);
    (!url.is_empty()).then(|| url.to_string())
}

/// Whether a line looks like the CLI's own failure line, to use as the card's "why" when
/// the run exits without success. Broader than `signin::signs_out`: anything with
/// "error", "failed" or "cancelled" counts.
fn is_failure_line(line: &str) -> bool {
    let clean = alerts::clean_line(line).to_lowercase();
    let hits = [
        "error",
        "failed",
        "failure",
        "cancelled",
        "canceled",
        "timed out",
        "timeout",
        "could not",
        "couldn't",
        "unable to",
        "invalid",
        "denied",
        "aborted",
    ];
    hits.iter().any(|h| clean.contains(h))
}

/// The newest likely-reason line from the output tail: a line that looks like an error,
/// otherwise the last non-empty line, otherwise an empty string.
fn tail_reason(lines: &[String]) -> String {
    for line in lines.iter().rev() {
        if is_failure_line(line) {
            return alerts::clean_line(line);
        }
    }
    lines
        .iter()
        .rev()
        .find(|l| !l.trim().is_empty())
        .map(|l| alerts::clean_line(l))
        .unwrap_or_default()
}

/// Build the login command for a provider: its bin (with the agent's PATH), followed by
/// the login arguments from the manifest. None when the provider has no login command.
pub(crate) fn login_command(provider: &str) -> Option<(std::process::Command, String)> {
    let spec = registry::spec(provider)?;
    if spec.login.is_empty() {
        return None;
    }
    let p = agent::provider(provider);
    let bin = install::agent_path(p)
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.bin.to_string());
    let mut command = std::process::Command::new(&bin);
    command.args(&spec.login);
    tty_env(&mut command);
    let shown = install::sign_in_command(provider, &bin);
    Some((command, shown))
}

/// The logout command, when the CLI has one. The same bin selection and PATH as
/// `login_command`.
pub(crate) fn logout_command(provider: &str) -> Option<(std::process::Command, String)> {
    let spec = registry::spec(provider)?;
    if spec.logout.is_empty() {
        return None;
    }
    let p = agent::provider(provider);
    let bin = install::agent_path(p)
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.bin.to_string());
    let mut command = std::process::Command::new(&bin);
    command.args(&spec.logout);
    tty_env(&mut command);
    let shown = format!("{} {}", shell_quote(&bin), spec.logout.join(" "));
    Some((command, shown))
}

fn shell_quote(bin: &str) -> String {
    if bin
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "/._-+:\\".contains(c))
    {
        bin.to_string()
    } else if cfg!(windows) {
        format!("\"{bin}\"")
    } else {
        format!("'{}'", bin.replace('\'', "'\\''"))
    }
}

/// Settings for a CLI we read with pipes: no $TERM session line editing, no colour.
fn tty_env(command: &mut std::process::Command) {
    command.env("TERM", "dumb");
    command.env("NO_COLOR", "1");
    // The parent app sets this so Claude Code sees itself as a child session; a login
    // run has nothing to do with that.
    command.env_remove("CLAUDE_CODE_CHILD_SESSION");
}

/// Poll the sign-in check every few seconds, emitting a result when it says the CLI is
/// signed in. Runs on its own thread; stops once `alive` returns false (the run is gone),
/// after `LIMIT`, or on the first `Some(true)`.
pub(crate) fn poll(
    provider: &'static str,
    alive: std::sync::Arc<std::sync::atomic::AtomicBool>,
    tx: Sender<agent::Event>,
) {
    use std::sync::atomic::Ordering;
    std::thread::spawn(move || {
        let started = Instant::now();
        while alive.load(Ordering::Relaxed) && started.elapsed() < LIMIT {
            std::thread::sleep(POLL_EVERY);
            if !alive.load(Ordering::Relaxed) {
                return;
            }
            // The check reads login files and runs `… auth status` where that is how the
            // CLI reports. It is given a long enough budget to answer.
            if let Some(true) = signin::verify(provider) {
                let _ = tx.send(agent::Event::Output(check_key(provider), Ok("yes".into())));
                return;
            }
        }
    });
}

impl RunoApp {
    /// Start a sign-in for `provider` from the Installed card or a failed turn. Uses the
    /// CLI's own login command when the manifest names one, streaming its output into the
    /// card; otherwise falls back to the terminal panel (`sign_in_here`).
    pub(crate) fn start_sign_in(&mut self, provider: &'static str, cx: &mut Context<Self>) {
        // Cove servers and sessions on them keep their own sign-in flow: a login run on
        // this machine would sign in here, not there.
        let on_cove = self
            .selected
            .and_then(|id| self.state.session(id))
            .and_then(|s| connections::get(s.connection))
            .is_some_and(|c| c.kind == connections::Kind::Cove);
        if on_cove && cove::keeps_login(provider) {
            self.sign_in_here(provider, cx);
            return;
        }
        let Some((mut command, shown)) = login_command(provider) else {
            // No `login` subcommand: fall back to opening the CLI in a terminal.
            self.sign_in_here(provider, cx);
            return;
        };
        // If one is already running for this provider, bring the card to the front and
        // leave the existing run as it is.
        if self.sign_in_runs.get(provider).is_some_and(|r| r.running()) {
            self.info = Some((
                format!(
                    "{} sign-in is already running. The card at the top shows the steps.",
                    agent::provider(provider).name
                ),
                Instant::now(),
            ));
            return;
        }
        // The person's project envs may hold a key; forwarding them avoids a surprising
        // mismatch between what Runo sees and what the CLI sees in the same shell.
        let env = cx.global::<AppSettings>().agent(provider).env;
        for line in env.lines() {
            if let Some((k, v)) = line.split_once('=') {
                let (k, v) = (k.trim(), v.trim());
                if !k.is_empty() && !v.is_empty() {
                    command.env(k, v);
                }
            }
        }
        let run = SignInRun::new(shown.clone());
        self.sign_in_runs.insert(provider, run);
        crate::applog::info("sign-in", format!("{provider}: running `{shown}`"));
        alerts::run_streaming(run_key(provider), command, self.tx.clone());
        let alive = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        // The entry is cleaned up on done/cancel; the alive flag flips with it so the
        // poller stops. The poller can only report success here, which skips the retry
        // loop and dismisses the card.
        self.sign_in_pollers.insert(provider, alive.clone());
        poll(provider, alive, self.tx.clone());
        cx.notify();
    }

    /// A streamed line from the login command: parse it, keep it, update the card.
    pub(crate) fn sign_in_streamed(&mut self, key: &str, line: String) {
        let Some(provider) = key
            .strip_prefix("sign-in:")
            .and_then(|rest| rest.strip_suffix("-line"))
        else {
            return;
        };
        let Some(id) = agent::PROVIDERS
            .iter()
            .find(|p| p.id == provider)
            .map(|p| p.id)
        else {
            return;
        };
        if let Some(run) = self.sign_in_runs.get_mut(id) {
            run.add_line(line);
        }
    }

    /// The login process ended or the poller said the CLI is signed in. The flow is: a
    /// positive check from the poller wins; otherwise, parse what the CLI said.
    pub(crate) fn sign_in_result(&mut self, key: &str, result: Result<String, String>) {
        if let Some(provider) = key.strip_prefix("sign-in-check:") {
            self.sign_in_check_arrived(provider, result);
            return;
        }
        let Some(provider) = key.strip_prefix("sign-in:") else {
            return;
        };
        let Some(id) = agent::PROVIDERS
            .iter()
            .find(|p| p.id == provider)
            .map(|p| p.id)
        else {
            return;
        };
        if let Some(alive) = self.sign_in_pollers.get(id) {
            alive.store(false, std::sync::atomic::Ordering::Relaxed);
        }
        let Some(run) = self.sign_in_runs.get_mut(id) else {
            return;
        };
        if run.signed() {
            // The poller already answered yes; the exit is just the clean-up.
            return;
        }
        let cancelled = result
            .as_ref()
            .err()
            .is_some_and(|e| e.contains("Cancelled") || run.cancelled());
        if cancelled {
            run.status = SignInStatus::Cancelled;
            run.finished = Some(Instant::now());
            crate::applog::info("sign-in", format!("{} cancelled", id));
            return;
        }
        // Successful exit: ask the local check once, in case the poller missed the window.
        let ok = result.is_ok();
        let verified = signin::verify(id);
        if ok || verified == Some(true) {
            if verified == Some(true) {
                run.status = SignInStatus::Signed;
                run.finished = Some(Instant::now());
                crate::applog::info("sign-in", format!("{} signed in", id));
                return;
            }
            // The CLI said it succeeded but Runo can't confirm. Try to parse "signed in
            // as" from the tail; otherwise call it done and let the next check catch up.
            if run.signed_in_as.is_some() {
                run.status = SignInStatus::Signed;
                run.finished = Some(Instant::now());
                crate::applog::info("sign-in", format!("{} signed in", id));
            } else {
                run.status = SignInStatus::Signed;
                run.finished = Some(Instant::now());
                crate::applog::info(
                    "sign-in",
                    format!("{} login finished; the sign-in check will catch up", id),
                );
            }
            // The next check runs when any page opens; prompt it now too.
            signin::refresh(true);
            return;
        }
        let why = match &result {
            Err(e) if !e.trim().is_empty() => tail_reason(run.lines.as_slice())
                .is_empty()
                .then(|| e.lines().next().unwrap_or(e).trim().to_string())
                .unwrap_or_else(|| tail_reason(run.lines.as_slice())),
            _ => tail_reason(run.lines.as_slice()),
        };
        let why = if why.trim().is_empty() {
            format!(
                "The {} login command stopped without a reason.",
                agent::provider(id).name
            )
        } else {
            why
        };
        crate::applog::warn("sign-in", format!("{} failed: {why}", id));
        run.status = SignInStatus::Failed(why);
        run.finished = Some(Instant::now());
    }

    /// The poller found the CLI is signed in. If a run is live, mark it signed and clean
    /// up; otherwise store the fact so pages that render soon show "Signed in".
    fn sign_in_check_arrived(&mut self, provider: &str, result: Result<String, String>) {
        let Some(id) = agent::PROVIDERS
            .iter()
            .find(|p| p.id == provider)
            .map(|p| p.id)
        else {
            return;
        };
        if result.as_deref() != Ok("yes") {
            return;
        }
        if let Some(alive) = self.sign_in_pollers.get(id) {
            alive.store(false, std::sync::atomic::Ordering::Relaxed);
        }
        if let Some(run) = self.sign_in_runs.get_mut(id) {
            if run.running() {
                run.status = SignInStatus::Signed;
                run.finished = Some(Instant::now());
                crate::applog::info("sign-in", format!("{} signed in (via check)", id));
                // Stop the login process; it will exit cleanly when the user closes the
                // browser, but the person has what they need.
                alerts::cancel_run(&run_key(id));
            }
        }
    }

    /// The person pressed Cancel on a running sign-in: stop the login process. Its exit
    /// marks the run cancelled.
    pub(crate) fn cancel_sign_in(&mut self, provider: &'static str, cx: &mut Context<Self>) {
        if let Some(run) = self.sign_in_runs.get_mut(provider) {
            if run.running() {
                run.status = SignInStatus::Cancelled;
            }
        }
        if let Some(alive) = self.sign_in_pollers.get(provider) {
            alive.store(false, std::sync::atomic::Ordering::Relaxed);
        }
        alerts::cancel_run(&run_key(provider));
        cx.notify();
    }

    /// The person pressed Retry on a finished sign-in: dismiss the old card and start
    /// a new run.
    pub(crate) fn retry_sign_in(&mut self, provider: &'static str, cx: &mut Context<Self>) {
        self.sign_in_runs.remove(provider);
        self.sign_in_pollers.remove(provider);
        self.start_sign_in(provider, cx);
    }

    /// Dismiss a finished sign-in card.
    pub(crate) fn dismiss_sign_in(&mut self, provider: &'static str, cx: &mut Context<Self>) {
        self.sign_in_runs.remove(provider);
        self.sign_in_pollers.remove(provider);
        cx.notify();
    }

    /// Sign the agent out, locally. Runs the CLI's own logout command on a background
    /// thread; the person sees the result as the next sign-in state. For CLIs with no
    /// logout command, the fallback explains the file to delete or the key to clear.
    pub(crate) fn sign_out_here(&mut self, provider: &'static str, cx: &mut Context<Self>) {
        let name = agent::provider(provider).name;
        let Some((mut command, shown)) = logout_command(provider) else {
            self.info = Some((sign_out_hint(provider).to_string(), Instant::now()));
            cx.notify();
            return;
        };
        command.stdin(std::process::Stdio::null());
        std::thread::spawn(move || {
            let result = command.output();
            let ok = result.as_ref().is_ok_and(|o| o.status.success());
            if ok {
                crate::applog::info("sign-in", format!("signed out of {name} via `{shown}`"));
            } else {
                let out = result
                    .as_ref()
                    .map(|o| String::from_utf8_lossy(&o.stderr).to_string())
                    .unwrap_or_default();
                crate::applog::warn(
                    "sign-in",
                    format!("`{shown}` failed: {}", out.lines().next().unwrap_or("")),
                );
            }
            // Either way the saved state is stale; have the next check look again.
            signin::refresh(true);
        });
        self.sign_in_runs.remove(provider);
        self.sign_in_pollers.remove(provider);
        self.info = Some((format!("Signing out of {name}…"), Instant::now()));
        cx.notify();
    }
}

impl RunoApp {
    /// Alerts cards for every live and recently finished sign-in run.
    pub(crate) fn sign_in_cards(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut cards = Vec::new();
        let mut entries: Vec<(&&'static str, &SignInRun)> = self.sign_in_runs.iter().collect();
        entries.sort_by_key(|(_, r)| r.started);
        for (n, (&provider, run)) in entries.into_iter().enumerate() {
            cards.push(self.sign_in_card(n, provider, run, cx));
        }
        cards
    }

    fn sign_in_card(
        &self,
        n: usize,
        provider: &'static str,
        run: &SignInRun,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = agent::provider(provider);
        let key = format!("sign-in:{provider}:{:?}", run.started);
        let signed_in = run.signed();
        let failed = run.failed().map(str::to_string);
        let cancelled = run.cancelled();
        let (accent, glyph, title) = match &run.status {
            SignInStatus::Running => (c_blue(), IconName::Loader, "Signing in"),
            SignInStatus::Signed => (c_green(), IconName::CircleCheck, "Signed in"),
            SignInStatus::Failed(_) => (c_red(), IconName::TriangleAlert, "Sign-in failed"),
            SignInStatus::Cancelled => (c_amber(), IconName::X, "Sign-in cancelled"),
        };
        let body = if signed_in {
            match &run.signed_in_as {
                Some(name) => format!("{} is signed in as {name}.", p.name),
                None => format!("{} is signed in.", p.name),
            }
        } else if let Some(why) = &failed {
            why.clone()
        } else if cancelled {
            format!("{} sign-in was cancelled.", p.name)
        } else {
            run.lines
                .iter()
                .rev()
                .find(|l| !l.trim().is_empty())
                .cloned()
                .unwrap_or_else(|| format!("Running `{}`…", run.command))
        };
        let actions = self.sign_in_actions(n, provider, run, cx);
        self.alert_card(
            SharedString::from(format!("alert-{key}")),
            accent,
            glyph,
            title,
            Some(p.name.to_string()),
            Some(self.provider_badge(p, 18.)),
            body,
            actions,
            key,
            None,
            cx,
        )
    }

    fn sign_in_actions(
        &self,
        n: usize,
        provider: &'static str,
        run: &SignInRun,
        cx: &mut Context<Self>,
    ) -> Div {
        let has_logout = logout_command(provider).is_some();
        let small = |el: Stateful<Div>| el.h(px(26.)).px_2().text_size(px(12.));
        let mut row = div().flex().flex_wrap().gap_2();
        if let Some(url) = run.url.clone().filter(|_| run.running()) {
            row = row.child(
                small(self.button(("sign-in-copy", n), "Copy URL", false)).on_click(cx.listener(
                    move |this, _, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(url.clone()));
                        this.info = Some((
                            "Copied the sign-in URL. Open it in your browser.".into(),
                            std::time::Instant::now(),
                        ));
                        cx.notify();
                    },
                )),
            );
        }
        match &run.status {
            SignInStatus::Running => {
                row = row.child(
                    small(self.button(("sign-in-cancel", n), "Cancel", false)).on_click(
                        cx.listener(move |this, _, _, cx| this.cancel_sign_in(provider, cx)),
                    ),
                );
            }
            SignInStatus::Signed => {
                row = row.child(
                    div()
                        .flex()
                        .items_center()
                        .child(alerts::update_done_badge(n, "Signed in")),
                );
                if has_logout {
                    row = row.child(
                        small(self.button(("sign-in-out", n), "Sign out", false)).on_click(
                            cx.listener(move |this, _, _, cx| this.sign_out_here(provider, cx)),
                        ),
                    );
                }
                row = row.child(
                    small(self.button(("sign-in-dismiss", n), "Dismiss", false)).on_click(
                        cx.listener(move |this, _, _, cx| this.dismiss_sign_in(provider, cx)),
                    ),
                );
            }
            SignInStatus::Failed(_) | SignInStatus::Cancelled => {
                row = row.child(
                    small(self.button(("sign-in-retry", n), "Try again", true)).on_click(
                        cx.listener(move |this, _, _, cx| this.retry_sign_in(provider, cx)),
                    ),
                );
                row = row.child(
                    small(self.button(("sign-in-dismiss", n), "Dismiss", false)).on_click(
                        cx.listener(move |this, _, _, cx| this.dismiss_sign_in(provider, cx)),
                    ),
                );
            }
        }
        row
    }
}

/// A fallback hint for signing out when the CLI has no logout command.
fn sign_out_hint(provider: &str) -> &'static str {
    match provider {
        "claude" => "Claude Code has no logout command. Delete ~/.claude/.credentials.json, or clear ANTHROPIC_API_KEY, to sign out.",
        "copilot" => "Copilot has no logout command. Run `copilot` and type `/logout`.",
        "gemini" => "Gemini has no logout command. Delete ~/.gemini/oauth_creds.json to sign out.",
        "qwen" => "Qwen Code has no logout command. Clear the API key you signed in with.",
        "droid" => "Droid has no logout command. Clear FACTORY_API_KEY, or sign in again with another key.",
        _ => "This CLI has no logout command.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn signed_in_as_catches_each_clis_line() {
        // Codex.
        assert_eq!(
            signed_in_as("Signed in as person@example.com"),
            Some("person@example.com".into())
        );
        assert_eq!(
            signed_in_as("Logged in using ChatGPT (person@example.com)"),
            None,
            "codex login status says 'using', which isn't 'as'; the status check handles it"
        );
        // Grok.
        assert_eq!(
            signed_in_as("✓ Successfully authenticated as @ada"),
            Some("@ada".into())
        );
        // Amp and OpenCode write a short confirmation.
        assert_eq!(
            signed_in_as("You are logged in as ada@example.com."),
            Some("ada@example.com".into())
        );
        assert_eq!(
            signed_in_as("Logged in as: anthropic (oauth)"),
            Some("anthropic".into())
        );
        // Cursor's phrasing.
        assert_eq!(
            signed_in_as("Login successful - signed in as ada"),
            Some("ada".into())
        );
        // Lines that aren't a sign-in: no match.
        assert_eq!(signed_in_as("Opening browser for sign-in…"), None);
        assert_eq!(
            signed_in_as("Visit https://x.ai/auth/login to continue"),
            None
        );
        // Colour codes don't trip it up.
        assert_eq!(
            signed_in_as("\x1b[32m✓\x1b[0m Signed in as person@example.com"),
            Some("person@example.com".into())
        );
    }

    #[test]
    fn url_in_catches_the_sign_in_link() {
        assert_eq!(
            url_in("Open this URL in your browser: https://x.ai/auth/login?code=abc"),
            Some("https://x.ai/auth/login?code=abc".into())
        );
        assert_eq!(
            url_in("Visit http://127.0.0.1:65123/callback to finish."),
            Some("http://127.0.0.1:65123/callback".into())
        );
        // Trailing punctuation is left out.
        assert_eq!(
            url_in("See https://cursor.com/login for details."),
            Some("https://cursor.com/login".into())
        );
        assert_eq!(url_in("Nothing to open yet."), None);
    }

    #[test]
    fn failure_lines_show_as_the_reason() {
        assert!(is_failure_line("Error: could not reach the auth server"));
        assert!(is_failure_line("Login failed: invalid code"));
        assert!(is_failure_line("Timed out waiting for the browser"));
        assert!(!is_failure_line("Signed in as person@example.com"));
        assert!(!is_failure_line("Opening browser…"));
    }

    #[test]
    fn tail_reason_picks_the_newest_error_line() {
        let lines = vec![
            "Opening browser…".to_string(),
            "Visit https://x.ai/auth/login to continue".to_string(),
            "Error: could not reach the auth server".to_string(),
            "Retrying…".to_string(),
            "Login failed: invalid code".to_string(),
        ];
        assert_eq!(tail_reason(&lines), "Login failed: invalid code");
        // Without an error, the last non-empty line is used.
        let lines = vec!["Opening browser…".to_string(), "".to_string()];
        assert_eq!(tail_reason(&lines), "Opening browser…");
        assert_eq!(tail_reason(&[]), "");
    }
}
