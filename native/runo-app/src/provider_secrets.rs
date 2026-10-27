//! Per-host access tokens, used by the CI watcher when it falls back to a host's REST
//! API (GitHub without `gh`, GitLab without `glab`, Bitbucket, Gitea). Tokens live in
//! `provider_tokens.json` under the private data folder, written 0600 by
//! [`crate::store::write_private`]. A keychain backend can hook into [`Store::load`]
//! and [`Store::save`] later without changing the UI.
//!
//! The CI watcher's REST fallbacks for non-GitHub hosts are the next step; this
//! module is the scaffold they will plug into. The Settings > Git providers page
//! that reads [`Store::placeholder`] isn't rendered yet; it is tracked as follow-up.
//!
//! Tokens must never land in a log line or diagnostic. [`redact`] scrubs them from any
//! string that might end up in `applog`.
#![allow(dead_code)]

use crate::provider::Host;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// What a host's token is called in the on-disk JSON.
fn key(host: Host) -> Option<&'static str> {
    match host {
        Host::Github => Some("github"),
        Host::Gitlab => Some("gitlab"),
        Host::BitbucketCloud => Some("bitbucket_cloud"),
        Host::BitbucketServer => Some("bitbucket_server"),
        Host::Gitea => Some("gitea"),
        Host::AzureDevops => Some("azure_devops"),
        Host::None | Host::Unknown => None,
    }
}

fn tokens_path() -> PathBuf {
    crate::store::data_dir().join("provider_tokens.json")
}

/// All known host tokens, loaded and saved as one file. The file is 0600; a missing
/// file is an empty store, not an error.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Store {
    #[serde(default)]
    tokens: BTreeMap<String, String>,
}

impl Store {
    pub fn load() -> Self {
        crate::store::read_json_or_backup(&tokens_path())
    }

    pub fn save(&self) {
        crate::store::write_json(&tokens_path(), self);
    }

    pub fn token(&self, host: Host) -> Option<&str> {
        let k = key(host)?;
        self.tokens
            .get(k)
            .map(String::as_str)
            .filter(|s| !s.is_empty())
    }

    /// Set or clear a token. An empty token removes the entry.
    pub fn set(&mut self, host: Host, token: &str) {
        let Some(k) = key(host) else { return };
        let value = token.trim();
        if value.is_empty() {
            self.tokens.remove(k);
        } else {
            self.tokens.insert(k.into(), value.into());
        }
    }

    /// What a Settings page should show in the token's box — not the token itself.
    pub fn placeholder(&self, host: Host) -> &'static str {
        if self.token(host).is_some() {
            "••••••• (saved)"
        } else {
            match host {
                Host::Github => "ghp_… (optional; gh handles login)",
                Host::Gitlab => "glpat-… (required for API fallback)",
                Host::BitbucketCloud => "App password (required for API)",
                Host::BitbucketServer => "HTTP access token",
                Host::Gitea => "Personal access token",
                Host::AzureDevops => "PAT (not supported yet)",
                Host::None | Host::Unknown => "",
            }
        }
    }
}

/// Scrub anything that looks like a GitHub, GitLab, Bitbucket or generic bearer token
/// out of `text`. Used before writing to the log. Keeps surrounding context so error
/// messages still make sense.
pub fn redact(text: &str) -> String {
    // Known prefixes: GitHub (ghp_, gho_, ghs_, ghu_, github_pat_), GitLab (glpat-),
    // Gitea (gta_), plus `token <value>` and `Bearer <value>` headers that may leak.
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(idx) = find_token_start(rest) {
        out.push_str(&rest[..idx]);
        let after = &rest[idx..];
        let end = after
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-')
            .unwrap_or(after.len());
        out.push_str("[redacted]");
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

fn find_token_start(s: &str) -> Option<usize> {
    const PREFIXES: &[&str] = &[
        "ghp_",
        "gho_",
        "ghs_",
        "ghu_",
        "github_pat_",
        "glpat-",
        "gta_",
    ];
    PREFIXES.iter().filter_map(|p| s.find(p)).min().or_else(|| {
        // `Authorization: Bearer <t>` and `token <t>` headers.
        for header in ["Bearer ", "token "] {
            if let Some(i) = s.find(header) {
                return Some(i + header.len());
            }
        }
        None
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn redact_scrubs_known_token_shapes() {
        assert_eq!(
            redact("failed: ghp_abc123def456 is invalid"),
            "failed: [redacted] is invalid"
        );
        assert_eq!(
            redact("Authorization: Bearer glpat-xyz\n"),
            "Authorization: Bearer [redacted]\n"
        );
        assert_eq!(
            redact("header token github_pat_11AAA_etc end"),
            "header token [redacted] end"
        );
        // Nothing to redact leaves the text alone.
        assert_eq!(redact("plain error"), "plain error");
    }

    #[test]
    fn store_set_and_clear() {
        let mut s = Store::default();
        s.set(Host::Github, " ghp_abc ");
        assert_eq!(s.token(Host::Github), Some("ghp_abc"));
        s.set(Host::Github, "");
        assert!(s.token(Host::Github).is_none());
        // Hosts without a key don't store anything.
        s.set(Host::None, "x");
        assert!(s.token(Host::None).is_none());
    }
}
