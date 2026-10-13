//! Every copy of each agent CLI on this computer: the ones on PATH and the ones in the
//! folders installers use (nvm, ~/.local/bin, Homebrew, npm, Bun, Cargo, Runo's own), with
//! their versions. Settings lists them so the person can pick the one Runo runs, and an
//! older copy that comes first on PATH is flagged.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

/// One copy of an agent CLI.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Install {
    /// Where it was found, as it would be run.
    pub path: PathBuf,
    /// The file it leads to, after links.
    pub real: PathBuf,
    /// "0.162.1", or "" when it didn't say.
    pub version: String,
    /// The app that ships it inside its own folder, like Orca.
    pub owner: Option<String>,
    /// `path` is a link to a file somewhere else.
    pub link: bool,
    /// Its folder is on PATH, so a plain `codex` may find it.
    pub on_path: bool,
}

static FOUND: RwLock<Option<HashMap<String, Vec<Install>>>> = RwLock::new(None);

/// The copies of an agent found by the last search.
pub fn get(provider: &str) -> Vec<Install> {
    FOUND
        .read()
        .ok()
        .and_then(|m| m.as_ref()?.get(provider).cloned())
        .unwrap_or_default()
}

/// Look for every copy of an agent again, on this thread.
pub fn refresh(provider: &'static crate::agent::Provider) {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_default();
    let dirs = search_dirs(&home, std::env::var_os("PATH").as_deref());
    let mut found: Vec<Install> = find(provider.bin, &dirs)
        .into_iter()
        .map(|mut c| {
            c.version = crate::caps::short_version(&crate::updater::version_of(&c.path));
            c
        })
        .collect();
    // A copy picked by hand somewhere else still belongs in the list.
    let bin = crate::caps::binary(provider);
    if let Some(chosen) = Path::new(&bin).is_absolute().then(|| PathBuf::from(&bin)) {
        let real = std::fs::canonicalize(&chosen).unwrap_or_else(|_| chosen.clone());
        if chosen.is_file() && !found.iter().any(|c| c.real == real) {
            found.push(Install {
                version: crate::caps::short_version(&crate::updater::version_of(&chosen)),
                owner: crate::updater::owner_app(&real),
                link: is_link(&chosen),
                path: chosen,
                real,
                on_path: false,
            });
        }
    }
    if let Ok(mut map) = FOUND.write() {
        map.get_or_insert_with(HashMap::new)
            .insert(provider.id.to_string(), found);
    }
    crate::caps::mark_updated();
}

/// Folders to look in, in order, each with whether it is on PATH: PATH first, then the
/// folders installers use, then every Node.js version nvm or fnm keeps.
pub(crate) fn search_dirs(home: &Path, path: Option<&std::ffi::OsStr>) -> Vec<(PathBuf, bool)> {
    let mut dirs: Vec<(PathBuf, bool)> = path
        .map(|p| std::env::split_paths(p).map(|d| (d, true)).collect())
        .unwrap_or_default();
    let mut more = vec![
        home.join(".local/bin"),
        home.join(".npm-global/bin"),
        home.join(".bun/bin"),
        home.join(".cargo/bin"),
        home.join(".volta/bin"),
        home.join(".opencode/bin"),
        home.join(".factory/bin"),
        home.join(".grok/bin"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        crate::store::data_dir().join("agents").join("bin"),
    ];
    if let Some(appdata) = std::env::var_os("APPDATA") {
        more.push(PathBuf::from(appdata).join("npm"));
    }
    for versions in [
        home.join(".nvm/versions/node"),
        home.join(".local/share/fnm/node-versions"),
        home.join("Library/Application Support/fnm/node-versions"),
    ] {
        let Ok(entries) = std::fs::read_dir(&versions) else {
            continue;
        };
        let mut found: Vec<(String, PathBuf)> = entries
            .flatten()
            .map(|e| {
                let dir = e.path();
                let name = e.file_name().to_string_lossy().into_owned();
                let fnm = dir.join("installation/bin");
                let bin = if fnm.is_dir() { fnm } else { dir.join("bin") };
                (name, bin)
            })
            .collect();
        // Newest Node.js first, by the version folder's own name: the rest of the path
        // can hold digits too.
        found.sort_by_key(|(name, _)| std::cmp::Reverse(crate::alerts::version_parts(name)));
        more.extend(found.into_iter().map(|(_, bin)| bin));
    }
    for dir in more {
        if !dirs.iter().any(|(d, _)| *d == dir) {
            dirs.push((dir, false));
        }
    }
    dirs
}

fn is_link(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink())
}

/// The copies of `bin` in `dirs`, one per real file, without their versions yet.
pub(crate) fn find(bin: &str, dirs: &[(PathBuf, bool)]) -> Vec<Install> {
    let suffixes: &[&str] = if cfg!(windows) {
        &["", ".exe", ".cmd", ".bat"]
    } else {
        &[""]
    };
    let mut out: Vec<Install> = Vec::new();
    for (dir, on_path) in dirs {
        for suffix in suffixes {
            let path = dir.join(format!("{bin}{suffix}"));
            if !path.is_file() {
                continue;
            }
            let real = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
            if let Some(seen) = out.iter_mut().find(|c| c.real == real) {
                seen.on_path |= on_path;
                continue;
            }
            out.push(Install {
                owner: crate::updater::owner_app(&real),
                link: is_link(&path),
                path,
                real,
                version: String::new(),
                on_path: *on_path,
            });
        }
    }
    out
}

/// The newest copy that reported a version.
pub fn newest(copies: &[Install]) -> Option<&Install> {
    copies
        .iter()
        .filter(|c| !c.version.is_empty())
        .max_by_key(|c| crate::alerts::version_parts(&c.version))
}

/// The copy Runo runs for an agent, among `copies`.
pub fn in_use<'a>(
    provider: &'static crate::agent::Provider,
    copies: &'a [Install],
) -> Option<&'a Install> {
    let runs = crate::updater::resolve(&crate::caps::binary(provider))?;
    let real = std::fs::canonicalize(&runs).unwrap_or(runs);
    copies.iter().find(|c| c.real == real)
}

/// The copy Runo runs and a newer one it could run instead, when the one it runs is older.
pub fn shadow<'a>(
    copies: &'a [Install],
    using: Option<&'a Install>,
) -> Option<(&'a Install, &'a Install)> {
    let using = using?;
    let newest = newest(copies)?;
    crate::alerts::is_newer(&newest.version, &using.version).then_some((using, newest))
}

/// For an agent: the copy Runo runs and a newer one already on this computer.
pub fn shadowed(provider: &'static crate::agent::Provider) -> Option<(Install, Install)> {
    let copies = get(provider.id);
    shadow(&copies, in_use(provider, &copies)).map(|(a, b)| (a.clone(), b.clone()))
}

/// A newer copy than `version` already on this computer: (path, version).
pub fn newer_than(provider: &'static crate::agent::Provider, version: &str) -> Option<Install> {
    newest(&get(provider.id))
        .filter(|c| crate::alerts::is_newer(&c.version, version))
        .cloned()
}

/// Remove a link to a CLI. Only links are removed, never the file they lead to.
pub fn remove_link(path: &Path) -> Result<(), String> {
    if !is_link(path) {
        return Err(format!(
            "{} isn't a link, so Runo leaves it alone. Remove it the way it was installed.",
            path.display()
        ));
    }
    std::fs::remove_file(path).map_err(|e| format!("Couldn't remove {}: {e}", path.display()))?;
    if let Ok(mut map) = FOUND.write() {
        if let Some(map) = map.as_mut() {
            for list in map.values_mut() {
                list.retain(|c| c.path != path);
            }
        }
    }
    crate::caps::mark_updated();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("runo-copies-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn copy(path: &str, version: &str) -> Install {
        Install {
            path: path.into(),
            real: path.into(),
            version: version.into(),
            ..Default::default()
        }
    }

    #[test]
    fn search_covers_path_then_installer_and_nvm_folders() {
        let home = temp("dirs");
        for v in ["v18.20.0", "v22.12.0"] {
            std::fs::create_dir_all(home.join(".nvm/versions/node").join(v).join("bin")).unwrap();
        }
        let path = std::env::join_paths([home.join("bin"), home.join(".local/bin")]).unwrap();
        let dirs = search_dirs(&home, Some(&path));
        assert_eq!(dirs[0], (home.join("bin"), true));
        assert_eq!(dirs[1], (home.join(".local/bin"), true));
        assert_eq!(
            dirs.iter()
                .filter(|(d, _)| *d == home.join(".local/bin"))
                .count(),
            1,
            "a PATH folder isn't listed twice"
        );
        assert!(dirs.contains(&(home.join(".cargo/bin"), false)));
        assert!(dirs.contains(&(home.join(".bun/bin"), false)));
        assert!(dirs.contains(&(PathBuf::from("/opt/homebrew/bin"), false)));
        let nvm: Vec<&PathBuf> = dirs
            .iter()
            .map(|(d, _)| d)
            .filter(|d| d.to_string_lossy().contains(".nvm"))
            .collect();
        assert_eq!(
            nvm,
            [
                &home.join(".nvm/versions/node/v22.12.0/bin"),
                &home.join(".nvm/versions/node/v18.20.0/bin")
            ],
            "newest Node.js first"
        );
    }

    #[cfg(unix)]
    #[test]
    fn finds_each_real_file_once_and_marks_links() {
        let home = temp("find");
        let (local, nvm, app) = (
            home.join(".local/bin"),
            home.join(".nvm/versions/node/v22.12.0/bin"),
            home.join("Library/Application Support/Orca/runtime/bin"),
        );
        for d in [&local, &nvm, &app] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(app.join("codex"), "#!/bin/sh\n").unwrap();
        std::os::unix::fs::symlink(app.join("codex"), local.join("codex")).unwrap();
        std::fs::write(nvm.join("codex"), "#!/bin/sh\n").unwrap();
        let dirs = vec![
            (local.clone(), true),
            (nvm.clone(), false),
            (app.clone(), false),
        ];
        let found = find("codex", &dirs);
        assert_eq!(found.len(), 2, "the link and its target are one copy");
        assert_eq!(found[0].path, local.join("codex"));
        assert!(found[0].link && found[0].on_path);
        assert_eq!(found[0].owner.as_deref(), Some("Orca"));
        assert_eq!(found[1].path, nvm.join("codex"));
        assert!(!found[1].link && !found[1].on_path && found[1].owner.is_none());
        assert!(find("gemini", &dirs).is_empty());

        assert!(
            remove_link(&nvm.join("codex")).is_err(),
            "only links are removed"
        );
        remove_link(&local.join("codex")).unwrap();
        assert!(!local.join("codex").exists() && app.join("codex").is_file());
    }

    #[test]
    fn an_older_copy_in_use_is_shadowing_a_newer_one() {
        let copies = vec![
            copy("/home/a/.local/bin/codex", "0.147.0"),
            copy("/home/a/.nvm/versions/node/v22.12.0/bin/codex", "0.162.1"),
            copy("/opt/homebrew/bin/codex", ""),
        ];
        assert_eq!(newest(&copies).unwrap().version, "0.162.1");
        let (using, better) = shadow(&copies, Some(&copies[0])).unwrap();
        assert_eq!(
            (using.version.as_str(), better.version.as_str()),
            ("0.147.0", "0.162.1")
        );
        assert!(shadow(&copies, Some(&copies[1])).is_none());
        assert!(shadow(&copies, None).is_none());
        assert!(newest(&[copy("/x", "")]).is_none());
    }
}
