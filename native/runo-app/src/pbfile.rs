//! Playbook files: a playbook as one Markdown file people share through Git, a link or
//! a chat, and Runo installs from the Playbooks page.
//!
//! ```text
//! +++
//! name = "Release notes"
//! description = "Draft notes for the next release"
//! author = "anish"
//! version = "1.0.0"
//! agent = "claude"
//! permissions = ["read", "shell"]
//!
//! [[inputs]]
//! key = "since"
//! label = "Compare from"
//! default = "the last tag"
//! +++
//!
//! Read the commits since {{since}} and write release notes…
//! ```
//!
//! The TOML header between the `+++` lines describes it; everything below is the
//! instructions, with `{{key}}` where an input goes.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The file name suffix Runo looks for.
pub const SUFFIX: &str = ".playbook.md";

/// What a playbook may do. Missing from a file means it only reads.
pub const PERMISSIONS: [(&str, &str, &str); 4] = [
    ("read", "Read files", "Look through the project"),
    ("edit", "Edit files", "Change files in the project"),
    (
        "shell",
        "Run commands",
        "Run shell commands such as tests or builds",
    ),
    (
        "network",
        "Use the network",
        "Fetch web pages or call services",
    ),
];

/// A value the person fills in when the playbook runs.
#[derive(Clone, Serialize, Deserialize, PartialEq, Debug, Default)]
pub struct Input {
    pub key: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub default: String,
}

impl Input {
    /// The label, or the key made readable: `base_branch` reads "Base branch".
    pub fn title(&self) -> String {
        if !self.label.trim().is_empty() {
            return self.label.trim().to_string();
        }
        let words = self.key.replace(['_', '-'], " ");
        let mut chars = words.chars();
        match chars.next() {
            Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
            None => String::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
struct Header {
    name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    description: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    author: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    version: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    agent: String,
    #[serde(default)]
    permissions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    inputs: Vec<Input>,
}

/// One playbook read from a file.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct PlaybookFile {
    pub name: String,
    pub description: String,
    pub author: String,
    pub version: String,
    /// Suggested agent id; empty leaves the choice to the person.
    pub agent: String,
    pub permissions: Vec<String>,
    pub inputs: Vec<Input>,
    pub prompt: String,
}

/// Read a playbook file's text.
pub fn parse(text: &str) -> Result<PlaybookFile, String> {
    let text = text.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let rest = text
        .trim_start()
        .strip_prefix("+++\n")
        .ok_or("It doesn't start with a +++ header.")?;
    let end = rest
        .find("\n+++")
        .ok_or("Its +++ header is never closed.")?;
    let header: Header = toml::from_str(&rest[..end]).map_err(|e| {
        let message = e.message().to_string();
        format!("Its header isn't valid: {message}")
    })?;
    let body = rest[end + 4..].trim().to_string();
    if header.name.trim().is_empty() {
        return Err("It has no name.".into());
    }
    if body.is_empty() {
        return Err("It has no instructions below the header.".into());
    }
    let known: Vec<&str> = PERMISSIONS.iter().map(|p| p.0).collect();
    let mut permissions: Vec<String> = header
        .permissions
        .iter()
        .map(|p| p.trim().to_lowercase())
        .filter(|p| known.contains(&p.as_str()))
        .collect();
    if !permissions.iter().any(|p| p == "read") {
        permissions.insert(0, "read".into());
    }
    permissions.dedup();
    Ok(PlaybookFile {
        name: header.name.trim().to_string(),
        description: header.description.trim().to_string(),
        author: header.author.trim().to_string(),
        version: header.version.trim().to_string(),
        agent: header.agent.trim().to_string(),
        permissions,
        inputs: header
            .inputs
            .into_iter()
            .filter(|i| valid_key(&i.key))
            .collect(),
        prompt: body,
    })
}

/// Write a playbook as file text.
pub fn render(file: &PlaybookFile) -> String {
    let header = Header {
        name: file.name.clone(),
        description: file.description.clone(),
        author: file.author.clone(),
        version: file.version.clone(),
        agent: file.agent.clone(),
        permissions: file.permissions.clone(),
        inputs: file.inputs.clone(),
    };
    let toml = toml::to_string(&header).unwrap_or_default();
    format!("+++\n{}+++\n\n{}\n", toml, file.prompt.trim())
}

/// A file name for a playbook: `Release notes` → `release-notes.playbook.md`.
pub fn file_name(name: &str) -> String {
    let slug: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    let slug = slug
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    format!(
        "{}{SUFFIX}",
        if slug.is_empty() { "playbook" } else { &slug }
    )
}

fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// The `{{key}}` placeholders in the instructions, in order, once each.
pub fn placeholders(prompt: &str) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    let mut rest = prompt;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else { break };
        let key = after[..end].trim();
        if valid_key(key) && !keys.iter().any(|k| k == key) {
            keys.push(key.to_string());
        }
        rest = &after[end + 2..];
    }
    keys
}

/// The inputs a playbook asks for: the declared ones its instructions use, then any other
/// placeholder in the instructions.
pub fn inputs_of(declared: &[Input], prompt: &str) -> Vec<Input> {
    placeholders(prompt)
        .into_iter()
        .map(|key| {
            declared
                .iter()
                .find(|i| i.key == key)
                .cloned()
                .unwrap_or(Input {
                    key,
                    ..Input::default()
                })
        })
        .collect()
}

/// The instructions with each `{{key}}` replaced (spaces inside the braces allowed): the
/// given value, else the default.
pub fn fill(prompt: &str, inputs: &[Input], values: &[(String, String)]) -> String {
    let value_of = |input: &Input| {
        let value = values
            .iter()
            .find(|(k, _)| *k == input.key)
            .map(|(_, v)| v.trim())
            .filter(|v| !v.is_empty())
            .unwrap_or(input.default.trim());
        if value.is_empty() {
            format!("(no {} given)", input.title().to_lowercase())
        } else {
            value.to_string()
        }
    };
    let mut out = String::with_capacity(prompt.len());
    let mut rest = prompt;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else { break };
        let key = after[..end].trim();
        out.push_str(&rest[..start]);
        match inputs.iter().find(|i| i.key == key) {
            Some(input) => out.push_str(&value_of(input)),
            None => out.push_str(&rest[start..start + 2 + end + 2]),
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
}

/// A playbook found while installing, with where it came from inside the source.
#[derive(Clone, Debug)]
pub struct Found {
    pub file: PlaybookFile,
    /// Path inside the source, or the link.
    pub origin: String,
}

/// What came back from a source: playbooks, and files that didn't read.
#[derive(Clone, Debug, Default)]
pub struct Fetched {
    pub found: Vec<Found>,
    pub skipped: Vec<(String, String)>,
}

/// Read every playbook in `source`: a `.playbook.md` file, a folder, a link to a file,
/// or a Git repository (optionally a GitHub `/tree/<branch>/<folder>` link).
pub fn fetch(source: &str) -> Result<Fetched, String> {
    let source = source.trim();
    if source.is_empty() {
        return Err("Paste a link, a Git repository or a file path.".into());
    }
    let web = source.starts_with("https://") || source.starts_with("http://");
    if web && is_file_link(source) {
        let text = download(&raw_link(source))?;
        let file = parse(&text).map_err(|e| format!("That file isn't a playbook. {e}"))?;
        return Ok(Fetched {
            found: vec![Found {
                file,
                origin: source.to_string(),
            }],
            skipped: Vec::new(),
        });
    }
    if web
        || source.starts_with("git@")
        || source.starts_with("file://")
        || source.ends_with(".git")
    {
        return fetch_repo(source);
    }
    let path = expand_home(source);
    if path.is_file() {
        let text = std::fs::read_to_string(&path).map_err(|e| format!("Can't read it: {e}"))?;
        let file = parse(&text).map_err(|e| format!("That file isn't a playbook. {e}"))?;
        return Ok(Fetched {
            found: vec![Found {
                file,
                origin: path.display().to_string(),
            }],
            skipped: Vec::new(),
        });
    }
    if path.is_dir() {
        return scan(&path).map(|mut fetched| {
            relative_to(&mut fetched, &path);
            fetched
        });
    }
    Err("Nothing found there. Use a link, a Git repository, or a file or folder path.".into())
}

fn expand_home(source: &str) -> PathBuf {
    match source.strip_prefix("~/") {
        Some(rest) => std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join(rest))
            .unwrap_or_else(|| PathBuf::from(source)),
        None => PathBuf::from(source),
    }
}

fn is_file_link(url: &str) -> bool {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    path.ends_with(".md") || path.ends_with(".txt")
}

/// A GitHub or GitLab page link turned into the raw file.
fn raw_link(url: &str) -> String {
    if let Some(rest) = url.strip_prefix("https://github.com/") {
        if let Some((repo, path)) = rest.split_once("/blob/") {
            return format!("https://raw.githubusercontent.com/{repo}/{path}");
        }
    }
    if url.contains("gitlab") && url.contains("/-/blob/") {
        return url.replacen("/-/blob/", "/-/raw/", 1);
    }
    url.to_string()
}

fn download(url: &str) -> Result<String, String> {
    let out = Command::new("curl")
        .args(["-fsSL", "--max-time", "20", url])
        .output()
        .map_err(|_| "Couldn't download it: curl isn't available.".to_string())?;
    if !out.status.success() {
        return Err(format!(
            "Couldn't download it. {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    String::from_utf8(out.stdout).map_err(|_| "That file isn't text.".into())
}

/// Split a GitHub `/tree/<branch>/<folder>` link into the repository, branch and folder.
fn split_tree(url: &str) -> (String, Option<String>, Option<String>) {
    let clean = url.trim_end_matches('/');
    if let Some((repo, tail)) = clean.split_once("/tree/") {
        let mut parts = tail.splitn(2, '/');
        let branch = parts.next().map(str::to_string);
        let folder = parts.next().map(str::to_string);
        return (repo.to_string(), branch, folder);
    }
    (clean.to_string(), None, None)
}

fn fetch_repo(source: &str) -> Result<Fetched, String> {
    let (repo, branch, folder) = split_tree(source);
    // git would read `--upload-pack=…` as an option, and `..` would leave the clone.
    if repo.starts_with('-') || branch.as_deref().is_some_and(|b| b.starts_with('-')) {
        return Err("That isn't a repository link.".into());
    }
    if folder
        .as_deref()
        .is_some_and(|f| f.split(['/', '\\']).any(|part| part == ".."))
    {
        return Err("That link's folder points outside the repository.".into());
    }
    let dir = std::env::temp_dir().join(format!(
        "runo-playbooks-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    ));
    let mut command = Command::new("git");
    command.args(["clone", "--depth", "1", "--quiet"]);
    if let Some(branch) = &branch {
        command.args(["--branch", branch]);
    }
    command
        .arg("--")
        .arg(&repo)
        .arg(&dir)
        .env("GIT_TERMINAL_PROMPT", "0");
    let out = command
        .output()
        .map_err(|_| "Couldn't clone it: git isn't available.".to_string())?;
    let result = if out.status.success() {
        let root = match &folder {
            Some(f) => dir.join(f),
            None => dir.clone(),
        };
        scan(&root).map(|mut fetched| {
            relative_to(&mut fetched, &dir);
            fetched
        })
    } else {
        Err(format!(
            "Couldn't clone {repo}. {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    };
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// Origins as paths inside `root`, which stay the same wherever it was read from.
fn relative_to(fetched: &mut Fetched, root: &Path) {
    let prefix = root.display().to_string();
    for found in &mut fetched.found {
        found.origin = found
            .origin
            .strip_prefix(&prefix)
            .unwrap_or(&found.origin)
            .trim_start_matches(['/', '\\'])
            .to_string();
    }
}

/// Every `.playbook.md` under `root`, a few folders deep.
fn scan(root: &Path) -> Result<Fetched, String> {
    let mut fetched = Fetched::default();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<_> = entries.flatten().map(|e| e.path()).collect();
        entries.sort();
        for path in entries {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if path.is_dir() {
                if depth < 4 && !name.starts_with('.') && name != "node_modules" {
                    stack.push((path, depth + 1));
                }
            } else if name.ends_with(SUFFIX) && fetched.found.len() < 200 {
                let origin = path.display().to_string();
                match std::fs::read_to_string(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|t| parse(&t))
                {
                    Ok(file) => fetched.found.push(Found { file, origin }),
                    Err(e) => fetched.skipped.push((name, e)),
                }
            }
        }
    }
    fetched.found.sort_by(|a, b| a.file.name.cmp(&b.file.name));
    if fetched.found.is_empty() && fetched.skipped.is_empty() {
        return Err(format!("No {SUFFIX} files there."));
    }
    Ok(fetched)
}

/// Compare dotted versions; anything unreadable counts as 0.
pub fn newer(candidate: &str, installed: &str) -> bool {
    let parts = |v: &str| -> Vec<u64> {
        v.trim_start_matches('v')
            .split('.')
            .map(|p| {
                p.chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect::<String>()
                    .parse()
                    .unwrap_or(0)
            })
            .collect()
    };
    parts(candidate) > parts(installed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_repository_link_cant_carry_options_or_leave_the_clone() {
        for source in [
            "--upload-pack=touch /tmp/x;.git",
            "https://github.com/o/r/tree/main/../../etc",
            "https://github.com/o/r/tree/--upload-pack=x/f",
        ] {
            assert!(fetch_repo(source).is_err(), "{source}");
        }
    }

    const SAMPLE: &str = "+++\nname = \"Release notes\"\ndescription = \"Draft notes\"\nauthor = \"anish\"\nversion = \"1.2.0\"\nagent = \"claude\"\npermissions = [\"shell\", \"bogus\"]\n\n[[inputs]]\nkey = \"since\"\nlabel = \"Compare from\"\ndefault = \"the last tag\"\n+++\n\nRead the commits since {{since}} on {{ branch }}.\n";

    #[test]
    fn parses_header_and_body() {
        let file = parse(SAMPLE).unwrap();
        assert_eq!(file.name, "Release notes");
        assert_eq!(file.author, "anish");
        assert_eq!(file.version, "1.2.0");
        assert_eq!(file.permissions, vec!["read", "shell"]);
        assert_eq!(file.inputs.len(), 1);
        assert_eq!(
            file.prompt,
            "Read the commits since {{since}} on {{ branch }}."
        );
    }

    #[test]
    fn round_trips() {
        let file = parse(SAMPLE).unwrap();
        assert_eq!(parse(&render(&file)).unwrap(), file);
    }

    #[test]
    fn rejects_bad_files() {
        assert!(parse("no header").is_err());
        assert!(parse("+++\nname = \"x\"\n").is_err());
        assert!(parse("+++\nname = \"x\"\n+++\n").is_err());
        assert!(parse("+++\nname = \n+++\nbody").is_err());
    }

    #[test]
    fn inputs_follow_placeholders() {
        let file = parse(SAMPLE).unwrap();
        let inputs = inputs_of(&file.inputs, &file.prompt);
        assert_eq!(inputs.len(), 2);
        assert_eq!(inputs[0].title(), "Compare from");
        assert_eq!(inputs[1].title(), "Branch");
        let filled = fill(&file.prompt, &inputs, &[("branch".into(), "main".into())]);
        assert_eq!(filled, "Read the commits since the last tag on main.");
        assert!(fill("{{x}}", &inputs_of(&[], "{{x}}"), &[]).contains("(no x given)"));
        // Spaces inside the braces, on either side, still count.
        let prompt = "Since {{ since}} until {{until }} on {{ branch }}, not {{other}}.";
        let inputs = inputs_of(&[], prompt);
        let values: Vec<(String, String)> = ["since", "until", "branch"]
            .iter()
            .map(|k| (k.to_string(), format!("<{k}>")))
            .collect();
        assert_eq!(
            fill(prompt, &inputs[..3], &values),
            "Since <since> until <until> on <branch>, not {{other}}."
        );
    }

    #[test]
    fn links() {
        assert_eq!(
            raw_link("https://github.com/a/b/blob/main/p/x.playbook.md"),
            "https://raw.githubusercontent.com/a/b/main/p/x.playbook.md"
        );
        assert!(is_file_link("https://x.dev/a.playbook.md?raw=1"));
        assert!(!is_file_link("https://github.com/a/b"));
        assert_eq!(
            split_tree("https://github.com/a/b/tree/main/playbooks"),
            (
                "https://github.com/a/b".into(),
                Some("main".into()),
                Some("playbooks".into())
            )
        );
    }

    #[test]
    fn names_and_versions() {
        assert_eq!(file_name("Release notes!"), "release-notes.playbook.md");
        assert!(newer("1.10.0", "1.9.2"));
        assert!(!newer("1.0", "1.0.0"));
        assert!(newer("v2", "1.9"));
    }

    #[test]
    fn scans_a_folder() {
        let dir = std::env::temp_dir().join(format!("runo-pbscan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/a.playbook.md"), SAMPLE).unwrap();
        std::fs::write(dir.join("broken.playbook.md"), "nope").unwrap();
        std::fs::write(dir.join("readme.md"), "# hi").unwrap();
        let fetched = fetch(dir.to_str().unwrap()).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(fetched.found.len(), 1);
        assert_eq!(fetched.skipped.len(), 1);
    }
}
