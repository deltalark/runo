//! Comments on the design: notes pinned to a point on a screen, with replies, kept in
//! `design/comments.toml` so they travel with the design and agents can read them.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub(crate) const FILE: &str = "comments.toml";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Reply {
    pub author: String,
    pub text: String,
    /// When it was written, as shown: "Oct 16, 18:04".
    pub at: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Comment {
    pub id: u32,
    pub screen: String,
    /// Where it's pinned, in CSS pixels from the screen's top left.
    pub x: f32,
    pub y: f32,
    pub author: String,
    pub text: String,
    pub at: String,
    #[serde(default)]
    pub resolved: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replies: Vec<Reply>,
}

#[derive(Default, Serialize, Deserialize)]
struct Notes {
    #[serde(default, rename = "comment")]
    comments: Vec<Comment>,
}

pub(crate) fn path(root: &Path) -> PathBuf {
    super::model::folder(root).join(FILE)
}

pub(crate) fn load(root: &Path) -> Vec<Comment> {
    std::fs::read_to_string(path(root))
        .ok()
        .and_then(|t| toml::from_str::<Notes>(&t).ok())
        .map(|n| n.comments)
        .unwrap_or_default()
}

pub(crate) fn save(root: &Path, comments: &[Comment]) -> Result<(), String> {
    let file = path(root);
    let text = toml::to_string_pretty(&Notes {
        comments: comments.to_vec(),
    })
    .map_err(|e| e.to_string())?;
    std::fs::create_dir_all(super::model::folder(root))
        .and_then(|_| std::fs::write(&file, text))
        .map_err(|e| format!("Couldn't save {}: {e}", file.display()))
}

pub(crate) fn next_id(comments: &[Comment]) -> u32 {
    comments.iter().map(|c| c.id).max().unwrap_or(0) + 1
}

/// Who's commenting: the project's git user, else "You".
pub(crate) fn author(root: &Path) -> String {
    std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["config", "user.name"])
        .stdin(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "You".to_string())
}

pub(crate) fn now() -> String {
    chrono::Local::now().format("%b %-d, %H:%M").to_string()
}

/// One or two letters for an avatar: "Ada Lovelace" gives "AL".
pub(crate) fn initials(name: &str) -> String {
    let out: String = name
        .split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .flat_map(char::to_uppercase)
        .collect();
    if out.is_empty() {
        "?".into()
    } else {
        out
    }
}

/// A comment where it's pinned, for an agent: "On Inbox (inbox.html) at 20, 141, on the
/// button there: Ada: Make it bigger", with its replies after.
pub(crate) fn line(c: &Comment, screen_name: &str, under: Option<&str>) -> String {
    let mut out = format!(
        "On {screen_name} ({}.html) at {}, {}{}: {}: {}",
        c.screen,
        c.x.round(),
        c.y.round(),
        under
            .map(|u| format!(", on the {u} there"))
            .unwrap_or_default(),
        c.author,
        c.text
    );
    for r in &c.replies {
        out.push_str(&format!(" / {}: {}", r.author, r.text));
    }
    out
}

/// What an agent is asked when comments are handed to it to fix, each said by `line`.
pub(crate) fn task(lines: &[String]) -> String {
    let mut out = String::from(match lines.len() {
        1 => "Address this comment on the design (pinned in CSS px from the screen's top left):\n",
        _ => "Address these comments on the design (each pinned in CSS px from its screen's top left):\n",
    });
    for l in lines {
        out.push_str(&format!("- {l}\n"));
    }
    out.push_str(if lines.len() == 1 {
        "\nChange the design so the comment is resolved."
    } else {
        "\nChange the design so each comment is resolved, keeping the screens consistent."
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_are_kept_with_the_design() {
        let root = std::env::temp_dir().join(format!("runo-notes-{}", std::process::id()));
        assert!(load(&root).is_empty());
        let c = Comment {
            id: 1,
            screen: "home".into(),
            x: 20.,
            y: 140.5,
            author: "Ada Lovelace".into(),
            text: "Make the \"Save\" button bigger".into(),
            at: "Oct 16, 18:04".into(),
            resolved: false,
            replies: vec![Reply {
                author: "Bo".into(),
                text: "On it".into(),
                at: "Oct 16, 18:05".into(),
            }],
        };
        save(&root, std::slice::from_ref(&c)).unwrap();
        assert_eq!(load(&root), vec![c.clone()]);
        assert_eq!(next_id(std::slice::from_ref(&c)), 2);
        assert_eq!(initials(&c.author), "AL");
        assert_eq!(initials(""), "?");
        let l = line(&c, "Home", Some("button"));
        assert!(l.starts_with("On Home (home.html) at 20, 141, on the button there: Ada"));
        assert!(l.ends_with(" / Bo: On it"));
        let t = task(std::slice::from_ref(&l));
        assert!(t.contains("this comment") && t.contains(&l));
        assert!(task(&[l.clone(), l]).contains("each comment"));
        let _ = std::fs::remove_dir_all(root);
    }
}
