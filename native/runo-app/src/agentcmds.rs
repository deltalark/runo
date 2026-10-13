//! The agent's own slash commands for the `/` menu: its built-ins, custom commands kept
//! as files (`.claude/commands`, `~/.codex/prompts`, `.gemini/commands` and the like) and
//! Claude Code skills.

use std::path::{Path, PathBuf};

/// Where a command comes from, shown next to it in the menu.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Source {
    /// Runo handles it itself (compact and clear context).
    Runo,
    BuiltIn,
    Project,
    User,
    Skill,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Runo => "Runo",
            Source::BuiltIn => "built-in",
            Source::Project => "project",
            Source::User => "personal",
            Source::Skill => "skill",
        }
    }
}

/// One entry of the `/` menu besides playbooks.
#[derive(Clone, PartialEq, Debug)]
pub struct Command {
    /// What is typed after the slash, such as `review` or `frontend:test`.
    pub name: String,
    pub description: String,
    pub source: Source,
    /// The command's file, for agents whose headless mode does not expand it.
    pub path: Option<PathBuf>,
}

impl Command {
    fn new(name: &str, description: &str, source: Source) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            source,
            path: None,
        }
    }
}

/// Built-ins that work when the CLI runs one prompt at a time.
fn built_ins(provider: &str) -> Vec<Command> {
    let mut out = vec![
        Command::new(
            "compact",
            "Summarize the conversation so far to free up context",
            Source::Runo,
        ),
        Command::new(
            "clear",
            "Start a fresh conversation in the same folder",
            Source::Runo,
        ),
    ];
    let own: &[(&str, &str)] = match provider {
        "claude" => &[
            ("init", "Write a CLAUDE.md that describes this codebase"),
            ("review", "Review a pull request"),
            (
                "security-review",
                "Review the pending changes for security issues",
            ),
            ("pr-comments", "Get the comments on a pull request"),
        ],
        "gemini" | "qwen" => &[("init", "Write a GEMINI.md that describes this codebase")],
        _ => &[],
    };
    out.extend(own.iter().map(|(n, d)| Command::new(n, d, Source::BuiltIn)));
    out
}

/// Every command the agent offers in folder `dir`, project ones first.
pub fn discover(provider: &str, dir: &Path, home: Option<&Path>) -> Vec<Command> {
    let mut out = built_ins(provider);
    let mut add = |found: Vec<Command>| {
        for c in found {
            if !out.iter().any(|o| o.name == c.name) {
                out.push(c);
            }
        }
    };
    // (folder inside the project, folder inside home, separator for subfolders, toml?)
    let places: &[(&str, &str, &str, bool)] = match provider {
        "claude" => &[(".claude/commands", ".claude/commands", ":", false)],
        "gemini" => &[(".gemini/commands", ".gemini/commands", ":", true)],
        "qwen" => &[(".qwen/commands", ".qwen/commands", ":", true)],
        "opencode" => &[
            (".opencode/command", ".config/opencode/command", "/", false),
            (
                ".opencode/commands",
                ".config/opencode/commands",
                "/",
                false,
            ),
        ],
        "cursor" => &[(".cursor/commands", ".cursor/commands", "/", false)],
        "droid" => &[(".factory/commands", ".factory/commands", "/", false)],
        _ => &[],
    };
    for (project, user, sep, toml) in places {
        add(scan(&dir.join(project), sep, *toml, Source::Project, ""));
        if let Some(home) = home {
            add(scan(&home.join(user), sep, *toml, Source::User, ""));
        }
    }
    match provider {
        "claude" => {
            add(skills(&dir.join(".claude/skills")));
            if let Some(home) = home {
                add(skills(&home.join(".claude/skills")));
            }
        }
        "codex" => {
            let prompts = std::env::var_os("CODEX_HOME")
                .map(PathBuf::from)
                .or_else(|| home.map(|h| h.join(".codex")))
                .map(|p| p.join("prompts"));
            if let Some(prompts) = prompts {
                add(scan(&prompts, "/", false, Source::User, "prompts:"));
            }
        }
        _ => {}
    }
    out
}

/// Command files under `root`; a file in a subfolder is named `folder<sep>file`.
fn scan(root: &Path, sep: &str, toml: bool, source: Source, prefix: &str) -> Vec<Command> {
    let mut out = Vec::new();
    walk(root, root, sep, toml, source, prefix, 0, &mut out);
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

#[allow(clippy::too_many_arguments)]
fn walk(
    root: &Path,
    dir: &Path,
    sep: &str,
    toml: bool,
    source: Source,
    prefix: &str,
    depth: usize,
    out: &mut Vec<Command>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth < 3 {
                walk(root, &path, sep, toml, source, prefix, depth + 1, out);
            }
            continue;
        }
        let ext = if toml { "toml" } else { "md" };
        if path.extension().and_then(|e| e.to_str()) != Some(ext) || out.len() >= 200 {
            continue;
        }
        let Ok(rel) = path
            .with_extension("")
            .strip_prefix(root)
            .map(Path::to_path_buf)
        else {
            continue;
        };
        let name: Vec<String> = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let description = if toml {
            toml_description(&text)
        } else {
            markdown_description(&text)
        };
        out.push(Command {
            name: format!("{prefix}{}", name.join(sep)),
            description,
            source,
            path: Some(path),
        });
    }
}

/// Claude Code skills: one folder each with a SKILL.md.
fn skills(root: &Path) -> Vec<Command> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out: Vec<Command> = entries
        .flatten()
        .filter_map(|entry| {
            let file = entry.path().join("SKILL.md");
            let text = std::fs::read_to_string(&file).ok()?;
            let folder = entry.file_name().to_string_lossy().into_owned();
            let name = front_matter(&text, "name").unwrap_or(folder);
            Some(Command {
                name,
                description: markdown_description(&text),
                source: Source::Skill,
                path: Some(file),
            })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// A field of the `---` header at the top of a markdown file.
fn front_matter(text: &str, key: &str) -> Option<String> {
    let rest = text.trim_start().strip_prefix("---")?;
    let (header, _) = rest.split_once("\n---")?;
    header.lines().find_map(|line| {
        let (k, v) = line.split_once(':')?;
        (k.trim() == key)
            .then(|| v.trim().trim_matches(|c| c == '"' || c == '\'').to_string())
            .filter(|v| !v.is_empty())
    })
}

/// The header's description, else the first line of the body.
fn markdown_description(text: &str) -> String {
    if let Some(d) = front_matter(text, "description") {
        return d;
    }
    let body = text
        .trim_start()
        .strip_prefix("---")
        .and_then(|rest| rest.split_once("\n---"))
        .map(|(_, body)| body)
        .unwrap_or(text);
    body.lines()
        .map(|l| l.trim().trim_start_matches('#').trim())
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .chars()
        .take(120)
        .collect()
}

fn toml_description(text: &str) -> String {
    text.parse::<toml::Table>()
        .ok()
        .and_then(|t| t.get("description")?.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Commands matching the `/query`, best first.
pub fn matching<'a>(commands: &'a [Command], query: &str) -> Vec<&'a Command> {
    let q = query.to_lowercase();
    let mut hits: Vec<(usize, &Command)> = commands
        .iter()
        .filter_map(|c| {
            let name = c.name.to_lowercase();
            if name.starts_with(&q) {
                Some((0, c))
            } else if name.contains(&q) || c.description.to_lowercase().contains(&q) {
                Some((1, c))
            } else {
                None
            }
        })
        .collect();
    hits.sort_by_key(|(rank, c)| (*rank, c.name.len()));
    hits.into_iter().map(|(_, c)| c).collect()
}

/// The text a Codex prompt file stands for: `codex exec` takes the prompt as written, so
/// `/prompts:name args` is expanded here. `$ARGUMENTS` gets everything after the name and
/// `$1`…`$9` each word.
pub fn expand_codex(text: &str, home: Option<&Path>) -> Option<String> {
    let rest = text.trim().strip_prefix("/prompts:")?;
    let (name, args) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    let dir = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| home.map(|h| h.join(".codex")))?;
    let body = std::fs::read_to_string(dir.join("prompts").join(format!("{name}.md"))).ok()?;
    let body = match body.trim_start().strip_prefix("---") {
        Some(rest) => rest.split_once("\n---").map(|(_, b)| b).unwrap_or(&body),
        None => &body,
    };
    Some(fill_args(body.trim(), args.trim()))
}

fn fill_args(body: &str, args: &str) -> String {
    let words: Vec<&str> = args.split_whitespace().collect();
    let mut out = body.replace("$ARGUMENTS", args);
    for i in (1..=9).rev() {
        out = out.replace(&format!("${i}"), words.get(i - 1).copied().unwrap_or(""));
    }
    if !body.contains('$') && !args.is_empty() {
        out.push_str(&format!("\n\n{args}"));
    }
    out
}

pub fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("runo-cmds-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn finds_claude_commands_and_skills() {
        let root = temp("claude");
        let (project, home) = (root.join("p"), root.join("h"));
        write(
            &project.join(".claude/commands/fix-issue.md"),
            "---\ndescription: Fix a GitHub issue\nallowed-tools: Bash\n---\nFix issue $ARGUMENTS",
        );
        write(
            &project.join(".claude/commands/frontend/test.md"),
            "# Run the frontend tests\n\nThen report.",
        );
        write(
            &home.join(".claude/commands/standup.md"),
            "Write my standup",
        );
        write(
            &home.join(".claude/skills/pdf/SKILL.md"),
            "---\nname: pdf-tools\ndescription: \"Work with PDFs\"\n---\n# PDF",
        );
        let found = discover("claude", &project, Some(&home));
        let get = |n: &str| found.iter().find(|c| c.name == n).cloned().unwrap();
        assert_eq!(get("fix-issue").description, "Fix a GitHub issue");
        assert_eq!(get("fix-issue").source, Source::Project);
        assert_eq!(get("frontend:test").description, "Run the frontend tests");
        assert_eq!(get("standup").source, Source::User);
        assert_eq!(get("pdf-tools").source, Source::Skill);
        assert_eq!(get("pdf-tools").description, "Work with PDFs");
        assert_eq!(get("compact").source, Source::Runo);
        assert_eq!(get("review").source, Source::BuiltIn);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn finds_gemini_toml_commands() {
        let root = temp("gemini");
        write(
            &root.join(".gemini/commands/git/commit.toml"),
            "description = \"Write a commit message\"\nprompt = \"\"\"Look at {{args}}\"\"\"\n",
        );
        write(&root.join(".gemini/commands/broken.toml"), "not = [toml");
        let found = discover("gemini", &root, None);
        let commit = found.iter().find(|c| c.name == "git:commit").unwrap();
        assert_eq!(commit.description, "Write a commit message");
        assert!(found
            .iter()
            .any(|c| c.name == "broken" && c.description.is_empty()));
        // Other agents' files are not offered.
        assert!(!discover("codex", &root, None)
            .iter()
            .any(|c| c.name == "git:commit"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn expands_codex_prompts() {
        let home = temp("codex");
        write(
            &home.join(".codex/prompts/explain.md"),
            "---\ndescription: Explain code\n---\nExplain $1 in $ARGUMENTS",
        );
        write(&home.join(".codex/prompts/plain.md"), "Tidy the code");
        if std::env::var_os("CODEX_HOME").is_none() {
            let found = discover("codex", &home, Some(&home));
            let explain = found.iter().find(|c| c.name == "prompts:explain").unwrap();
            assert_eq!(explain.description, "Explain code");
            assert_eq!(
                expand_codex("/prompts:explain main.rs", Some(&home)).unwrap(),
                "Explain main.rs in main.rs"
            );
            assert_eq!(
                expand_codex("/prompts:plain src/", Some(&home)).unwrap(),
                "Tidy the code\n\nsrc/"
            );
            assert_eq!(expand_codex("/prompts:missing", Some(&home)), None);
        }
        assert_eq!(expand_codex("fix it", Some(&home)), None);
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn matches_by_prefix_first() {
        let commands = vec![
            Command::new("review", "Review a pull request", Source::BuiltIn),
            Command::new("pr-comments", "Get the comments", Source::BuiltIn),
            Command::new("security-review", "Security", Source::BuiltIn),
        ];
        let names: Vec<&str> = matching(&commands, "re")
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(names, ["review", "security-review"]);
        assert!(matching(&commands, "zzz").is_empty());
    }
}
