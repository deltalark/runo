//! What part a shape plays in a system, read from its kind, icon, name and technology:
//! the clients people use, the edge that takes their requests, the services and the jobs
//! behind it, the messaging between them, the data they keep, and the systems outside.
//! Beautify uses it to put related parts in one boundary and to stack the tiers in the
//! order a request flows through them.

use super::model::{Element, Kind};

/// The tag on boundaries Beautify made, so running it again regroups instead of nesting.
pub(crate) const AUTO_TAG: &str = "beautify-group";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum Role {
    /// What people use: a person, a browser, a phone, a web or mobile app.
    Client,
    /// What takes requests first: a CDN, a load balancer, a gateway, a proxy.
    Edge,
    /// Services that answer requests.
    Service,
    /// Workers, scheduled jobs, batch tasks and the schedulers that start them.
    Job,
    /// Queues, topics and event buses.
    Messaging,
    /// Databases, caches, buckets and search indexes.
    Data,
    /// Systems run by someone else.
    External,
    /// Logs, metrics and traces.
    Ops,
    /// Notes, text and drawing aids: left where they are.
    Other,
}

impl Role {
    /// The name of the boundary that holds parts of this role.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Role::Client => "Clients",
            Role::Edge => "Edge",
            Role::Service => "Services",
            Role::Job => "Jobs and workers",
            Role::Messaging => "Messaging",
            Role::Data => "Data",
            Role::External => "External services",
            Role::Ops => "Observability",
            Role::Other => "Other",
        }
    }

    pub(crate) fn icon(self) -> &'static str {
        match self {
            Role::Client => "users",
            Role::Edge => "shield",
            Role::Service => "server",
            Role::Job => "clock",
            Role::Messaging => "radio",
            Role::Data => "database",
            Role::External => "globe",
            Role::Ops => "activity",
            Role::Other => "frame",
        }
    }

    /// The boundary's colours: a light wash and a stronger border, per tier.
    pub(crate) fn colors(self) -> (&'static str, &'static str) {
        match self {
            Role::Client => ("#f5f3ff", "#8b5cf6"),
            Role::Edge => ("#fff7ed", "#f97316"),
            Role::Service => ("#eff6ff", "#3b82f6"),
            Role::Job => ("#fefce8", "#ca8a04"),
            Role::Messaging => ("#fdf4ff", "#c026d3"),
            Role::Data => ("#ecfdf5", "#10b981"),
            Role::External => ("#f8fafc", "#64748b"),
            Role::Ops => ("#f0fdfa", "#0d9488"),
            Role::Other => ("#f8fafc", "#94a3b8"),
        }
    }

    /// Where the tier sits in a request's path: clients first, data and outside systems
    /// last. Jobs sit beside services; observability off to the end.
    pub(crate) fn rank(self) -> usize {
        match self {
            Role::Client => 0,
            Role::Edge => 1,
            Role::Service | Role::Job => 2,
            Role::Messaging => 3,
            Role::Data | Role::External => 4,
            Role::Ops => 5,
            Role::Other => 2,
        }
    }
}

/// Whether `text` holds any of `words` as a whole word (split on anything not a letter
/// or digit), so "api" matches "orders-api" but not "rapid".
fn has_word(text: &str, words: &[&str]) -> bool {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .any(|w| words.contains(&w))
}

/// The part an element plays.
pub(crate) fn role_of(e: &Element) -> Role {
    let text = format!("{} {} {} {}", e.name, e.tech, e.icon, e.tags.join(" ")).to_lowercase();
    let says = |words: &[&str]| has_word(&text, words);
    let edge_words = [
        "gateway",
        "cdn",
        "cloudfront",
        "nginx",
        "traefik",
        "envoy",
        "haproxy",
        "caddy",
        "ingress",
        "proxy",
        "alb",
        "elb",
        "lb",
        "balancer",
        "waf",
        "split",
    ];
    let ops_words = [
        "monitoring",
        "datadog",
        "grafana",
        "prometheus",
        "jaeger",
        "otel",
        "opentelemetry",
        "sentry",
        "cloudwatch",
        "logs",
        "logging",
        "metrics",
        "tracing",
        "activity",
    ];
    let job_words = [
        "worker",
        "workers",
        "job",
        "jobs",
        "cron",
        "scheduled",
        "batch",
        "consumer",
        "celery",
        "sidekiq",
        "nightly",
        "etl",
        "cog",
        "clock",
        "timer",
        "migrate",
        "migrations",
        "scheduler",
        "schedule",
        "calendar",
        "pipeline",
        "workflow",
    ];
    let messaging_words = [
        "queue",
        "kafka",
        "rabbitmq",
        "sqs",
        "sns",
        "topic",
        "eventbridge",
        "nats",
        "pubsub",
        "bus",
        "kinesis",
        "redpanda",
        "radio",
    ];
    let data_words = [
        "database",
        "db",
        "postgres",
        "postgresql",
        "mysql",
        "mariadb",
        "mongodb",
        "mongo",
        "redis",
        "cache",
        "memcached",
        "dynamodb",
        "rds",
        "aurora",
        "s3",
        "bucket",
        "storage",
        "minio",
        "elasticsearch",
        "opensearch",
        "search",
        "warehouse",
        "snowflake",
        "elasticache",
    ];
    let client_words = [
        "webapp",
        "frontend",
        "ui",
        "spa",
        "mobile",
        "ios",
        "android",
        "browser",
        "client",
        "dashboard",
        "admin",
        "smartphone",
        "monitor",
        "react",
        "vue",
        "angular",
        "svelte",
        "swift",
        "kotlin",
        "flutter",
    ];
    match e.kind {
        Kind::Note
        | Kind::Text
        | Kind::Boundary
        | Kind::Callout
        | Kind::Arrow
        | Kind::Triangle
        | Kind::Diamond
        | Kind::Terminator
        | Kind::Parallelogram => return Role::Other,
        Kind::Person | Kind::Actor | Kind::Users | Kind::Browser | Kind::Phone | Kind::Desktop => {
            return Role::Client
        }
        Kind::Queue => return Role::Messaging,
        Kind::Database | Kind::Document | Kind::Folder => return Role::Data,
        Kind::Shield => return Role::Edge,
        _ => {}
    }
    if says(&ops_words) {
        return Role::Ops;
    }
    if e.kind == Kind::External || e.kind == Kind::Cloud {
        return if says(&edge_words) {
            Role::Edge
        } else {
            Role::External
        };
    }
    if says(&edge_words) {
        return Role::Edge;
    }
    if says(&messaging_words) && !says(&job_words) {
        return Role::Messaging;
    }
    if says(&job_words) {
        return Role::Job;
    }
    if says(&data_words) {
        return Role::Data;
    }
    // A front end served as a container is still what people use.
    if says(&client_words) && !says(&["api", "backend", "server", "service", "ecs", "fargate"]) {
        return Role::Client;
    }
    Role::Service
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn el(name: &str, kind: Kind, tech: &str, icon: &str) -> Element {
        Element {
            id: name.to_lowercase(),
            name: name.into(),
            kind,
            tech: tech.into(),
            icon: icon.into(),
            ..Default::default()
        }
    }

    #[test]
    fn roles_come_from_kind_name_and_tech() {
        let cases = [
            (el("Mobile app", Kind::Container, "Swift", ""), Role::Client),
            (el("Web UI", Kind::Container, "React", ""), Role::Client),
            (
                el("Web app", Kind::Container, "Web", "monitor"),
                Role::Client,
            ),
            (el("API gateway", Kind::Container, "Envoy", ""), Role::Edge),
            (
                el("Load balancer", Kind::Container, "ALB", "split"),
                Role::Edge,
            ),
            (el("Orders", Kind::Container, "Java", ""), Role::Service),
            (el("Orders API", Kind::Container, "Go", ""), Role::Service),
            (
                el("Nightly report", Kind::Container, "ECS scheduled task", ""),
                Role::Job,
            ),
            (
                el("Email worker", Kind::Container, "Node", "cog"),
                Role::Job,
            ),
            (el("Event bus", Kind::Queue, "Kafka", ""), Role::Messaging),
            (
                el(
                    "EventBridge",
                    Kind::Container,
                    "EventBridge Scheduler",
                    "calendar-range",
                ),
                Role::Job,
            ),
            (
                el("Events", Kind::Container, "EventBridge", "radio"),
                Role::Messaging,
            ),
            (
                el("web", Kind::Container, "ECS Fargate service", "app-window"),
                Role::Service,
            ),
            (el("Orders DB", Kind::Database, "Postgres", ""), Role::Data),
            (el("Cache", Kind::Container, "Redis", ""), Role::Data),
            (el("Stripe", Kind::External, "Payments", ""), Role::External),
            (el("CDN", Kind::External, "CloudFront", ""), Role::Edge),
            (el("Grafana", Kind::Container, "", ""), Role::Ops),
            (el("A note", Kind::Note, "", ""), Role::Other),
        ];
        for (e, want) in cases {
            assert_eq!(role_of(&e), want, "{}", e.name);
        }
    }

    #[test]
    fn words_match_whole() {
        assert!(has_word("orders-api", &["api"]));
        assert!(!has_word("rapid", &["api"]));
    }
}
