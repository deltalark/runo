//! Installing agent CLIs from inside Runo, with each vendor's own installer or npm package.
//! The exact command is shown before anything runs.

use super::*;
use std::path::PathBuf;

pub(crate) struct Install {
    /// The command as the person would type it, shown before it runs.
    pub shown: String,
    pub program: Vec<String>,
}

fn script(command: &str, windows: bool) -> Install {
    let program = if windows {
        vec![
            "powershell".into(),
            "-NoProfile".into(),
            "-ExecutionPolicy".into(),
            "Bypass".into(),
            "-Command".into(),
            command.into(),
        ]
    } else {
        vec!["sh".into(), "-c".into(), command.into()]
    };
    Install {
        shown: command.into(),
        program,
    }
}

fn npm(package: &str) -> Install {
    Install {
        shown: format!("npm install -g {package}"),
        // `--loglevel http` prints each download, so the install card shows live progress.
        program: vec![
            "npm".into(),
            "install".into(),
            "-g".into(),
            package.into(),
            "--loglevel".into(),
            "http".into(),
        ],
    }
}

pub(crate) const GROK: &str =
    "curl -fsSL https://raw.githubusercontent.com/superagent-ai/grok-cli/main/install.sh | bash";

/// How to install an agent here, from its vendor's documentation, or why it can't be.
pub(crate) fn install_plan(
    provider: &str,
    has_npm: bool,
    windows: bool,
) -> Result<Install, String> {
    let need_node = || {
        Err("This agent installs with npm, and Runo couldn't find npm. It looked on your login shell's PATH and in the nvm, fnm, Volta, asdf, Homebrew and npm global folders. Install Node.js from nodejs.org and press Re-detect, or, if the agent is already installed, locate it.".into())
    };
    let unix_only = |name: &str| {
        Err(format!(
            "{name} has no Windows installer yet. Install it under WSL."
        ))
    };
    match provider {
        "claude" => Ok(script(
            if windows {
                "irm https://claude.ai/install.ps1 | iex"
            } else {
                "curl -fsSL https://claude.ai/install.sh | bash"
            },
            windows,
        )),
        "cursor" => Ok(script(
            if windows {
                "irm 'https://cursor.com/install?win32=true' | iex"
            } else {
                "curl https://cursor.com/install -fsS | bash"
            },
            windows,
        )),
        "droid" => Ok(if has_npm {
            npm("droid")
        } else if windows {
            script("irm https://app.factory.ai/cli/windows | iex", windows)
        } else {
            script("curl -fsSL https://app.factory.ai/cli | sh", windows)
        }),
        "opencode" if has_npm => Ok(npm("opencode-ai")),
        "opencode" if windows => need_node(),
        "opencode" => Ok(script(
            "curl -fsSL https://opencode.ai/install | bash",
            windows,
        )),
        "grok" if windows => unix_only("Grok CLI"),
        "grok" => Ok(script(GROK, windows)),
        other => match alerts::npm_package(other) {
            Some(package) if has_npm => Ok(npm(package)),
            Some(_) => need_node(),
            None => Err("Runo doesn't know how to install this agent.".into()),
        },
    }
}

/// Folders installers commonly use that a desktop app's PATH may not include.
pub(crate) fn extra_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_default();
    let mut dirs = vec![
        home.join(".local/bin"),
        home.join(".npm-global/bin"),
        home.join(".bun/bin"),
        home.join(".opencode/bin"),
        home.join(".factory/bin"),
        home.join(".grok/bin"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ];
    if let Some(appdata) = std::env::var_os("APPDATA") {
        dirs.push(PathBuf::from(appdata).join("npm"));
    }
    dirs
}

/// Where a CLI ended up: on PATH, or in a folder installers use.
pub(crate) fn locate(bin: &str) -> Option<PathBuf> {
    let suffixes: &[&str] = if cfg!(windows) {
        &["", ".exe", ".cmd", ".bat"]
    } else {
        &[""]
    };
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .chain(extra_dirs())
        .flat_map(|dir| suffixes.iter().map(move |s| dir.join(format!("{bin}{s}"))))
        .find(|p| p.is_file())
}

/// Look for agents and Node.js again: reread the login shell's PATH, then ask each CLI
/// what it supports. Runs off the window thread, since the shell can take a moment.
pub(crate) fn redetect() {
    std::thread::spawn(|| {
        crate::shellenv::init();
        caps::refresh();
    });
}

static NODE: std::sync::RwLock<Option<(String, PathBuf)>> = std::sync::RwLock::new(None);

/// Look for Node.js again; called with agent detection, off the window thread.
pub(crate) fn refresh_node() {
    let found = locate("node").and_then(|path| {
        let out = std::process::Command::new(&path)
            .arg("--version")
            .output()
            .ok()?;
        let version = String::from_utf8_lossy(&out.stdout).trim().to_string();
        Some((version, path))
    });
    if let Ok(mut node) = NODE.write() {
        *node = found;
    }
}

/// Node.js as Runo last saw it: its version and where it is.
pub(crate) fn node() -> Option<(String, PathBuf)> {
    NODE.read().ok().and_then(|n| n.clone())
}

/// Where an agent's CLI is: the file set in Settings, or the one found on the PATH.
pub(crate) fn agent_path(provider: &'static agent::Provider) -> Option<PathBuf> {
    let bin = caps::binary(provider);
    let as_path = PathBuf::from(&bin);
    if as_path.is_absolute() {
        return as_path.is_file().then_some(as_path);
    }
    locate(&bin).or_else(|| (provider.id == "cursor").then(|| locate("agent")).flatten())
}

/// The command that signs an agent in, as typed in a terminal: its login command where
/// it has one, else the CLI itself, which asks on first run.
pub(crate) fn sign_in_command(provider: &str, bin: &str) -> String {
    let bin = if bin
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "/._-+:\\".contains(c))
    {
        bin.to_string()
    } else if cfg!(windows) {
        format!("\"{bin}\"")
    } else {
        format!("'{}'", bin.replace('\'', "'\\''"))
    };
    match provider {
        "codex" | "cursor" | "amp" => format!("{bin} login"),
        "opencode" => format!("{bin} auth login"),
        _ => bin,
    }
}

/// Run `line` in a new window of the computer's terminal app, left open afterwards.
pub(crate) fn open_in_terminal(line: &str) -> Result<(), String> {
    use std::process::Command;
    let started = if cfg!(target_os = "macos") {
        // Terminal runs a .command file it is asked to open.
        let file = std::env::temp_dir().join("runo-sign-in.command");
        std::fs::write(&file, format!("#!/bin/sh\n{line}\n")).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755));
        }
        Command::new("open").arg(&file).spawn().is_ok()
    } else if cfg!(windows) {
        Command::new("cmd")
            .args(["/C", "start", "", "cmd", "/K", line])
            .spawn()
            .is_ok()
    } else {
        let script = format!("{line}; exec \"${{SHELL:-sh}}\"");
        [
            ("x-terminal-emulator", "-e"),
            ("gnome-terminal", "--"),
            ("konsole", "-e"),
            ("xfce4-terminal", "-x"),
            ("kitty", "--"),
            ("alacritty", "-e"),
            ("xterm", "-e"),
        ]
        .iter()
        .filter_map(|(app, flag)| locate(app).map(|path| (path, flag)))
        .any(|(path, flag)| {
            Command::new(path)
                .args([flag, "sh", "-c", &script])
                .spawn()
                .is_ok()
        })
    };
    if started {
        Ok(())
    } else {
        Err("No terminal app was found.".into())
    }
}

impl RunoApp {
    /// Open a terminal that signs the agent in, or copy the command when there's none.
    pub(crate) fn sign_in_agent(&mut self, provider: &'static str, cx: &mut Context<Self>) {
        let p = agent::provider(provider);
        let bin = agent_path(p)
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|| p.bin.to_string());
        let line = sign_in_command(provider, &bin);
        let said = match open_in_terminal(&line) {
            Ok(()) => format!(
                "Sign in to {} in the terminal window, then pick it in New session.",
                p.name
            ),
            Err(_) => {
                cx.write_to_clipboard(ClipboardItem::new_string(line.clone()));
                format!(
                    "Copied `{line}`. Run it in a terminal to sign in to {}.",
                    p.name
                )
            }
        };
        self.info = Some((said, std::time::Instant::now()));
        cx.notify();
    }

    /// Point an agent at a CLI Runo didn't find, chosen in a file dialog.
    pub(crate) fn locate_agent(&mut self, provider: &'static str, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(format!("Use this {}", agent::provider(provider).name).into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                if let Some(path) = paths.first() {
                    let path = path.to_string_lossy().into_owned();
                    let _ = this.update(cx, |this, cx| {
                        this.install_confirm = None;
                        this.pending_binary = Some((provider, path));
                        this.info = Some((
                            format!("Using the {} you picked.", agent::provider(provider).name),
                            std::time::Instant::now(),
                        ));
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    /// Ask before installing: the confirm card shows the exact command.
    pub(crate) fn ask_install(&mut self, provider: &'static str) {
        if self.update_runs.get(provider).is_some_and(|r| r.running()) {
            self.fail(
                "install",
                format!("{} is already installing.", agent::provider(provider).name),
            );
            return;
        }
        self.install_confirm = Some(provider);
    }

    pub(crate) fn run_install(&mut self, provider: &'static str, cx: &mut Context<Self>) {
        self.install_confirm = None;
        let plan = match install_plan(provider, agent::installed("npm"), cfg!(windows)) {
            Ok(plan) => plan,
            Err(why) => {
                self.fail("install", why);
                cx.notify();
                return;
            }
        };
        let mut cmd = std::process::Command::new(&plan.program[0]);
        cmd.args(&plan.program[1..]);
        let mut run = alerts::UpdateRun::new(true, format!("Running {}…", plan.shown));
        run.command = plan.shown.clone();
        crate::applog::info("install", format!("{provider}: running `{}`", plan.shown));
        self.update_runs.insert(provider, run);
        alerts::run_streaming(format!("install:{provider}"), cmd, self.tx.clone());
        cx.notify();
    }

    /// An install finished: find the new CLI and tell the person what's next.
    pub(crate) fn finish_install(&mut self, provider: &str, result: Result<String, String>) {
        let Some(p) = agent::PROVIDERS.iter().find(|p| p.id == provider) else {
            return;
        };
        if let Err(error) = result {
            // The install card stays and says what went wrong, with the log and a retry.
            if let Some(run) = self.update_runs.get_mut(p.id) {
                let failure = alerts::run_failure(&error, &run.command);
                crate::applog::warn(
                    "install",
                    format!(
                        "installing {} with `{}` failed: {} ({})",
                        p.name,
                        run.command,
                        failure.why,
                        crate::applog::gist(&error)
                    ),
                );
                run.failed = Some(failure);
            }
            return;
        }
        // Cursor's installer names its CLI `agent`; installers may also use a folder that
        // isn't on this app's PATH. Point the agent's setting at the real file then.
        let found = if agent::installed(p.bin) {
            None
        } else {
            locate(p.bin).or_else(|| (p.id == "cursor").then(|| locate("agent")).flatten())
        };
        if let Some(path) = &found {
            self.pending_binary = Some((p.id, path.to_string_lossy().into_owned()));
        }
        caps::refresh();
        let installed = found.is_some() || agent::installed(p.bin);
        if installed {
            crate::applog::info("install", format!("installed {}", p.name));
        } else {
            crate::applog::warn(
                "install",
                format!(
                    "the {} installer finished, but `{}` wasn't found",
                    p.name, p.bin
                ),
            );
        }
        if let Some(run) = self.update_runs.get_mut(p.id) {
            if installed {
                run.finished = Some(std::time::Instant::now());
            } else {
                run.failed = Some(updater::Failure {
                    why: format!(
                        "The installer finished, but `{}` wasn't found. Restart Runo, or set its path in Settings.",
                        p.bin
                    ),
                    ..Default::default()
                });
            }
        }
        self.info = Some((
            if installed {
                format!(
                    "{} is installed. Sign in once by running `{}` in a terminal, then pick it in New session.",
                    p.name,
                    found
                        .as_ref()
                        .map(|f| f.to_string_lossy().into_owned())
                        .unwrap_or_else(|| p.bin.to_string())
                )
            } else {
                format!(
                    "The {} installer finished, but `{}` wasn't found. Restart Runo, or set its path in Settings.",
                    p.name, p.bin
                )
            },
            std::time::Instant::now(),
        ));
    }

    /// The confirm card for installing an agent.
    pub(crate) fn install_modal(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let provider = self.install_confirm?;
        let p = agent::provider(provider);
        let plan = install_plan(provider, agent::installed("npm"), cfg!(windows));
        let backdrop = if is_dark() {
            hsla(0., 0., 0., 0.55)
        } else {
            hsla(0., 0., 0., 0.3)
        };
        let body = match &plan {
            Ok(plan) => div()
                .v_flex()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(c_secondary()))
                        .child("Runo runs the official installer from the vendor:"),
                )
                .child(
                    div()
                        .px_3()
                        .py_2()
                        .rounded_lg()
                        .bg(tint(c_subtle()))
                        .border_1()
                        .border_color(rgb(c_border()))
                        .font_family(self.mono_font.clone())
                        .text_size(px(12.))
                        .child(plan.shown.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(c_muted()))
                        .child("It downloads the CLI and puts it on your computer. Afterwards, sign in once by running it in a terminal."),
                ),
            Err(why) => div()
                .text_sm()
                .text_color(rgb(c_secondary()))
                .child(why.clone()),
        };
        let card = div()
            .id("install-card")
            .w(px(480.))
            .max_w(relative(0.94))
            .p_6()
            .v_flex()
            .gap_4()
            .rounded_2xl()
            .border_1()
            .border_color(float_edge())
            .bg(float(c_canvas()))
            .shadow_2xl()
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(self.provider_badge(p, 36.))
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("Install {}?", p.name)),
                    ),
            )
            .child(body)
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .when(plan.is_err(), |el| {
                        el.child(
                            self.button("install-locate", "Locate…", false)
                                .child(icon(IconName::FolderOpen, 13., c_secondary()))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.locate_agent(provider, cx)
                                })),
                        )
                        .child(
                            self.button("install-redetect", "Re-detect", false)
                                .child(icon(IconName::RefreshCw, 13., c_secondary()))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.install_confirm = None;
                                    this.info = Some((
                                        "Looking for agents and Node.js again…".into(),
                                        std::time::Instant::now(),
                                    ));
                                    redetect();
                                    cx.notify();
                                })),
                        )
                    })
                    .child(
                        self.button("install-cancel", "Cancel", false)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.install_confirm = None;
                                cx.notify();
                            })),
                    )
                    .when(plan.is_ok(), |el| {
                        el.child(
                            self.button("install-go", "Install", true)
                                .child(icon(IconName::Download, 13., c_canvas()))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.run_install(provider, cx)
                                })),
                        )
                    }),
            );
        Some(
            fade(
                div()
                    .id("install-backdrop")
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(backdrop)
                    .occlude()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.install_confirm = None;
                        cx.notify();
                    }))
                    .child(rise(card, "install-card-rise", 300)),
                "install-backdrop-fade",
                200,
            )
            .into_any_element(),
        )
    }

    /// "Install" (or "Installing…") for an agent that isn't on this computer.
    pub(crate) fn install_button(
        &self,
        provider: &'static str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let busy = self
            .update_runs
            .get(provider)
            .is_some_and(|r| r.install && r.running());
        div()
            .id(SharedString::from(format!("install-{provider}")))
            .flex_none()
            .h(px(22.))
            .px_2()
            .flex()
            .items_center()
            .gap_1()
            .rounded_md()
            .border_1()
            .border_color(rgb(c_border()))
            .bg(rgb(c_raised()))
            .text_size(px(11.5))
            .font_weight(FontWeight::MEDIUM)
            .when(!busy, |el| {
                el.cursor_pointer()
                    .hover(|el| el.bg(tint(c_subtle())))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.ask_install(provider);
                        cx.notify();
                    }))
            })
            .child(icon(
                if busy {
                    IconName::Loader
                } else {
                    IconName::Download
                },
                11.,
                c_secondary(),
            ))
            .child(if busy { "Installing…" } else { "Install" })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn every_agent_has_an_installer_or_a_reason() {
        let shown = |p, npm, win| install_plan(p, npm, win).map(|i| i.shown);
        assert_eq!(
            shown("claude", true, false).unwrap(),
            "curl -fsSL https://claude.ai/install.sh | bash"
        );
        assert_eq!(
            shown("codex", true, false).unwrap(),
            "npm install -g @openai/codex"
        );
        assert!(shown("codex", false, false)
            .unwrap_err()
            .contains("Node.js"));
        assert_eq!(
            shown("cursor", false, true).unwrap(),
            "irm 'https://cursor.com/install?win32=true' | iex"
        );
        assert_eq!(shown("droid", true, false).unwrap(), "npm install -g droid");
        assert!(shown("grok", false, true).is_err());
        for p in agent::PROVIDERS.iter() {
            assert!(
                install_plan(p.id, true, false).is_ok(),
                "{} has no installer",
                p.id
            );
        }
        let windows = install_plan("claude", false, true).unwrap();
        assert_eq!(windows.program[0], "powershell");
    }

    #[test]
    fn each_agent_has_a_way_to_sign_in() {
        assert_eq!(sign_in_command("codex", "codex"), "codex login");
        assert_eq!(
            sign_in_command("opencode", "/opt/bin/opencode"),
            "/opt/bin/opencode auth login"
        );
        assert_eq!(sign_in_command("claude", "claude"), "claude");
        if !cfg!(windows) {
            assert_eq!(
                sign_in_command("amp", "/Users/me/My Tools/amp"),
                "'/Users/me/My Tools/amp' login"
            );
        }
    }
}
