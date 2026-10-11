//! Running a session inside its own Cove VM (https://github.com/deltalark/cove).
//!
//! Cove is a separate product. Runo only talks to it through the `cove` command:
//! every process a Cove session starts (the agent, the setup command, terminal tabs)
//! is wrapped as `cove exec runo-<id> --create … -- <program>`, which creates or
//! starts the VM on first use. The worktree is shared into the VM at the same path,
//! so git, diffs and commits keep running on the Mac against the same files.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Credentials passed into the VM when set in Runo's environment. Nothing else from
/// the Mac (home folder, keychain, SSH agent) reaches the VM.
const FORWARDED_ENV: [&str; 4] = [
    "ANTHROPIC_API_KEY",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "OPENAI_API_KEY",
    "OPENROUTER_API_KEY",
];

/// The `cove` executable, looking in the usual install folders too because apps
/// started from the Dock don't get the shell's PATH.
pub fn binary() -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(Path::new(&home).join(".cargo/bin"));
    }
    dirs.into_iter()
        .map(|d| d.join("cove"))
        .find(|p| p.is_file())
}

/// Whether Cove is installed and its server answers. Cached and refreshed in the
/// background, so it is cheap to call while rendering.
pub fn available() -> bool {
    static STATE: OnceLock<Mutex<(Option<Instant>, bool)>> = OnceLock::new();
    let state = STATE.get_or_init(|| Mutex::new((None, false)));
    let Ok(mut guard) = state.lock() else {
        return false;
    };
    let stale = guard
        .0
        .is_none_or(|t| t.elapsed() > Duration::from_secs(15));
    if stale {
        guard.0 = Some(Instant::now());
        std::thread::spawn(|| {
            let ok = binary().is_some_and(|bin| {
                Command::new(bin)
                    .args(["--json", "ls"])
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .is_ok_and(|s| s.success())
            });
            if let Some(state) = STATE.get() {
                if let Ok(mut g) = state.lock() {
                    g.1 = ok;
                }
            }
        });
    }
    guard.1
}

pub fn vm_name(session: u64) -> String {
    format!("runo-{session}")
}

/// Delete a session's VM, in the background.
pub fn remove(session: u64) {
    let Some(bin) = binary() else { return };
    std::thread::spawn(move || {
        let _ = Command::new(bin)
            .args(["rm", &vm_name(session)])
            .stdin(std::process::Stdio::null())
            .output();
    });
}

/// How to run things for one Cove session.
#[derive(Clone, Debug)]
pub struct Sandbox {
    pub name: String,
    /// `--mount` values: the session folder, and the repository's shared `.git`
    /// read-only when the folder is a worktree (so git works inside but the agent
    /// can't rewrite the main repository).
    pub mounts: Vec<String>,
}

impl Sandbox {
    pub fn for_session(session: u64, dir: &str) -> Sandbox {
        let mut mounts = vec![format!("{dir}:{dir}")];
        if let Some(common) = git_common_dir(dir) {
            if !Path::new(&common).starts_with(dir) {
                mounts.push(format!("{common}:{common}:ro"));
            }
        }
        Sandbox {
            name: vm_name(session),
            mounts,
        }
    }

    /// The `cove exec` arguments up to (not including) the program.
    fn exec_args(&self, cwd: &str, pty: bool) -> Vec<String> {
        let mut args = vec!["exec".to_string(), self.name.clone(), "--create".into()];
        for m in &self.mounts {
            args.push("--mount".into());
            args.push(m.clone());
        }
        args.push("--cwd".into());
        args.push(cwd.into());
        if pty {
            args.push("-t".into());
        }
        args
    }

    /// Turn `command` (an agent or shell run on the Mac) into the same run inside the
    /// VM. Its environment travels as `-e` flags; the agent CLI is installed in the
    /// VM on first use.
    pub fn wrap(&self, command: &Command, cwd: &str, pty: bool) -> Result<Command, String> {
        let bin = binary().ok_or("Cove is not installed, so this session can't start its VM.")?;
        let program = Path::new(command.get_program())
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut args = self.exec_args(cwd, pty);
        let mut env: Vec<(String, String)> = FORWARDED_ENV
            .iter()
            .filter_map(|k| std::env::var(k).ok().map(|v| (k.to_string(), v)))
            .collect();
        for (k, v) in command.get_envs() {
            if let Some(v) = v {
                env.retain(|(key, _)| key.as_str() != k);
                env.push((
                    k.to_string_lossy().into_owned(),
                    v.to_string_lossy().into_owned(),
                ));
            }
        }
        for (k, v) in env {
            args.push("-e".into());
            args.push(format!("{k}={v}"));
        }
        args.extend([
            "--".into(),
            "bash".into(),
            "-lc".into(),
            launcher(&program),
            "runo".into(),
            program,
        ]);
        args.extend(command.get_args().map(|a| a.to_string_lossy().into_owned()));
        let mut wrapped = Command::new(bin);
        wrapped.args(args);
        Ok(wrapped)
    }

    /// Arguments for an interactive login shell in the VM (a terminal tab).
    pub fn shell_args(&self, cwd: &str) -> Vec<String> {
        let mut args = self.exec_args(cwd, true);
        args.extend(["--".into(), "bash".into(), "-l".into()]);
        args
    }
}

/// A `bash -lc` script that installs `program` in the VM if it is missing, then runs
/// it with the remaining arguments.
fn launcher(program: &str) -> String {
    let install = match program {
        "claude" => "curl -fsSL https://claude.ai/install.sh | bash",
        "codex" => {
            "(command -v npm >/dev/null || (apt-get update -qq && apt-get install -y -qq nodejs npm)) \
             && npm install -g @openai/codex"
        }
        "opencode" => "curl -fsSL https://opencode.ai/install | bash",
        _ => "",
    };
    if install.is_empty() {
        return r#"exec "$@""#.to_string();
    }
    format!(
        r#"export PATH="$HOME/.local/bin:$HOME/.opencode/bin:$PATH"
command -v "$1" >/dev/null 2>&1 || {{ echo "Installing $1 in the Cove VM…" >&2; {{ {install}; }} >&2 || exit 127; }}
exec "$@""#
    )
}

/// The repository's shared git folder for a worktree, as an absolute path.
fn git_common_dir(dir: &str) -> Option<String> {
    let out = Command::new("git")
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .current_dir(dir)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!path.is_empty()).then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sandbox() -> Sandbox {
        Sandbox {
            name: "runo-7".into(),
            mounts: vec![
                "/code/.runo-worktrees/app/x:/code/.runo-worktrees/app/x".into(),
                "/code/app/.git:/code/app/.git:ro".into(),
            ],
        }
    }

    #[test]
    fn shell_runs_in_the_session_folder() {
        assert_eq!(
            sandbox().shell_args("/w").join(" "),
            "exec runo-7 --create --mount /code/.runo-worktrees/app/x:/code/.runo-worktrees/app/x \
             --mount /code/app/.git:/code/app/.git:ro --cwd /w -t -- bash -l"
        );
    }

    #[test]
    fn launcher_installs_known_agents_only() {
        assert!(launcher("claude").contains("claude.ai/install.sh"));
        assert!(launcher("codex").contains("@openai/codex"));
        assert_eq!(launcher("sh"), r#"exec "$@""#);
    }

    #[test]
    fn worktrees_get_the_shared_git_folder_read_only() {
        let root = std::env::temp_dir().join(format!("runo-cove-{}", std::process::id()));
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let git = |dir: &Path, args: &[&str]| {
            assert!(Command::new("git")
                .args(args)
                .current_dir(dir)
                .output()
                .unwrap()
                .status
                .success());
        };
        git(&repo, &["init", "-q"]);
        git(
            &repo,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "x",
            ],
        );
        let wt = root.join("wt");
        git(&repo, &["worktree", "add", "-q", wt.to_str().unwrap()]);
        let wt = wt.canonicalize().unwrap();
        let s = Sandbox::for_session(3, wt.to_str().unwrap());
        let git_dir = repo.canonicalize().unwrap().join(".git");
        assert_eq!(s.name, "runo-3");
        assert_eq!(s.mounts[1], format!("{0}:{0}:ro", git_dir.display()));
        // The project folder itself needs no extra mount.
        let own = Sandbox::for_session(4, repo.canonicalize().unwrap().to_str().unwrap());
        assert_eq!(own.mounts.len(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Runs a real command through Cove. Needs a running `cove serve` that allows the
    /// dev backend: `COVE_E2E=1 COVE_BACKEND=host cargo test -p runo cove`.
    #[test]
    fn wrapped_commands_run_in_the_vm() {
        if std::env::var("COVE_E2E").is_err() || binary().is_none() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("runo-cove-e2e-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dir = dir.canonicalize().unwrap();
        let dir = dir.to_str().unwrap();
        let sandbox = Sandbox::for_session(900_000 + std::process::id() as u64, dir);
        let mut command = Command::new("sh");
        command
            .args(["-c", "pwd; echo \"$GREETING\""])
            .env("GREETING", "hi from runo");
        let out = sandbox
            .wrap(&command, dir, false)
            .unwrap()
            .output()
            .unwrap();
        let _ = Command::new(binary().unwrap())
            .args(["rm", &sandbox.name])
            .output();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            format!("{dir}\nhi from runo\n")
        );
    }
}
