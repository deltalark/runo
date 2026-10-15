//! Getting diagrams in and out: draw.io files both ways, Mermaid, SVG and PNG, and a first
//! diagram made from the project's code.

use super::editor::Editor;
use super::geom;
use super::model::*;
use super::shape::{self, Seg, Theme};
use crate::architecture::ProjectMap;
use std::collections::HashMap;
use std::fmt::Write as _;

fn esc(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

// ---- Mermaid

fn mermaid_id(id: &str) -> String {
    id.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

fn mermaid_label(text: &str) -> String {
    text.replace('"', "'").replace('\n', "<br/>")
}

/// The diagram as a Mermaid flowchart, with boundaries as subgraphs.
pub(crate) fn mermaid(doc: &Doc, view: &View) -> String {
    let mut out = String::from("flowchart TD\n");
    fn node_line(doc: &Doc, id: &str, indent: &str, out: &mut String) {
        let Some(e) = doc.element(id) else { return };
        let mut label = mermaid_label(&e.name);
        if !e.tech.is_empty() {
            label.push_str(&format!("<br/><small>{}</small>", mermaid_label(&e.tech)));
        }
        let (open, close) = match e.kind {
            Kind::Database => ("[(", ")]"),
            Kind::Queue => ("[[", "]]"),
            Kind::Person => ("([", "])"),
            Kind::Ellipse => ("((", "))"),
            Kind::Diamond => ("{", "}"),
            Kind::Cloud | Kind::Rounded | Kind::Container | Kind::Component | Kind::System => {
                ("(", ")")
            }
            Kind::External => ("[/", "/]"),
            _ => ("[", "]"),
        };
        let _ = writeln!(out, "{indent}{}{open}\"{label}\"{close}", mermaid_id(id));
    }
    fn level(doc: &Doc, view: &View, parent: Option<&str>, depth: usize, out: &mut String) {
        let indent = "  ".repeat(depth + 1);
        for n in view.nodes.iter().filter(|n| n.parent.as_deref() == parent) {
            let kind = doc.element(&n.id).map(|e| e.kind).unwrap_or_default();
            if kind == Kind::Boundary {
                let name = doc.element(&n.id).map(|e| e.name.as_str()).unwrap_or("");
                let _ = writeln!(
                    out,
                    "{indent}subgraph {}[\"{}\"]",
                    mermaid_id(&n.id),
                    mermaid_label(name)
                );
                level(doc, view, Some(&n.id), depth + 1, out);
                let _ = writeln!(out, "{indent}end");
            } else {
                node_line(doc, &n.id, &indent, out);
            }
        }
    }
    level(doc, view, None, 0, &mut out);
    for e in &view.edges {
        let Some(r) = doc.relation(&e.id) else {
            continue;
        };
        let line = match (e.arrow, e.style.dashed) {
            (Arrow::None, false) => "---",
            (Arrow::None, true) => "-.-",
            (Arrow::Both, false) => "<-->",
            (Arrow::Both, true) => "<-.->",
            (Arrow::End, false) => "-->",
            (Arrow::End, true) => "-.->",
        };
        let label = [r.label.as_str(), r.tech.as_str()]
            .iter()
            .filter(|s| !s.is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join(" · ");
        if label.is_empty() {
            let _ = writeln!(
                out,
                "  {} {line} {}",
                mermaid_id(&r.from),
                mermaid_id(&r.to)
            );
        } else {
            let _ = writeln!(
                out,
                "  {} {line}|\"{}\"| {}",
                mermaid_id(&r.from),
                mermaid_label(&label),
                mermaid_id(&r.to)
            );
        }
    }
    out
}

// ---- draw.io

fn drawio_style(kind: Kind, n: &Node, edge_like: bool) -> String {
    let mut s = String::from(match kind {
        Kind::Ellipse => "ellipse;whiteSpace=wrap;html=1;",
        Kind::Diamond => "rhombus;whiteSpace=wrap;html=1;",
        Kind::Database => "shape=cylinder3;whiteSpace=wrap;html=1;boundedLbl=1;size=12;",
        Kind::Queue => "shape=cylinder3;whiteSpace=wrap;html=1;boundedLbl=1;direction=south;size=12;",
        Kind::Person => "shape=umlActor;verticalLabelPosition=bottom;verticalAlign=top;html=1;",
        Kind::Cloud => "ellipse;shape=cloud;whiteSpace=wrap;html=1;",
        Kind::Note => "shape=note;whiteSpace=wrap;html=1;size=14;",
        Kind::Text => "text;html=1;align=center;verticalAlign=middle;",
        Kind::Boundary => {
            "rounded=1;whiteSpace=wrap;html=1;container=1;collapsible=0;dashed=1;verticalAlign=top;align=left;spacingLeft=10;arcSize=4;"
        }
        Kind::Box => "rounded=0;whiteSpace=wrap;html=1;",
        _ => "rounded=1;whiteSpace=wrap;html=1;arcSize=8;",
    });
    let _ = edge_like;
    let paint = shape::paint(kind, &n.style, &Theme::LIGHT);
    match paint.fill {
        Some(f) => {
            let _ = write!(s, "fillColor={};", hex(f));
        }
        None => s.push_str("fillColor=none;"),
    }
    match paint.stroke {
        Some(c) => {
            let _ = write!(s, "strokeColor={};", hex(c));
        }
        None => s.push_str("strokeColor=none;"),
    }
    let _ = write!(
        s,
        "fontColor={};fontSize={};",
        hex(paint.text),
        paint.font.round()
    );
    if n.style.dashed {
        s.push_str("dashed=1;");
    }
    if let Some(o) = n.style.opacity {
        let _ = write!(s, "opacity={};", (o * 100.).round());
    }
    if n.locked {
        s.push_str("movable=0;resizable=0;");
    }
    s
}

/// Every diagram as one draw.io file, a page per diagram.
pub(crate) fn drawio(doc: &Doc) -> String {
    let mut out = String::from("<mxfile host=\"Runo\" type=\"device\">\n");
    for v in &doc.views {
        let _ = writeln!(
            out,
            "  <diagram id=\"{}\" name=\"{}\">",
            esc(&v.id),
            esc(&v.name)
        );
        out.push_str("    <mxGraphModel grid=\"1\" gridSize=\"10\" guides=\"1\" connect=\"1\" arrows=\"1\" page=\"0\">\n      <root>\n        <mxCell id=\"0\" />\n        <mxCell id=\"1\" parent=\"0\" />\n");
        for n in &v.nodes {
            let Some(e) = doc.element(&n.id) else {
                continue;
            };
            let (px, py, parent) = match n.parent.as_ref().and_then(|p| v.node(p)) {
                Some(p) => (p.x, p.y, format!("n-{}", p.id)),
                None => (0., 0., "1".to_string()),
            };
            let mut value = esc(&e.name);
            if let Some(c4) = e.kind.c4() {
                let tech = if e.tech.is_empty() {
                    c4.to_string()
                } else {
                    format!("{c4}: {}", e.tech)
                };
                value.push_str(&format!(
                    "&lt;br&gt;&lt;font style=&quot;font-size:11px&quot;&gt;[{}]&lt;/font&gt;",
                    esc(&tech)
                ));
            }
            let _ = writeln!(
                out,
                "        <mxCell id=\"n-{}\" value=\"{value}\" style=\"{}\" vertex=\"1\" parent=\"{parent}\">\n          <mxGeometry x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" as=\"geometry\" />\n        </mxCell>",
                esc(&n.id),
                esc(&drawio_style(e.kind, n, false)),
                n.x - px,
                n.y - py,
                n.w,
                n.h
            );
        }
        for e in &v.edges {
            let Some(r) = doc.relation(&e.id) else {
                continue;
            };
            let mut style = String::from(match e.route {
                Route::Straight => "endArrow=block;html=1;",
                Route::Elbow => "edgeStyle=orthogonalEdgeStyle;rounded=1;endArrow=block;html=1;",
                Route::Curved => "curved=1;endArrow=block;html=1;edgeStyle=orthogonalEdgeStyle;",
            });
            match e.arrow {
                Arrow::Both => style.push_str("startArrow=block;"),
                Arrow::None => style = style.replace("endArrow=block;", "endArrow=none;"),
                Arrow::End => {}
            }
            if e.style.dashed {
                style.push_str("dashed=1;");
            }
            if let Some(c) = e.style.stroke.as_deref().and_then(parse_hex) {
                let _ = write!(style, "strokeColor={};", hex(c));
            }
            let label = [r.label.as_str(), r.tech.as_str()]
                .iter()
                .filter(|s| !s.is_empty())
                .cloned()
                .collect::<Vec<_>>()
                .join(" · ");
            let _ = writeln!(
                out,
                "        <mxCell id=\"e-{}\" value=\"{}\" style=\"{}\" edge=\"1\" parent=\"1\" source=\"n-{}\" target=\"n-{}\">\n          <mxGeometry relative=\"1\" as=\"geometry\" />\n        </mxCell>",
                esc(&e.id),
                esc(&label),
                esc(&style),
                esc(&r.from),
                esc(&r.to)
            );
        }
        out.push_str("      </root>\n    </mxGraphModel>\n  </diagram>\n");
    }
    out.push_str("</mxfile>\n");
    out
}

fn style_map(style: &str) -> HashMap<String, String> {
    style
        .split(';')
        .filter(|s| !s.is_empty())
        .map(|kv| match kv.split_once('=') {
            Some((k, v)) => (k.to_string(), v.to_string()),
            None => (kv.to_string(), "1".to_string()),
        })
        .collect()
}

/// Plain text from a draw.io label, which may be HTML.
fn label_text(value: &str) -> String {
    let with_breaks = value
        .replace("<br>", "\n")
        .replace("<br/>", "\n")
        .replace("<br />", "\n")
        .replace("</div>", "\n")
        .replace("</p>", "\n");
    let mut out = String::new();
    let mut in_tag = false;
    for c in with_breaks.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn kind_from_style(s: &HashMap<String, String>) -> Kind {
    let shape = s.get("shape").map(String::as_str).unwrap_or("");
    let has = |k: &str| s.contains_key(k);
    if s.get("container").is_some_and(|v| v == "1") || has("swimlane") || has("group") {
        return Kind::Boundary;
    }
    if has("text") {
        return Kind::Text;
    }
    match shape {
        "cylinder" | "cylinder3" | "datastore" | "mxgraph.flowchart.database" => {
            if s.get("direction")
                .is_some_and(|d| d == "south" || d == "north")
            {
                Kind::Queue
            } else {
                Kind::Database
            }
        }
        "umlActor" | "actor" => Kind::Person,
        "cloud" => Kind::Cloud,
        "note" | "card" => Kind::Note,
        "process" => Kind::Box,
        _ if shape.contains("person") => Kind::Person,
        _ if has("ellipse") => Kind::Ellipse,
        _ if has("rhombus") => Kind::Diamond,
        _ if s.get("rounded").is_some_and(|v| v == "1") => Kind::Rounded,
        _ => Kind::Box,
    }
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&text[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A compressed draw.io page: base64 of raw deflate of URL-encoded XML.
fn inflate_page(text: &str) -> Option<String> {
    use base64::Engine;
    use std::io::Read;
    let raw = base64::engine::general_purpose::STANDARD
        .decode(text.trim())
        .ok()?;
    let mut xml = String::new();
    flate2::read::DeflateDecoder::new(&raw[..])
        .read_to_string(&mut xml)
        .ok()?;
    Some(percent_decode(&xml))
}

/// Read a draw.io file into a model, one diagram per page.
pub(crate) fn import_drawio(text: &str) -> Result<Doc, String> {
    let xml = roxmltree::Document::parse(text).map_err(|e| format!("Not a draw.io file: {e}"))?;
    let mut doc = Doc::default();
    let mut pages: Vec<(String, String)> = Vec::new();
    for d in xml.descendants().filter(|n| n.has_tag_name("diagram")) {
        let name = d.attribute("name").unwrap_or("Page").to_string();
        if d.children().any(|c| c.has_tag_name("mxGraphModel")) {
            let range = d.range();
            let inner = &text[range];
            pages.push((name, inner.to_string()));
        } else if let Some(t) = d.text().filter(|t| !t.trim().is_empty()) {
            pages.push((
                name,
                inflate_page(t).ok_or("A page of the file couldn't be decompressed.")?,
            ));
        }
    }
    if pages.is_empty() && xml.descendants().any(|n| n.has_tag_name("mxGraphModel")) {
        pages.push(("Page 1".into(), text.to_string()));
    }
    if pages.is_empty() {
        return Err("The file has no diagrams.".into());
    }
    for (name, page) in pages {
        let px = roxmltree::Document::parse(&page).map_err(|e| format!("{name}: {e}"))?;
        let view_id = doc.new_view_id(&name);
        let mut view = View {
            id: view_id,
            name,
            ..Default::default()
        };
        // Cell id → (element id, absolute position) as cells are read in order.
        let mut ids: HashMap<String, String> = HashMap::new();
        let mut abs: HashMap<String, (f32, f32)> = HashMap::new();
        let cells: Vec<roxmltree::Node> = px
            .descendants()
            .filter(|n| n.has_tag_name("mxCell"))
            .collect();
        // Parents may come after children; read vertices until nothing new resolves.
        let mut pending: Vec<roxmltree::Node> = cells
            .iter()
            .copied()
            .filter(|c| c.attribute("vertex") == Some("1"))
            .collect();
        let mut guard = 0;
        while !pending.is_empty() && guard < 64 {
            guard += 1;
            let mut later = Vec::new();
            for c in pending {
                let cell_id = c.attribute("id").unwrap_or_default().to_string();
                let parent = c.attribute("parent").unwrap_or("1");
                let parent_vertex = cells.iter().any(|p| {
                    p.attribute("id") == Some(parent) && p.attribute("vertex") == Some("1")
                });
                if parent_vertex && !abs.contains_key(parent) {
                    later.push(c);
                    continue;
                }
                let (ox, oy) = abs.get(parent).copied().unwrap_or((0., 0.));
                let geo = c.children().find(|g| g.has_tag_name("mxGeometry"));
                let num = |k: &str| {
                    geo.and_then(|g| g.attribute(k))
                        .and_then(|v| v.parse::<f32>().ok())
                        .unwrap_or(0.)
                };
                let (x, y, w, h) = (num("x") + ox, num("y") + oy, num("width"), num("height"));
                let style = style_map(c.attribute("style").unwrap_or(""));
                let kind = kind_from_style(&style);
                let text = label_text(c.attribute("value").unwrap_or(""));
                let mut lines = text.lines();
                let name = lines.next().unwrap_or("").to_string();
                let rest: Vec<&str> = lines.collect();
                // "[Container: Go]" under the name is C4 notation for kind and tech.
                let (tech, description) = match rest.first() {
                    Some(l) if l.starts_with('[') && l.ends_with(']') => (
                        l.trim_matches(['[', ']'])
                            .split_once(':')
                            .map(|(_, t)| t.trim().to_string())
                            .unwrap_or_default(),
                        rest[1..].join("\n"),
                    ),
                    _ => (String::new(), rest.join("\n")),
                };
                let id = doc.new_id(if name.is_empty() { kind.label() } else { &name });
                let mut st = Style::default();
                let color = |k: &str| {
                    style.get(k).and_then(|v| {
                        if v == "none" {
                            Some("none".to_string())
                        } else {
                            parse_hex(v).map(hex)
                        }
                    })
                };
                st.fill = color("fillColor");
                st.stroke = color("strokeColor");
                st.text = color("fontColor");
                st.dashed = style.get("dashed").is_some_and(|v| v == "1") && kind != Kind::Boundary;
                st.font = style.get("fontSize").and_then(|v| v.parse().ok());
                st.opacity = style
                    .get("opacity")
                    .and_then(|v| v.parse::<f32>().ok())
                    .map(|o| o / 100.);
                doc.elements.push(Element {
                    id: id.clone(),
                    name: if name.is_empty() && kind != Kind::Text {
                        String::new()
                    } else {
                        name
                    },
                    kind,
                    tech,
                    description,
                    ..Default::default()
                });
                view.nodes.push(Node {
                    id: id.clone(),
                    x,
                    y,
                    w: w.max(10.),
                    h: h.max(10.),
                    parent: ids.get(parent).cloned(),
                    locked: style.get("movable").is_some_and(|v| v == "0"),
                    style: st,
                });
                abs.insert(cell_id.clone(), (x, y));
                ids.insert(cell_id, id);
            }
            pending = later;
        }
        for c in cells.iter().filter(|c| c.attribute("edge") == Some("1")) {
            let (Some(s), Some(t)) = (
                c.attribute("source").and_then(|s| ids.get(s)),
                c.attribute("target").and_then(|t| ids.get(t)),
            ) else {
                continue;
            };
            let style = style_map(c.attribute("style").unwrap_or(""));
            let id = doc.new_id(&format!("{s}-to-{t}"));
            doc.relations.push(Relation {
                id: id.clone(),
                from: s.clone(),
                to: t.clone(),
                label: label_text(c.attribute("value").unwrap_or("")),
                ..Default::default()
            });
            let route = if style.get("curved").is_some_and(|v| v == "1") {
                Route::Curved
            } else if style.contains_key("edgeStyle") {
                Route::Elbow
            } else {
                Route::Straight
            };
            let start = style.get("startArrow").is_some_and(|v| v != "none");
            let end = style.get("endArrow").is_none_or(|v| v != "none");
            view.edges.push(Edge {
                id,
                route,
                arrow: match (start, end) {
                    (true, _) => Arrow::Both,
                    (false, true) => Arrow::End,
                    (false, false) => Arrow::None,
                },
                style: Style {
                    dashed: style.get("dashed").is_some_and(|v| v == "1"),
                    stroke: style.get("strokeColor").and_then(|v| parse_hex(v)).map(hex),
                    ..Default::default()
                },
            });
        }
        doc.views.push(view);
    }
    Ok(doc)
}

/// Add an imported model to the editor's as new diagrams, renaming ids that clash.
pub(crate) fn merge(ed: &mut Editor, other: Doc) -> Option<String> {
    ed.checkpoint();
    let mut map: HashMap<String, String> = HashMap::new();
    for e in &other.elements {
        let id = if ed.doc.element(&e.id).is_some() || ed.doc.relation(&e.id).is_some() {
            ed.doc.new_id(&e.id)
        } else {
            e.id.clone()
        };
        map.insert(e.id.clone(), id.clone());
        ed.doc.elements.push(Element { id, ..e.clone() });
    }
    for r in &other.relations {
        let id = if ed.doc.element(&r.id).is_some() || ed.doc.relation(&r.id).is_some() {
            ed.doc.new_id(&r.id)
        } else {
            r.id.clone()
        };
        map.insert(r.id.clone(), id.clone());
        ed.doc.relations.push(Relation {
            id,
            from: map.get(&r.from).cloned().unwrap_or_else(|| r.from.clone()),
            to: map.get(&r.to).cloned().unwrap_or_else(|| r.to.clone()),
            ..r.clone()
        });
    }
    let mut first = None;
    for v in other.views {
        let id = ed.doc.new_view_id(&v.name);
        first.get_or_insert(id.clone());
        let m = |s: &String| map.get(s).cloned().unwrap_or_else(|| s.clone());
        ed.doc.views.push(View {
            id,
            nodes: v
                .nodes
                .iter()
                .map(|n| Node {
                    id: m(&n.id),
                    parent: n.parent.as_ref().map(m),
                    ..n.clone()
                })
                .collect(),
            edges: v
                .edges
                .iter()
                .map(|e| Edge {
                    id: m(&e.id),
                    ..e.clone()
                })
                .collect(),
            ..v
        });
    }
    ed.doc.tidy();
    if let Some(id) = &first {
        ed.open_view(id);
    }
    first
}

// ---- From code

/// Well-known dependencies that mean the project talks to a store or a service.
const KNOWN: &[(&str, &str, Kind, &str)] = &[
    ("postgres", "PostgreSQL", Kind::Database, "database"),
    ("pg", "PostgreSQL", Kind::Database, "database"),
    ("psycopg", "PostgreSQL", Kind::Database, "database"),
    ("psycopg2", "PostgreSQL", Kind::Database, "database"),
    ("tokio-postgres", "PostgreSQL", Kind::Database, "database"),
    ("mysql", "MySQL", Kind::Database, "database"),
    ("mysql2", "MySQL", Kind::Database, "database"),
    ("sqlite", "SQLite", Kind::Database, "database"),
    ("rusqlite", "SQLite", Kind::Database, "database"),
    ("better-sqlite3", "SQLite", Kind::Database, "database"),
    ("mongodb", "MongoDB", Kind::Database, "database"),
    ("mongoose", "MongoDB", Kind::Database, "database"),
    ("redis", "Redis", Kind::Database, "gauge"),
    ("ioredis", "Redis", Kind::Database, "gauge"),
    ("kafka", "Kafka", Kind::Queue, "radio"),
    ("kafkajs", "Kafka", Kind::Queue, "radio"),
    ("rdkafka", "Kafka", Kind::Queue, "radio"),
    ("amqplib", "RabbitMQ", Kind::Queue, "list-ordered"),
    ("lapin", "RabbitMQ", Kind::Queue, "list-ordered"),
    ("celery", "Celery", Kind::Queue, "list-ordered"),
    ("stripe", "Stripe", Kind::External, "credit-card"),
    ("twilio", "Twilio", Kind::External, "message-square"),
    ("@sendgrid/mail", "SendGrid", Kind::External, "mail"),
    ("sendgrid", "SendGrid", Kind::External, "mail"),
    ("openai", "OpenAI", Kind::External, "sparkles"),
    ("@anthropic-ai/sdk", "Anthropic", Kind::External, "sparkles"),
    ("anthropic", "Anthropic", Kind::External, "sparkles"),
    (
        "@aws-sdk/client-s3",
        "Amazon S3",
        Kind::Database,
        "hard-drive",
    ),
    ("aws-sdk-s3", "Amazon S3", Kind::Database, "hard-drive"),
    ("boto3", "AWS", Kind::External, "cloud"),
    ("firebase", "Firebase", Kind::External, "flame"),
    (
        "@supabase/supabase-js",
        "Supabase",
        Kind::External,
        "database",
    ),
    ("elasticsearch", "Elasticsearch", Kind::Database, "search"),
    (
        "@elastic/elasticsearch",
        "Elasticsearch",
        Kind::Database,
        "search",
    ),
];

/// A model and diagram drawn from the scan: packages as containers, their dependencies on
/// each other, and the stores and services their dependencies point at.
pub(crate) fn from_scan(map: &ProjectMap, project: &str) -> Doc {
    let mut doc = Doc::default();
    let mut view = View {
        id: "from-code".into(),
        name: "From code".into(),
        ..Default::default()
    };
    let mut pkg_ids: HashMap<String, String> = HashMap::new();
    let mut boundary = None;
    if map.packages.len() > 1 {
        let id = doc.new_id(project);
        doc.elements.push(Element {
            id: id.clone(),
            name: project.to_string(),
            kind: Kind::Boundary,
            icon: "frame".into(),
            ..Default::default()
        });
        view.nodes.push(Node {
            id: id.clone(),
            w: 400.,
            h: 300.,
            ..Default::default()
        });
        boundary = Some(id);
    }
    for p in &map.packages {
        let id = doc.new_id(&p.name);
        pkg_ids.insert(p.name.clone(), id.clone());
        let lines: usize = p.modules.iter().map(|m| m.lines).sum();
        doc.elements.push(Element {
            id: id.clone(),
            name: p.name.clone(),
            kind: Kind::Container,
            tech: p.kind.to_string(),
            description: if lines > 0 {
                format!("{} files, {lines} lines", p.files)
            } else {
                format!("{} files", p.files)
            },
            code: vec![if p.path.is_empty() {
                "**".to_string()
            } else {
                format!("{}/**", p.path)
            }],
            icon: "box".into(),
            ..Default::default()
        });
        let (w, h) = Kind::Container.size();
        view.nodes.push(Node {
            id,
            w,
            h,
            parent: boundary.clone(),
            ..Default::default()
        });
    }
    let mut external: HashMap<&str, String> = HashMap::new();
    for p in &map.packages {
        let from = pkg_ids[&p.name].clone();
        for dep in &p.internal {
            if let Some(to) = pkg_ids.get(dep) {
                let id = doc.new_id(&format!("{from}-to-{to}"));
                doc.relations.push(Relation {
                    id: id.clone(),
                    from: from.clone(),
                    to: to.clone(),
                    label: "uses".into(),
                    ..Default::default()
                });
                view.edges.push(Edge {
                    id,
                    ..Default::default()
                });
            }
        }
        for dep in &p.deps {
            let Some((_, name, kind, icon)) = KNOWN.iter().find(|(k, ..)| k == dep) else {
                continue;
            };
            let to = external
                .entry(name)
                .or_insert_with(|| {
                    let id = doc.new_id(name);
                    doc.elements.push(Element {
                        id: id.clone(),
                        name: name.to_string(),
                        kind: *kind,
                        icon: icon.to_string(),
                        description: format!("Found from the {dep} dependency"),
                        ..Default::default()
                    });
                    let (w, h) = kind.size();
                    view.nodes.push(Node {
                        id: id.clone(),
                        w,
                        h,
                        ..Default::default()
                    });
                    id
                })
                .clone();
            if view.edges.iter().any(|e| {
                doc.relation(&e.id)
                    .is_some_and(|r| r.from == from && r.to == to)
            }) {
                continue;
            }
            let id = doc.new_id(&format!("{from}-to-{to}"));
            doc.relations.push(Relation {
                id: id.clone(),
                from: from.clone(),
                to: to.clone(),
                tech: dep.clone(),
                ..Default::default()
            });
            view.edges.push(Edge {
                id,
                ..Default::default()
            });
        }
    }
    doc.views.push(view);
    // Lay it out once so it opens readable.
    let mut ed = Editor::new(doc);
    ed.auto_layout(false);
    ed.doc
}

// ---- SVG and PNG

fn segs_d(segs: &[Seg]) -> String {
    shape::svg_d(segs)
}

/// The diagram as a standalone SVG in the light theme.
pub(crate) fn svg(doc: &Doc, view: &View) -> String {
    let theme = Theme::LIGHT;
    let area = Rect::bounds(view.nodes.iter().map(|n| n.rect()))
        .unwrap_or(Rect::new(0., 0., 400., 300.))
        .inflate(40.);
    let mut out = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{} {} {} {}\" width=\"{}\" height=\"{}\" font-family=\"Inter, Helvetica, Arial, sans-serif\">\n<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"#ffffff\"/>\n",
        area.x, area.y, area.w, area.h, area.w.round(), area.h.round(), area.x, area.y, area.w, area.h
    );
    let kind = |id: &str| doc.element(id).map(|e| e.kind).unwrap_or_default();
    let text_block = |out: &mut String, r: &Rect, lines: &[(String, f32, u32, bool)], top: bool| {
        let total: f32 = lines.iter().map(|l| l.1 * 1.3).sum();
        let mut y = if top { r.y } else { r.y + (r.h - total) / 2. };
        for (t, size, color, bold) in lines {
            y += size * 1.3;
            let _ = writeln!(
                out,
                "<text x=\"{}\" y=\"{}\" font-size=\"{}\" fill=\"{}\" text-anchor=\"{}\"{}>{}</text>",
                if top { r.x } else { r.x + r.w / 2. },
                y - size * 0.3,
                size,
                hex(*color),
                if top { "start" } else { "middle" },
                if *bold { " font-weight=\"600\"" } else { "" },
                esc(t)
            );
        }
    };
    // Boundaries first, then connectors, then shapes, as on the canvas.
    let draw_node = |out: &mut String, n: &Node| {
        let Some(e) = doc.element(&n.id) else { return };
        let o = shape::outline(e.kind, &n.rect(), &n.style);
        let p = shape::paint(e.kind, &n.style, &theme);
        let dash = if p.dashed {
            " stroke-dasharray=\"6 4\""
        } else {
            ""
        };
        if !o.fill.is_empty() {
            let _ = writeln!(
                out,
                "<path d=\"{}\" fill=\"{}\" stroke=\"{}\" stroke-width=\"1.5\"{dash} opacity=\"{}\"/>",
                segs_d(&o.fill),
                p.fill.map(hex).unwrap_or_else(|| "none".into()),
                p.stroke.map(hex).unwrap_or_else(|| "none".into()),
                p.opacity
            );
        }
        if !o.detail.is_empty() {
            let _ = writeln!(
                out,
                "<path d=\"{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"1.5\"/>",
                segs_d(&o.detail),
                p.stroke.map(hex).unwrap_or_else(|| "none".into())
            );
        }
        let mut lines = Vec::new();
        for l in e.name.lines() {
            lines.push((
                l.to_string(),
                p.font,
                p.text,
                e.kind != Kind::Text || p.bold,
            ));
        }
        if let Some(c4) = e.kind.c4() {
            let t = if e.tech.is_empty() {
                format!("[{c4}]")
            } else {
                format!("[{c4}: {}]", e.tech)
            };
            lines.push((t, p.font * 0.78, p.sub, false));
            // The description, wrapped to the shape as the canvas shows it.
            let size = p.font * 0.78;
            let per_line = ((o.text.w / (size * 0.55)) as usize).max(8);
            for l in wrap(&e.description, per_line).into_iter().take(3) {
                lines.push((l, size, p.sub, false));
            }
        }
        text_block(out, &o.text, &lines, e.kind == Kind::Boundary);
    };
    for n in view.nodes.iter().filter(|n| kind(&n.id) == Kind::Boundary) {
        draw_node(&mut out, n);
    }
    let _ = writeln!(
        out,
        "<defs><marker id=\"arrow\" viewBox=\"0 0 10 10\" refX=\"9\" refY=\"5\" markerWidth=\"8\" markerHeight=\"8\" orient=\"auto-start-reverse\"><path d=\"M0 0 L10 5 L0 10 Z\" fill=\"{}\"/></marker></defs>",
        hex(theme.muted)
    );
    // Labels go on top of the shapes, so a short gap doesn't hide them.
    let mut labels = String::new();
    for e in &view.edges {
        let Some(r) = doc.relation(&e.id) else {
            continue;
        };
        let (Some(a), Some(b)) = (view.node(&r.from), view.node(&r.to)) else {
            continue;
        };
        let w = geom::route(&a.rect(), &b.rect(), e.route);
        let d = if w.curved && w.points.len() == 4 {
            let p = &w.points;
            format!(
                "M{} {} C{} {} {} {} {} {}",
                p[0].0, p[0].1, p[1].0, p[1].1, p[2].0, p[2].1, p[3].0, p[3].1
            )
        } else {
            w.points
                .iter()
                .enumerate()
                .map(|(i, p)| format!("{}{} {}", if i == 0 { "M" } else { "L" }, p.0, p.1))
                .collect::<Vec<_>>()
                .join(" ")
        };
        let color = e
            .style
            .stroke
            .as_deref()
            .and_then(parse_hex)
            .unwrap_or(theme.muted);
        let marks = match e.arrow {
            Arrow::End => " marker-end=\"url(#arrow)\"".to_string(),
            Arrow::Both => " marker-start=\"url(#arrow)\" marker-end=\"url(#arrow)\"".to_string(),
            Arrow::None => String::new(),
        };
        let _ = writeln!(
            out,
            "<path d=\"{d}\" fill=\"none\" stroke=\"{}\" stroke-width=\"1.5\"{}{marks}/>",
            hex(color),
            if e.style.dashed {
                " stroke-dasharray=\"6 4\""
            } else {
                ""
            }
        );
        let label = [r.label.as_str(), r.tech.as_str()]
            .iter()
            .filter(|s| !s.is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join(" · ");
        if !label.is_empty() {
            let (mx, my) = w.middle();
            let width = label.chars().count() as f32 * 6.4 + 12.;
            let _ = writeln!(
                labels,
                "<rect x=\"{}\" y=\"{}\" width=\"{width}\" height=\"18\" rx=\"4\" fill=\"#ffffff\"/><text x=\"{mx}\" y=\"{}\" font-size=\"11\" fill=\"{}\" text-anchor=\"middle\">{}</text>",
                mx - width / 2.,
                my - 9.,
                my + 4.,
                hex(theme.muted),
                esc(&label)
            );
        }
    }
    for n in view.nodes.iter().filter(|n| kind(&n.id) != Kind::Boundary) {
        draw_node(&mut out, n);
    }
    out.push_str(&labels);
    out.push_str("</svg>\n");
    out
}

/// Words in lines of at most `width` characters.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// The diagram as a PNG at twice its size.
pub(crate) fn png(doc: &Doc, view: &View) -> Result<Vec<u8>, String> {
    let text = svg(doc, view);
    let mut opts = resvg::usvg::Options::default();
    opts.fontdb_mut().load_system_fonts();
    let tree = resvg::usvg::Tree::from_str(&text, &opts).map_err(|e| e.to_string())?;
    let size = tree.size();
    let scale = 2.;
    let (w, h) = (
        (size.width() * scale).ceil() as u32,
        (size.height() * scale).ceil() as u32,
    );
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w.max(1), h.max(1))
        .ok_or("The diagram is too large for a PNG.")?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap.encode_png().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::architecture::{Module, Package};

    fn sample() -> Doc {
        let mut ed = Editor::new(Doc::default());
        let p = |l: &str| {
            super::super::editor::PRESETS
                .iter()
                .find(|p| p.label == l)
                .unwrap()
        };
        let g = ed.add(p("VPC / network"), (300., 300.));
        let api = ed.add(p("Service"), (300., 300.));
        let db = ed.add(p("Database"), (300., 600.));
        let r = ed.connect(&api, &db).unwrap();
        ed.doc.relation_mut(&r).unwrap().label = "reads & writes".into();
        ed.doc.element_mut(&g).unwrap().name = "Prod VPC".into();
        ed.doc
    }

    #[test]
    fn mermaid_has_subgraphs_shapes_and_labels() {
        let doc = sample();
        let m = mermaid(&doc, &doc.views[0]);
        assert!(m.starts_with("flowchart TD"));
        assert!(
            m.contains("subgraph group[\"Prod VPC\"]")
                || m.contains("subgraph vpc_network[\"Prod VPC\"]")
        );
        assert!(m.contains("database[(\"Database\")]"));
        assert!(m.contains("-->|\"reads & writes\"|"));
    }

    #[test]
    fn drawio_round_trips() {
        let doc = sample();
        let xml = drawio(&doc);
        assert!(xml.contains("container=1"));
        assert!(xml.contains("shape=cylinder3"));
        let back = import_drawio(&xml).unwrap();
        assert_eq!(back.views.len(), 1);
        assert_eq!(back.elements.len(), 3);
        let db = back
            .elements
            .iter()
            .find(|e| e.kind == Kind::Database)
            .unwrap();
        assert_eq!(db.name, "Database");
        let vpc = back
            .elements
            .iter()
            .find(|e| e.kind == Kind::Boundary)
            .unwrap();
        assert_eq!(vpc.name, "Prod VPC");
        let api = back.elements.iter().find(|e| e.tech == "Service").unwrap();
        let v = &back.views[0];
        assert_eq!(
            v.node(&api.id).unwrap().parent.as_deref(),
            Some(vpc.id.as_str())
        );
        // Positions come back absolute.
        let orig_api = doc.views[0].node("service").unwrap();
        assert_eq!(v.node(&api.id).unwrap().x, orig_api.x);
        assert_eq!(back.relations[0].label, "reads & writes");
    }

    #[test]
    fn compressed_drawio_pages_are_read() {
        use base64::Engine;
        use std::io::Write;
        let page = r#"<mxGraphModel><root><mxCell id="0"/><mxCell id="1" parent="0"/><mxCell id="a" value="Gateway" style="rounded=1;" vertex="1" parent="1"><mxGeometry x="10" y="20" width="120" height="60" as="geometry"/></mxCell><mxCell id="b" value="&lt;b&gt;Users&lt;/b&gt;&lt;br&gt;[Database: Postgres]" style="shape=cylinder3;" vertex="1" parent="1"><mxGeometry x="10" y="200" width="120" height="80" as="geometry"/></mxCell><mxCell id="e" value="SQL" style="endArrow=block;dashed=1;" edge="1" parent="1" source="a" target="b"><mxGeometry relative="1" as="geometry"/></mxCell></root></mxGraphModel>"#;
        let encoded: String = page
            .bytes()
            .map(|b| {
                if b.is_ascii_alphanumeric() {
                    (b as char).to_string()
                } else {
                    format!("%{b:02X}")
                }
            })
            .collect();
        let mut z = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(encoded.as_bytes()).unwrap();
        let b64 = base64::engine::general_purpose::STANDARD.encode(z.finish().unwrap());
        let file = format!("<mxfile><diagram name=\"Old\">{b64}</diagram></mxfile>");
        let doc = import_drawio(&file).unwrap();
        assert_eq!(doc.views[0].name, "Old");
        let users = doc.elements.iter().find(|e| e.name == "Users").unwrap();
        assert_eq!(users.kind, Kind::Database);
        assert_eq!(users.tech, "Postgres");
        assert!(doc.views[0].edges[0].style.dashed);
        assert_eq!(doc.relations[0].label, "SQL");
    }

    #[test]
    fn merge_renames_clashing_ids() {
        let mut ed = Editor::new(sample());
        let first = ed.view.clone();
        let view = merge(&mut ed, sample()).unwrap();
        assert_ne!(view, first);
        assert_eq!(ed.view, view);
        assert_eq!(ed.doc.elements.len(), 6);
        assert_eq!(ed.cur().edges.len(), 1);
        let r = ed.doc.relation(&ed.cur().edges[0].id).unwrap();
        assert!(ed.cur().node(&r.from).is_some() && ed.cur().node(&r.to).is_some());
    }

    #[test]
    fn code_scan_becomes_a_diagram_with_known_services() {
        let map = ProjectMap {
            packages: vec![
                Package {
                    name: "web".into(),
                    path: "web".into(),
                    kind: "npm package",
                    deps: vec!["react".into(), "stripe".into()],
                    internal: vec!["api".into()],
                    files: 10,
                    modules: vec![Module::default()],
                },
                Package {
                    name: "api".into(),
                    path: "api".into(),
                    kind: "Go module",
                    deps: vec!["pg".into(), "redis".into()],
                    internal: vec![],
                    files: 20,
                    modules: vec![],
                },
            ],
            ..Default::default()
        };
        let doc = from_scan(&map, "Shop");
        let names: Vec<&str> = doc.elements.iter().map(|e| e.name.as_str()).collect();
        for n in ["Shop", "web", "api", "Stripe", "PostgreSQL", "Redis"] {
            assert!(names.contains(&n), "{n} in {names:?}");
        }
        assert_eq!(doc.relations.len(), 4);
        let web = doc.element("web").unwrap();
        assert_eq!(web.code, vec!["web/**"]);
        // Laid out: web above api.
        let v = &doc.views[0];
        assert!(v.node("web").unwrap().y < v.node("api").unwrap().y);
    }

    #[test]
    fn svg_and_png_export() {
        let doc = sample();
        let s = svg(&doc, &doc.views[0]);
        assert!(
            s.starts_with("<svg") && s.contains("reads &amp; writes") && s.contains("marker-end")
        );
        let png = png(&doc, &doc.views[0]).unwrap();
        assert_eq!(&png[1..4], b"PNG");
    }
}
