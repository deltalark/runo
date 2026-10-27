//! Which git host a project is on and what CI it has. Host detection reads the remote
//! URL once per repository; CI detection scans the worktree for known config files and
//! leaves the API probe to the host-specific provider. Everything that watches CI goes
//! through [`Host`] rather than talking to `gh` directly, so GitLab, Bitbucket, Gitea and
//! self-hosted repositories can plug in without touching the watcher.
//!
//! Only GitHub is implemented through its own [`gh`] CLI. Other hosts are classified so
//! the UI can tell the person they aren't supported yet, instead of silently pretending
//! to watch something it isn't.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// The git host of a repository, classified from its remote URL.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Host {
    #[default]
    None,
    Github,
    Gitlab,
    BitbucketCloud,
    BitbucketServer,
    Gitea,
    AzureDevops,
    Unknown,
}

impl Host {
    /// A short name for the UI, such as "GitHub".
    pub fn name(self) -> &'static str {
        match self {
            Host::None => "No remote",
            Host::Github => "GitHub",
            Host::Gitlab => "GitLab",
            Host::BitbucketCloud => "Bitbucket",
            Host::BitbucketServer => "Bitbucket Server",
            Host::Gitea => "Gitea",
            Host::AzureDevops => "Azure DevOps",
            Host::Unknown => "Unknown host",
        }
    }

    /// Whether the CI watcher can talk to this host today. Hosts outside this set show
    /// a "not supported yet" message instead of a silent no-op.
    pub fn watcher_supported(self) -> bool {
        matches!(self, Host::Github)
    }

    /// What a pull request on this host is called.
    #[allow(dead_code)]
    pub fn request_name(self) -> &'static str {
        match self {
            Host::Gitlab => "merge request",
            _ => "pull request",
        }
    }
}

/// A known CI system, picked up from a config file in the repository.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CiKind {
    GithubActions,
    GitlabPipelines,
    BitbucketPipelines,
    Circle,
    Jenkins,
    Buildkite,
    Azure,
    Drone,
    Woodpecker,
    Earthly,
    TeamCity,
}

impl CiKind {
    /// A label for the UI, such as "GitHub Actions".
    pub fn label(self) -> &'static str {
        match self {
            CiKind::GithubActions => "GitHub Actions",
            CiKind::GitlabPipelines => "GitLab Pipelines",
            CiKind::BitbucketPipelines => "Bitbucket Pipelines",
            CiKind::Circle => "CircleCI",
            CiKind::Jenkins => "Jenkins",
            CiKind::Buildkite => "Buildkite",
            CiKind::Azure => "Azure Pipelines",
            CiKind::Drone => "Drone",
            CiKind::Woodpecker => "Woodpecker",
            CiKind::Earthly => "Earthly",
            CiKind::TeamCity => "TeamCity",
        }
    }

    /// Which host runs this CI natively, if any. External CIs (Circle, Jenkins,
    /// Buildkite…) report checks back to whatever host they're wired to.
    pub fn native_host(self) -> Option<Host> {
        match self {
            CiKind::GithubActions => Some(Host::Github),
            CiKind::GitlabPipelines => Some(Host::Gitlab),
            CiKind::BitbucketPipelines => Some(Host::BitbucketCloud),
            _ => None,
        }
    }
}

/// Per-project CI tracking preference, saved on [`crate::store::Project`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum CiTracking {
    /// Detect from config files and the host's API; the default.
    #[default]
    Auto,
    /// Only track this specific CI, ignore everything else.
    Only(CiKind),
    /// A custom status URL to open; Runo doesn't poll it.
    Custom(String),
    /// Don't watch CI for this project.
    Off,
}

impl CiTracking {
    /// Parse the project's stored string. "" and "auto" both mean Auto; "none"/"off" is
    /// Off; "github_actions", "gitlab_pipelines", "bitbucket_pipelines" pin a kind;
    /// "custom:<url>" keeps the URL.
    pub fn parse(value: &str) -> Self {
        let v = value.trim();
        if v.is_empty() || v.eq_ignore_ascii_case("auto") {
            return CiTracking::Auto;
        }
        if v.eq_ignore_ascii_case("off") || v.eq_ignore_ascii_case("none") {
            return CiTracking::Off;
        }
        if let Some(url) = v.strip_prefix("custom:") {
            let url = url.trim();
            if url.is_empty() {
                return CiTracking::Auto;
            }
            return CiTracking::Custom(url.to_string());
        }
        match v {
            "github_actions" => CiTracking::Only(CiKind::GithubActions),
            "gitlab_pipelines" => CiTracking::Only(CiKind::GitlabPipelines),
            "bitbucket_pipelines" => CiTracking::Only(CiKind::BitbucketPipelines),
            _ => CiTracking::Auto,
        }
    }

    /// Serialise back to the project's stored string. Used by the settings UI (follow-up).
    #[allow(dead_code)]
    pub fn as_key(&self) -> String {
        match self {
            CiTracking::Auto => "auto".into(),
            CiTracking::Off => "off".into(),
            CiTracking::Custom(url) => format!("custom:{url}"),
            CiTracking::Only(CiKind::GithubActions) => "github_actions".into(),
            CiTracking::Only(CiKind::GitlabPipelines) => "gitlab_pipelines".into(),
            CiTracking::Only(CiKind::BitbucketPipelines) => "bitbucket_pipelines".into(),
            CiTracking::Only(other) => format!("only:{}", other.label().to_ascii_lowercase()),
        }
    }
}

/// Classify a remote URL into a [`Host`]. Handles `https://`, `git@` SCP-style, `ssh://`
/// with ports, and `.git` suffixes. Self-hosted GitLab and Gitea are picked up by
/// hostname prefixes (`gitlab.`, `gitea.`, `git.`) as a best effort.
pub fn detect_host(url: &str) -> Host {
    let host = host_of(url).unwrap_or_default().to_ascii_lowercase();
    if host.is_empty() {
        return Host::None;
    }
    if host == "github.com" || host.ends_with(".github.com") {
        return Host::Github;
    }
    if host == "gitlab.com" || host.starts_with("gitlab.") || host.contains(".gitlab.") {
        return Host::Gitlab;
    }
    if host == "bitbucket.org" {
        return Host::BitbucketCloud;
    }
    if host.starts_with("bitbucket.") || host.contains(".bitbucket.") {
        // A self-hosted Bitbucket is Server (Data Center) rather than Cloud.
        return Host::BitbucketServer;
    }
    if host == "codeberg.org" || host.starts_with("gitea.") || host.contains(".gitea.") {
        return Host::Gitea;
    }
    if host == "dev.azure.com" || host.ends_with(".visualstudio.com") {
        return Host::AzureDevops;
    }
    // `git.example.com` or similar: a self-hosted repository Runo can't classify.
    if host.starts_with("git.") {
        return Host::Unknown;
    }
    Host::Unknown
}

/// Pull out the hostname from a git URL, whatever its shape.
fn host_of(url: &str) -> Option<&str> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    // SCP-style: `git@host:owner/repo.git` or `user@host:path`.
    if !url.contains("://") {
        if let Some((before, _)) = url.split_once(':') {
            let host = before.rsplit_once('@').map_or(before, |(_, h)| h);
            return (!host.is_empty()).then_some(host);
        }
        return None;
    }
    // scheme://[user[:pw]@]host[:port]/path
    let after_scheme = url.split("://").nth(1)?;
    let authority = after_scheme.split('/').next()?;
    let after_user = authority.rsplit_once('@').map_or(authority, |(_, a)| a);
    let host = after_user.split_once(':').map_or(after_user, |(h, _)| h);
    (!host.is_empty()).then_some(host)
}

/// Pull "owner/repo" out of a git URL, trimming any `.git` suffix and `/-/`-style GitLab
/// subgroups. Returns None when the URL has no path. Used by the REST fallbacks for
/// non-GitHub hosts (follow-up).
#[allow(dead_code)]
pub fn owner_repo(url: &str) -> Option<(String, String)> {
    let path = match url.trim().split_once("://") {
        Some((_, rest)) => rest.split_once('/').map(|(_, p)| p)?.to_string(),
        None => url.split_once(':').map(|(_, p)| p)?.to_string(),
    };
    let path = path.trim_end_matches('/').trim_end_matches(".git");
    let mut parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    let repo = parts.pop()?;
    let owner = parts.pop()?;
    Some((owner.to_string(), repo.to_string()))
}

/// CI config files in `dir`, in the order they're found. Only the top-level config
/// files and `.github/workflows/*.{yml,yaml}` are checked; anything deeper is custom
/// enough that the person can tell Runo through the per-project override.
pub fn detect_ci_configs(dir: &Path) -> Vec<CiKind> {
    let mut found = Vec::new();
    if actions_workflow_present(dir) {
        found.push(CiKind::GithubActions);
    }
    for (file, kind) in [
        (".gitlab-ci.yml", CiKind::GitlabPipelines),
        ("bitbucket-pipelines.yml", CiKind::BitbucketPipelines),
        ("Jenkinsfile", CiKind::Jenkins),
        ("azure-pipelines.yml", CiKind::Azure),
        (".drone.yml", CiKind::Drone),
        (".woodpecker.yml", CiKind::Woodpecker),
        ("Earthfile", CiKind::Earthly),
    ] {
        if dir.join(file).is_file() {
            found.push(kind);
        }
    }
    if dir.join(".circleci").join("config.yml").is_file() {
        found.push(CiKind::Circle);
    }
    if dir.join(".buildkite").join("pipeline.yml").is_file() {
        found.push(CiKind::Buildkite);
    }
    if dir.join(".teamcity").is_dir() {
        found.push(CiKind::TeamCity);
    }
    found
}

fn actions_workflow_present(dir: &Path) -> bool {
    let workflows = dir.join(".github").join("workflows");
    let Ok(entries) = std::fs::read_dir(&workflows) else {
        return false;
    };
    entries
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .any(|name| {
            let lower = name.to_ascii_lowercase();
            lower.ends_with(".yml") || lower.ends_with(".yaml")
        })
}

/// What the CI watcher should do for a repository, after host detection, config scan
/// and the per-project override have had a say. The `custom_url` points to a status
/// page the UI links to without polling.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackingPlan {
    pub host: Host,
    /// Which CI the watcher will try to read; None means no CI to watch.
    pub ci: Option<CiKind>,
    /// Every CI config found in the repo (useful for diagnostics and UI messages).
    pub configs: Vec<CiKind>,
    /// Why the watcher won't follow this project. Empty when CI is being watched.
    pub reason: TrackingReason,
    pub custom_url: Option<String>,
}

/// Why the watcher won't follow a repository, with enough information for the UI to
/// show the right line without any string matching.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum TrackingReason {
    #[default]
    Watching,
    /// The repo has no remote, so nothing to watch.
    NoRemote,
    /// The host isn't supported yet (Bitbucket Server, Azure DevOps, Gitea…).
    HostUnsupported,
    /// The repo is on a known host but has no CI config and no run on push.
    /// Set by the Settings > Project CI dropdown when the person wants Runo to
    /// skip the API probe (follow-up UI); the watcher itself falls back to
    /// `TrackingReason::Watching` with the probe timeout for now.
    #[allow(dead_code)]
    NoCiConfigured,
    /// The person turned the watcher off for this project.
    DisabledByProject,
    /// A custom CI the person pointed Runo at; the UI links it, we don't poll.
    Custom,
}

/// Work out what to watch for a repo on `host`, with `override_` from the project's
/// setting and the CI configs picked up by `detect_ci_configs`.
pub fn plan(host: Host, override_: &CiTracking, configs: Vec<CiKind>) -> TrackingPlan {
    if matches!(override_, CiTracking::Off) {
        return TrackingPlan {
            host,
            ci: None,
            configs,
            reason: TrackingReason::DisabledByProject,
            custom_url: None,
        };
    }
    if let CiTracking::Custom(url) = override_ {
        return TrackingPlan {
            host,
            ci: None,
            configs,
            reason: TrackingReason::Custom,
            custom_url: Some(url.clone()),
        };
    }
    if host == Host::None {
        return TrackingPlan {
            host,
            ci: None,
            configs,
            reason: TrackingReason::NoRemote,
            custom_url: None,
        };
    }
    if let CiTracking::Only(kind) = override_ {
        return TrackingPlan {
            host,
            ci: Some(*kind),
            configs,
            reason: if host.watcher_supported() {
                TrackingReason::Watching
            } else {
                TrackingReason::HostUnsupported
            },
            custom_url: None,
        };
    }
    if !host.watcher_supported() {
        return TrackingPlan {
            host,
            ci: None,
            configs,
            reason: TrackingReason::HostUnsupported,
            custom_url: None,
        };
    }
    // Auto: prefer a CI that matches the host, else leave `ci` None until the API probe
    // after push either turns up a run or times out.
    let ci = configs
        .iter()
        .find(|c| c.native_host() == Some(host))
        .copied()
        .or_else(|| configs.first().copied());
    TrackingPlan {
        host,
        ci,
        configs,
        reason: TrackingReason::Watching,
        custom_url: None,
    }
}

/// A one-line label for the watcher chip, provider-aware. `pending_probe` is true in
/// the first couple of minutes after a push, before the API probe has had a chance to
/// find a run; the chip stays quiet rather than claiming to watch nothing.
///
/// The watcher's `label` function builds its text from the plan and `Watch` state
/// together; this helper is for pre-watch UI (Git panel chips) and tests.
#[allow(dead_code)]
pub fn chip_label(plan: &TrackingPlan, pending_probe: bool) -> String {
    match plan.reason {
        TrackingReason::NoRemote => "No remote, no CI tracking".into(),
        TrackingReason::DisabledByProject => "CI tracking off".into(),
        TrackingReason::Custom => "Custom CI".into(),
        TrackingReason::HostUnsupported => {
            format!("{} not supported yet", plan.host.name())
        }
        TrackingReason::NoCiConfigured => "No CI configured".into(),
        TrackingReason::Watching => match plan.ci {
            Some(ci) => format!("Watching {}", ci.label()),
            None if pending_probe => format!("Looking for CI on {}", plan.host.name()),
            None => "No CI detected on this push".into(),
        },
    }
}

/// Project id → the host last read for it, cached in `RunoApp` so Runo doesn't shell
/// out to git on every tick. [`HostCache::invalidate`] clears an entry when the
/// remote changes. Hooked into `RunoApp` by the follow-up that moves `tracking_plan`
/// over to this cache.
#[allow(dead_code)]
#[derive(Default)]
pub struct HostCache {
    /// (remote URL, host) by project.
    by_project: HashMap<u64, (String, Host)>,
}

#[allow(dead_code)]
impl HostCache {
    pub fn get_or_fetch(
        &mut self,
        project: u64,
        remote_url: &str,
    ) -> Host {
        if remote_url.is_empty() {
            self.by_project.remove(&project);
            return Host::None;
        }
        if let Some((cached_url, host)) = self.by_project.get(&project) {
            if cached_url == remote_url {
                return *host;
            }
        }
        let host = detect_host(remote_url);
        self.by_project.insert(project, (remote_url.to_string(), host));
        host
    }

    pub fn invalidate(&mut self, project: u64) {
        self.by_project.remove(&project);
    }

    #[cfg(test)]
    pub fn seen(&self, project: u64) -> Option<(&str, Host)> {
        self.by_project
            .get(&project)
            .map(|(u, h)| (u.as_str(), *h))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::prelude::v1::test;

    #[test]
    fn host_detection_covers_url_shapes() {
        assert_eq!(detect_host("https://github.com/a/b.git"), Host::Github);
        assert_eq!(detect_host("git@github.com:a/b.git"), Host::Github);
        assert_eq!(detect_host("ssh://git@github.com:22/a/b"), Host::Github);
        assert_eq!(detect_host("https://gitlab.com/a/b"), Host::Gitlab);
        assert_eq!(
            detect_host("git@gitlab.company.io:team/service.git"),
            Host::Gitlab
        );
        assert_eq!(
            detect_host("https://bitbucket.org/a/b.git"),
            Host::BitbucketCloud
        );
        assert_eq!(
            detect_host("https://bitbucket.internal.example/a/b"),
            Host::BitbucketServer
        );
        assert_eq!(detect_host("https://codeberg.org/a/b"), Host::Gitea);
        assert_eq!(detect_host("https://gitea.company.io/a/b"), Host::Gitea);
        assert_eq!(
            detect_host("https://dev.azure.com/org/project/_git/repo"),
            Host::AzureDevops
        );
        assert_eq!(
            detect_host("https://git.example.com/a/b"),
            Host::Unknown
        );
        assert_eq!(detect_host(""), Host::None);
        assert_eq!(detect_host("not a url"), Host::None);
    }

    #[test]
    fn owner_repo_handles_dot_git_and_scp() {
        assert_eq!(
            owner_repo("git@github.com:a/b.git"),
            Some(("a".into(), "b".into()))
        );
        assert_eq!(
            owner_repo("https://github.com/a/b"),
            Some(("a".into(), "b".into()))
        );
        assert_eq!(
            owner_repo("https://gitlab.com/group/sub/repo.git"),
            Some(("sub".into(), "repo".into()))
        );
        assert_eq!(owner_repo(""), None);
    }

    #[test]
    fn config_detection_picks_up_each_known_file() {
        let dir = tempdir();
        fs::create_dir_all(dir.join(".github/workflows")).unwrap();
        fs::write(dir.join(".github/workflows/ci.yml"), "name: ci").unwrap();
        fs::write(dir.join(".gitlab-ci.yml"), "stages:").unwrap();
        fs::write(dir.join("Jenkinsfile"), "pipeline {}").unwrap();
        fs::create_dir_all(dir.join(".circleci")).unwrap();
        fs::write(dir.join(".circleci/config.yml"), "version: 2").unwrap();
        let found = detect_ci_configs(&dir);
        assert!(found.contains(&CiKind::GithubActions));
        assert!(found.contains(&CiKind::GitlabPipelines));
        assert!(found.contains(&CiKind::Jenkins));
        assert!(found.contains(&CiKind::Circle));
        // A folder with no CI yields nothing.
        let empty = tempdir();
        assert!(detect_ci_configs(&empty).is_empty());
    }

    #[test]
    fn config_detection_requires_a_workflow_file() {
        let dir = tempdir();
        fs::create_dir_all(dir.join(".github/workflows")).unwrap();
        // An empty workflows dir is not a GitHub Actions config.
        assert!(detect_ci_configs(&dir).is_empty());
        fs::write(dir.join(".github/workflows/x.yaml"), "name: x").unwrap();
        assert_eq!(detect_ci_configs(&dir), vec![CiKind::GithubActions]);
    }

    #[test]
    fn tracking_parses_round_trip() {
        for key in [
            "auto",
            "off",
            "github_actions",
            "gitlab_pipelines",
            "bitbucket_pipelines",
        ] {
            assert_eq!(CiTracking::parse(key).as_key(), key);
        }
        assert_eq!(CiTracking::parse(""), CiTracking::Auto);
        assert_eq!(CiTracking::parse("none"), CiTracking::Off);
        assert_eq!(
            CiTracking::parse("custom:https://ci.me/x"),
            CiTracking::Custom("https://ci.me/x".into())
        );
        assert_eq!(CiTracking::parse("custom:"), CiTracking::Auto);
    }

    #[test]
    fn plan_picks_the_matching_ci() {
        let p = plan(
            Host::Github,
            &CiTracking::Auto,
            vec![CiKind::GithubActions, CiKind::Circle],
        );
        assert_eq!(p.ci, Some(CiKind::GithubActions));
        assert_eq!(p.reason, TrackingReason::Watching);
        // External-only CI on GitHub still watches via the host's checks API.
        let p = plan(Host::Github, &CiTracking::Auto, vec![CiKind::Jenkins]);
        assert_eq!(p.ci, Some(CiKind::Jenkins));
        // No remote means nothing to watch, ever.
        let p = plan(Host::None, &CiTracking::Auto, Vec::new());
        assert_eq!(p.reason, TrackingReason::NoRemote);
        assert_eq!(p.ci, None);
        // Unsupported hosts say so instead of silently watching.
        let p = plan(Host::BitbucketServer, &CiTracking::Auto, Vec::new());
        assert_eq!(p.reason, TrackingReason::HostUnsupported);
        // The per-project override wins.
        let p = plan(Host::Github, &CiTracking::Off, vec![CiKind::GithubActions]);
        assert_eq!(p.reason, TrackingReason::DisabledByProject);
        // Custom URL is linked, not polled.
        let p = plan(
            Host::Github,
            &CiTracking::Custom("https://ci.me/x".into()),
            Vec::new(),
        );
        assert_eq!(p.reason, TrackingReason::Custom);
        assert_eq!(p.custom_url.as_deref(), Some("https://ci.me/x"));
    }

    #[test]
    fn chip_label_matches_the_state() {
        let watching = TrackingPlan {
            host: Host::Github,
            ci: Some(CiKind::GithubActions),
            configs: vec![CiKind::GithubActions],
            reason: TrackingReason::Watching,
            custom_url: None,
        };
        assert_eq!(chip_label(&watching, false), "Watching GitHub Actions");
        let probing = TrackingPlan {
            ci: None,
            ..watching.clone()
        };
        assert_eq!(chip_label(&probing, true), "Looking for CI on GitHub");
        assert_eq!(chip_label(&probing, false), "No CI detected on this push");
        let no_remote = TrackingPlan {
            host: Host::None,
            ci: None,
            configs: Vec::new(),
            reason: TrackingReason::NoRemote,
            custom_url: None,
        };
        assert_eq!(chip_label(&no_remote, false), "No remote, no CI tracking");
        let unsupported = TrackingPlan {
            host: Host::BitbucketServer,
            ci: None,
            configs: Vec::new(),
            reason: TrackingReason::HostUnsupported,
            custom_url: None,
        };
        assert_eq!(
            chip_label(&unsupported, false),
            "Bitbucket Server not supported yet"
        );
    }

    #[test]
    fn host_cache_refetches_when_the_remote_changes() {
        let mut cache = HostCache::default();
        assert_eq!(
            cache.get_or_fetch(1, "git@github.com:a/b.git"),
            Host::Github
        );
        // Second call is cached: the URL is remembered.
        assert_eq!(cache.seen(1), Some(("git@github.com:a/b.git", Host::Github)));
        // Changing the URL re-detects.
        assert_eq!(
            cache.get_or_fetch(1, "https://gitlab.com/a/b"),
            Host::Gitlab
        );
        cache.invalidate(1);
        assert!(cache.seen(1).is_none());
    }

    fn tempdir() -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!(
            "runo-provider-test-{}-{}",
            std::process::id(),
            unique()
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn unique() -> u64 {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        N.fetch_add(1, Ordering::Relaxed)
    }
}
