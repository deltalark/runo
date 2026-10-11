//! How to update the exact agent CLI Runo runs, and what to tell the person when an
//! update can't or shouldn't run: another app owns the copy, npm needs admin rights,
//! a file is in the way, or the network is down.

use std::path::{Path, PathBuf};

/// How to update one copy of an agent CLI.
#[derive(Debug, PartialEq)]
pub(crate) enum Plan {
    /// Run this updater. `installs_to` is where the updated CLI will be, when known.
    Run {
        program: Vec<String>,
        installs_to: Option<PathBuf>,
    },
    /// Runo shouldn't update this copy itself: say why, and what to run instead.
    Manual {
        why: String,
        command: Option<String>,
    },
}

/// A failed or blocked update or install, explained.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Failure {
    pub why: String,
    /// The tool's full log, when it named one.
    pub log: Option<String>,
    /// A command the person can copy and run in a terminal.
    pub command: Option<String>,
    /// Another copy of the CLI that is newer, to switch Runo to: (path, version).
    pub use_copy: Option<(String, String)>,
    /// Nothing went wrong yet; the person has to decide or act first.
    pub needs_you: bool,
    /// Offer "Update anyway" (the agent is busy in running sessions).
    pub force: bool,
}

/// The file Runo starts for an agent: its Settings path, or the first match on PATH.
pub(crate) fn resolve(bin: &str) -> Option<PathBuf> {
    if bin.contains('/') || bin.contains('\\') {
        let path = PathBuf::from(bin);
        return path.is_file().then_some(path);
    }
    let suffixes: &[&str] = if cfg!(windows) {
        &["", ".exe", ".cmd", ".bat"]
    } else {
        &[""]
    };
    std::env::split_paths(&std::env::var_os("PATH")?).find_map(|dir| {
        suffixes
            .iter()
            .map(|s| dir.join(format!("{bin}{s}")))
            .find(|p| p.is_file())
    })
}

fn slashes(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// The app that ships this copy of a CLI inside its own folder, like Orca.
pub(crate) fn owner_app(real: &Path) -> Option<String> {
    let s = slashes(real);
    let after = |marker: &str| {
        let (_, rest) = s.split_once(marker)?;
        rest.split('/').next().map(str::to_string)
    };
    let app = after("/Application Support/")
        .or_else(|| {
            s.split('/')
                .find(|seg| seg.ends_with(".app"))
                .map(|seg| seg.trim_end_matches(".app").to_string())
        })
        .or_else(|| after("/AppData/Local/Programs/"))?;
    // Runo itself and generic tool folders aren't "another app".
    let lower = app.to_lowercase();
    (!app.is_empty() && !["runo", "npm", "node", "nvm", "fnm", "volta"].contains(&lower.as_str()))
        .then(|| {
            let mut chars = app.chars();
            chars
                .next()
                .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
                .unwrap_or_default()
        })
}

/// npm's global prefix and the package, for a CLI that lives in a global node_modules.
pub(crate) fn npm_location(real: &Path) -> Option<(String, String)> {
    let s = slashes(real);
    let (before, after) = s.split_once("/node_modules/")?;
    let prefix = before.strip_suffix("/lib").unwrap_or(before).to_string();
    let mut parts = after.split('/');
    let first = parts.next()?;
    let package = if first.starts_with('@') {
        format!("{first}/{}", parts.next()?)
    } else {
        first.to_string()
    };
    Some((prefix, package))
}

/// How to update the copy of `provider` that Runo runs, at `runs` (resolving to `real`).
pub(crate) fn plan(provider: &str, name: &str, runs: &Path, real: &Path) -> Plan {
    let package = crate::alerts::npm_package(provider);
    let npm_cmd = |pkg: &str| format!("npm install -g {pkg}@latest");
    let s = slashes(real);
    if let Some(app) = owner_app(real) {
        return Plan::Manual {
            why: format!(
                "The {name} Runo uses comes with {app} ({}), so {app} keeps it updated. To have your own copy, install it with the command below, then pick it under Settings › {name}.",
                runs.display()
            ),
            command: package.map(npm_cmd),
        };
    }
    for (marker, cask) in [("/Cellar/", false), ("/Caskroom/", true)] {
        if let Some((_, rest)) = s.split_once(marker) {
            let formula = rest.split('/').next().unwrap_or(name).to_string();
            let mut program = vec!["brew".to_string(), "upgrade".into()];
            if cask {
                program.push("--cask".into());
            }
            program.push(formula);
            return Plan::Run {
                program,
                installs_to: None,
            };
        }
    }
    if let Some((prefix, pkg)) = npm_location(real) {
        // Use the npm that belongs to that prefix (nvm, Homebrew node, a bundled node),
        // so the update lands in the copy Runo runs, not in some other npm's folder.
        let own_npm = Path::new(&prefix).join(if cfg!(windows) { "npm.cmd" } else { "bin/npm" });
        let npm = if own_npm.is_file() {
            own_npm.to_string_lossy().into_owned()
        } else {
            "npm".into()
        };
        let bin_dir = if cfg!(windows) {
            PathBuf::from(&prefix)
        } else {
            Path::new(&prefix).join("bin")
        };
        let file = runs.file_name().map(|f| bin_dir.join(f));
        return Plan::Run {
            program: vec![
                npm,
                "install".into(),
                "-g".into(),
                "--prefix".into(),
                prefix,
                format!("{pkg}@latest"),
                "--loglevel".into(),
                "http".into(),
            ],
            installs_to: file,
        };
    }
    let own = |arg: &str| Plan::Run {
        program: vec![runs.to_string_lossy().into_owned(), arg.into()],
        installs_to: None,
    };
    match provider {
        "claude" => own("update"),
        "opencode" => own("upgrade"),
        "cursor" => own("update"),
        _ => Plan::Manual {
            why: format!(
                "Runo can't tell how {} was installed, so it won't overwrite it. Update it the way you installed it, or install a fresh copy with the command below.",
                runs.display()
            ),
            command: package.map(npm_cmd),
        },
    }
}

/// Other copies of a CLI on this computer (PATH, installer folders, nvm), newest first by
/// path order, excluding `except`.
pub(crate) fn other_copies(bin: &str, except: &Path) -> Vec<PathBuf> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_default();
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    dirs.extend(crate::install::extra_dirs());
    if let Ok(entries) = std::fs::read_dir(home.join(".nvm/versions/node")) {
        let mut nvm: Vec<PathBuf> = entries.flatten().map(|e| e.path().join("bin")).collect();
        nvm.sort();
        nvm.reverse();
        dirs.extend(nvm);
    }
    let skip = std::fs::canonicalize(except).unwrap_or_else(|_| except.to_path_buf());
    let mut seen = vec![skip];
    let mut found = Vec::new();
    for dir in dirs {
        let path = dir.join(bin);
        if !path.is_file() {
            continue;
        }
        let real = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if seen.contains(&real) || owner_app(&real).is_some() {
            continue;
        }
        seen.push(real);
        found.push(path);
    }
    found
}

/// The version a CLI reports, or "" when it doesn't run.
pub(crate) fn version_of(path: &Path) -> String {
    std::process::Command::new(path)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// Why an update or install failed, in words a person can act on, from the tool's output.
/// `command` is the command that ran, to suggest a variant of it.
pub(crate) fn explain(output: &str, command: &str) -> Failure {
    let has = |s: &str| output.contains(s);
    let after = |marker: &str| {
        output.lines().find_map(|l| {
            let (_, rest) = l.split_once(marker)?;
            Some(rest.trim().trim_matches(['\'', '"']).to_string())
        })
    };
    let log = crate::alerts::log_path(output);
    let failure = |why: String, command: Option<String>| Failure {
        why,
        log: log.clone(),
        command,
        ..Failure::default()
    };
    let program = command.split_whitespace().next().unwrap_or("");
    let program = Path::new(program)
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default();
    if has("could not start") {
        return failure(
            match program.as_str() {
                "npm" | "npm.cmd" => "npm isn't installed, or Runo can't see it. Install Node.js from nodejs.org, then try again.".into(),
                "brew" => "Homebrew isn't on Runo's PATH. Run the command below in a terminal instead.".into(),
                _ => format!("Runo couldn't start `{program}`. Check that it's installed."),
            },
            Some(command.to_string()),
        );
    }
    if has("EACCES") || has("EPERM") && !cfg!(windows) {
        let folder = after("npm error path").unwrap_or_else(|| "npm's global folder".into());
        return failure(
            format!("npm needs admin rights to write to {folder}. Run the command below in a terminal, or point npm at a folder you own with `npm config set prefix ~/.npm-global`."),
            Some(format!("sudo {command}")),
        );
    }
    if has("EEXIST") {
        let file = after("File exists:")
            .or_else(|| after("npm error path"))
            .unwrap_or_else(|| "A file".into());
        let owner = std::fs::canonicalize(&file)
            .ok()
            .and_then(|real| owner_app(&real))
            .map(|app| format!(" It points into {app}, so replacing it changes {app}'s copy too."))
            .unwrap_or_default();
        return failure(
            format!("{file} already exists from another install, and npm won't replace it.{owner} Remove it, or run the command below to overwrite it."),
            Some(format!("{command} --force")),
        );
    }
    if has("ENOTEMPTY") {
        let folder = output
            .lines()
            .find(|l| l.contains("ENOTEMPTY") && l.contains(" -> "))
            // npm renames the old copy to a temporary ".name-xxxx" folder; that leftover
            // folder is what blocks the next update.
            .and_then(|l| l.split('\'').nth(3))
            .unwrap_or("the folder npm names in the log")
            .to_string();
        return failure(
            format!("An earlier npm update left a half-finished copy behind. Delete {folder} and try again."),
            Some(format!("rm -rf '{folder}'")),
        );
    }
    if has("EBUSY") || has("Text file busy") || has("EPERM") {
        return failure(
            "The CLI's files are in use, probably by a running session. Stop the sessions using it, then try again.".into(),
            None,
        );
    }
    if has("ETARGET") || has("E404") || has("No available formula") {
        return failure(
            "That version isn't available yet. It may still be publishing, so try again in a few minutes.".into(),
            None,
        );
    }
    if [
        "ENOTFOUND",
        "ETIMEDOUT",
        "ECONNRESET",
        "ECONNREFUSED",
        "EAI_AGAIN",
        "network",
    ]
    .iter()
    .any(|code| has(code))
    {
        return failure(
            "Couldn't reach the internet to download the update. Check your connection, then try again.".into(),
            None,
        );
    }
    failure(crate::alerts::failure_summary(output), None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn plans_follow_the_copy_runo_runs() {
        // Anish's Mac: ~/.local/bin/codex links into Orca's bundled runtime.
        let orca = Path::new("/Users/a/Library/Application Support/orca/codex-runtime-home/standalone/current/bin/codex");
        assert_eq!(owner_app(orca).as_deref(), Some("Orca"));
        match plan(
            "codex",
            "Codex",
            Path::new("/Users/a/.local/bin/codex"),
            orca,
        ) {
            Plan::Manual { why, command } => {
                assert!(why.contains("comes with Orca"), "{why}");
                assert_eq!(
                    command.as_deref(),
                    Some("npm install -g @openai/codex@latest")
                );
            }
            other => panic!("{other:?}"),
        }
        // nvm: update with that prefix, so the copy Runo runs is the one that changes.
        let nvm = Path::new(
            "/Users/a/.nvm/versions/node/v22.12.0/lib/node_modules/@openai/codex/bin/codex.js",
        );
        assert_eq!(
            npm_location(nvm),
            Some((
                "/Users/a/.nvm/versions/node/v22.12.0".into(),
                "@openai/codex".into()
            ))
        );
        match plan(
            "codex",
            "Codex",
            Path::new("/Users/a/.nvm/versions/node/v22.12.0/bin/codex"),
            nvm,
        ) {
            Plan::Run {
                program,
                installs_to,
            } => {
                assert_eq!(
                    &program[1..6],
                    [
                        "install",
                        "-g",
                        "--prefix",
                        "/Users/a/.nvm/versions/node/v22.12.0",
                        "@openai/codex@latest"
                    ]
                );
                assert_eq!(
                    installs_to,
                    Some(PathBuf::from(
                        "/Users/a/.nvm/versions/node/v22.12.0/bin/codex"
                    ))
                );
            }
            other => panic!("{other:?}"),
        }
        // Homebrew formula and cask.
        let brew = Path::new("/opt/homebrew/Cellar/opencode/0.9.1/bin/opencode");
        assert_eq!(
            plan("opencode", "OpenCode", brew, brew),
            Plan::Run {
                program: vec!["brew".into(), "upgrade".into(), "opencode".into()],
                installs_to: None
            }
        );
        let cask = Path::new("/opt/homebrew/Caskroom/codex/0.50.0/codex-aarch64-apple-darwin");
        match plan("codex", "Codex", cask, cask) {
            Plan::Run { program, .. } => {
                assert_eq!(program, ["brew", "upgrade", "--cask", "codex"])
            }
            other => panic!("{other:?}"),
        }
        // Claude Code's own install updates itself.
        let native = Path::new("/Users/a/.local/share/claude/versions/2.1.200");
        match plan(
            "claude",
            "Claude Code",
            Path::new("/Users/a/.local/bin/claude"),
            native,
        ) {
            Plan::Run { program, .. } => {
                assert_eq!(program, ["/Users/a/.local/bin/claude", "update"])
            }
            other => panic!("{other:?}"),
        }
        // Windows npm puts node_modules right under the prefix.
        let win = Path::new(
            r"C:\Users\a\AppData\Roaming\npm\node_modules\@google\gemini-cli\dist\index.js",
        );
        assert_eq!(
            npm_location(win),
            Some((
                "C:/Users/a/AppData/Roaming/npm".into(),
                "@google/gemini-cli".into()
            ))
        );
        assert_eq!(owner_app(win), None);
        // A standalone binary of unknown origin isn't overwritten.
        let odd = Path::new("/opt/tools/codex");
        assert!(matches!(
            plan("codex", "Codex", odd, odd),
            Plan::Manual { .. }
        ));
    }

    #[test]
    fn failures_are_explained() {
        let cmd = "npm install -g --prefix /usr/local @openai/codex@latest";
        let eacces = explain(
            "npm error code EACCES\nnpm error path /usr/local/lib/node_modules",
            cmd,
        );
        assert!(eacces
            .why
            .contains("admin rights to write to /usr/local/lib/node_modules"));
        assert_eq!(eacces.command.as_deref(), Some(&*format!("sudo {cmd}")));
        let eexist = explain("npm error code EEXIST\nnpm error File exists: /Users/a/.local/bin/codex\nnpm error A complete log of this run can be found in: /Users/a/.npm/_logs/1.log", cmd);
        assert!(eexist
            .why
            .starts_with("/Users/a/.local/bin/codex already exists"));
        assert_eq!(eexist.log.as_deref(), Some("/Users/a/.npm/_logs/1.log"));
        assert!(eexist.command.unwrap().ends_with("--force"));
        assert!(explain(
            "npm error code ENOTFOUND\nnpm error network request failed",
            cmd
        )
        .why
        .contains("internet"));
        assert!(explain("could not start \"npm\": No such file", cmd)
            .why
            .contains("Node.js"));
        assert!(explain("npm error code EBUSY", cmd).why.contains("in use"));
    }
}
