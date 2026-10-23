//! What a project runs, read from how it's deployed: Docker Compose files, Terraform for
//! AWS (ECS services, scheduled ECS jobs, load balancers, queues, topics, databases,
//! caches and buckets), ECS task definitions and AWS Copilot manifests. Each source
//! becomes a plan for one diagram: its parts, the boundaries they sit in (a Compose
//! network, a VPC, an ECS cluster, a group of scheduled jobs) and what talks to what, with
//! the shape and icon that say what each part is and a logo from the icon library to
//! look for.

use super::model::Kind;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

/// What a picture is set to while its logo is still to be found in the icon library.
pub(crate) const LOGO_WANTED: &str = "logo:";

/// What a project's deployment files say it runs: one plan per kind of source.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Infra {
    pub plans: Vec<Plan>,
}

/// One diagram's worth: parts, the boundaries they sit in, and what talks to what.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Plan {
    /// The diagram's name, e.g. "Docker Compose".
    pub name: String,
    /// The files it was read from.
    pub files: Vec<String>,
    pub parts: Vec<Part>,
    pub groups: Vec<Group>,
    pub links: Vec<Link>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Part {
    /// Unique in its plan.
    pub key: String,
    pub name: String,
    pub kind: Kind,
    /// A Lucide icon name.
    pub icon: String,
    /// A logo to look for in the icon library, e.g. "PostgreSQL"; empty for none.
    pub logo: String,
    pub tech: String,
    pub description: String,
    pub code: Vec<String>,
    /// The key of the group it sits in.
    pub group: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Group {
    pub key: String,
    pub name: String,
    pub icon: String,
    pub logo: String,
    pub parent: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Link {
    pub from: String,
    pub to: String,
    pub label: String,
    pub tech: String,
}

impl Plan {
    fn has(&self, key: &str) -> bool {
        self.parts.iter().any(|p| p.key == key)
    }

    fn part_mut(&mut self, key: &str) -> Option<&mut Part> {
        self.parts.iter_mut().find(|p| p.key == key)
    }

    fn add(&mut self, part: Part) {
        if !self.has(&part.key) {
            self.parts.push(part);
        }
    }

    fn group(&mut self, key: &str, name: &str, icon: &str, logo: &str, parent: Option<&str>) {
        if !self.groups.iter().any(|g| g.key == key) {
            self.groups.push(Group {
                key: key.into(),
                name: name.into(),
                icon: icon.into(),
                logo: logo.into(),
                parent: parent.map(String::from),
            });
        }
    }

    /// Connect two parts once, whichever way it was found first.
    fn link(&mut self, from: &str, to: &str, label: &str, tech: &str) {
        if from == to || !self.has(from) || !self.has(to) {
            return;
        }
        if self.links.iter().any(|l| l.from == from && l.to == to) {
            return;
        }
        self.links.push(Link {
            from: from.into(),
            to: to.into(),
            label: label.into(),
            tech: tech.into(),
        });
    }

    fn kind_of(&self, key: &str) -> Option<(Kind, &str)> {
        self.parts
            .iter()
            .find(|p| p.key == key)
            .map(|p| (p.kind, p.icon.as_str()))
    }

    /// What one part does with another, from what the other is.
    fn verb(&self, from: &str, to: &str) -> &'static str {
        let worker = self
            .parts
            .iter()
            .find(|p| p.key == from)
            .is_some_and(|p| matches!(p.icon.as_str(), "cog" | "clock" | "play"));
        match self.kind_of(to) {
            Some((Kind::Database, "gauge")) => "caches in",
            Some((Kind::Database, "search")) => "indexes and searches",
            Some((Kind::Database, "hard-drive")) => "stores files in",
            Some((Kind::Database, _)) => "reads and writes",
            Some((Kind::Queue, _)) if worker => "consumes",
            Some((Kind::Queue, _)) => "publishes to",
            Some((Kind::External, "mail")) => "sends email",
            Some((Kind::External, _)) => "calls",
            _ => "calls",
        }
    }
}

// ---- What things are

/// How a kind of thing is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Look {
    kind: Kind,
    icon: &'static str,
    logo: &'static str,
    tech: &'static str,
}

const fn look(kind: Kind, icon: &'static str, logo: &'static str, tech: &'static str) -> Look {
    Look {
        kind,
        icon,
        logo,
        tech,
    }
}

/// Well-known images, by a word in the image's name.
const IMAGES: &[(&[&str], Look)] = &[
    (
        &["postgres", "postgis", "timescaledb"],
        look(Kind::Database, "database", "PostgreSQL", "PostgreSQL"),
    ),
    (
        &["mysql"],
        look(Kind::Database, "database", "MySQL", "MySQL"),
    ),
    (
        &["mariadb"],
        look(Kind::Database, "database", "MariaDB", "MariaDB"),
    ),
    (
        &["mongo"],
        look(Kind::Database, "database", "MongoDB", "MongoDB"),
    ),
    (
        &["redis", "valkey", "keydb"],
        look(Kind::Database, "gauge", "Redis", "Redis"),
    ),
    (
        &["memcached"],
        look(Kind::Database, "gauge", "", "Memcached"),
    ),
    (
        &["elasticsearch"],
        look(Kind::Database, "search", "Elasticsearch", "Elasticsearch"),
    ),
    (
        &["opensearch"],
        look(Kind::Database, "search", "OpenSearch", "OpenSearch"),
    ),
    (
        &["rabbitmq"],
        look(Kind::Queue, "list-ordered", "RabbitMQ", "RabbitMQ"),
    ),
    (
        &["kafka", "redpanda"],
        look(Kind::Queue, "radio", "Apache Kafka", "Kafka"),
    ),
    (&["nats"], look(Kind::Queue, "radio", "NATS.io", "NATS")),
    (
        &["minio"],
        look(
            Kind::Database,
            "hard-drive",
            "MinIO",
            "S3-compatible storage",
        ),
    ),
    (
        &["localstack"],
        look(Kind::External, "cloud", "", "LocalStack"),
    ),
    (&["nginx"], look(Kind::Container, "split", "NGINX", "NGINX")),
    (
        &["traefik"],
        look(Kind::Container, "split", "Traefik Proxy", "Traefik"),
    ),
    (
        &["envoy"],
        look(Kind::Container, "split", "Envoy Proxy", "Envoy"),
    ),
    (&["haproxy"], look(Kind::Container, "split", "", "HAProxy")),
    (&["caddy"], look(Kind::Container, "split", "Caddy", "Caddy")),
    (
        &["mailpit", "mailhog", "smtp"],
        look(Kind::External, "mail", "", "SMTP"),
    ),
    (
        &["grafana"],
        look(Kind::Container, "activity", "Grafana", "Grafana"),
    ),
    (
        &["prometheus"],
        look(Kind::Container, "activity", "Prometheus", "Prometheus"),
    ),
    (
        &["jaeger"],
        look(Kind::Container, "activity", "Jaeger", "Jaeger"),
    ),
    (
        &["otel", "opentelemetry"],
        look(
            Kind::Container,
            "activity",
            "OpenTelemetry",
            "OpenTelemetry",
        ),
    ),
    (
        &["keycloak"],
        look(Kind::Container, "key-round", "Keycloak", "Keycloak"),
    ),
    (
        &["temporal"],
        look(Kind::Container, "workflow", "Temporal", "Temporal"),
    ),
    (
        &["airflow"],
        look(Kind::Container, "workflow", "Apache Airflow", "Airflow"),
    ),
    (
        &["ofelia", "cron"],
        look(Kind::Container, "clock", "", "Scheduler"),
    ),
];

/// What an image is, from the words in its name: `postgres:16` is PostgreSQL,
/// `docker.elastic.co/elasticsearch/elasticsearch:8` is Elasticsearch.
fn look_of_image(image: &str) -> Option<Look> {
    let name = image.rsplit('/').next().unwrap_or(image);
    let name = name.split([':', '@']).next().unwrap_or(name).to_lowercase();
    IMAGES
        .iter()
        .find(|(words, _)| words.iter().any(|w| name.contains(w)))
        .map(|(_, l)| *l)
}

/// Services outside, found from the settings a part is given (`STRIPE_API_KEY`).
const SAAS: &[(&str, &str, &str, &str)] = &[
    ("STRIPE", "Stripe", "credit-card", "Stripe"),
    ("SENDGRID", "SendGrid", "mail", ""),
    ("TWILIO", "Twilio", "message-square", ""),
    ("MAILGUN", "Mailgun", "mail", "Mailgun"),
    ("POSTMARK", "Postmark", "mail", ""),
    ("SLACK", "Slack", "message-square", "Slack"),
    ("OPENAI", "OpenAI", "sparkles", ""),
    ("ANTHROPIC", "Anthropic", "sparkles", "Anthropic"),
    ("SENTRY", "Sentry", "activity", "Sentry"),
    ("DATADOG", "Datadog", "activity", "Datadog"),
    ("DD_API_KEY", "Datadog", "activity", "Datadog"),
    ("AUTH0", "Auth0", "key-round", "Auth0"),
    ("ALGOLIA", "Algolia", "search", "Algolia"),
    ("SHOPIFY", "Shopify", "shopping-cart", "Shopify"),
    ("PAYPAL", "PayPal", "credit-card", "PayPal"),
];

fn saas_of(key: &str) -> Option<(&'static str, &'static str, &'static str)> {
    let key = key.to_uppercase();
    SAAS.iter()
        .find(|(prefix, ..)| key.starts_with(prefix) || key.contains(&format!("_{prefix}")))
        .map(|(_, name, icon, logo)| (*name, *icon, *logo))
}

/// Words in a name that say what a service is.
fn has_word(text: &str, words: &[&str]) -> bool {
    text.to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|w| words.contains(&w))
}

/// The icon for a service built from the project's own code, from its name.
fn icon_of_service(name: &str, command: &str) -> &'static str {
    let both = format!("{name} {command}");
    if has_word(
        &both,
        &[
            "cron", "job", "jobs", "report", "export", "sync", "batch", "nightly",
        ],
    ) {
        "clock"
    } else if has_word(
        &both,
        &["worker", "consumer", "consume", "celery", "sidekiq"],
    ) {
        "cog"
    } else if has_word(&both, &["migrate", "migration", "migrations", "seed"]) {
        "play"
    } else if has_word(
        &both,
        &["web", "frontend", "ui", "app", "site", "dashboard", "admin"],
    ) {
        "app-window"
    } else if has_word(&both, &["gateway", "proxy", "edge"]) {
        "split"
    } else {
        "server"
    }
}

/// The language a folder's code is in, as a logo and a word for the technology.
fn language_of(dir: &Path) -> Option<(&'static str, &'static str)> {
    let has = |f: &str| dir.join(f).is_file();
    if has("package.json") {
        let text = std::fs::read_to_string(dir.join("package.json")).unwrap_or_default();
        return Some(if text.contains("\"next\"") {
            ("Next.js", "Next.js")
        } else if text.contains("\"react\"") {
            ("React", "React")
        } else {
            ("Node.js", "Node.js")
        });
    }
    if has("go.mod") {
        return Some(("Go", "Go"));
    }
    if has("pyproject.toml") || has("requirements.txt") || has("setup.py") {
        return Some(("Python", "Python"));
    }
    if has("Cargo.toml") {
        return Some(("Rust", "Rust"));
    }
    if has("pom.xml") || has("build.gradle") || has("build.gradle.kts") {
        return Some(("OpenJDK", "Java"));
    }
    if has("Gemfile") {
        return Some(("Ruby", "Ruby"));
    }
    if has("composer.json") {
        return Some(("PHP", "PHP"));
    }
    None
}

/// Whether `text` names `host` as a host: `http://api:8080`, `@postgres:5432`,
/// `orders.shipyard.local`, or just `mailpit`. A scheme (`redis://`) doesn't count.
fn mentions_host(text: &str, host: &str) -> bool {
    if host.is_empty() {
        return false;
    }
    let t = text.to_lowercase();
    let host = host.to_lowercase();
    let bytes = t.as_bytes();
    let mut from = 0;
    while let Some(i) = t[from..].find(&host) {
        let at = from + i;
        let end = at + host.len();
        let before = at.checked_sub(1).map(|j| bytes[j] as char);
        let after = t[end..].chars().next();
        let ok_before = matches!(before, None | Some('/' | '@' | '=' | ' ' | ',' | '"'));
        let scheme = t[end..].starts_with("://");
        let ok_after = matches!(after, None | Some(':' | '/' | '.' | ' ' | ',' | '"'));
        if ok_before && ok_after && !scheme {
            return true;
        }
        from = end;
    }
    false
}

/// The protocol a connection string speaks, from its scheme.
fn tech_of_url(text: &str) -> &'static str {
    let t = text.trim().to_lowercase();
    let scheme = t.split("://").next().unwrap_or("");
    if !t.contains("://") {
        return "";
    }
    match scheme {
        "http" | "https" => "HTTP",
        "postgres" | "postgresql" | "mysql" | "mariadb" => "SQL",
        "amqp" | "amqps" => "AMQP",
        "redis" | "rediss" => "Redis",
        "mongodb" | "mongodb+srv" => "MongoDB",
        "nats" => "NATS",
        "kafka" => "Kafka",
        "smtp" | "smtps" => "SMTP",
        _ => "",
    }
}

/// A schedule as people say it: `cron(0 2 * * ? *)` is "daily at 02:00", `rate(15
/// minutes)` "every 15 minutes".
pub(crate) fn humanize_schedule(expr: &str) -> String {
    let e = expr.trim();
    if let Some(inner) = e.strip_prefix("rate(").and_then(|s| s.strip_suffix(')')) {
        let inner = inner.trim();
        return match inner {
            "1 minute" => "every minute".into(),
            "1 hour" => "hourly".into(),
            "1 day" => "daily".into(),
            _ => format!("every {inner}"),
        };
    }
    let inner = e
        .strip_prefix("cron(")
        .and_then(|s| s.strip_suffix(')'))
        .unwrap_or(e);
    let f: Vec<&str> = inner.split_whitespace().collect();
    if f.len() < 5 {
        return e.to_string();
    }
    let (min, hour, dom, dow) = (f[0], f[1], f[2], f[4]);
    let (Ok(m), Ok(h)) = (min.parse::<u32>(), hour.parse::<u32>()) else {
        return e.to_string();
    };
    let at = format!("{h:02}:{m:02}");
    let any = |s: &str| s == "*" || s == "?";
    let day = |d: &str| -> Option<&'static str> {
        Some(match d.to_uppercase().as_str() {
            "SUN" | "1" => "Sunday",
            "MON" | "2" => "Monday",
            "TUE" | "3" => "Tuesday",
            "WED" | "4" => "Wednesday",
            "THU" | "5" => "Thursday",
            "FRI" | "6" => "Friday",
            "SAT" | "7" => "Saturday",
            "MON-FRI" | "2-6" => "weekdays",
            _ => return None,
        })
    };
    if any(dom) && any(dow) {
        format!("daily at {at}")
    } else if any(dom) {
        match day(dow) {
            Some("weekdays") => format!("weekdays at {at}"),
            Some(d) => format!("{d}s at {at}"),
            None => e.to_string(),
        }
    } else if dom.parse::<u32>().is_ok() && any(dow) {
        format!("monthly on day {dom} at {at}")
    } else {
        e.to_string()
    }
}

// ---- A little YAML

/// A YAML value, as far as deployment files need: mappings, lists and text.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Yaml {
    Map(Vec<(String, Yaml)>),
    List(Vec<Yaml>),
    Text(String),
    Null,
}

impl Yaml {
    pub(crate) fn get(&self, key: &str) -> Option<&Yaml> {
        match self {
            Yaml::Map(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub(crate) fn text(&self) -> Option<&str> {
        match self {
            Yaml::Text(t) => Some(t),
            _ => None,
        }
    }

    pub(crate) fn entries(&self) -> &[(String, Yaml)] {
        match self {
            Yaml::Map(m) => m,
            _ => &[],
        }
    }

    /// A list's items or a mapping's keys, as text: `depends_on` and `networks` come
    /// either way.
    pub(crate) fn names(&self) -> Vec<String> {
        match self {
            Yaml::List(l) => l
                .iter()
                .filter_map(|v| v.text().map(String::from))
                .collect(),
            Yaml::Map(m) => m.iter().map(|(k, _)| k.clone()).collect(),
            Yaml::Text(t) => vec![t.clone()],
            Yaml::Null => Vec::new(),
        }
    }

    /// Settings as pairs: a mapping, or a list of `KEY=value`.
    pub(crate) fn pairs(&self) -> Vec<(String, String)> {
        match self {
            Yaml::Map(m) => m
                .iter()
                .map(|(k, v)| (k.clone(), v.text().unwrap_or("").to_string()))
                .collect(),
            Yaml::List(l) => l
                .iter()
                .filter_map(|v| v.text())
                .map(|t| match t.split_once('=') {
                    Some((k, v)) => (k.trim().to_string(), v.trim().to_string()),
                    None => (t.trim().to_string(), String::new()),
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Everything written in it, flattened to one line: for finding names in commands.
    pub(crate) fn flat(&self) -> String {
        match self {
            Yaml::Text(t) => t.clone(),
            Yaml::List(l) => l.iter().map(Yaml::flat).collect::<Vec<_>>().join(" "),
            Yaml::Map(m) => m
                .iter()
                .map(|(k, v)| format!("{k} {}", v.flat()))
                .collect::<Vec<_>>()
                .join(" "),
            Yaml::Null => String::new(),
        }
    }
}

/// Read the block style YAML that Compose files and manifests are written in: nested
/// mappings and lists, flow lists and mappings on one line, quoted text and block text.
/// Anchors are read as plain values and merges (`<<: *x`) are skipped. Only the first
/// document of a file is read.
pub(crate) fn parse_yaml(text: &str) -> Yaml {
    let mut lines: Vec<(usize, String)> = Vec::new();
    let mut started = false;
    let mut raw = text.lines().peekable();
    while let Some(line) = raw.next() {
        if line.trim_end() == "---" {
            if started {
                break;
            }
            continue;
        }
        let content = strip_comment(line);
        if content.trim().is_empty() {
            continue;
        }
        started = true;
        let indent = content.len() - content.trim_start().len();
        let body = content.trim().to_string();
        // Block text (`|` or `>`): the more indented lines after it, joined.
        let block = body.ends_with(": |")
            || body.ends_with(": >")
            || body.ends_with(": |-")
            || body.ends_with(": >-")
            || body == "- |"
            || body == "- >";
        if block {
            let mut joined = Vec::new();
            while let Some(next) = raw.peek() {
                let ni = next.len() - next.trim_start().len();
                if !next.trim().is_empty() && ni <= indent {
                    break;
                }
                joined.push(next.trim().to_string());
                raw.next();
            }
            let head = body
                .trim_end_matches(['|', '>', '-'])
                .trim_end()
                .to_string();
            let value = joined.join(" ").replace('"', "'");
            lines.push((indent, format!("{head} \"{}\"", value.trim())));
            continue;
        }
        lines.push((indent, body));
    }
    let mut i = 0;
    if lines.is_empty() {
        return Yaml::Null;
    }
    let first = lines[0].0;
    yaml_node(&mut lines, &mut i, first)
}

fn strip_comment(line: &str) -> &str {
    let mut quote: Option<char> = None;
    let mut prev = ' ';
    for (i, c) in line.char_indices() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c == '#' && prev.is_whitespace() => return &line[..i],
            None => {}
        }
        prev = c;
    }
    line
}

fn yaml_node(lines: &mut Vec<(usize, String)>, i: &mut usize, indent: usize) -> Yaml {
    let Some((ind, body)) = lines.get(*i).cloned() else {
        return Yaml::Null;
    };
    if ind < indent {
        return Yaml::Null;
    }
    if body == "-" || body.starts_with("- ") {
        let mut items = Vec::new();
        while let Some((ind, body)) = lines.get(*i).cloned() {
            if ind != indent || !(body == "-" || body.starts_with("- ")) {
                break;
            }
            let rest = body[1..].trim().to_string();
            if rest.is_empty() {
                *i += 1;
                let deeper = lines.get(*i).map(|l| l.0).unwrap_or(0);
                items.push(if deeper > indent {
                    yaml_node(lines, i, deeper)
                } else {
                    Yaml::Null
                });
            } else if key_value(&rest).is_some() && !rest.starts_with(['"', '\'', '[', '{']) {
                // `- key: value` starts a mapping, indented where `key` is.
                let inner = indent + (body.len() - rest.len());
                lines[*i] = (inner, rest);
                items.push(yaml_node(lines, i, inner));
            } else {
                *i += 1;
                items.push(scalar(&rest));
            }
        }
        return Yaml::List(items);
    }
    let mut map = Vec::new();
    while let Some((ind, body)) = lines.get(*i).cloned() {
        if ind != indent || body.starts_with("- ") || body == "-" {
            break;
        }
        *i += 1;
        let Some((key, value)) = key_value(&body) else {
            continue;
        };
        // An anchor alone (`x-app: &app`) names the mapping under it.
        let value = if value.starts_with('&') && !value.contains(' ') {
            String::new()
        } else {
            value
        };
        let v = if value.is_empty() {
            match lines.get(*i) {
                Some((ni, nb)) if *ni > indent => {
                    let ni = *ni;
                    yaml_node(lines, i, ni)
                }
                // Compose often writes a key's list at the key's own indent.
                Some((ni, nb)) if *ni == indent && (nb.starts_with("- ") || nb == "-") => {
                    yaml_node(lines, i, indent)
                }
                _ => Yaml::Null,
            }
        } else {
            // Text running on over more lines belongs to this value.
            while lines.get(*i).is_some_and(|l| l.0 > indent) {
                *i += 1;
            }
            scalar(&value)
        };
        if key == "<<" {
            continue;
        }
        map.push((key, v));
    }
    Yaml::Map(map)
}

/// Split `key: value` at the first colon outside quotes that ends a word.
fn key_value(body: &str) -> Option<(String, String)> {
    let mut quote: Option<char> = None;
    let chars: Vec<(usize, char)> = body.char_indices().collect();
    for (k, &(i, c)) in chars.iter().enumerate() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if (c == '"' || c == '\'') && k == 0 => quote = Some(c),
            None if c == '[' || c == '{' => return None,
            None if c == ':' => {
                let next = chars.get(k + 1).map(|x| x.1);
                if next.is_none() || next == Some(' ') {
                    let key = unquote(body[..i].trim());
                    let value = body[i + 1..].trim().to_string();
                    return Some((key, value));
                }
            }
            None => {}
        }
    }
    None
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2
        && ((s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')))
    {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

/// A value on one line: text, or a flow list or mapping.
fn scalar(s: &str) -> Yaml {
    let mut s = s.trim();
    // An anchor names the value; the value is what counts.
    if s.starts_with('&') {
        s = s.split_once(' ').map(|(_, r)| r.trim()).unwrap_or("");
        if s.is_empty() {
            return Yaml::Null;
        }
    }
    if s.starts_with('[') && s.ends_with(']') {
        return Yaml::List(
            split_flow(&s[1..s.len() - 1])
                .into_iter()
                .map(|p| scalar(&p))
                .collect(),
        );
    }
    if s.starts_with('{') && s.ends_with('}') {
        return Yaml::Map(
            split_flow(&s[1..s.len() - 1])
                .into_iter()
                .map(|p| match key_value(&p) {
                    Some((k, v)) => (k, scalar(&v)),
                    None => (unquote(&p), Yaml::Null),
                })
                .collect(),
        );
    }
    if s == "~" || s == "null" {
        return Yaml::Null;
    }
    Yaml::Text(unquote(s))
}

/// Split a flow list's insides at the commas between its items.
fn split_flow(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut cur = String::new();
    for c in s.chars() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
                cur.push(c);
            }
            None => match c {
                '"' | '\'' => {
                    quote = Some(c);
                    cur.push(c);
                }
                '[' | '{' => {
                    depth += 1;
                    cur.push(c);
                }
                ']' | '}' => {
                    depth -= 1;
                    cur.push(c);
                }
                ',' if depth == 0 => {
                    if !cur.trim().is_empty() {
                        out.push(cur.trim().to_string());
                    }
                    cur.clear();
                }
                _ => cur.push(c),
            },
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

// ---- Docker Compose

/// Whether a file is a Compose file.
fn is_compose(name: &str) -> bool {
    let n = name.to_lowercase();
    (n.starts_with("docker-compose") || n.starts_with("compose"))
        && (n.ends_with(".yml") || n.ends_with(".yaml"))
        // Overrides change a few settings; the main file says what runs.
        && !n.contains("override")
}

/// The plan for one Compose file: each service a part, grouped by the network it's on
/// first (and its profile, for services only started on demand), connected by what each
/// depends on and the hosts its settings name, with the people who use what it publishes.
pub(crate) fn read_compose(text: &str, dir: &Path, rel_dir: &str, project: &str) -> Plan {
    let doc = parse_yaml(text);
    let mut plan = Plan {
        name: "Docker Compose".into(),
        ..Default::default()
    };
    let Some(services) = doc.get("services") else {
        return plan;
    };
    let name = doc
        .get("name")
        .and_then(Yaml::text)
        .unwrap_or(project)
        .to_string();
    let top = "compose";
    plan.group(
        top,
        &format!("{name} (Docker Compose)"),
        "container",
        "Docker",
        None,
    );
    let declared: Vec<String> = doc.get("networks").map(Yaml::names).unwrap_or_default();
    let by_network = declared.len() >= 2;
    let service_names: Vec<String> = services.entries().iter().map(|(k, _)| k.clone()).collect();
    let mut entry: Vec<(String, bool)> = Vec::new();
    for (svc, def) in services.entries() {
        let image = def.get("image").and_then(Yaml::text).unwrap_or("");
        let build = match def.get("build") {
            Some(Yaml::Text(t)) => Some(t.clone()),
            Some(b @ Yaml::Map(_)) => Some(
                b.get("context")
                    .and_then(Yaml::text)
                    .unwrap_or(".")
                    .to_string(),
            ),
            _ => None,
        };
        let command = def.get("command").map(Yaml::flat).unwrap_or_default();
        let known = if build.is_none() {
            look_of_image(image)
        } else {
            None
        };
        let mut part = Part {
            key: svc.clone(),
            name: svc.clone(),
            ..Default::default()
        };
        match (known, &build) {
            (Some(l), _) => {
                part.kind = l.kind;
                part.icon = l.icon.into();
                part.logo = l.logo.into();
                part.tech = l.tech.into();
                part.description = format!("Runs {}", image.rsplit('/').next().unwrap_or(image));
            }
            (None, Some(ctx)) => {
                let ctx = ctx.trim_start_matches("./").trim_end_matches('/');
                let folder = dir.join(ctx);
                let lang = language_of(&folder);
                part.kind = Kind::Container;
                part.icon = icon_of_service(svc, &command).into();
                part.logo = lang.map(|l| l.0).unwrap_or("Docker").into();
                part.tech = lang.map(|l| l.1).unwrap_or("Docker").into();
                let path = if rel_dir.is_empty() || ctx == "." {
                    ctx.to_string()
                } else {
                    format!("{rel_dir}/{ctx}")
                };
                if !path.is_empty() && path != "." {
                    part.code = vec![format!("{path}/**")];
                    part.description = format!("Built from {path}");
                }
            }
            (None, None) => {
                part.kind = Kind::Container;
                part.icon = icon_of_service(svc, &command).into();
                part.logo = "Docker".into();
                part.tech = "Docker".into();
                if !image.is_empty() {
                    part.description =
                        format!("Runs {}", image.rsplit('/').next().unwrap_or(image));
                }
            }
        }
        // The boundary it sits in: its first network, then its profile.
        let nets = def.get("networks").map(Yaml::names).unwrap_or_default();
        let mut group = top.to_string();
        if by_network {
            if let Some(n) = nets.first() {
                let key = format!("net:{n}");
                plan.group(&key, &format!("{n} network"), "network", "", Some(top));
                group = key;
            }
        }
        if let Some(p) = def
            .get("profiles")
            .map(Yaml::names)
            .and_then(|p| p.first().cloned())
        {
            let key = format!("{group}/profile:{p}");
            let label = if has_word(&p, &["jobs", "job", "cron", "tasks", "batch"]) {
                format!("{} (on demand)", capitalize(&p))
            } else {
                format!("{} profile", capitalize(&p))
            };
            plan.group(&key, &label, "clock", "", Some(&group));
            group = key;
        }
        part.group = Some(group);
        let ports = def.get("ports").map(Yaml::flat).unwrap_or_default();
        let public = ports.split_whitespace().any(|p| {
            p.trim_matches('"').split(':').next_back().is_some_and(|c| {
                matches!(
                    c.split('/').next().unwrap_or(""),
                    "80" | "443" | "3000" | "8080"
                )
            })
        });
        entry.push((svc.clone(), public));
        plan.add(part);
    }
    // Connections: hosts named in settings, then what each depends on.
    let mut saas: BTreeMap<&str, (&str, &str)> = BTreeMap::new();
    let mut uses: Vec<(String, &str)> = Vec::new();
    for (svc, def) in services.entries() {
        let env = def.get("environment").map(Yaml::pairs).unwrap_or_default();
        for (key, value) in &env {
            for other in &service_names {
                if other != svc && mentions_host(value, other) {
                    let verb = plan.verb(svc, other);
                    plan.link(svc, other, verb, tech_of_url(value));
                }
            }
            if let Some((name, icon, logo)) = saas_of(key) {
                saas.insert(name, (icon, logo));
                uses.push((svc.clone(), name));
            }
        }
        let deps = [
            def.get("depends_on").map(Yaml::names).unwrap_or_default(),
            def.get("links")
                .map(Yaml::names)
                .unwrap_or_default()
                .into_iter()
                .map(|l| l.split(':').next().unwrap_or("").to_string())
                .collect(),
        ]
        .concat();
        let edge = plan
            .parts
            .iter()
            .find(|p| &p.key == svc)
            .is_some_and(|p| p.icon == "split");
        for d in deps {
            let verb = if edge {
                "routes to"
            } else {
                plan.verb(svc, &d)
            };
            plan.link(svc, &d, verb, if edge { "HTTP" } else { "" });
        }
    }
    for (name, (icon, logo)) in &saas {
        plan.add(Part {
            key: format!("saas:{name}"),
            name: name.to_string(),
            kind: Kind::External,
            icon: icon.to_string(),
            logo: logo.to_string(),
            tech: "External service".into(),
            ..Default::default()
        });
    }
    for (svc, name) in uses {
        let to = format!("saas:{name}");
        let verb = plan.verb(&svc, &to);
        plan.link(&svc, &to, verb, "HTTPS");
    }
    // People reach what is published on a web port: the proxy in front if there is one.
    let doors: Vec<String> = {
        let edges: Vec<String> = entry
            .iter()
            .filter(|(k, public)| {
                *public && plan.parts.iter().any(|p| &p.key == k && p.icon == "split")
            })
            .map(|(k, _)| k.clone())
            .collect();
        if edges.is_empty() {
            entry
                .iter()
                .filter(|(k, public)| {
                    *public
                        && plan
                            .parts
                            .iter()
                            .any(|p| &p.key == k && p.kind == Kind::Container)
                })
                .map(|(k, _)| k.clone())
                .collect()
        } else {
            edges
        }
    };
    if !doors.is_empty() {
        plan.add(Part {
            key: "people".into(),
            name: "Users".into(),
            kind: Kind::Users,
            icon: "users".into(),
            ..Default::default()
        });
        for d in doors {
            plan.link("people", &d, "uses", "HTTPS");
        }
    }
    plan
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

// ---- Terraform

/// A `resource "type" "name" { ... }` block.
#[derive(Clone, Debug, PartialEq)]
struct Resource {
    ty: String,
    name: String,
    body: String,
}

impl Resource {
    fn key(&self) -> String {
        format!("{}.{}", self.ty, self.name)
    }

    /// A plain string setting: `name = "orders"`. Interpolations are left out, so
    /// `"${local.app}-orders"` is "orders".
    fn attr(&self, key: &str) -> Option<String> {
        for line in self.body.lines() {
            let l = line.trim();
            let Some(rest) = l.strip_prefix(key) else {
                continue;
            };
            let Some(rest) = rest.trim_start().strip_prefix('=') else {
                continue;
            };
            let v = rest.trim();
            if !v.starts_with('"') {
                continue;
            }
            let v = v.trim_matches(|c| c == '"' || c == ',');
            let v = strip_interpolation(v);
            if !v.is_empty() {
                return Some(v);
            }
        }
        None
    }

    /// Every other resource it refers to, as `type.name`.
    fn refs(&self) -> Vec<String> {
        refs_in(&self.body)
    }
}

fn strip_interpolation(v: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    let mut chars = v.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '$' && chars.peek() == Some(&'{') {
            depth += 1;
            chars.next();
            continue;
        }
        if depth > 0 {
            if c == '}' {
                depth -= 1;
            }
            continue;
        }
        out.push(c);
    }
    out.trim_matches(|c: char| c == '-' || c == '_' || c == '/' || c.is_whitespace())
        .to_string()
}

/// `aws_x.y` references in Terraform text.
fn refs_in(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let b = text.as_bytes();
    let mut i = 0;
    while let Some(at) = text[i..].find("aws_") {
        let s = i + at;
        i = s + 4;
        if s > 0 && (b[s - 1].is_ascii_alphanumeric() || b[s - 1] == b'_' || b[s - 1] == b'.') {
            continue;
        }
        let mut e = s;
        while e < b.len() && (b[e].is_ascii_alphanumeric() || b[e] == b'_') {
            e += 1;
        }
        if e >= b.len() || b[e] != b'.' {
            continue;
        }
        let mut f = e + 1;
        while f < b.len() && (b[f].is_ascii_alphanumeric() || b[f] == b'_' || b[f] == b'-') {
            f += 1;
        }
        if f > e + 1 {
            let r = format!("{}.{}", &text[s..e], &text[e + 1..f]);
            if !out.contains(&r) {
                out.push(r);
            }
        }
        i = f;
    }
    out
}

/// The `resource` blocks of a Terraform file.
fn resources(text: &str) -> Vec<Resource> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("resource \"") {
        // Only at the start of a line.
        if at > 0 && !rest[..at].ends_with('\n') {
            rest = &rest[at + 9..];
            continue;
        }
        let head = &rest[at..];
        let labels: Vec<&str> = head.lines().next().unwrap_or("").split('"').collect();
        let Some(open) = head.find('{') else { break };
        let mut depth = 0;
        let mut end = None;
        let mut quote = false;
        let hb = head.as_bytes();
        let mut k = open;
        while k < hb.len() {
            let c = hb[k];
            if c == b'"' && (k == 0 || hb[k - 1] != b'\\') {
                quote = !quote;
            } else if !quote {
                if c == b'{' {
                    depth += 1;
                } else if c == b'}' {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(k);
                        break;
                    }
                }
            }
            k += 1;
        }
        let Some(end) = end else { break };
        if labels.len() >= 4 {
            out.push(Resource {
                ty: labels[1].to_string(),
                name: labels[3].to_string(),
                body: head[open + 1..end].to_string(),
            });
        }
        rest = &head[end + 1..];
    }
    out
}

/// How a Terraform resource type is drawn, and where it sits: inside the VPC or
/// beside it.
fn look_of_resource(ty: &str, r: &Resource) -> Option<(Look, bool)> {
    let engine = r.attr("engine").unwrap_or_default().to_lowercase();
    let db_logo = if engine.contains("mysql") {
        ("MySQL", "MySQL")
    } else if engine.contains("maria") {
        ("MariaDB", "MariaDB")
    } else {
        ("PostgreSQL", "PostgreSQL")
    };
    Some(match ty {
        "aws_ecs_service" => (look(Kind::Container, "container", "", "ECS service"), true),
        "aws_lambda_function" => (look(Kind::Container, "zap", "", "Lambda"), false),
        "aws_sfn_state_machine" => (
            look(Kind::Container, "workflow", "", "Step Functions"),
            false,
        ),
        "aws_batch_job_definition" => (look(Kind::Container, "layers", "", "Batch job"), true),
        "aws_db_instance" | "aws_rds_cluster" => (
            Look {
                kind: Kind::Database,
                icon: "database",
                logo: db_logo.0,
                tech: if ty == "aws_rds_cluster" {
                    "Aurora"
                } else {
                    "RDS"
                },
            },
            true,
        ),
        "aws_elasticache_replication_group" | "aws_elasticache_cluster" => {
            (look(Kind::Database, "gauge", "Redis", "ElastiCache"), true)
        }
        "aws_opensearch_domain" | "aws_elasticsearch_domain" => (
            look(Kind::Database, "search", "OpenSearch", "OpenSearch"),
            true,
        ),
        "aws_dynamodb_table" => (look(Kind::Database, "table", "", "DynamoDB"), false),
        "aws_s3_bucket" => (look(Kind::Database, "hard-drive", "", "S3 bucket"), false),
        "aws_sqs_queue" => (look(Kind::Queue, "list-ordered", "", "SQS"), false),
        "aws_sns_topic" => (look(Kind::Queue, "radio", "", "SNS topic"), false),
        "aws_kinesis_stream" => (look(Kind::Queue, "radio", "", "Kinesis"), false),
        "aws_mq_broker" => (look(Kind::Queue, "list-ordered", "", "Amazon MQ"), true),
        "aws_msk_cluster" => (look(Kind::Queue, "radio", "Apache Kafka", "MSK"), true),
        "aws_lb" | "aws_alb" => (look(Kind::Container, "split", "", "Load balancer"), true),
        "aws_cloudfront_distribution" => (look(Kind::Container, "globe", "", "CloudFront"), false),
        "aws_api_gateway_rest_api" | "aws_apigatewayv2_api" => {
            (look(Kind::Container, "webhook", "", "API Gateway"), false)
        }
        "aws_cognito_user_pool" => (look(Kind::External, "key-round", "", "Cognito"), false),
        _ => return None,
    })
}

/// A readable name for a resource: its `name` (or the like) if it has a plain one,
/// else its Terraform name with dashes.
fn resource_name(r: &Resource) -> String {
    for key in [
        "name",
        "function_name",
        "identifier",
        "replication_group_id",
        "cluster_id",
        "domain_name",
        "bucket",
        "family",
    ] {
        if let Some(v) = r.attr(key) {
            if !v.contains('.') || key == "bucket" {
                return v;
            }
        }
    }
    r.name.replace('_', "-")
}

/// The plan for a project's Terraform: AWS parts inside the VPC and the ECS cluster they
/// run in, scheduled jobs in a boundary of their own, and what calls, publishes to and
/// reads from what, through load balancers, topics, queues and schedules.
pub(crate) fn read_terraform(files: &[(String, String)]) -> Plan {
    let mut plan = Plan {
        name: "AWS".into(),
        ..Default::default()
    };
    let all: Vec<Resource> = files.iter().flat_map(|(_, t)| resources(t)).collect();
    if !all.iter().any(|r| r.ty.starts_with("aws_")) {
        return plan;
    }
    plan.files = files.iter().map(|(f, _)| f.clone()).collect();
    let by_key: HashMap<String, &Resource> = all.iter().map(|r| (r.key(), r)).collect();
    let of_type = |ty: &str| -> Vec<&Resource> { all.iter().filter(|r| r.ty == ty).collect() };

    let aws = "aws";
    plan.group(aws, "AWS", "cloud", "Amazon Web Services", None);
    let vpcs = of_type("aws_vpc");
    let vpc = (vpcs.len() == 1).then(|| {
        let v = vpcs[0];
        let tag = v
            .body
            .lines()
            .find_map(|l| l.trim().strip_prefix("tags").map(|_| l))
            .and_then(|l| l.split("Name").nth(1))
            .and_then(|l| l.split('"').nth(1))
            .map(strip_interpolation)
            .filter(|s| !s.is_empty());
        let name = tag.unwrap_or_else(|| v.name.replace('_', "-"));
        plan.group("vpc", &format!("VPC {name}"), "network", "", Some(aws));
        "vpc".to_string()
    });
    let inside = |plan: &Plan| -> Option<String> {
        let _ = plan;
        vpc.clone().or(Some(aws.to_string()))
    };
    let clusters = of_type("aws_ecs_cluster");
    for c in &clusters {
        let name = resource_name(c);
        let parent = inside(&plan);
        plan.group(
            &c.key(),
            &format!("ECS cluster {name}"),
            "boxes",
            "",
            parent.as_deref(),
        );
    }
    let only_cluster = (clusters.len() == 1).then(|| clusters[0].key());

    // Task definitions: run by a service, or by a schedule, a workflow or by hand.
    let mut run_by_service: HashMap<String, String> = HashMap::new();
    for s in of_type("aws_ecs_service") {
        for r in s.refs() {
            if r.starts_with("aws_ecs_task_definition.") {
                run_by_service.insert(r, s.key());
            }
        }
    }

    // Parts.
    for r in &all {
        let Some((l, in_vpc)) = look_of_resource(&r.ty, r) else {
            continue;
        };
        let mut part = Part {
            key: r.key(),
            name: resource_name(r),
            kind: l.kind,
            icon: l.icon.into(),
            logo: l.logo.into(),
            tech: l.tech.into(),
            ..Default::default()
        };
        if r.ty == "aws_ecs_service" {
            let fargate = r.attr("launch_type").is_some_and(|t| t == "FARGATE");
            part.tech = if fargate {
                "ECS Fargate service".into()
            } else {
                "ECS service".into()
            };
            part.icon = icon_of_service(&part.name, "").into();
            if part.icon == "server" {
                part.icon = "container".into();
            }
            if let Some(n) = r.attr("desired_count").or_else(|| {
                r.body
                    .lines()
                    .find_map(|l| l.trim().strip_prefix("desired_count"))
                    .map(|v| v.trim_start_matches([' ', '=']).trim().to_string())
            }) {
                part.description = format!("{n} tasks");
            }
            part.group = r
                .refs()
                .into_iter()
                .find(|x| x.starts_with("aws_ecs_cluster."))
                .or_else(|| only_cluster.clone());
        } else if in_vpc {
            part.group = inside(&plan);
        } else {
            part.group = Some(aws.into());
        }
        if r.ty == "aws_sqs_queue" && has_word(&part.name, &["dlq", "dead"]) {
            part.icon = "archive".into();
            part.tech = "SQS dead-letter queue".into();
        }
        plan.add(part);
    }

    // Scheduled and on-demand ECS jobs.
    let mut schedules: Vec<(String, String)> = Vec::new();
    for t in of_type("aws_cloudwatch_event_target") {
        let refs = t.refs();
        let rule = refs
            .iter()
            .find(|x| x.starts_with("aws_cloudwatch_event_rule."))
            .and_then(|x| by_key.get(x.as_str()));
        let when = rule
            .and_then(|r| r.attr("schedule_expression"))
            .map(|e| humanize_schedule(&e))
            .unwrap_or_else(|| "on event".into());
        for x in refs.iter().filter(|x| {
            x.starts_with("aws_ecs_task_definition.")
                || x.starts_with("aws_sfn_state_machine.")
                || x.starts_with("aws_lambda_function.")
                || x.starts_with("aws_batch_job_definition.")
        }) {
            schedules.push((x.clone(), when.clone()));
        }
    }
    for s in of_type("aws_scheduler_schedule") {
        let when = s
            .attr("schedule_expression")
            .map(|e| humanize_schedule(&e))
            .unwrap_or_else(|| "on schedule".into());
        for x in s.refs().into_iter().filter(|x| {
            x.starts_with("aws_ecs_task_definition.")
                || x.starts_with("aws_sfn_state_machine.")
                || x.starts_with("aws_lambda_function.")
        }) {
            schedules.push((x, when.clone()));
        }
    }
    let mut workflow_runs: Vec<(String, String)> = Vec::new();
    for m in of_type("aws_sfn_state_machine") {
        for x in m.refs() {
            if x.starts_with("aws_ecs_task_definition.") {
                workflow_runs.push((m.key(), x));
            }
        }
    }
    let jobs_group = only_cluster.as_ref().map(|c| format!("{c}/jobs"));
    for td in of_type("aws_ecs_task_definition") {
        let key = td.key();
        if run_by_service.contains_key(&key) {
            continue;
        }
        let scheduled = schedules.iter().any(|(x, _)| x == &key)
            || workflow_runs.iter().any(|(_, x)| x == &key);
        let name = resource_name(td);
        if let (Some(g), Some(c)) = (&jobs_group, &only_cluster) {
            plan.group(g, "Scheduled jobs", "clock", "", Some(c));
        }
        plan.add(Part {
            key: key.clone(),
            name,
            kind: Kind::Container,
            icon: if scheduled { "clock" } else { "play" }.into(),
            tech: if scheduled {
                "ECS scheduled task".into()
            } else {
                "ECS task".into()
            },
            description: if scheduled {
                String::new()
            } else {
                "Runs on demand".into()
            },
            group: jobs_group.clone().or_else(|| Some(aws.into())),
            ..Default::default()
        });
    }
    if !schedules.is_empty() {
        // The schedule sits with the jobs it starts, so the two read as one.
        let with_jobs = jobs_group
            .clone()
            .filter(|g| plan.groups.iter().any(|x| &x.key == g));
        plan.add(Part {
            key: "eventbridge".into(),
            name: "EventBridge".into(),
            kind: Kind::Container,
            icon: "calendar-range".into(),
            tech: "EventBridge Scheduler".into(),
            description: "Starts the scheduled jobs".into(),
            group: with_jobs.clone().or_else(|| Some(aws.into())),
            ..Default::default()
        });
        for (target, when) in &schedules {
            plan.link("eventbridge", target, when, "");
            if let (Some(g), Some(p)) = (&with_jobs, plan.part_mut(target)) {
                if p.key.starts_with("aws_sfn_state_machine.") {
                    p.group = Some(g.clone());
                }
            }
        }
    }
    for (m, td) in &workflow_runs {
        plan.link(m, td, "runs", "ECS RunTask");
    }

    // What each running thing uses: through its own settings and its task definition's.
    let services: Vec<(String, String)> = plan
        .parts
        .iter()
        .filter(|p| p.key.starts_with("aws_ecs_service."))
        .map(|p| (p.key.clone(), p.name.clone()))
        .collect();
    let runners: Vec<String> = plan
        .parts
        .iter()
        .filter(|p| {
            [
                "aws_ecs_service.",
                "aws_ecs_task_definition.",
                "aws_lambda_function.",
                "aws_batch_job_definition.",
            ]
            .iter()
            .any(|t| p.key.starts_with(t))
        })
        .map(|p| p.key.clone())
        .collect();
    let mut saas: BTreeMap<&str, (&str, &str)> = BTreeMap::new();
    let mut uses: Vec<(String, &str)> = Vec::new();
    for key in &runners {
        let Some(r) = by_key.get(key.as_str()) else {
            continue;
        };
        let mut body = r.body.clone();
        for x in r.refs() {
            if x.starts_with("aws_ecs_task_definition.") {
                if let Some(td) = by_key.get(x.as_str()) {
                    body.push('\n');
                    body.push_str(&td.body);
                }
            }
        }
        for x in refs_in(&body) {
            if plan.has(&x) && !x.starts_with("aws_ecs_task_definition.") {
                let verb = plan.verb(key, &x);
                plan.link(key, &x, verb, "");
            }
        }
        // Other services by the host names in its settings.
        for (other, name) in &services {
            if other != key && body.contains(&format!("//{name}.")) {
                plan.link(key, other, "calls", "HTTP");
            }
        }
        // Services outside, by the names of its settings.
        for line in body.lines() {
            if let Some(k) = line
                .split("name")
                .nth(1)
                .and_then(|s| s.split('"').nth(1))
                .filter(|k| {
                    k.chars()
                        .all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit())
                })
            {
                if let Some((name, icon, logo)) = saas_of(k) {
                    saas.insert(name, (icon, logo));
                    uses.push((key.clone(), name));
                }
            }
        }
    }
    for (name, (icon, logo)) in &saas {
        plan.add(Part {
            key: format!("saas:{name}"),
            name: name.to_string(),
            kind: Kind::External,
            icon: icon.to_string(),
            logo: logo.to_string(),
            tech: "External service".into(),
            ..Default::default()
        });
    }
    for (from, name) in uses {
        let to = format!("saas:{name}");
        let verb = plan.verb(&from, &to);
        plan.link(&from, &to, verb, "HTTPS");
    }

    // Load balancers to the services behind them, through listeners and target groups.
    let mut tg_lb: HashMap<String, String> = HashMap::new();
    let listener_lb: HashMap<String, String> = of_type("aws_lb_listener")
        .into_iter()
        .filter_map(|l| {
            let refs = l.refs();
            let lb = refs
                .iter()
                .find(|x| x.starts_with("aws_lb.") || x.starts_with("aws_alb."))?
                .clone();
            for tg in refs
                .iter()
                .filter(|x| x.starts_with("aws_lb_target_group."))
            {
                tg_lb.insert(tg.clone(), lb.clone());
            }
            Some((l.key(), lb))
        })
        .collect();
    for rule in of_type("aws_lb_listener_rule") {
        let refs = rule.refs();
        if let Some(lb) = refs
            .iter()
            .find(|x| x.starts_with("aws_lb_listener."))
            .and_then(|l| listener_lb.get(l))
        {
            for tg in refs
                .iter()
                .filter(|x| x.starts_with("aws_lb_target_group."))
            {
                tg_lb.insert(tg.clone(), lb.clone());
            }
        }
    }
    for (svc, _) in &services {
        if let Some(r) = by_key.get(svc.as_str()) {
            for tg in r
                .refs()
                .iter()
                .filter(|x| x.starts_with("aws_lb_target_group."))
            {
                if let Some(lb) = tg_lb.get(tg) {
                    plan.link(lb, svc, "routes to", "HTTPS");
                }
            }
        }
    }
    // Front doors: CloudFront and API Gateway to their origins.
    for ty in [
        "aws_cloudfront_distribution",
        "aws_api_gateway_rest_api",
        "aws_apigatewayv2_api",
    ] {
        for r in of_type(ty) {
            for x in r.refs() {
                if plan.has(&x) {
                    let label = if x.starts_with("aws_s3_bucket.") {
                        "serves files from"
                    } else {
                        "forwards to"
                    };
                    plan.link(&r.key(), &x, label, "HTTPS");
                }
            }
        }
    }
    // Messages: topics fan out to queues, queues start functions, queues give up to
    // their dead-letter queue.
    for s in of_type("aws_sns_topic_subscription") {
        let refs = s.refs();
        if let Some(topic) = refs.iter().find(|x| x.starts_with("aws_sns_topic.")) {
            let to: Vec<&String> = refs.iter().filter(|x| *x != topic && plan.has(x)).collect();
            for to in to {
                plan.link(topic, to, "delivers to", "SNS");
            }
        }
    }
    for m in of_type("aws_lambda_event_source_mapping") {
        let refs = m.refs();
        let src = refs.iter().find(|x| {
            x.starts_with("aws_sqs_queue.")
                || x.starts_with("aws_kinesis_stream.")
                || x.starts_with("aws_dynamodb_table.")
        });
        let f = refs.iter().find(|x| x.starts_with("aws_lambda_function."));
        if let (Some(src), Some(f)) = (src, f) {
            plan.link(src, f, "triggers", "");
        }
    }
    for q in of_type("aws_sqs_queue") {
        for x in q.refs() {
            if x.starts_with("aws_sqs_queue.") && plan.has(&x) {
                plan.link(&q.key(), &x, "dead letters", "");
            }
        }
    }
    // Workers that read a queue fed by a topic consume it, which reads better than
    // "publishes to".
    let fed: BTreeSet<String> = plan
        .links
        .iter()
        .filter(|l| l.from.starts_with("aws_sns_topic."))
        .map(|l| l.to.clone())
        .collect();
    for l in &mut plan.links {
        if fed.contains(&l.to) && l.label == "publishes to" {
            std::mem::swap(&mut l.from, &mut l.to);
            l.label = "consumed by".into();
        }
    }
    // People come in through the front door.
    let door = plan
        .parts
        .iter()
        .find(|p| p.key.starts_with("aws_cloudfront_distribution."))
        .or_else(|| plan.parts.iter().find(|p| p.key.starts_with("aws_api")))
        .or_else(|| {
            plan.parts
                .iter()
                .find(|p| p.key.starts_with("aws_lb.") || p.key.starts_with("aws_alb."))
        })
        .map(|p| p.key.clone());
    if let Some(d) = door {
        plan.add(Part {
            key: "people".into(),
            name: "Users".into(),
            kind: Kind::Users,
            icon: "users".into(),
            ..Default::default()
        });
        plan.link("people", &d, "uses", "HTTPS");
    }
    // Groups with nothing in them go.
    let used: BTreeSet<String> = plan.parts.iter().filter_map(|p| p.group.clone()).collect();
    let mut keep: BTreeSet<String> = BTreeSet::new();
    for g in &used {
        let mut cur = Some(g.clone());
        while let Some(k) = cur {
            cur = plan
                .groups
                .iter()
                .find(|x| x.key == k)
                .and_then(|x| x.parent.clone());
            keep.insert(k);
        }
    }
    plan.groups.retain(|g| keep.contains(&g.key));
    if let Some(part) = plan.part_mut("people") {
        part.group = None;
    }
    plan
}

// ---- ECS task definitions and Copilot

/// ECS task definitions kept as JSON (for `aws ecs register-task-definition`, ecspresso
/// or a deploy action): each family a task, added to `plan` unless it's there already.
fn read_task_definition(text: &str, plan: &mut Plan) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else {
        return;
    };
    let v = v.get("taskDefinition").unwrap_or(&v);
    let Some(defs) = v.get("containerDefinitions").and_then(|d| d.as_array()) else {
        return;
    };
    let family = v
        .get("family")
        .and_then(|f| f.as_str())
        .or_else(|| defs.first()?.get("name")?.as_str())
        .unwrap_or("task")
        .to_string();
    if plan.parts.iter().any(|p| p.name == family) {
        return;
    }
    let command = defs
        .first()
        .and_then(|d| d.get("command"))
        .map(|c| c.to_string())
        .unwrap_or_default();
    let icon = icon_of_service(&family, &command);
    let job = matches!(icon, "clock" | "play" | "cog");
    let group = plan
        .groups
        .iter()
        .find(|g| g.key.ends_with("/jobs") && job)
        .or_else(|| plan.groups.iter().find(|g| g.icon == "boxes"))
        .map(|g| g.key.clone());
    plan.add(Part {
        key: format!("task:{family}"),
        name: family,
        kind: Kind::Container,
        icon: if icon == "server" { "container" } else { icon }.into(),
        tech: "ECS task".into(),
        description: if job {
            "Runs on demand".into()
        } else {
            String::new()
        },
        group,
        ..Default::default()
    });
}

/// AWS Copilot manifests (`copilot/<name>/manifest.yml`): web, backend and worker
/// services and scheduled jobs in one ECS cluster, the topics they publish and subscribe
/// to, and the schedules that start the jobs.
fn read_copilot(manifests: &[(String, String)], app: &str) -> Plan {
    let mut plan = Plan {
        name: "AWS Copilot".into(),
        ..Default::default()
    };
    plan.group("aws", "AWS", "cloud", "Amazon Web Services", None);
    plan.group(
        "cluster",
        &format!("ECS cluster {app}"),
        "boxes",
        "",
        Some("aws"),
    );
    let mut has_lb = false;
    let mut subs: Vec<(String, String, String)> = Vec::new();
    for (file, text) in manifests {
        plan.files.push(file.clone());
        let doc = parse_yaml(text);
        let Some(name) = doc.get("name").and_then(Yaml::text).map(String::from) else {
            continue;
        };
        let ty = doc
            .get("type")
            .and_then(Yaml::text)
            .unwrap_or("")
            .to_string();
        let (icon, tech, group) = match ty.as_str() {
            "Load Balanced Web Service" => {
                has_lb = true;
                (
                    "app-window",
                    "ECS service behind a load balancer",
                    "cluster",
                )
            }
            "Backend Service" => ("container", "ECS service", "cluster"),
            "Worker Service" => ("cog", "ECS worker service", "cluster"),
            "Scheduled Job" => ("clock", "ECS scheduled task", "cluster/jobs"),
            "Request-Driven Web Service" => ("app-window", "App Runner service", "aws"),
            "Static Site" => ("globe", "Static site", "aws"),
            _ => continue,
        };
        if group == "cluster/jobs" {
            plan.group(
                "cluster/jobs",
                "Scheduled jobs",
                "clock",
                "",
                Some("cluster"),
            );
        }
        let dir = Path::new(file)
            .parent()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        plan.add(Part {
            key: name.clone(),
            name: name.clone(),
            kind: Kind::Container,
            icon: icon.into(),
            tech: tech.into(),
            group: Some(group.into()),
            code: if dir.is_empty() {
                Vec::new()
            } else {
                vec![format!("{dir}/**")]
            },
            ..Default::default()
        });
        if let Some(when) = doc
            .get("on")
            .and_then(|o| o.get("schedule"))
            .and_then(Yaml::text)
        {
            plan.add(Part {
                key: "eventbridge".into(),
                name: "EventBridge".into(),
                kind: Kind::Container,
                icon: "calendar-range".into(),
                tech: "EventBridge Scheduler".into(),
                group: Some("aws".into()),
                ..Default::default()
            });
            let when = match when {
                "@daily" => "daily".to_string(),
                "@hourly" => "hourly".to_string(),
                "@weekly" => "weekly".to_string(),
                "@monthly" => "monthly".to_string(),
                w if w.starts_with("@every ") => format!("every {}", &w[7..]),
                w => humanize_schedule(&format!("cron({w})")),
            };
            plan.link("eventbridge", &name, &when, "");
        }
        if let Some(Yaml::List(items)) = doc.get("publish").and_then(|p| p.get("topics")) {
            for t in items {
                if let Some(tn) = t.get("name").and_then(Yaml::text) {
                    let key = format!("topic:{name}-{tn}");
                    plan.add(Part {
                        key: key.clone(),
                        name: tn.into(),
                        kind: Kind::Queue,
                        icon: "radio".into(),
                        tech: "SNS topic".into(),
                        group: Some("aws".into()),
                        ..Default::default()
                    });
                    plan.link(&name, &key, "publishes to", "SNS");
                }
            }
        }
        if let Some(Yaml::List(items)) = doc.get("subscribe").and_then(|s| s.get("topics")) {
            for t in items {
                if let (Some(tn), Some(from)) = (
                    t.get("name").and_then(Yaml::text),
                    t.get("service").and_then(Yaml::text),
                ) {
                    subs.push((format!("topic:{from}-{tn}"), name.clone(), tn.to_string()));
                }
            }
        }
    }
    for (topic, worker, _) in subs {
        plan.link(&topic, &worker, "delivers to", "SQS");
    }
    if has_lb {
        plan.add(Part {
            key: "lb".into(),
            name: "Load balancer".into(),
            kind: Kind::Container,
            icon: "split".into(),
            tech: "Application Load Balancer".into(),
            group: Some("aws".into()),
            ..Default::default()
        });
        plan.add(Part {
            key: "people".into(),
            name: "Users".into(),
            kind: Kind::Users,
            icon: "users".into(),
            ..Default::default()
        });
        plan.link("people", "lb", "uses", "HTTPS");
        let webs: Vec<String> = plan
            .parts
            .iter()
            .filter(|p| p.tech == "ECS service behind a load balancer")
            .map(|p| p.key.clone())
            .collect();
        for w in webs {
            plan.link("lb", &w, "routes to", "HTTPS");
        }
    }
    plan
}

/// Read what a project runs from its deployment files. `files` are paths relative to
/// `root`, as the code scan walks them.
pub(crate) fn read(root: &Path, files: &[PathBuf], project: &str) -> Infra {
    let mut infra = Infra::default();
    let name_of = |f: &PathBuf| {
        f.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    let rel = |f: &PathBuf| f.to_string_lossy().replace('\\', "/");
    let read = |f: &PathBuf| std::fs::read_to_string(root.join(f)).unwrap_or_default();
    // Compose: the file nearest the top; others are usually variants of it.
    let mut composes: Vec<&PathBuf> = files.iter().filter(|f| is_compose(&name_of(f))).collect();
    composes.sort_by_key(|f| (f.components().count(), name_of(f).len()));
    if let Some(f) = composes.first() {
        let dir_rel = f
            .parent()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        let dir = root.join(&dir_rel);
        let mut plan = read_compose(&read(f), &dir, &dir_rel, project);
        if !plan.parts.is_empty() {
            plan.files.push(rel(f));
            infra.plans.push(plan);
        }
    }
    // Terraform, then task definitions in JSON, then Copilot.
    let tf: Vec<(String, String)> = files
        .iter()
        .filter(|f| name_of(f).ends_with(".tf"))
        .map(|f| (rel(f), read(f)))
        .collect();
    let mut cloud = read_terraform(&tf);
    for f in files.iter().filter(|f| name_of(f).ends_with(".json")) {
        let path = rel(f).to_lowercase();
        let named = path.contains("task-def")
            || path.contains("taskdef")
            || path.contains("task_def")
            || path.contains("ecs");
        if !named {
            continue;
        }
        let text = read(f);
        if text.contains("containerDefinitions") {
            if cloud.groups.is_empty() {
                cloud.name = "AWS".into();
                cloud.group("aws", "AWS", "cloud", "Amazon Web Services", None);
                cloud.group("ecs", "ECS", "boxes", "", Some("aws"));
            }
            read_task_definition(&text, &mut cloud);
            cloud.files.push(rel(f));
        }
    }
    if !cloud.parts.is_empty() {
        infra.plans.push(cloud);
    }
    let copilot: Vec<(String, String)> = files
        .iter()
        .filter(|f| {
            name_of(f) == "manifest.yml"
                && f.components()
                    .any(|c| c.as_os_str().to_string_lossy() == "copilot")
                && !f
                    .components()
                    .any(|c| c.as_os_str().to_string_lossy() == "environments")
        })
        .map(|f| (rel(f), read(f)))
        .collect();
    if !copilot.is_empty() {
        let app = files
            .iter()
            .find(|f| rel(f).ends_with("copilot/.workspace"))
            .and_then(|f| {
                read(f)
                    .lines()
                    .find_map(|l| l.strip_prefix("application:").map(|a| a.trim().to_string()))
            })
            .unwrap_or_else(|| project.to_string());
        let plan = read_copilot(&copilot, &app);
        if !plan.parts.is_empty() {
            infra.plans.push(plan);
        }
    }
    infra
}

/// Find each logo a diagram asks for (`logo:PostgreSQL`) in the icon library and copy it
/// into the diagram's `icons/` folder; a logo that can't be had leaves no picture. Blocks
/// on the library and downloads, so call it off the UI thread.
pub(crate) fn attach_logos(doc: &mut super::model::Doc, root: &Path) {
    use super::icons;
    let wanted: BTreeSet<String> = doc
        .elements
        .iter()
        .filter_map(|e| e.image.strip_prefix(LOGO_WANTED).map(String::from))
        .collect();
    let mut found: HashMap<String, String> = HashMap::new();
    if !wanted.is_empty() {
        if let Ok(lib) = icons::load() {
            for name in &wanted {
                let lower = name.to_lowercase();
                let hits = lib.search(name, 8);
                // An exact title first; a near one only if it starts the same way.
                let pick = hits
                    .iter()
                    .find(|i| i.title.to_lowercase() == lower)
                    .or_else(|| {
                        hits.iter()
                            .find(|i| i.title.to_lowercase().starts_with(&lower))
                    })
                    .copied();
                let Some(icon) = pick else { continue };
                if let Ok(file) = icons::fetch(icon) {
                    if let Ok(path) = icons::add_to_project(root, &file, name) {
                        found.insert(name.clone(), path);
                    }
                }
            }
        }
    }
    for e in &mut doc.elements {
        if let Some(name) = e.image.strip_prefix(LOGO_WANTED) {
            e.image = found.get(name).cloned().unwrap_or_default();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    pub(crate) fn fixture() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/archdraw/fixtures/shipyard")
    }

    #[test]
    fn yaml_reads_what_compose_files_use() {
        let y = parse_yaml(
            "name: demo # the project\n\
             x-app: &app\n  restart: always\n\
             services:\n\
             \x20 api:\n\
             \x20   <<: *app\n\
             \x20   build:\n\
             \x20     context: ./api\n\
             \x20   environment:\n\
             \x20     - DATABASE_URL=postgres://u:p@db:5432/x\n\
             \x20     - \"A=b\"\n\
             \x20   depends_on: [db, \"cache\"]\n\
             \x20   command: |\n\
             \x20     npm run\n\
             \x20     start\n\
             \x20 db:\n\
             \x20   image: postgres:16\n\
             \x20   ports:\n\
             \x20   - \"5432:5432\"\n\
             \x20   healthcheck: {test: [CMD, pg_isready], interval: 5s}\n",
        );
        assert_eq!(y.get("name").and_then(Yaml::text), Some("demo"));
        let api = y.get("services").and_then(|s| s.get("api")).unwrap();
        assert_eq!(
            api.get("build")
                .and_then(|b| b.get("context"))
                .and_then(Yaml::text),
            Some("./api")
        );
        assert_eq!(
            api.get("environment").unwrap().pairs()[0],
            (
                "DATABASE_URL".to_string(),
                "postgres://u:p@db:5432/x".to_string()
            )
        );
        assert_eq!(api.get("depends_on").unwrap().names(), vec!["db", "cache"]);
        assert_eq!(
            api.get("command").and_then(Yaml::text),
            Some("npm run start")
        );
        let db = y.get("services").and_then(|s| s.get("db")).unwrap();
        assert_eq!(db.get("ports").unwrap().names(), vec!["5432:5432"]);
        assert!(db
            .get("healthcheck")
            .and_then(|h| h.get("interval"))
            .is_some());
    }

    #[test]
    fn hosts_are_found_in_settings_but_not_schemes() {
        assert!(mentions_host("postgres://u:p@db:5432/x", "db"));
        assert!(mentions_host("http://orders.shipyard.local:9000", "orders"));
        assert!(mentions_host("mailpit", "mailpit"));
        assert!(!mentions_host("redis://cache:6379", "redis"));
        assert!(!mentions_host("http://reorders:80", "orders"));
    }

    #[test]
    fn schedules_read_as_people_say_them() {
        assert_eq!(humanize_schedule("cron(0 2 * * ? *)"), "daily at 02:00");
        assert_eq!(humanize_schedule("cron(0 3 ? * SUN *)"), "Sundays at 03:00");
        assert_eq!(
            humanize_schedule("cron(0 6 1 * ? *)"),
            "monthly on day 1 at 06:00"
        );
        assert_eq!(humanize_schedule("rate(15 minutes)"), "every 15 minutes");
        assert_eq!(
            humanize_schedule("cron(*/5 * * * ? *)"),
            "cron(*/5 * * * ? *)"
        );
    }

    #[test]
    fn images_say_what_they_run() {
        let l = look_of_image("docker.elastic.co/elasticsearch/elasticsearch:8.15.0").unwrap();
        assert_eq!((l.kind, l.icon), (Kind::Database, "search"));
        assert_eq!(look_of_image("postgres:16").unwrap().logo, "PostgreSQL");
        assert_eq!(
            look_of_image("rabbitmq:3-management").unwrap().kind,
            Kind::Queue
        );
        assert!(look_of_image("ghcr.io/acme/orders:1.2").is_none());
    }

    fn part<'a>(plan: &'a Plan, name: &str) -> &'a Part {
        plan.parts
            .iter()
            .find(|p| p.name == name)
            .unwrap_or_else(|| panic!("no part {name}"))
    }

    fn linked(plan: &Plan, a: &str, b: &str) -> Option<String> {
        let (ka, kb) = (&part(plan, a).key, &part(plan, b).key);
        plan.links
            .iter()
            .find(|l| &l.from == ka && &l.to == kb)
            .map(|l| l.label.clone())
    }

    #[test]
    fn the_compose_file_is_read_with_shapes_icons_groups_and_links() {
        let root = fixture();
        let text = std::fs::read_to_string(root.join("docker-compose.yml")).unwrap();
        let plan = read_compose(&text, &root, "", "shipyard");
        assert_eq!(
            plan.parts
                .iter()
                .filter(|p| p.key != "people" && !p.key.starts_with("saas:"))
                .count(),
            18
        );
        let pg = part(&plan, "postgres");
        assert_eq!((pg.kind, pg.logo.as_str()), (Kind::Database, "PostgreSQL"));
        assert_eq!(part(&plan, "rabbitmq").kind, Kind::Queue);
        assert_eq!(part(&plan, "redis").icon, "gauge");
        assert_eq!(part(&plan, "proxy").icon, "split");
        let web = part(&plan, "web");
        assert_eq!(
            (web.logo.as_str(), web.icon.as_str()),
            ("Next.js", "app-window")
        );
        assert_eq!(part(&plan, "orders").logo, "Go");
        assert_eq!(part(&plan, "billing").code, vec!["services/billing/**"]);
        assert_eq!(part(&plan, "nightly-report").icon, "clock");
        assert_eq!(part(&plan, "ingest-worker").icon, "cog");
        // Networks and the jobs profile are boundaries.
        assert_eq!(part(&plan, "postgres").group.as_deref(), Some("net:data"));
        assert_eq!(
            part(&plan, "invoice-run").group.as_deref(),
            Some("net:backend/profile:jobs")
        );
        assert!(plan.groups.iter().any(|g| g.name == "Jobs (on demand)"));
        // Links from settings, dependencies and outside services.
        assert_eq!(
            linked(&plan, "api", "postgres").as_deref(),
            Some("reads and writes")
        );
        assert_eq!(linked(&plan, "api", "redis").as_deref(), Some("caches in"));
        assert_eq!(
            linked(&plan, "ingest-worker", "rabbitmq").as_deref(),
            Some("consumes")
        );
        assert_eq!(linked(&plan, "proxy", "api").as_deref(), Some("routes to"));
        assert!(linked(&plan, "orders", "billing").is_some());
        assert!(linked(&plan, "billing", "Stripe").is_some());
        assert_eq!(
            linked(&plan, "notifications", "mailpit").as_deref(),
            Some("sends email")
        );
        assert_eq!(linked(&plan, "Users", "proxy").as_deref(), Some("uses"));
        assert!(linked(&plan, "Users", "rabbitmq").is_none());
    }

    #[test]
    fn terraform_gives_ecs_services_jobs_and_what_they_use() {
        let dir = fixture().join("infra/terraform");
        let mut files: Vec<(String, String)> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| {
                (
                    e.file_name().to_string_lossy().into_owned(),
                    std::fs::read_to_string(e.path()).unwrap(),
                )
            })
            .collect();
        files.sort();
        let plan = read_terraform(&files);
        let api = part(&plan, "api");
        assert_eq!(api.tech, "ECS Fargate service");
        assert_eq!(api.group.as_deref(), Some("aws_ecs_cluster.main"));
        let db = part(&plan, "shipyard-prod");
        assert_eq!((db.kind, db.logo.as_str()), (Kind::Database, "PostgreSQL"));
        assert_eq!(db.group.as_deref(), Some("vpc"));
        assert_eq!(part(&plan, "shipyard-assets").group.as_deref(), Some("aws"));
        // Jobs: scheduled ones in their own boundary, started by EventBridge.
        let job = part(&plan, "nightly-report");
        assert_eq!(job.icon, "clock");
        assert_eq!(job.group.as_deref(), Some("aws_ecs_cluster.main/jobs"));
        assert_eq!(
            linked(&plan, "EventBridge", "nightly-report").as_deref(),
            Some("daily at 02:00")
        );
        assert_eq!(
            linked(&plan, "EventBridge", "carrier-sync").as_deref(),
            Some("every 15 minutes")
        );
        assert_eq!(
            linked(&plan, "data-pipeline", "data-export").as_deref(),
            Some("runs")
        );
        assert!(linked(&plan, "EventBridge", "data-pipeline").is_some());
        // What runs uses what.
        assert_eq!(
            linked(&plan, "api", "shipyard-prod").as_deref(),
            Some("reads and writes")
        );
        assert_eq!(
            linked(&plan, "api", "shipyard-cache").as_deref(),
            Some("caches in")
        );
        assert_eq!(linked(&plan, "api", "orders").as_deref(), Some("calls"));
        assert_eq!(
            linked(&plan, "orders", "order-events").as_deref(),
            Some("publishes to")
        );
        assert!(linked(&plan, "billing", "Stripe").is_some());
        assert!(linked(&plan, "invoice-run", "billing").is_some());
        // The front door and the load balancer's services.
        assert_eq!(linked(&plan, "public", "api").as_deref(), Some("routes to"));
        assert_eq!(linked(&plan, "public", "web").as_deref(), Some("routes to"));
        assert!(linked(&plan, "web", "public").is_none());
        assert_eq!(linked(&plan, "Users", "web").as_deref(), None);
        assert!(plan
            .links
            .iter()
            .any(|l| l.from == "people" && l.to.starts_with("aws_cloudfront_distribution.")));
        // Messages.
        assert_eq!(
            linked(&plan, "order-events", "shipments").as_deref(),
            Some("delivers to")
        );
        assert_eq!(
            linked(&plan, "shipments", "shipyard-label-renderer").as_deref(),
            Some("triggers")
        );
        assert_eq!(
            linked(&plan, "shipments", "shipments-dlq").as_deref(),
            Some("dead letters")
        );
        assert_eq!(
            linked(&plan, "ingest-worker", "shipments").as_deref(),
            Some("consumes")
        );
        assert!(plan
            .links
            .iter()
            .any(|l| l.from == "aws_sqs_queue.notifications"
                && l.to == "aws_ecs_service.notifications"
                && l.label == "consumed by"));
        // Plumbing isn't drawn.
        assert!(!plan.parts.iter().any(|p| p.key.contains("aws_iam")
            || p.key.contains("aws_subnet")
            || p.key.contains("aws_ecr")
            || p.key.contains("secretsmanager")));
    }

    #[test]
    fn copilot_manifests_give_services_jobs_and_topics() {
        let manifests = vec![
            (
                "copilot/api/manifest.yml".to_string(),
                "name: api\ntype: Load Balanced Web Service\npublish:\n  topics:\n    - name: orders\n"
                    .to_string(),
            ),
            (
                "copilot/mailer/manifest.yml".to_string(),
                "name: mailer\ntype: Worker Service\nsubscribe:\n  topics:\n    - name: orders\n      service: api\n"
                    .to_string(),
            ),
            (
                "copilot/report/manifest.yml".to_string(),
                "name: report\ntype: Scheduled Job\non:\n  schedule: \"@daily\"\n".to_string(),
            ),
        ];
        let plan = read_copilot(&manifests, "shop");
        assert_eq!(
            linked(&plan, "api", "orders").as_deref(),
            Some("publishes to")
        );
        assert_eq!(
            linked(&plan, "orders", "mailer").as_deref(),
            Some("delivers to")
        );
        assert_eq!(
            linked(&plan, "EventBridge", "report").as_deref(),
            Some("daily")
        );
        assert_eq!(
            linked(&plan, "Load balancer", "api").as_deref(),
            Some("routes to")
        );
        assert_eq!(part(&plan, "report").group.as_deref(), Some("cluster/jobs"));
    }

    #[test]
    fn the_whole_project_is_read() {
        let root = fixture();
        let (files, _) = crate::architecture::walk(&root);
        let infra = read(&root, &files, "shipyard");
        let names: Vec<&str> = infra.plans.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["Docker Compose", "AWS"]);
        let aws = &infra.plans[1];
        // The task definition kept as JSON joins the ECS cluster as an on-demand job.
        let migrate = part(aws, "shipyard-db-migrate");
        assert_eq!(migrate.icon, "play");
        assert_eq!(migrate.group.as_deref(), Some("aws_ecs_cluster.main/jobs"));
    }
}
