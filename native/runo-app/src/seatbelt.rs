//! The seatbelt: what a commit or a push is about to send that people regret, found
//! before it goes. Secrets and keys, `.env` files, huge files, debugging leftovers and
//! clash markers, each with a plain name. Scanning only, no UI.

/// Files over this size make the repository heavy for everyone who clones it.
pub const HUGE: u64 = 10 * 1024 * 1024;

/// What the seatbelt found.
#[derive(Clone, Debug, PartialEq)]
pub enum Snag {
    /// A password, token or key, by what kind it looks like.
    Secret(&'static str),
    /// A file that holds secrets by its nature, such as `.env` or a private key.
    SecretFile,
    /// A file this many bytes big.
    Huge(u64),
    /// A line left in while debugging, by what it is.
    Leftover(&'static str),
    /// A marker from a clash that was never sorted out.
    Marker,
}

impl Snag {
    /// The snag in plain words.
    pub fn words(&self) -> String {
        match self {
            Snag::Secret(kind) => format!("Looks like {kind}"),
            Snag::SecretFile => "A file that usually holds passwords or keys".into(),
            Snag::Huge(bytes) => format!(
                "A big file ({:.0} MB) that every copy of the project will carry",
                *bytes as f64 / 1_048_576.0
            ),
            Snag::Leftover(what) => format!("{what} left in from debugging"),
            Snag::Marker => "A clash marker that was never sorted out".into(),
        }
    }

    /// Whether this one should stop a push, not just be pointed out.
    pub fn serious(&self) -> bool {
        matches!(self, Snag::Secret(_) | Snag::SecretFile | Snag::Marker)
    }
}

/// Characters a token is made of.
fn token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// Whether `prefix` appears in `line` followed by at least `min` token characters.
fn token_after(line: &str, prefix: &str, min: usize) -> bool {
    line.match_indices(prefix).any(|(i, _)| {
        // The prefix has to start a word, so `task-...` isn't an `sk-` key.
        let starts = line[..i].chars().next_back().is_none_or(|c| !token_char(c));
        starts
            && line[i + prefix.len()..]
                .chars()
                .take_while(|c| token_char(*c))
                .count()
                >= min
    })
}

/// A quoted value of at least `min` characters after `=` or `:` following `key`, that
/// isn't a placeholder or read from the environment.
fn assigned_literal(lower: &str, key: &str, min: usize) -> bool {
    lower.match_indices(key).any(|(i, _)| {
        let rest = lower[i + key.len()..]
            .trim_start_matches(|c: char| token_char(c) || c == '"' || c == '\'');
        let Some(rest) = rest.trim_start().strip_prefix(['=', ':']) else {
            return false;
        };
        let rest = rest.trim_start_matches(['=', ' ', '\t']);
        let Some(quote) = rest
            .chars()
            .next()
            .filter(|c| matches!(c, '"' | '\'' | '`'))
        else {
            return false;
        };
        let value: String = rest[1..].chars().take_while(|c| *c != quote).collect();
        let placeholder = value.contains("${")
            || value.contains('<')
            || value.contains("xxx")
            || value.contains("changeme")
            || value.contains("example")
            || value.contains("your")
            || value.chars().all(|c| c == '*' || c == '.');
        value.chars().count() >= min && !value.contains(' ') && !placeholder
    })
}

/// What an added line in `path` gives away, if anything.
pub fn scan_line(path: &str, line: &str) -> Option<Snag> {
    if line.contains("-----BEGIN") && line.contains("PRIVATE KEY") {
        return Some(Snag::Secret("a private key"));
    }
    let tokens: [(&str, usize, &'static str); 13] = [
        ("AKIA", 16, "an AWS access key"),
        ("ghp_", 30, "a GitHub token"),
        ("gho_", 30, "a GitHub token"),
        ("ghs_", 30, "a GitHub token"),
        ("github_pat_", 30, "a GitHub token"),
        ("sk-ant-", 30, "an Anthropic API key"),
        ("sk-proj-", 30, "an OpenAI API key"),
        ("sk_live_", 20, "a Stripe secret key"),
        ("rk_live_", 20, "a Stripe secret key"),
        ("xoxb-", 20, "a Slack token"),
        ("xoxp-", 20, "a Slack token"),
        ("AIza", 30, "a Google API key"),
        ("glpat-", 20, "a GitLab token"),
    ];
    for (prefix, min, kind) in tokens {
        if token_after(line, prefix, min) {
            return Some(Snag::Secret(kind));
        }
    }
    let lower = line.to_lowercase();
    for key in [
        "password",
        "passwd",
        "secret",
        "api_key",
        "apikey",
        "access_token",
        "auth_token",
    ] {
        if assigned_literal(&lower, key, 8) {
            return Some(Snag::Secret("a password or secret written into the code"));
        }
    }
    let trimmed = line.trim();
    if trimmed.starts_with("<<<<<<< ") || trimmed.starts_with(">>>>>>> ") || trimmed == "=======" {
        return Some(Snag::Marker);
    }
    let ext = path.rsplit_once('.').map_or("", |(_, e)| e);
    let leftover = match ext {
        "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs" | "vue" | "svelte" => {
            if trimmed.starts_with("console.log(") {
                Some("A console.log")
            } else if trimmed == "debugger" || trimmed == "debugger;" {
                Some("A debugger stop")
            } else {
                None
            }
        }
        "rs" => trimmed.contains("dbg!(").then_some("A dbg!"),
        "py" => (trimmed.starts_with("breakpoint()")
            || trimmed.contains("pdb.set_trace()")
            || trimmed == "import pdb")
            .then_some("A debugger stop"),
        "rb" => (trimmed == "binding.pry" || trimmed == "byebug" || trimmed == "debugger")
            .then_some("A debugger stop"),
        "php" => {
            (trimmed.starts_with("var_dump(") || trimmed.starts_with("dd(")).then_some("A var_dump")
        }
        _ => None,
    };
    leftover.map(Snag::Leftover)
}

/// Whether a file is one that holds secrets by its nature.
pub fn secret_file(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    let env = name == ".env"
        || (name.starts_with(".env.")
            && !matches!(
                name,
                ".env.example" | ".env.sample" | ".env.template" | ".env.dist"
            ));
    env || matches!(name, "id_rsa" | "id_ed25519" | "id_ecdsa" | "id_dsa")
        || [".pem", ".key", ".p12", ".pfx", ".keystore"]
            .iter()
            .any(|e| name.ends_with(e))
}

/// What a patch's added lines give away: (path, line number in the new file, snag).
pub fn scan_patch(patch: &str) -> Vec<(String, usize, Snag)> {
    let mut out = Vec::new();
    let mut path = String::new();
    let mut line_no = 0usize;
    for line in patch.lines() {
        if let Some(p) = line.strip_prefix("+++ ") {
            path = p.strip_prefix("b/").unwrap_or(p).to_string();
            if secret_file(&path) {
                out.push((path.clone(), 0, Snag::SecretFile));
            }
        } else if let Some(at) = line.strip_prefix("@@ ") {
            // "@@ -1,2 +3,4 @@": the new side starts at 3.
            line_no = at
                .split_whitespace()
                .find_map(|w| w.strip_prefix('+'))
                .and_then(|w| w.split(',').next()?.parse().ok())
                .unwrap_or(1);
        } else if let Some(text) = line.strip_prefix('+') {
            if let Some(snag) = scan_line(&path, text) {
                out.push((path.clone(), line_no, snag));
            }
            line_no += 1;
        } else if !line.starts_with('-') {
            line_no += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{scan_line, scan_patch, secret_file, Snag};

    #[test]
    fn finds_what_people_regret_sending() {
        let key = format!("const KEY = \"ghp_{}\";", "a".repeat(36));
        assert_eq!(
            scan_line("a.js", &key),
            Some(Snag::Secret("a GitHub token"))
        );
        assert_eq!(
            scan_line("a.py", "password = \"hunter2hunter2\""),
            Some(Snag::Secret("a password or secret written into the code"))
        );
        // Read from the environment, or a placeholder: fine.
        assert_eq!(scan_line("a.py", "password = os.environ[\"PW\"]"), None);
        assert_eq!(scan_line("a.py", "password = \"<your password>\""), None);
        assert_eq!(
            scan_line("a.rs", "let task-abcdefghijklmnopqrstuvwxyz0123456789 = 1;"),
            None
        );
        assert_eq!(
            scan_line("a.ts", "  console.log(x)"),
            Some(Snag::Leftover("A console.log"))
        );
        assert_eq!(scan_line("a.rs", "    console.log(x)"), None);
        assert_eq!(
            scan_line("a.rs", "    dbg!(x);"),
            Some(Snag::Leftover("A dbg!"))
        );
        assert_eq!(scan_line("a.txt", "<<<<<<< HEAD"), Some(Snag::Marker));
        assert!(secret_file("config/.env"));
        assert!(secret_file(".env.production"));
        assert!(!secret_file(".env.example"));
        assert!(secret_file("deploy/id_rsa"));
        assert!(!secret_file("id_rsa.pub"));
        let patch = "+++ b/.env\n@@ -0,0 +1,2 @@\n+A=1\n+DB_PASSWORD='s3cr3tpassw0rd'\n+++ b/src/a.rs\n@@ -10,2 +10,3 @@\n fn a() {\n+    dbg!(1);\n }\n";
        let found = scan_patch(patch);
        assert_eq!(found[0], (".env".into(), 0, Snag::SecretFile));
        assert_eq!(found[1].1, 2);
        assert_eq!(found[2], ("src/a.rs".into(), 11, Snag::Leftover("A dbg!")));
    }
}
