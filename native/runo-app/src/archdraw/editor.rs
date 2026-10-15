//! The diagram editor's state and commands, apart from drawing: selection, undo and
//! redo, adding and connecting shapes, moving, grouping, styling, clipboard and layout.

use super::geom::{self, Align, Pt, Wire};
use super::model::*;
use std::collections::{HashMap, HashSet};

/// A shape in the palette.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Preset {
    pub label: &'static str,
    pub group: &'static str,
    pub kind: Kind,
    /// Lucide icon name drawn on the shape and in the palette.
    pub icon: &'static str,
    pub tech: &'static str,
    /// Extra words the shape search matches.
    pub words: &'static str,
}

const fn p(
    group: &'static str,
    label: &'static str,
    kind: Kind,
    icon: &'static str,
    tech: &'static str,
    words: &'static str,
) -> Preset {
    Preset {
        label,
        group,
        kind,
        icon,
        tech,
        words,
    }
}

pub(crate) const GROUPS: [&str; 4] = ["C4", "Shapes", "Cloud and infra", "Groups"];

pub(crate) const PRESETS: &[Preset] = &[
    p(
        "C4",
        "Person",
        Kind::Person,
        "user",
        "",
        "user actor customer human",
    ),
    p(
        "C4",
        "Software system",
        Kind::System,
        "boxes",
        "",
        "system product",
    ),
    p(
        "C4",
        "Container",
        Kind::Container,
        "box",
        "",
        "service app api",
    ),
    p(
        "C4",
        "Component",
        Kind::Component,
        "component",
        "",
        "module class",
    ),
    p(
        "C4",
        "Database",
        Kind::Database,
        "database",
        "",
        "db sql store postgres mysql",
    ),
    p(
        "C4",
        "Queue",
        Kind::Queue,
        "list-ordered",
        "",
        "topic stream kafka sqs pubsub",
    ),
    p(
        "C4",
        "External system",
        Kind::External,
        "globe",
        "",
        "third party saas",
    ),
    p("Shapes", "Box", Kind::Box, "square", "", "rectangle"),
    p(
        "Shapes",
        "Rounded box",
        Kind::Rounded,
        "square",
        "",
        "rounded rectangle",
    ),
    p(
        "Shapes",
        "Ellipse",
        Kind::Ellipse,
        "circle",
        "",
        "circle oval",
    ),
    p(
        "Shapes",
        "Decision",
        Kind::Diamond,
        "diamond",
        "",
        "diamond rhombus if",
    ),
    p("Shapes", "Cloud", Kind::Cloud, "cloud", "", "internet"),
    p(
        "Shapes",
        "Sticky note",
        Kind::Note,
        "sticky-note",
        "",
        "note comment",
    ),
    p("Shapes", "Text", Kind::Text, "type", "", "label title"),
    p(
        "Cloud and infra",
        "Web app",
        Kind::Container,
        "monitor",
        "Web",
        "frontend spa browser react",
    ),
    p(
        "Cloud and infra",
        "Mobile app",
        Kind::Container,
        "smartphone",
        "Mobile",
        "ios android",
    ),
    p(
        "Cloud and infra",
        "API gateway",
        Kind::Container,
        "door-open",
        "API gateway",
        "ingress edge",
    ),
    p(
        "Cloud and infra",
        "Load balancer",
        Kind::Container,
        "split",
        "Load balancer",
        "lb alb nginx",
    ),
    p(
        "Cloud and infra",
        "Service",
        Kind::Container,
        "server",
        "Service",
        "backend microservice",
    ),
    p(
        "Cloud and infra",
        "Function",
        Kind::Container,
        "zap",
        "Serverless function",
        "lambda cloud function faas",
    ),
    p(
        "Cloud and infra",
        "Worker",
        Kind::Container,
        "cog",
        "Worker",
        "job background cron",
    ),
    p(
        "Cloud and infra",
        "Cache",
        Kind::Database,
        "gauge",
        "Cache",
        "redis memcached",
    ),
    p(
        "Cloud and infra",
        "Object storage",
        Kind::Database,
        "hard-drive",
        "Object storage",
        "s3 gcs blob bucket",
    ),
    p(
        "Cloud and infra",
        "Search index",
        Kind::Database,
        "search",
        "Search",
        "elasticsearch opensearch",
    ),
    p(
        "Cloud and infra",
        "Event bus",
        Kind::Queue,
        "radio",
        "Event bus",
        "kafka eventbridge sns",
    ),
    p(
        "Cloud and infra",
        "CDN",
        Kind::External,
        "globe",
        "CDN",
        "cloudfront fastly cloudflare",
    ),
    p(
        "Cloud and infra",
        "Identity provider",
        Kind::External,
        "key-round",
        "Identity provider",
        "auth oauth sso cognito auth0",
    ),
    p(
        "Cloud and infra",
        "Payments",
        Kind::External,
        "credit-card",
        "Payments",
        "stripe billing",
    ),
    p(
        "Cloud and infra",
        "Email",
        Kind::External,
        "mail",
        "Email",
        "smtp sendgrid ses",
    ),
    p(
        "Cloud and infra",
        "Monitoring",
        Kind::External,
        "activity",
        "Monitoring",
        "metrics logs datadog grafana",
    ),
    p(
        "Groups",
        "Boundary",
        Kind::Boundary,
        "frame",
        "",
        "group container scope",
    ),
    p(
        "Groups",
        "VPC / network",
        Kind::Boundary,
        "network",
        "VPC",
        "vpc vnet network subnet",
    ),
    p(
        "Groups",
        "Region",
        Kind::Boundary,
        "map",
        "Region",
        "zone datacenter",
    ),
    p(
        "Groups",
        "Cluster",
        Kind::Boundary,
        "boxes",
        "Kubernetes cluster",
        "k8s kubernetes eks gke",
    ),
    p(
        "Groups",
        "Trust boundary",
        Kind::Boundary,
        "shield",
        "Trust boundary",
        "security zone",
    ),
];

/// Shapes whose label, group or extra words match every word of the query.
pub(crate) fn search(query: &str) -> Vec<&'static Preset> {
    let q = query.to_lowercase();
    let words: Vec<&str> = q.split_whitespace().collect();
    PRESETS
        .iter()
        .filter(|p| {
            let hay = format!("{} {} {} {}", p.label, p.group, p.words, p.tech).to_lowercase();
            words.iter().all(|w| hay.contains(w))
        })
        .collect()
}

/// What is selected on the current diagram.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Sel {
    pub nodes: Vec<String>,
    pub edges: Vec<String>,
}

impl Sel {
    pub(crate) fn is_empty(&self) -> bool {
        self.nodes.is_empty() && self.edges.is_empty()
    }

    pub(crate) fn has_node(&self, id: &str) -> bool {
        self.nodes.iter().any(|n| n == id)
    }

    pub(crate) fn has_edge(&self, id: &str) -> bool {
        self.edges.iter().any(|n| n == id)
    }
}

/// What a click lands on.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Hit {
    Node(String),
    Edge(String),
    None,
}

/// Copied shapes and the connectors between them.
#[derive(Clone, Debug, Default)]
struct Clip {
    elements: Vec<Element>,
    relations: Vec<Relation>,
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    pasted: u32,
}

/// Height of a boundary's title strip, which is where it is grabbed.
pub(crate) const BOUNDARY_HEAD: f32 = 34.;
const UNDO_LIMIT: usize = 200;

pub(crate) struct Editor {
    pub doc: Doc,
    /// The diagram being edited.
    pub view: String,
    pub sel: Sel,
    undo: Vec<Doc>,
    redo: Vec<Doc>,
    /// Changed since the last save.
    pub dirty: bool,
    clip: Option<Clip>,
    pub style_clip: Option<Style>,
}

impl Editor {
    pub(crate) fn new(mut doc: Doc) -> Self {
        if doc.views.is_empty() {
            doc.views.push(View {
                id: "main".into(),
                name: "Main".into(),
                ..Default::default()
            });
        }
        let view = doc.views[0].id.clone();
        Self {
            doc,
            view,
            sel: Sel::default(),
            undo: Vec::new(),
            redo: Vec::new(),
            dirty: false,
            clip: None,
            style_clip: None,
        }
    }

    pub(crate) fn cur(&self) -> &View {
        self.doc
            .views
            .iter()
            .find(|v| v.id == self.view)
            .unwrap_or(&self.doc.views[0])
    }

    pub(crate) fn cur_mut(&mut self) -> &mut View {
        let i = self
            .doc
            .views
            .iter()
            .position(|v| v.id == self.view)
            .unwrap_or(0);
        &mut self.doc.views[i]
    }

    /// Remember the model before a change, so it can be undone.
    pub(crate) fn checkpoint(&mut self) {
        self.undo.push(self.doc.clone());
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.dirty = true;
    }

    pub(crate) fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub(crate) fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub(crate) fn undo(&mut self) -> bool {
        let Some(prev) = self.undo.pop() else {
            return false;
        };
        self.redo.push(std::mem::replace(&mut self.doc, prev));
        self.after_history();
        true
    }

    pub(crate) fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo.push(std::mem::replace(&mut self.doc, next));
        self.after_history();
        true
    }

    fn after_history(&mut self) {
        self.dirty = true;
        if !self.doc.views.iter().any(|v| v.id == self.view) {
            self.view = self.doc.views[0].id.clone();
        }
        let view = self.cur().clone();
        self.sel.nodes.retain(|n| view.node(n).is_some());
        self.sel.edges.retain(|e| view.edge(e).is_some());
    }

    /// Open another diagram.
    pub(crate) fn open_view(&mut self, id: &str) {
        if self.doc.views.iter().any(|v| v.id == id) {
            self.view = id.to_string();
            self.sel = Sel::default();
        }
    }

    pub(crate) fn kind_of(&self, id: &str) -> Kind {
        self.doc.element(id).map(|e| e.kind).unwrap_or_default()
    }

    // ---- Adding and connecting

    /// Add a shape centred on `at`. Returns the new element's id.
    pub(crate) fn add(&mut self, preset: &Preset, at: Pt) -> String {
        self.checkpoint();
        let name = match preset.kind {
            Kind::Text => "Text".to_string(),
            Kind::Note => "Note".to_string(),
            _ => preset.label.to_string(),
        };
        let id = self.doc.new_id(&name);
        self.doc.elements.push(Element {
            id: id.clone(),
            name,
            kind: preset.kind,
            tech: preset.tech.to_string(),
            icon: preset.icon.to_string(),
            ..Default::default()
        });
        let (w, h) = preset.kind.size();
        let node = Node {
            id: id.clone(),
            x: (at.0 - w / 2.).round(),
            y: (at.1 - h / 2.).round(),
            w,
            h,
            ..Default::default()
        };
        let boundary = preset.kind == Kind::Boundary;
        let rect = node.rect();
        let view = self.cur_mut();
        if boundary {
            // A new boundary goes behind everything and takes in what it covers.
            view.nodes.insert(0, node);
        } else {
            view.nodes.push(node);
        }
        if boundary {
            let covered: Vec<String> = view
                .nodes
                .iter()
                .filter(|n| n.id != id && rect.contains_rect(&n.rect()))
                .map(|n| n.id.clone())
                .collect();
            self.adopt(&covered);
        }
        self.adopt(std::slice::from_ref(&id));
        self.sel = Sel {
            nodes: vec![id.clone()],
            edges: Vec::new(),
        };
        id
    }

    /// Show an element of the model on this diagram, with its connectors to what's here.
    pub(crate) fn place(&mut self, id: &str, at: Pt) {
        if self.cur().node(id).is_some() {
            return;
        }
        let Some(kind) = self.doc.element(id).map(|e| e.kind) else {
            return;
        };
        self.checkpoint();
        let (w, h) = kind.size();
        self.cur_mut().nodes.push(Node {
            id: id.to_string(),
            x: at.0 - w / 2.,
            y: at.1 - h / 2.,
            w,
            h,
            ..Default::default()
        });
        self.show_relations();
        self.adopt(&[id.to_string()]);
    }

    /// Draw every relation of the model whose two ends are on this diagram.
    pub(crate) fn show_relations(&mut self) {
        let shown: HashSet<String> = self.cur().nodes.iter().map(|n| n.id.clone()).collect();
        let missing: Vec<String> = self
            .doc
            .relations
            .iter()
            .filter(|r| shown.contains(&r.from) && shown.contains(&r.to))
            .filter(|r| self.cur().edge(&r.id).is_none())
            .map(|r| r.id.clone())
            .collect();
        for id in missing {
            self.cur_mut().edges.push(Edge {
                id,
                ..Default::default()
            });
        }
    }

    /// Connect two shapes. Returns the new relation's id.
    pub(crate) fn connect(&mut self, from: &str, to: &str) -> Option<String> {
        if self.doc.element(from).is_none() || self.doc.element(to).is_none() {
            return None;
        }
        self.checkpoint();
        let id = self.doc.new_id(&format!("{from}-to-{to}"));
        self.doc.relations.push(Relation {
            id: id.clone(),
            from: from.to_string(),
            to: to.to_string(),
            ..Default::default()
        });
        self.cur_mut().edges.push(Edge {
            id: id.clone(),
            ..Default::default()
        });
        self.sel = Sel {
            nodes: Vec::new(),
            edges: vec![id.clone()],
        };
        Some(id)
    }

    // ---- Selection

    pub(crate) fn select(&mut self, hit: &Hit, add: bool) {
        if !add {
            self.sel = Sel::default();
        }
        match hit {
            Hit::Node(id) => {
                if add && self.sel.has_node(id) {
                    self.sel.nodes.retain(|n| n != id);
                } else if !self.sel.has_node(id) {
                    self.sel.nodes.push(id.clone());
                }
            }
            Hit::Edge(id) => {
                if add && self.sel.has_edge(id) {
                    self.sel.edges.retain(|n| n != id);
                } else if !self.sel.has_edge(id) {
                    self.sel.edges.push(id.clone());
                }
            }
            Hit::None => {}
        }
    }

    pub(crate) fn select_all(&mut self) {
        let v = self.cur();
        self.sel = Sel {
            nodes: v.nodes.iter().map(|n| n.id.clone()).collect(),
            edges: v.edges.iter().map(|e| e.id.clone()).collect(),
        };
    }

    /// Select what lies wholly inside `area`.
    pub(crate) fn select_area(&mut self, area: Rect, add: bool) {
        if !add {
            self.sel = Sel::default();
        }
        let inside: Vec<String> = self
            .cur()
            .nodes
            .iter()
            .filter(|n| area.contains_rect(&n.rect()))
            .map(|n| n.id.clone())
            .collect();
        for id in inside {
            if !self.sel.has_node(&id) {
                self.sel.nodes.push(id);
            }
        }
        let edges: Vec<String> = self
            .wires()
            .into_iter()
            .filter(|(_, w)| w.points.iter().all(|p| area.contains(*p)))
            .map(|(id, _)| id)
            .collect();
        for id in edges {
            if !self.sel.has_edge(&id) {
                self.sel.edges.push(id);
            }
        }
    }

    // ---- Geometry of the current diagram

    pub(crate) fn rect(&self, id: &str) -> Option<Rect> {
        self.cur().node(id).map(|n| n.rect())
    }

    /// Every connector of the diagram with its route.
    pub(crate) fn wires(&self) -> Vec<(String, Wire)> {
        let v = self.cur();
        v.edges
            .iter()
            .filter_map(|e| {
                let r = self.doc.relation(&e.id)?;
                let a = v.node(&r.from)?.rect();
                let b = v.node(&r.to)?.rect();
                Some((e.id.clone(), geom::route(&a, &b, e.route)))
            })
            .collect()
    }

    /// What's under `p`, with `tol` of slack for thin things.
    pub(crate) fn hit(&self, p: Pt, tol: f32) -> Hit {
        let v = self.cur();
        for n in v.nodes.iter().rev() {
            if self.kind_of(&n.id) == Kind::Boundary {
                continue;
            }
            if n.rect().contains(p) {
                return Hit::Node(n.id.clone());
            }
        }
        let mut best: Option<(f32, String)> = None;
        for (id, w) in self.wires() {
            let d = geom::wire_dist(&w, p);
            if d <= tol && best.as_ref().is_none_or(|(b, _)| d < *b) {
                best = Some((d, id));
            }
        }
        if let Some((_, id)) = best {
            return Hit::Edge(id);
        }
        // A boundary is grabbed by its title strip or its border, so dragging inside it
        // still draws a selection box.
        for n in v.nodes.iter().rev() {
            if self.kind_of(&n.id) != Kind::Boundary {
                continue;
            }
            let r = n.rect();
            let head = Rect::new(r.x, r.y, r.w, BOUNDARY_HEAD.min(r.h));
            let near_border = r.inflate(tol).contains(p) && !r.inflate(-tol).contains(p);
            if head.contains(p) || near_border {
                return Hit::Node(n.id.clone());
            }
        }
        Hit::None
    }

    /// The innermost boundary under the centre of `id`, other than itself and its insides.
    fn container_for(&self, id: &str) -> Option<String> {
        let v = self.cur();
        let r = v.node(id)?.rect();
        let inside: HashSet<String> = v.descendants(id).into_iter().collect();
        v.nodes
            .iter()
            .filter(|n| n.id != id && !inside.contains(&n.id))
            .filter(|n| self.kind_of(&n.id) == Kind::Boundary)
            .filter(|n| n.rect().contains(r.center()))
            .min_by(|a, b| (a.w * a.h).total_cmp(&(b.w * b.h)))
            .map(|n| n.id.clone())
    }

    /// Put each node in the boundary it now sits in, and keep boundaries behind what they
    /// hold.
    pub(crate) fn adopt(&mut self, ids: &[String]) {
        for id in ids {
            let parent = self.container_for(id);
            if let Some(n) = self.cur_mut().node_mut(id) {
                n.parent = parent.clone();
            }
            if let Some(p) = parent {
                let v = self.cur_mut();
                let pi = v.nodes.iter().position(|n| n.id == p);
                let ci = v.nodes.iter().position(|n| &n.id == id);
                if let (Some(pi), Some(ci)) = (pi, ci) {
                    if ci < pi {
                        let node = v.nodes.remove(ci);
                        v.nodes.insert(pi, node);
                    }
                }
            }
        }
    }

    /// The selected nodes plus everything inside selected boundaries, minus locked ones.
    pub(crate) fn movable(&self) -> Vec<String> {
        let v = self.cur();
        let mut out: Vec<String> = Vec::new();
        for id in &self.sel.nodes {
            for m in std::iter::once(id.clone()).chain(v.descendants(id)) {
                if !out.contains(&m) && v.node(&m).is_some_and(|n| !n.locked) {
                    out.push(m);
                }
            }
        }
        out
    }

    /// Place nodes at new rectangles, without a checkpoint (drags call `checkpoint` once).
    pub(crate) fn set_rects(&mut self, rects: &[(String, Rect)]) {
        let v = self.cur_mut();
        for (id, r) in rects {
            if let Some(n) = v.node_mut(id) {
                n.x = r.x.round();
                n.y = r.y.round();
                n.w = r.w.round().max(10.);
                n.h = r.h.round().max(10.);
            }
        }
        self.dirty = true;
    }

    /// Move the selection by `d` as one step (arrow keys).
    pub(crate) fn nudge(&mut self, d: Pt) {
        let ids = self.movable();
        if ids.is_empty() {
            return;
        }
        self.checkpoint();
        let rects: Vec<(String, Rect)> = ids
            .iter()
            .filter_map(|id| {
                let r = self.rect(id)?;
                Some((id.clone(), Rect::new(r.x + d.0, r.y + d.1, r.w, r.h)))
            })
            .collect();
        self.set_rects(&rects);
    }

    // ---- Editing the selection

    /// Remove the selection from this diagram. Parts no other diagram shows leave the model.
    pub(crate) fn delete(&mut self) {
        if self.sel.is_empty() {
            return;
        }
        self.checkpoint();
        let view_id = self.view.clone();
        let nodes = std::mem::take(&mut self.sel.nodes);
        let edges = std::mem::take(&mut self.sel.edges);
        {
            let v = self.cur_mut();
            for id in &nodes {
                let parent = v.node(id).and_then(|n| n.parent.clone());
                for n in v.nodes.iter_mut() {
                    if n.parent.as_deref() == Some(id.as_str()) {
                        n.parent = parent.clone();
                    }
                }
            }
            v.nodes.retain(|n| !nodes.contains(&n.id));
            v.edges.retain(|e| !edges.contains(&e.id));
        }
        let elsewhere = |doc: &Doc, id: &str, node: bool| {
            doc.views.iter().any(|v| {
                v.id != view_id
                    && if node {
                        v.node(id).is_some()
                    } else {
                        v.edge(id).is_some()
                    }
            })
        };
        let gone: Vec<String> = nodes
            .iter()
            .filter(|id| !elsewhere(&self.doc, id, true))
            .cloned()
            .collect();
        self.doc.elements.retain(|e| !gone.contains(&e.id));
        let gone_rel: Vec<String> = edges
            .iter()
            .filter(|id| !elsewhere(&self.doc, id, false))
            .cloned()
            .collect();
        self.doc.relations.retain(|r| !gone_rel.contains(&r.id));
        self.doc.tidy();
    }

    /// Copy the selection (shapes and the connectors between them).
    pub(crate) fn copy(&mut self) -> bool {
        let v = self.cur();
        let ids: HashSet<String> = self.sel.nodes.iter().cloned().collect();
        if ids.is_empty() {
            return false;
        }
        let nodes: Vec<Node> = v
            .nodes
            .iter()
            .filter(|n| ids.contains(&n.id))
            .cloned()
            .collect();
        let elements: Vec<Element> = self
            .doc
            .elements
            .iter()
            .filter(|e| ids.contains(&e.id))
            .cloned()
            .collect();
        let relations: Vec<Relation> = self
            .doc
            .relations
            .iter()
            .filter(|r| ids.contains(&r.from) && ids.contains(&r.to))
            .filter(|r| v.edge(&r.id).is_some())
            .cloned()
            .collect();
        let edges: Vec<Edge> = relations
            .iter()
            .filter_map(|r| v.edge(&r.id).cloned())
            .collect();
        self.clip = Some(Clip {
            elements,
            relations,
            nodes,
            edges,
            pasted: 0,
        });
        true
    }

    pub(crate) fn can_paste(&self) -> bool {
        self.clip.is_some()
    }

    /// Paste copies with new ids, a little offset each time.
    pub(crate) fn paste(&mut self) {
        let Some(mut clip) = self.clip.take() else {
            return;
        };
        clip.pasted += 1;
        let off = 24. * clip.pasted as f32;
        self.paste_clip(&clip, (off, off));
        self.clip = Some(clip);
    }

    /// Copy and paste in one step.
    pub(crate) fn duplicate(&mut self) {
        let saved = self.clip.take();
        if self.copy() {
            if let Some(clip) = self.clip.take() {
                self.paste_clip(&clip, (24., 24.));
            }
        }
        self.clip = saved;
    }

    fn paste_clip(&mut self, clip: &Clip, off: Pt) {
        self.checkpoint();
        let mut map: HashMap<String, String> = HashMap::new();
        for e in &clip.elements {
            let id = self.doc.new_id(&e.name);
            map.insert(e.id.clone(), id.clone());
            self.doc.elements.push(Element { id, ..e.clone() });
        }
        let mut new_nodes = Vec::new();
        for n in &clip.nodes {
            let Some(id) = map.get(&n.id) else { continue };
            new_nodes.push(Node {
                id: id.clone(),
                x: n.x + off.0,
                y: n.y + off.1,
                parent: n
                    .parent
                    .as_ref()
                    .map(|p| map.get(p).cloned().unwrap_or_else(|| p.clone())),
                locked: false,
                ..n.clone()
            });
        }
        let mut new_edges = Vec::new();
        for r in &clip.relations {
            let (Some(from), Some(to)) = (map.get(&r.from), map.get(&r.to)) else {
                continue;
            };
            let id = self.doc.new_id(&format!("{from}-to-{to}"));
            if let Some(e) = clip.edges.iter().find(|e| e.id == r.id) {
                new_edges.push(Edge {
                    id: id.clone(),
                    ..e.clone()
                });
            }
            self.doc.relations.push(Relation {
                id,
                from: from.clone(),
                to: to.clone(),
                ..r.clone()
            });
        }
        self.sel = Sel {
            nodes: new_nodes.iter().map(|n| n.id.clone()).collect(),
            edges: new_edges.iter().map(|e| e.id.clone()).collect(),
        };
        let v = self.cur_mut();
        v.nodes.extend(new_nodes);
        v.edges.extend(new_edges);
        self.doc.tidy();
    }

    /// Line up the selected shapes.
    pub(crate) fn align(&mut self, how: Align) {
        let ids: Vec<String> = self
            .sel
            .nodes
            .iter()
            .filter(|id| self.cur().node(id).is_some_and(|n| !n.locked))
            .cloned()
            .collect();
        let min = if matches!(how, Align::SpreadX | Align::SpreadY) {
            3
        } else {
            2
        };
        if ids.len() < min {
            return;
        }
        let rects: Vec<Rect> = ids.iter().filter_map(|id| self.rect(id)).collect();
        let after = geom::align(&rects, how);
        self.checkpoint();
        // Boundaries carry their contents along.
        let mut moves: Vec<(String, Rect)> = Vec::new();
        for (i, id) in ids.iter().enumerate() {
            let (before, now) = (rects[i], after[i]);
            moves.push((id.clone(), now));
            let (dx, dy) = (now.x - before.x, now.y - before.y);
            for d in self.cur().descendants(id) {
                if let Some(r) = self.rect(&d) {
                    moves.push((d, Rect::new(r.x + dx, r.y + dy, r.w, r.h)));
                }
            }
        }
        self.set_rects(&moves);
    }

    /// Bring the selection in front of everything, or send it behind.
    pub(crate) fn order(&mut self, front: bool) {
        if self.sel.nodes.is_empty() {
            return;
        }
        self.checkpoint();
        let ids = self.sel.nodes.clone();
        let v = self.cur_mut();
        let (mut picked, rest): (Vec<Node>, Vec<Node>) =
            v.nodes.drain(..).partition(|n| ids.contains(&n.id));
        if front {
            v.nodes = rest;
            v.nodes.append(&mut picked);
        } else {
            picked.extend(rest);
            v.nodes = picked;
        }
    }

    /// Lock the selection in place, or unlock it if it was all locked.
    pub(crate) fn toggle_lock(&mut self) {
        if self.sel.nodes.is_empty() {
            return;
        }
        self.checkpoint();
        let ids = self.sel.nodes.clone();
        let v = self.cur_mut();
        let all = ids.iter().all(|id| v.node(id).is_some_and(|n| n.locked));
        for id in &ids {
            if let Some(n) = v.node_mut(id) {
                n.locked = !all;
            }
        }
    }

    /// Put the selected shapes in a new boundary.
    pub(crate) fn group(&mut self) -> Option<String> {
        let ids = self.sel.nodes.clone();
        let rects: Vec<Rect> = ids.iter().filter_map(|id| self.rect(id)).collect();
        let area = Rect::bounds(rects)?;
        let outer = self.cur().node(&ids[0]).and_then(|n| n.parent.clone());
        self.checkpoint();
        let id = self.doc.new_id("Group");
        self.doc.elements.push(Element {
            id: id.clone(),
            name: "Group".into(),
            kind: Kind::Boundary,
            icon: "frame".into(),
            ..Default::default()
        });
        let pad = 30.;
        let first = self
            .cur()
            .nodes
            .iter()
            .position(|n| ids.contains(&n.id))
            .unwrap_or(0);
        let v = self.cur_mut();
        v.nodes.insert(
            first,
            Node {
                id: id.clone(),
                x: area.x - pad,
                y: area.y - pad - BOUNDARY_HEAD,
                w: area.w + 2. * pad,
                h: area.h + 2. * pad + BOUNDARY_HEAD,
                parent: outer,
                ..Default::default()
            },
        );
        for n in v.nodes.iter_mut() {
            if ids.contains(&n.id) {
                n.parent = Some(id.clone());
            }
        }
        self.sel = Sel {
            nodes: vec![id.clone()],
            edges: Vec::new(),
        };
        Some(id)
    }

    /// Take the selected boundaries away, keeping what they held.
    pub(crate) fn ungroup(&mut self) {
        let ids: Vec<String> = self
            .sel
            .nodes
            .iter()
            .filter(|id| self.kind_of(id) == Kind::Boundary)
            .cloned()
            .collect();
        if ids.is_empty() {
            return;
        }
        let children: Vec<String> = ids
            .iter()
            .flat_map(|id| {
                self.cur()
                    .nodes
                    .iter()
                    .filter(|n| n.parent.as_deref() == Some(id.as_str()))
                    .map(|n| n.id.clone())
                    .collect::<Vec<_>>()
            })
            .collect();
        self.sel = Sel {
            nodes: ids,
            edges: Vec::new(),
        };
        self.delete();
        self.sel.nodes = children;
    }

    /// Change the style of everything selected.
    pub(crate) fn style(&mut self, f: impl Fn(&mut Style)) {
        if self.sel.is_empty() {
            return;
        }
        self.checkpoint();
        let sel = self.sel.clone();
        let v = self.cur_mut();
        for id in &sel.nodes {
            if let Some(n) = v.node_mut(id) {
                f(&mut n.style);
            }
        }
        for id in &sel.edges {
            if let Some(e) = v.edge_mut(id) {
                f(&mut e.style);
            }
        }
    }

    /// Change the selected connectors.
    pub(crate) fn edges(&mut self, f: impl Fn(&mut Edge)) {
        if self.sel.edges.is_empty() {
            return;
        }
        self.checkpoint();
        let ids = self.sel.edges.clone();
        let v = self.cur_mut();
        for id in &ids {
            if let Some(e) = v.edge_mut(id) {
                f(e);
            }
        }
    }

    /// Format painter: remember the style of the first selected thing.
    pub(crate) fn copy_style(&mut self) {
        let v = self.cur();
        self.style_clip = self
            .sel
            .nodes
            .first()
            .and_then(|id| v.node(id))
            .map(|n| n.style.clone())
            .or_else(|| {
                self.sel
                    .edges
                    .first()
                    .and_then(|id| v.edge(id))
                    .map(|e| e.style.clone())
            });
    }

    pub(crate) fn paste_style(&mut self) {
        if let Some(s) = self.style_clip.clone() {
            self.style(move |st| *st = s.clone());
        }
    }

    // ---- Layout

    /// Lay out the selection (two or more shapes) or the whole diagram, boundaries and all.
    pub(crate) fn auto_layout(&mut self, horizontal: bool) {
        let v = self.cur().clone();
        if v.nodes.is_empty() {
            return;
        }
        self.checkpoint();
        let links: Vec<(String, String)> = v
            .edges
            .iter()
            .filter_map(|e| self.doc.relation(&e.id))
            .map(|r| (r.from.clone(), r.to.clone()))
            .collect();
        let selected: Vec<String> = self
            .sel
            .nodes
            .iter()
            .filter(|id| v.node(id).is_some_and(|n| !n.locked))
            .cloned()
            .collect();
        if selected.len() >= 2 {
            let rects: Vec<Rect> = selected.iter().filter_map(|id| self.rect(id)).collect();
            let area = Rect::bounds(rects).unwrap_or_default();
            let sizes: HashMap<String, Pt> = selected
                .iter()
                .filter_map(|id| v.node(id).map(|n| (id.clone(), (n.w, n.h))))
                .collect();
            let pos = geom::layered(&selected, &sizes, &links, (area.x, area.y), horizontal);
            let mut moves = Vec::new();
            for (id, (x, y)) in pos {
                let Some(r) = self.rect(&id) else { continue };
                let (dx, dy) = (x - r.x, y - r.y);
                moves.push((id.clone(), Rect::new(x, y, r.w, r.h)));
                for d in v.descendants(&id) {
                    if let Some(c) = self.rect(&d) {
                        moves.push((d, Rect::new(c.x + dx, c.y + dy, c.w, c.h)));
                    }
                }
            }
            self.set_rects(&moves);
            return;
        }
        let origin = Rect::bounds(v.nodes.iter().map(|n| n.rect()))
            .map(|r| (r.x, r.y))
            .unwrap_or((0., 0.));
        let mut placed: HashMap<String, Rect> = HashMap::new();
        let size = self.layout_level(&v, None, &links, horizontal, &mut placed);
        let _ = size;
        let moves: Vec<(String, Rect)> = placed
            .into_iter()
            .map(|(id, r)| (id, Rect::new(r.x + origin.0, r.y + origin.1, r.w, r.h)))
            .collect();
        self.set_rects(&moves);
    }

    /// Lay out the direct children of `parent` (recursing into boundaries) at (0, 0), and
    /// return the size of the result. Positions go into `placed`, relative to that origin.
    fn layout_level(
        &self,
        v: &View,
        parent: Option<&str>,
        links: &[(String, String)],
        horizontal: bool,
        placed: &mut HashMap<String, Rect>,
    ) -> Pt {
        let kids: Vec<&Node> = v
            .nodes
            .iter()
            .filter(|n| n.parent.as_deref() == parent)
            .collect();
        let mut sizes: HashMap<String, Pt> = HashMap::new();
        let mut inner: HashMap<String, HashMap<String, Rect>> = HashMap::new();
        let pad = 30.;
        for k in &kids {
            if self.kind_of(&k.id) == Kind::Boundary
                && v.nodes
                    .iter()
                    .any(|n| n.parent.as_deref() == Some(k.id.as_str()))
            {
                let mut sub = HashMap::new();
                let (w, h) = self.layout_level(v, Some(&k.id), links, horizontal, &mut sub);
                sizes.insert(k.id.clone(), (w + 2. * pad, h + 2. * pad + BOUNDARY_HEAD));
                inner.insert(k.id.clone(), sub);
            } else {
                sizes.insert(k.id.clone(), (k.w, k.h));
            }
        }
        // A link between anything inside two children counts as a link between them.
        let top = |id: &str| -> Option<String> {
            let mut cur = v.node(id)?;
            loop {
                if cur.parent.as_deref() == parent {
                    return Some(cur.id.clone());
                }
                cur = v.node(cur.parent.as_deref()?)?;
            }
        };
        let lifted: Vec<(String, String)> = links
            .iter()
            .filter_map(|(a, b)| Some((top(a)?, top(b)?)))
            .filter(|(a, b)| a != b)
            .collect();
        let ids: Vec<String> = kids.iter().map(|k| k.id.clone()).collect();
        let pos = geom::layered(&ids, &sizes, &lifted, (0., 0.), horizontal);
        let mut area: Option<Rect> = None;
        for id in &ids {
            let (x, y) = pos.get(id).copied().unwrap_or_default();
            let (w, h) = sizes[id];
            let r = Rect::new(x, y, w, h);
            area = Some(area.map_or(r, |a| a.union(&r)));
            placed.insert(id.clone(), r);
            if let Some(sub) = inner.remove(id) {
                for (cid, c) in sub {
                    placed.insert(
                        cid,
                        Rect::new(c.x + x + pad, c.y + y + pad + BOUNDARY_HEAD, c.w, c.h),
                    );
                }
            }
        }
        area.map(|a| (a.right(), a.bottom())).unwrap_or((0., 0.))
    }

    /// Everything on the diagram, for "fit".
    pub(crate) fn extent(&self) -> Option<Rect> {
        Rect::bounds(self.cur().nodes.iter().map(|n| n.rect()))
    }

    // ---- Diagrams

    pub(crate) fn add_view(&mut self, name: &str) -> String {
        self.checkpoint();
        let id = self.doc.new_view_id(name);
        self.doc.views.push(View {
            id: id.clone(),
            name: name.to_string(),
            ..Default::default()
        });
        self.open_view(&id);
        id
    }

    /// A copy of the current diagram, showing the same elements.
    pub(crate) fn duplicate_view(&mut self) -> String {
        self.checkpoint();
        let mut v = self.cur().clone();
        v.name = format!("{} copy", v.name);
        v.id = self.doc.new_view_id(&v.name);
        let id = v.id.clone();
        self.doc.views.push(v);
        self.open_view(&id);
        id
    }

    #[cfg(test)]
    pub(crate) fn rename_view(&mut self, name: &str) {
        let name = name.trim();
        if name.is_empty() || self.cur().name == name {
            return;
        }
        self.checkpoint();
        self.cur_mut().name = name.to_string();
    }

    /// Delete the current diagram (never the last one). Elements no diagram shows stay in
    /// the model, so they can be placed again.
    pub(crate) fn delete_view(&mut self) {
        if self.doc.views.len() < 2 {
            return;
        }
        self.checkpoint();
        let id = self.view.clone();
        self.doc.views.retain(|v| v.id != id);
        self.view = self.doc.views[0].id.clone();
        self.sel = Sel::default();
    }

    /// Elements of the model this diagram doesn't show.
    pub(crate) fn unplaced(&self) -> Vec<&Element> {
        let v = self.cur();
        self.doc
            .elements
            .iter()
            .filter(|e| v.node(&e.id).is_none())
            .collect()
    }

    /// Connectors on a dependency cycle, for drawing them as a warning.
    pub(crate) fn cycles(&self) -> HashSet<String> {
        let v = self.cur();
        let links: Vec<(String, String)> = v
            .edges
            .iter()
            .filter_map(|e| self.doc.relation(&e.id))
            .filter(|r| self.kind_of(&r.from).is_part() && self.kind_of(&r.to).is_part())
            .map(|r| (r.from.clone(), r.to.clone()))
            .collect();
        let on = geom::cyclic_links(&links);
        v.edges
            .iter()
            .filter_map(|e| self.doc.relation(&e.id))
            .filter(|r| on.contains(&(r.from.clone(), r.to.clone())))
            .map(|r| r.id.clone())
            .collect()
    }

    /// Replace the model and diagram with edited text. Returns why it couldn't.
    pub(crate) fn apply_text(&mut self, model: &str, view: &str) -> Result<(), String> {
        let (doc, v) = Doc::parse(model, &self.view, view)?;
        self.checkpoint();
        let others: Vec<View> = self
            .doc
            .views
            .iter()
            .filter(|x| x.id != self.view)
            .cloned()
            .collect();
        let i = self
            .doc
            .views
            .iter()
            .position(|x| x.id == self.view)
            .unwrap_or(0);
        let mut views = others;
        views.insert(i.min(views.len()), v);
        self.doc = Doc {
            elements: doc.elements,
            relations: doc.relations,
            views,
        };
        self.doc.tidy();
        self.after_history();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preset(label: &str) -> &'static Preset {
        PRESETS.iter().find(|p| p.label == label).unwrap()
    }

    #[test]
    fn search_finds_shapes_by_any_word() {
        let hits: Vec<&str> = search("redis").iter().map(|p| p.label).collect();
        assert_eq!(hits, vec!["Cache"]);
        assert!(search("db").iter().any(|p| p.label == "Database"));
        assert!(search("zzz").is_empty());
        assert_eq!(search("").len(), PRESETS.len());
    }

    #[test]
    fn add_connect_undo_redo() {
        let mut ed = Editor::new(Doc::default());
        let a = ed.add(preset("Container"), (100., 100.));
        let b = ed.add(preset("Database"), (100., 400.));
        let r = ed.connect(&a, &b).unwrap();
        assert_eq!(ed.doc.relations.len(), 1);
        assert_eq!(ed.cur().edges.len(), 1);
        assert_eq!(ed.sel.edges, vec![r]);
        assert!(ed.undo());
        assert!(ed.doc.relations.is_empty());
        assert!(ed.sel.edges.is_empty());
        assert!(ed.redo());
        assert_eq!(ed.doc.relations.len(), 1);
        assert_ne!(a, b);
    }

    #[test]
    fn hits_prefer_shapes_then_connectors_then_boundary_titles() {
        let mut ed = Editor::new(Doc::default());
        let g = ed.add(preset("Boundary"), (300., 300.));
        let a = ed.add(preset("Container"), (200., 250.));
        let b = ed.add(preset("Container"), (200., 450.));
        ed.connect(&a, &b);
        assert_eq!(ed.hit((200., 250.), 4.), Hit::Node(a.clone()));
        // The connector runs down the middle between the two boxes.
        assert!(matches!(ed.hit((200., 350.), 4.), Hit::Edge(_)));
        // Empty space inside the boundary isn't the boundary...
        assert_eq!(ed.hit((420., 300.), 4.), Hit::None);
        // ...but its title strip is.
        let r = ed.rect(&g).unwrap();
        assert_eq!(ed.hit((r.x + 50., r.y + 10.), 4.), Hit::Node(g));
    }

    #[test]
    fn shapes_dropped_in_a_boundary_move_with_it() {
        let mut ed = Editor::new(Doc::default());
        let g = ed.add(preset("VPC / network"), (300., 300.));
        let a = ed.add(preset("Service"), (300., 300.));
        assert_eq!(
            ed.cur().node(&a).unwrap().parent.as_deref(),
            Some(g.as_str())
        );
        ed.sel = Sel {
            nodes: vec![g.clone()],
            edges: vec![],
        };
        assert_eq!(ed.movable(), vec![g.clone(), a.clone()]);
        let before = ed.rect(&a).unwrap();
        ed.nudge((10., 0.));
        assert_eq!(ed.rect(&a).unwrap().x, before.x + 10.);
        // The boundary sits behind what it holds.
        let order: Vec<&str> = ed.cur().nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(order, vec![g.as_str(), a.as_str()]);
    }

    #[test]
    fn delete_keeps_parts_other_diagrams_show() {
        let mut ed = Editor::new(Doc::default());
        let a = ed.add(preset("Container"), (0., 0.));
        let b = ed.add(preset("Container"), (300., 0.));
        ed.connect(&a, &b);
        ed.duplicate_view();
        ed.sel = Sel {
            nodes: vec![a.clone()],
            edges: vec![],
        };
        ed.delete();
        assert!(ed.doc.element(&a).is_some(), "still on the first diagram");
        assert!(ed.cur().edges.is_empty());
        let first = ed.doc.views[0].id.clone();
        ed.open_view(&first);
        ed.sel.nodes = vec![a.clone()];
        ed.delete();
        assert!(ed.doc.element(&a).is_none());
        assert!(ed.doc.relations.is_empty());
    }

    #[test]
    fn copy_paste_and_duplicate_make_new_ids() {
        let mut ed = Editor::new(Doc::default());
        let a = ed.add(preset("Container"), (0., 0.));
        let b = ed.add(preset("Database"), (0., 300.));
        ed.connect(&a, &b);
        ed.select_all();
        assert!(ed.copy());
        ed.paste();
        assert_eq!(ed.doc.elements.len(), 4);
        assert_eq!(ed.doc.relations.len(), 2);
        assert_eq!(ed.cur().edges.len(), 2);
        assert_eq!(ed.sel.nodes.len(), 2);
        assert!(!ed.sel.has_node(&a));
        ed.duplicate();
        assert_eq!(ed.doc.elements.len(), 6);
    }

    #[test]
    fn group_and_ungroup() {
        let mut ed = Editor::new(Doc::default());
        let a = ed.add(preset("Container"), (0., 0.));
        let b = ed.add(preset("Container"), (300., 0.));
        ed.sel.nodes = vec![a.clone(), b.clone()];
        let g = ed.group().unwrap();
        assert_eq!(ed.cur().descendants(&g).len(), 2);
        let gr = ed.rect(&g).unwrap();
        assert!(gr.contains_rect(&ed.rect(&a).unwrap()));
        ed.ungroup();
        assert!(ed.doc.element(&g).is_none());
        assert!(ed.cur().nodes.iter().all(|n| n.parent.is_none()));
        assert_eq!(ed.sel.nodes.len(), 2);
    }

    #[test]
    fn layout_keeps_boundary_contents_inside() {
        let mut ed = Editor::new(Doc::default());
        let web = ed.add(preset("Web app"), (0., 0.));
        let g = ed.add(preset("Cluster"), (1000., 1000.));
        let api = ed.add(preset("Service"), (1000., 1000.));
        let db = ed.add(preset("Database"), (2000., 0.));
        ed.connect(&web, &api);
        ed.connect(&api, &db);
        ed.sel = Sel::default();
        ed.auto_layout(false);
        let (rw, rg, ra, rd) = (
            ed.rect(&web).unwrap(),
            ed.rect(&g).unwrap(),
            ed.rect(&api).unwrap(),
            ed.rect(&db).unwrap(),
        );
        assert!(rg.contains_rect(&ra));
        assert!(rw.bottom() <= rg.y);
        assert!(rg.bottom() <= rd.y);
    }

    #[test]
    fn align_moves_boundary_contents_too() {
        let mut ed = Editor::new(Doc::default());
        let g = ed.add(preset("Boundary"), (500., 500.));
        let inside = ed.add(preset("Container"), (500., 500.));
        let other = ed.add(preset("Container"), (0., 0.));
        let before = ed.rect(&inside).unwrap();
        let gx = ed.rect(&g).unwrap().x;
        ed.sel.nodes = vec![g.clone(), other];
        ed.align(Align::Left);
        let shift = ed.rect(&g).unwrap().x - gx;
        assert!(shift < 0.);
        assert_eq!(ed.rect(&inside).unwrap().x, before.x + shift);
    }

    #[test]
    fn cycles_are_reported_on_connectors() {
        let mut ed = Editor::new(Doc::default());
        let a = ed.add(preset("Service"), (0., 0.));
        let b = ed.add(preset("Service"), (300., 0.));
        let c = ed.add(preset("Database"), (300., 300.));
        let ab = ed.connect(&a, &b).unwrap();
        let ba = ed.connect(&b, &a).unwrap();
        let bc = ed.connect(&b, &c).unwrap();
        let on = ed.cycles();
        assert!(on.contains(&ab) && on.contains(&ba) && !on.contains(&bc));
    }

    #[test]
    fn text_edits_apply_or_explain() {
        let mut ed = Editor::new(Doc::default());
        ed.add(preset("Container"), (0., 0.));
        let model = ed
            .doc
            .model_text()
            .replace("name = \"Container\"", "name = \"Orders API\"");
        let view = Doc::view_text(ed.cur());
        ed.apply_text(&model, &view).unwrap();
        assert_eq!(ed.doc.elements[0].name, "Orders API");
        assert!(ed.apply_text("[[element]]\nid = 1", &view).is_err());
        assert!(ed.undo());
        assert_eq!(ed.doc.elements[0].name, "Container");
    }

    #[test]
    fn views_add_rename_and_delete() {
        let mut ed = Editor::new(Doc::default());
        let id = ed.add_view("Deployment");
        assert_eq!(ed.view, id);
        ed.rename_view("Prod deployment");
        assert_eq!(ed.cur().name, "Prod deployment");
        ed.delete_view();
        assert_eq!(ed.doc.views.len(), 1);
        ed.delete_view();
        assert_eq!(ed.doc.views.len(), 1, "the last diagram stays");
    }
}
