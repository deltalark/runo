//! The diagram editor's state and commands, apart from drawing: selection, undo and
//! redo, adding and connecting shapes, moving, grouping, styling, clipboard and layout.

use super::geom::{self, Align, Pt, Side, Wire};
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
    /// Connector routes for the diagram as it was last asked about, since routing round
    /// boxes is the costly part of drawing a big diagram.
    routes: std::cell::RefCell<Option<(u64, Routes)>>,
    /// Each connector's route and what it was worked out from.
    edge_routes: std::cell::RefCell<HashMap<String, (u64, Wire)>>,
}

/// Every connector of a diagram with its route.
pub(crate) type Routes = Vec<(String, Wire)>;

/// What is out of sight on a diagram: shapes on hidden layers, and shapes folded into a
/// collapsed boundary (with the boundary that stands in for them).
#[derive(Default)]
pub(crate) struct Sight {
    pub hidden: HashSet<String>,
    pub folded: HashMap<String, String>,
}

impl Sight {
    pub(crate) fn shows(&self, id: &str) -> bool {
        !self.hidden.contains(id) && !self.folded.contains_key(id)
    }

    /// The shape a connector to `id` is drawn to: `id`, the collapsed boundary it is
    /// folded into, or none when it's hidden.
    pub(crate) fn stand_in<'a>(&'a self, id: &'a str) -> Option<&'a str> {
        if self.hidden.contains(id) {
            return None;
        }
        Some(self.folded.get(id).map(String::as_str).unwrap_or(id))
    }
}

/// The size of a collapsed boundary.
pub(crate) const COLLAPSED: (f32, f32) = (220., 110.);

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
            routes: Default::default(),
            edge_routes: Default::default(),
        }
    }

    /// What's out of sight on the current diagram.
    pub(crate) fn sight(&self) -> Sight {
        view_sight(self.cur())
    }

    /// Whether a shape can't be moved: locked itself or on a locked layer.
    pub(crate) fn is_locked(&self, n: &Node) -> bool {
        n.locked
            || n.layer
                .as_deref()
                .and_then(|l| self.cur().layer(l))
                .is_some_and(|l| l.locked)
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
    /// Take a model changed outside the editor (an agent's edit), as one undoable step.
    /// Its files are already on disk, so nothing is waiting to be saved.
    pub(crate) fn replace_doc(&mut self, doc: Doc) {
        self.checkpoint();
        self.doc = doc;
        if self.doc.views.is_empty() {
            self.doc.views.push(View {
                id: "main".into(),
                name: "Main".into(),
                ..Default::default()
            });
        }
        self.after_history();
        self.dirty = false;
    }

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
        let label = default_label(self.kind_of(from), self.kind_of(to)).to_string();
        self.doc.relations.push(Relation {
            id: id.clone(),
            from: from.to_string(),
            to: to.to_string(),
            label,
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
        let sight = self.sight();
        let inside: Vec<String> = self
            .cur()
            .nodes
            .iter()
            .filter(|n| sight.shows(&n.id) && area.contains_rect(&n.rect()))
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
        let key = self.route_key();
        if let Some((k, w)) = self.routes.borrow().as_ref() {
            if *k == key {
                return w.clone();
            }
        }
        let w = view_wires_cached(
            &self.doc,
            self.cur(),
            &self.sight(),
            &mut self.edge_routes.borrow_mut(),
        );
        *self.routes.borrow_mut() = Some((key, w.clone()));
        w
    }

    /// A fingerprint of everything a route depends on.
    fn route_key(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let v = self.cur();
        let mut h = std::collections::hash_map::DefaultHasher::new();
        v.id.hash(&mut h);
        let els = self.elements();
        for n in &v.nodes {
            n.id.hash(&mut h);
            for f in [n.x, n.y, n.w, n.h] {
                f.to_bits().hash(&mut h);
            }
            n.parent.hash(&mut h);
            n.layer.hash(&mut h);
            n.collapsed.hash(&mut h);
            els.get(n.id.as_str())
                .map(|e| e.kind == Kind::Boundary)
                .hash(&mut h);
        }
        for l in &v.layers {
            l.name.hash(&mut h);
            l.hidden.hash(&mut h);
        }
        let rels = self.relations();
        for e in &v.edges {
            e.id.hash(&mut h);
            (e.route as u8).hash(&mut h);
            e.from_side.hash(&mut h);
            e.to_side.hash(&mut h);
            for p in &e.waypoints {
                p.0.to_bits().hash(&mut h);
                p.1.to_bits().hash(&mut h);
            }
            if let Some(r) = rels.get(e.id.as_str()) {
                r.from.hash(&mut h);
                r.to.hash(&mut h);
            }
        }
        h.finish()
    }

    /// The model's relations by id, for lookups in loops over a big diagram.
    pub(crate) fn relations(&self) -> HashMap<&str, &Relation> {
        self.doc
            .relations
            .iter()
            .map(|r| (r.id.as_str(), r))
            .collect()
    }

    /// The model's elements by id, for lookups in loops over a big diagram.
    pub(crate) fn elements(&self) -> HashMap<&str, &Element> {
        self.doc
            .elements
            .iter()
            .map(|e| (e.id.as_str(), e))
            .collect()
    }

    /// What's under `p`, with `tol` of slack for thin things.
    pub(crate) fn hit(&self, p: Pt, tol: f32) -> Hit {
        let v = self.cur();
        let sight = self.sight();
        let els = self.elements();
        let kind = |id: &str| els.get(id).map(|e| e.kind).unwrap_or_default();
        let boundary = |n: &Node| kind(&n.id) == Kind::Boundary && !n.collapsed;
        for n in v.nodes.iter().rev() {
            if boundary(n) || !sight.shows(&n.id) {
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
            if !boundary(n) || !sight.shows(&n.id) {
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
        let kids = v.children();
        let nodes: HashMap<&str, &Node> = v.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
        let mut seen: HashSet<&str> = HashSet::new();
        let mut out: Vec<String> = Vec::new();
        let mut queue: Vec<&str> = self.sel.nodes.iter().map(String::as_str).collect();
        while let Some(id) = queue.pop() {
            if !seen.insert(id) {
                continue;
            }
            if nodes.get(id).is_some_and(|n| !self.is_locked(n)) {
                out.push(id.to_string());
            }
            queue.extend(kids.get(id).into_iter().flatten());
        }
        out
    }

    /// Place nodes at new rectangles, without a checkpoint (drags call `checkpoint` once).
    pub(crate) fn set_rects(&mut self, rects: &[(String, Rect)]) {
        let mut moved: HashMap<String, Pt> = HashMap::new();
        let v = self.cur_mut();
        for (id, r) in rects {
            if let Some(n) = v.node_mut(id) {
                let (x, y) = (r.x.round(), r.y.round());
                if (r.w.round().max(10.) - n.w).abs() < 0.5
                    && (r.h.round().max(10.) - n.h).abs() < 0.5
                {
                    moved.insert(id.clone(), (x - n.x, y - n.y));
                }
                n.x = x;
                n.y = y;
                n.w = r.w.round().max(10.);
                n.h = r.h.round().max(10.);
            }
        }
        // Bends placed by hand travel with a connector whose two ends moved together.
        let ends: HashMap<String, (String, String)> = self
            .doc
            .relations
            .iter()
            .map(|r| (r.id.clone(), (r.from.clone(), r.to.clone())))
            .collect();
        for e in &mut self.cur_mut().edges {
            if e.waypoints.is_empty() {
                continue;
            }
            let Some((a, b)) = ends.get(&e.id) else {
                continue;
            };
            if let (Some(da), Some(db)) = (moved.get(a), moved.get(b)) {
                if (da.0 - db.0).abs() < 0.5 && (da.1 - db.1).abs() < 0.5 {
                    for p in &mut e.waypoints {
                        p.0 += da.0;
                        p.1 += da.1;
                    }
                }
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

    /// Give the selected shapes an effect.
    pub(crate) fn effect(&mut self, effect: Effect) {
        if self.sel.nodes.is_empty() {
            return;
        }
        self.checkpoint();
        let ids = self.sel.nodes.clone();
        let v = self.cur_mut();
        for id in &ids {
            if let Some(n) = v.node_mut(id) {
                n.effect = effect;
            }
        }
    }

    /// Turn motion on or off for this diagram.
    pub(crate) fn set_still(&mut self, still: bool) {
        if self.cur().still != still {
            self.checkpoint();
            self.cur_mut().still = still;
        }
    }

    /// A new story from the selected connectors, in the order a request would follow
    /// them: each step starts where one before it ended, where it can.
    pub(crate) fn story_from_selection(&mut self, name: &str) -> Option<usize> {
        let picked: Vec<String> = self
            .sel
            .edges
            .iter()
            .filter(|id| self.cur().edge(id).is_some())
            .cloned()
            .collect();
        if picked.is_empty() {
            return None;
        }
        let ends = |id: &str| {
            self.doc
                .relation(id)
                .map(|r| (r.from.clone(), r.to.clone()))
                .unwrap_or_default()
        };
        let mut left = picked.clone();
        let mut order: Vec<String> = Vec::new();
        // Start from a connector nothing else in the selection leads into.
        let start = left
            .iter()
            .position(|id| {
                let (from, _) = ends(id);
                !left.iter().any(|o| o != id && ends(o).1 == from)
            })
            .unwrap_or(0);
        order.push(left.remove(start));
        while !left.is_empty() {
            let (_, at) = ends(order.last().unwrap());
            let next = left.iter().position(|id| ends(id).0 == at).unwrap_or(0);
            order.push(left.remove(next));
        }
        self.checkpoint();
        let steps = order
            .into_iter()
            .map(|edge| Step {
                caption: self
                    .doc
                    .relation(&edge)
                    .map(|r| r.label.clone())
                    .unwrap_or_default(),
                edge,
            })
            .collect();
        let v = self.cur_mut();
        v.stories.push(Story {
            name: name.to_string(),
            steps,
        });
        Some(v.stories.len() - 1)
    }

    pub(crate) fn rename_story(&mut self, i: usize, name: &str) {
        let name = name.trim();
        if name.is_empty() || self.cur().stories.get(i).is_none_or(|s| s.name == name) {
            return;
        }
        self.checkpoint();
        self.cur_mut().stories[i].name = name.to_string();
    }

    pub(crate) fn delete_story(&mut self, i: usize) {
        if i < self.cur().stories.len() {
            self.checkpoint();
            self.cur_mut().stories.remove(i);
        }
    }

    /// Add the selected connectors to the end of a story.
    pub(crate) fn extend_story(&mut self, i: usize) {
        let ids: Vec<String> = self.sel.edges.clone();
        if ids.is_empty() || i >= self.cur().stories.len() {
            return;
        }
        self.checkpoint();
        let captions: Vec<String> = ids
            .iter()
            .map(|id| {
                self.doc
                    .relation(id)
                    .map(|r| r.label.clone())
                    .unwrap_or_default()
            })
            .collect();
        let story = &mut self.cur_mut().stories[i];
        for (edge, caption) in ids.into_iter().zip(captions) {
            story.steps.push(Step { edge, caption });
        }
    }

    pub(crate) fn remove_step(&mut self, i: usize, step: usize) {
        if self
            .cur()
            .stories
            .get(i)
            .is_some_and(|s| step < s.steps.len())
        {
            self.checkpoint();
            self.cur_mut().stories[i].steps.remove(step);
        }
    }

    pub(crate) fn set_caption(&mut self, i: usize, step: usize, caption: &str) {
        let same = self
            .cur()
            .stories
            .get(i)
            .and_then(|s| s.steps.get(step))
            .is_none_or(|s| s.caption == caption);
        if !same {
            self.checkpoint();
            self.cur_mut().stories[i].steps[step].caption = caption.to_string();
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
        let sight = self.sight();
        Rect::bounds(
            self.cur()
                .nodes
                .iter()
                .filter(|n| sight.shows(&n.id))
                .map(|n| n.rect()),
        )
    }

    // ---- Layers

    /// Add a layer and put the selected shapes on it.
    pub(crate) fn add_layer(&mut self, name: &str) -> String {
        self.checkpoint();
        let v = self.cur_mut();
        let mut name = name.trim().to_string();
        if name.is_empty() {
            name = "Layer".into();
        }
        let base = name.clone();
        let mut i = 2;
        while v.layer(&name).is_some() {
            name = format!("{base} {i}");
            i += 1;
        }
        v.layers.push(Layer {
            name: name.clone(),
            ..Default::default()
        });
        let sel = self.sel.nodes.clone();
        for id in sel {
            if let Some(n) = self.cur_mut().node_mut(&id) {
                n.layer = Some(name.clone());
            }
        }
        self.dirty = true;
        name
    }

    /// Put the selected shapes on a layer, or on the base layer with none.
    pub(crate) fn set_layer(&mut self, name: Option<&str>) {
        self.checkpoint();
        let sel = self.sel.nodes.clone();
        for id in sel {
            if let Some(n) = self.cur_mut().node_mut(&id) {
                n.layer = name.map(String::from);
            }
        }
        self.dirty = true;
    }

    pub(crate) fn rename_layer(&mut self, i: usize, name: &str) {
        let name = name.trim();
        let v = self.cur();
        if name.is_empty()
            || v.layers
                .iter()
                .enumerate()
                .any(|(j, l)| j != i && l.name == name)
        {
            return;
        }
        let Some(old) = v.layers.get(i).map(|l| l.name.clone()) else {
            return;
        };
        self.checkpoint();
        let v = self.cur_mut();
        v.layers[i].name = name.to_string();
        for n in &mut v.nodes {
            if n.layer.as_deref() == Some(old.as_str()) {
                n.layer = Some(name.to_string());
            }
        }
        self.dirty = true;
    }

    /// Remove a layer; its shapes go back to the base layer.
    pub(crate) fn delete_layer(&mut self, i: usize) {
        if i >= self.cur().layers.len() {
            return;
        }
        self.checkpoint();
        let v = self.cur_mut();
        let old = v.layers.remove(i).name;
        for n in &mut v.nodes {
            if n.layer.as_deref() == Some(old.as_str()) {
                n.layer = None;
            }
        }
        self.dirty = true;
    }

    /// Show or hide, lock or unlock a layer.
    pub(crate) fn layer_flags(&mut self, i: usize, hidden: Option<bool>, locked: Option<bool>) {
        if i >= self.cur().layers.len() {
            return;
        }
        self.checkpoint();
        let sight_before: Vec<String> = self.sel.nodes.clone();
        let v = self.cur_mut();
        let l = &mut v.layers[i];
        if let Some(h) = hidden {
            l.hidden = h;
        }
        if let Some(k) = locked {
            l.locked = k;
        }
        // Nothing hidden stays selected.
        let sight = self.sight();
        self.sel.nodes = sight_before
            .into_iter()
            .filter(|id| sight.shows(id))
            .collect();
        let shown: HashSet<String> = self.wires().into_iter().map(|(id, _)| id).collect();
        self.sel.edges.retain(|id| shown.contains(id));
        self.dirty = true;
    }

    // ---- Collapsing boundaries

    /// Fold a boundary to one box, or open it back to its size.
    pub(crate) fn toggle_collapse(&mut self, id: &str) {
        if self.kind_of(id) != Kind::Boundary {
            return;
        }
        self.checkpoint();
        let Some(n) = self.cur_mut().node_mut(id) else {
            return;
        };
        if n.collapsed {
            let (w, h) = n.open_size.take().unwrap_or((n.w, n.h));
            n.w = w;
            n.h = h;
            n.collapsed = false;
        } else {
            n.open_size = Some((n.w, n.h));
            n.w = COLLAPSED.0;
            n.h = COLLAPSED.1;
            n.collapsed = true;
        }
        let sight = self.sight();
        self.sel.nodes.retain(|id| sight.shows(id));
        let shown: HashSet<String> = self.wires().into_iter().map(|(id, _)| id).collect();
        self.sel.edges.retain(|id| shown.contains(id));
        self.dirty = true;
    }

    // ---- Connector paths

    /// Pin the selected connectors to sides, or let them pick (None leaves a side as is).
    pub(crate) fn pin_sides(&mut self, from: Option<Option<Side>>, to: Option<Option<Side>>) {
        self.edges(|e| {
            if let Some(f) = from {
                e.from_side = f;
            }
            if let Some(t) = to {
                e.to_side = t;
            }
        });
    }

    /// Set a connector's hand-placed bends, without a checkpoint (drags call it once).
    pub(crate) fn set_waypoints(&mut self, id: &str, points: Vec<Pt>) {
        if let Some(e) = self.cur_mut().edge_mut(id) {
            e.waypoints = points
                .into_iter()
                .map(|p| (p.0.round(), p.1.round()))
                .collect();
            self.dirty = true;
        }
    }

    /// Let the selected connectors find their own way again.
    pub(crate) fn reset_paths(&mut self) {
        self.edges(|e| {
            e.waypoints.clear();
            e.from_side = None;
            e.to_side = None;
        });
    }

    // ---- Finding

    /// Shapes on this diagram matching `query` by name, technology, description or tag,
    /// in reading order. `#tag` matches a tag exactly.
    pub(crate) fn find(&self, query: &str) -> Vec<String> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }
        let els = self.elements();
        let sight = self.sight();
        let mut hits: Vec<(&Node, &Element)> = self
            .cur()
            .nodes
            .iter()
            .filter_map(|n| Some((n, *els.get(n.id.as_str())?)))
            .filter(|(n, e)| {
                if !sight.hidden.contains(&n.id) {
                    if let Some(tag) = q.strip_prefix('#') {
                        e.tags.iter().any(|t| t.to_lowercase() == tag)
                    } else {
                        [&e.name, &e.tech, &e.description, &e.id]
                            .iter()
                            .any(|f| f.to_lowercase().contains(&q))
                            || e.tags.iter().any(|t| t.to_lowercase().contains(&q))
                    }
                } else {
                    false
                }
            })
            .collect();
        hits.sort_by(|a, b| a.0.y.total_cmp(&b.0.y).then(a.0.x.total_cmp(&b.0.x)));
        hits.into_iter().map(|(n, _)| n.id.clone()).collect()
    }

    /// Open every collapsed boundary that hides `id`, so it can be shown.
    pub(crate) fn reveal(&mut self, id: &str) {
        let sight = self.sight();
        let Some(outer) = sight.folded.get(id).cloned() else {
            return;
        };
        let mut chain = Vec::new();
        let v = self.cur();
        let mut at = v.node(id).and_then(|n| n.parent.clone());
        while let Some(p) = at {
            if p == outer {
                chain.push(p);
                break;
            }
            at = v.node(&p).and_then(|n| n.parent.clone());
            chain.push(p);
        }
        for p in chain {
            if self.cur().node(&p).is_some_and(|n| n.collapsed) {
                self.toggle_collapse(&p);
            }
        }
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
        let rels = self.relations();
        let els = self.elements();
        let part = |id: &str| els.get(id).is_some_and(|e| e.kind.is_part());
        let links: Vec<(String, String)> = v
            .edges
            .iter()
            .filter_map(|e| rels.get(e.id.as_str()))
            .filter(|r| part(&r.from) && part(&r.to))
            .map(|r| (r.from.clone(), r.to.clone()))
            .collect();
        let on = geom::cyclic_links(&links);
        v.edges
            .iter()
            .filter_map(|e| rels.get(e.id.as_str()))
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

/// How a diagram changed between two versions of the model, for previewing an agent's edit.
#[derive(Default)]
pub(crate) struct Changes {
    /// Shapes new on the diagram.
    pub added: Vec<String>,
    /// Shapes gone from it: where they were and what they were called.
    pub removed: Vec<(Rect, String)>,
    /// Shapes moved or resized: where they were.
    pub moved: Vec<(String, Rect)>,
    /// Shapes whose part was renamed or redescribed.
    pub edited: Vec<String>,
    pub edges_added: Vec<String>,
    /// Connectors gone: the routes they took.
    pub edges_removed: Vec<Wire>,
    /// Diagrams added or removed elsewhere in the model.
    pub views_added: usize,
    pub views_removed: usize,
}

impl Changes {
    pub(crate) fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.removed.is_empty()
            && self.moved.is_empty()
            && self.edited.is_empty()
            && self.edges_added.is_empty()
            && self.edges_removed.is_empty()
            && self.views_added == 0
            && self.views_removed == 0
    }

    /// A short account: "2 added, 1 removed, 3 moved".
    pub(crate) fn summary(&self) -> String {
        let mut parts = Vec::new();
        let mut say = |n: usize, what: &str| {
            if n > 0 {
                parts.push(format!("{n} {what}"));
            }
        };
        say(self.added.len(), "added");
        say(self.removed.len(), "removed");
        say(self.moved.len(), "moved");
        say(self.edited.len(), "edited");
        let edges = self.edges_added.len() + self.edges_removed.len();
        say(
            edges,
            if edges == 1 {
                "connector changed"
            } else {
                "connectors changed"
            },
        );
        say(
            self.views_added,
            if self.views_added == 1 {
                "new diagram"
            } else {
                "new diagrams"
            },
        );
        say(self.views_removed, "diagrams removed");
        if parts.is_empty() {
            "nothing visible on this diagram".into()
        } else {
            parts.join(", ")
        }
    }
}

/// What changed on diagram `view` from `before` to `after`.
pub(crate) fn changes(before: &Doc, after: &Doc, view: &str) -> Changes {
    let mut out = Changes::default();
    let ids = |d: &Doc| -> HashSet<String> { d.views.iter().map(|v| v.id.clone()).collect() };
    let (bv, av) = (ids(before), ids(after));
    out.views_added = av.difference(&bv).count();
    out.views_removed = bv.difference(&av).count();
    let empty = View::default();
    let b = before.views.iter().find(|v| v.id == view).unwrap_or(&empty);
    let a = after.views.iter().find(|v| v.id == view).unwrap_or(&empty);
    let bn: HashMap<&str, &Node> = b.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let an: HashMap<&str, &Node> = a.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let name = |d: &Doc, id: &str| d.element(id).map(|e| e.name.clone()).unwrap_or_default();
    for n in &a.nodes {
        match bn.get(n.id.as_str()) {
            None => out.added.push(n.id.clone()),
            Some(old) => {
                if old.rect() != n.rect() {
                    out.moved.push((n.id.clone(), old.rect()));
                }
                let (e0, e1) = (before.element(&n.id), after.element(&n.id));
                if let (Some(e0), Some(e1)) = (e0, e1) {
                    if e0 != e1 || old.style != n.style || old.effect != n.effect {
                        out.edited.push(n.id.clone());
                    }
                }
            }
        }
    }
    for n in &b.nodes {
        if !an.contains_key(n.id.as_str()) {
            out.removed.push((n.rect(), name(before, &n.id)));
        }
    }
    let be: HashSet<&str> = b.edges.iter().map(|e| e.id.as_str()).collect();
    let ae: HashSet<&str> = a.edges.iter().map(|e| e.id.as_str()).collect();
    out.edges_added = a
        .edges
        .iter()
        .filter(|e| !be.contains(e.id.as_str()))
        .map(|e| e.id.clone())
        .collect();
    let gone: HashSet<&str> = be.difference(&ae).copied().collect();
    if !gone.is_empty() {
        out.edges_removed = view_wires(before, b, &view_sight(b))
            .into_iter()
            .filter(|(id, _)| gone.contains(id.as_str()))
            .map(|(_, w)| w)
            .collect();
    }
    out
}

/// What a new connector between two parts of these kinds most likely means; nothing
/// for drawing shapes, whose meaning the canvas can't guess.
pub(crate) fn default_label(from: Kind, to: Kind) -> &'static str {
    if from.c4().is_none() || to.c4().is_none() {
        return "";
    }
    match (from, to) {
        (Kind::Person, _) => "Uses",
        (_, Kind::Database) => "Reads and writes",
        (_, Kind::Queue) => "Publishes to",
        (Kind::Queue, _) => "Delivers to",
        (_, Kind::Person) => "Notifies",
        _ => "Calls",
    }
}

/// Something on a diagram worth a second look, and what it's about.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Finding {
    pub text: String,
    pub nodes: Vec<String>,
    pub edge: Option<String>,
}

impl Editor {
    /// The nearest place to `at` where a shape of `size` sits clear of the other parts
    /// (boundaries don't count: shapes go inside them).
    pub(crate) fn free_spot(&self, at: Pt, size: (f32, f32)) -> Pt {
        let sight = self.sight();
        let taken: Vec<Rect> = self
            .cur()
            .nodes
            .iter()
            .filter(|n| sight.shows(&n.id) && self.kind_of(&n.id) != Kind::Boundary)
            .map(|n| n.rect().inflate(20.))
            .collect();
        let clear = |p: Pt| {
            let r = Rect::new(p.0 - size.0 / 2., p.1 - size.1 / 2., size.0, size.1);
            !taken.iter().any(|o| o.intersects(&r))
        };
        if clear(at) {
            return at;
        }
        for ring in 1..40 {
            let d = ring as f32 * 40.;
            let steps = ring * 8;
            for i in 0..steps {
                let a = i as f32 / steps as f32 * std::f32::consts::TAU;
                let p = (at.0 + d * a.cos(), at.1 + d * a.sin());
                if clear(p) {
                    return p;
                }
            }
        }
        at
    }

    /// Checks of the diagram open now: parts on their own, parts and connectors missing
    /// what a reader needs, and parts drawn over each other.
    pub(crate) fn findings(&self) -> Vec<Finding> {
        let v = self.cur();
        let sight = self.sight();
        let name = |id: &str| {
            self.doc
                .element(id)
                .map(|e| e.name.replace('\n', " "))
                .unwrap_or_else(|| id.to_string())
        };
        let mut out = Vec::new();
        let mut touched = HashSet::new();
        for e in &v.edges {
            let Some(r) = self.doc.relation(&e.id) else {
                continue;
            };
            touched.insert(r.from.as_str());
            touched.insert(r.to.as_str());
            if r.label.trim().is_empty() && sight.shows(&r.from) && sight.shows(&r.to) {
                out.push(Finding {
                    text: format!(
                        "The connector from {} to {} says nothing about what it carries",
                        name(&r.from),
                        name(&r.to)
                    ),
                    nodes: Vec::new(),
                    edge: Some(e.id.clone()),
                });
            }
        }
        let parts: Vec<&Node> = v
            .nodes
            .iter()
            .filter(|n| sight.shows(&n.id) && self.kind_of(&n.id).is_part())
            .collect();
        for n in &parts {
            let Some(el) = self.doc.element(&n.id) else {
                continue;
            };
            if !touched.contains(n.id.as_str()) {
                out.push(Finding {
                    text: format!("{} isn't connected to anything", name(&n.id)),
                    nodes: vec![n.id.clone()],
                    edge: None,
                });
            }
            let built = matches!(
                el.kind,
                Kind::Container | Kind::Component | Kind::Database | Kind::Queue
            );
            if built && el.tech.trim().is_empty() {
                out.push(Finding {
                    text: format!("{} doesn't say what it's built with", name(&n.id)),
                    nodes: vec![n.id.clone()],
                    edge: None,
                });
            }
        }
        // Overlaps, among a sensible number of parts.
        if parts.len() <= 400 {
            for (i, a) in parts.iter().enumerate() {
                for b in &parts[i + 1..] {
                    let nested = a.parent.as_deref() == Some(b.id.as_str())
                        || b.parent.as_deref() == Some(a.id.as_str());
                    if !nested && a.rect().intersects(&b.rect()) {
                        out.push(Finding {
                            text: format!("{} and {} overlap", name(&a.id), name(&b.id)),
                            nodes: vec![a.id.clone(), b.id.clone()],
                            edge: None,
                        });
                    }
                }
            }
        }
        out
    }
}

/// What's out of sight on a diagram.
pub(crate) fn view_sight(v: &View) -> Sight {
    let mut sight = Sight::default();
    let hidden_layers: HashSet<&str> = v
        .layers
        .iter()
        .filter(|l| l.hidden)
        .map(|l| l.name.as_str())
        .collect();
    let nodes: HashMap<&str, &Node> = v.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    for n in &v.nodes {
        if n.layer
            .as_deref()
            .is_some_and(|l| hidden_layers.contains(l))
        {
            sight.hidden.insert(n.id.clone());
        }
        // The outermost collapsed boundary it sits in, if any.
        let mut at = n.parent.as_deref();
        let mut outer = None;
        for _ in 0..64 {
            let Some(p) = at.and_then(|p| nodes.get(p)) else {
                break;
            };
            if p.collapsed {
                outer = Some(p.id.clone());
            }
            at = p.parent.as_deref();
        }
        if let Some(o) = outer {
            sight.folded.insert(n.id.clone(), o);
        }
    }
    sight
}

/// Every shown connector of a diagram with its route: to a collapsed boundary for parts
/// folded into one, round the boxes in its way, through bends placed by hand.
pub(crate) fn view_wires(doc: &Doc, v: &View, sight: &Sight) -> Vec<(String, Wire)> {
    view_wires_cached(doc, v, sight, &mut HashMap::new())
}

/// Connectors that pass more boxes than this keep their plain route: going round that
/// many would cost more than it helps on a crowded diagram.
const MAX_OBSTACLES: usize = 60;

/// Boxes bucketed by area, so a connector finds the ones near it without looking at all.
struct Buckets<'a> {
    cell: f32,
    map: HashMap<(i32, i32), Vec<(&'a str, Rect)>>,
}

impl<'a> Buckets<'a> {
    const CELL: f32 = 400.;

    fn new(items: impl Iterator<Item = (&'a str, Rect)>) -> Self {
        let mut map: HashMap<(i32, i32), Vec<(&'a str, Rect)>> = HashMap::new();
        for (id, r) in items {
            for key in Self::keys(&r) {
                map.entry(key).or_default().push((id, r));
            }
        }
        Self {
            cell: Self::CELL,
            map,
        }
    }

    fn keys(r: &Rect) -> impl Iterator<Item = (i32, i32)> {
        let c = Self::CELL;
        let (x0, y0) = ((r.x / c).floor() as i32, (r.y / c).floor() as i32);
        let (x1, y1) = (
            (r.right() / c).floor() as i32,
            (r.bottom() / c).floor() as i32,
        );
        (x0..=x1).flat_map(move |x| (y0..=y1).map(move |y| (x, y)))
    }

    /// Boxes touching `area`, each once, in a stable order.
    fn near(&self, area: &Rect) -> Vec<(&'a str, Rect)> {
        let _ = self.cell;
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for key in Self::keys(area) {
            for (id, r) in self.map.get(&key).into_iter().flatten() {
                if r.intersects(area) && seen.insert(*id) {
                    out.push((*id, *r));
                }
            }
        }
        out.sort_by(|a, b| a.0.cmp(b.0));
        out
    }
}

/// Like `view_wires`, reusing routes in `cache` whose ends, settings and nearby boxes
/// haven't changed, so dragging one shape on a big diagram reroutes only what it touches.
pub(crate) fn view_wires_cached(
    doc: &Doc,
    v: &View,
    sight: &Sight,
    cache: &mut HashMap<String, (u64, Wire)>,
) -> Vec<(String, Wire)> {
    use std::hash::{Hash, Hasher};
    let rects: HashMap<&str, Rect> = v.nodes.iter().map(|n| (n.id.as_str(), n.rect())).collect();
    let rels: HashMap<&str, &Relation> = doc.relations.iter().map(|r| (r.id.as_str(), r)).collect();
    let kinds: HashMap<&str, Kind> = doc
        .elements
        .iter()
        .map(|e| (e.id.as_str(), e.kind))
        .collect();
    // What a connector may need to go round: shown boxes, and collapsed boundaries.
    let solid = Buckets::new(
        v.nodes
            .iter()
            .filter(|n| sight.shows(&n.id))
            .filter(|n| {
                n.collapsed
                    || kinds
                        .get(n.id.as_str())
                        .is_some_and(|k| *k != Kind::Boundary)
            })
            .map(|n| (n.id.as_str(), n.rect())),
    );
    let mut drawn: HashSet<(String, String)> = HashSet::new();
    let mut used = HashSet::new();
    let out: Vec<(String, Wire)> = v
        .edges
        .iter()
        .filter_map(|e| {
            let r = rels.get(e.id.as_str())?;
            let (fa, fb) = (sight.stand_in(&r.from)?, sight.stand_in(&r.to)?);
            let folded = fa != r.from || fb != r.to;
            if folded && (fa == fb || !drawn.insert((fa.to_string(), fb.to_string()))) {
                // Inside one collapsed boundary, or already drawn between the same boxes.
                return None;
            }
            let a = rects.get(fa)?;
            let b = rects.get(fb)?;
            let mut obstacles: Vec<Rect> = if e.route == Route::Elbow && e.waypoints.is_empty() {
                solid
                    .near(&a.union(b).inflate(120.))
                    .into_iter()
                    .filter(|(id, _)| *id != fa && *id != fb)
                    // A box the connector starts or ends inside isn't in its way.
                    .filter(|(_, r)| !r.contains_rect(a) && !r.contains_rect(b))
                    .map(|(_, r)| r)
                    .collect()
            } else {
                Vec::new()
            };
            if obstacles.len() > MAX_OBSTACLES {
                obstacles.clear();
            }
            let waypoints: &[Pt] = if folded { &[] } else { &e.waypoints };
            let mut h = std::collections::hash_map::DefaultHasher::new();
            for r in [a, b].into_iter().chain(&obstacles) {
                for f in [r.x, r.y, r.w, r.h] {
                    f.to_bits().hash(&mut h);
                }
            }
            (e.route as u8, e.from_side, e.to_side).hash(&mut h);
            for p in waypoints {
                (p.0.to_bits(), p.1.to_bits()).hash(&mut h);
            }
            let key = h.finish();
            used.insert(e.id.clone());
            if let Some((k, w)) = cache.get(&e.id) {
                if *k == key {
                    return Some((e.id.clone(), w.clone()));
                }
            }
            let way = geom::Way {
                from: e.from_side,
                to: e.to_side,
                waypoints,
                obstacles: &obstacles,
            };
            let w = geom::route_with(a, b, e.route, &way);
            cache.insert(e.id.clone(), (key, w.clone()));
            Some((e.id.clone(), w))
        })
        .collect();
    cache.retain(|id, _| used.contains(id));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preset(label: &str) -> &'static Preset {
        PRESETS.iter().find(|p| p.label == label).unwrap()
    }

    #[test]
    fn stories_follow_the_request_and_lose_deleted_steps() {
        let mut ed = Editor::new(Doc::default());
        let a = ed.add(preset("Person"), (0., 0.));
        let b = ed.add(preset("Container"), (0., 300.));
        let c = ed.add(preset("Database"), (0., 600.));
        let ab = ed.connect(&a, &b).unwrap();
        let bc = ed.connect(&b, &c).unwrap();
        // Picked in the wrong order, played in the request's order.
        ed.sel = Sel {
            nodes: vec![],
            edges: vec![bc.clone(), ab.clone()],
        };
        let i = ed.story_from_selection("Read").unwrap();
        let steps: Vec<&str> = ed.cur().stories[i]
            .steps
            .iter()
            .map(|s| s.edge.as_str())
            .collect();
        assert_eq!(steps, vec![ab.as_str(), bc.as_str()]);
        ed.set_caption(i, 0, "Opens the app");
        assert_eq!(ed.cur().stories[i].steps[0].caption, "Opens the app");
        ed.sel = Sel {
            nodes: vec![c.clone()],
            edges: vec![],
        };
        ed.effect(Effect::Pulse);
        assert_eq!(ed.cur().node(&c).unwrap().effect, Effect::Pulse);
        ed.delete();
        assert_eq!(ed.cur().stories[i].steps.len(), 1);
        ed.set_still(true);
        assert!(ed.cur().still);
        assert!(ed.undo());
        assert!(!ed.cur().still);
    }

    #[test]
    fn new_connectors_are_labelled_and_findings_point_at_gaps() {
        let mut ed = Editor::new(Doc::default());
        let p = ed.add(preset("Person"), (0., 0.));
        let a = ed.add(preset("Container"), (0., 300.));
        let d = ed.add(preset("Database"), (0., 600.));
        let lone = ed.add(preset("Container"), (600., 300.));
        let note = ed.add(preset("Sticky note"), (600., 600.));
        let pa = ed.connect(&p, &a).unwrap();
        let ad = ed.connect(&a, &d).unwrap();
        assert_eq!(ed.doc.relation(&pa).unwrap().label, "Uses");
        assert_eq!(ed.doc.relation(&ad).unwrap().label, "Reads and writes");
        let pn = ed.connect(&p, &note).unwrap();
        assert_eq!(ed.doc.relation(&pn).unwrap().label, "");
        let f = ed.findings();
        let about = |id: &str| {
            f.iter()
                .filter(|x| x.nodes.contains(&id.to_string()))
                .count()
        };
        assert!(f.iter().any(|x| x.edge.as_deref() == Some(pn.as_str())));
        assert!(f
            .iter()
            .any(|x| x.text.contains("isn't connected") && x.nodes == [lone.clone()]));
        // No technology on the containers and the database; a note needs none.
        assert_eq!(about(&note), 0);
        assert!(about(&a) >= 1 && about(&d) >= 1);
        // A free spot keeps clear of what's there.
        let r = ed.rect(&a).unwrap();
        let (x, y) = ed.free_spot(r.center(), (r.w, r.h));
        let spot = Rect::new(x - r.w / 2., y - r.h / 2., r.w, r.h);
        assert!(!ed.cur().nodes.iter().any(|n| n.rect().intersects(&spot)));
        // Drawn over another part.
        ed.set_rects(&[(lone.clone(), ed.rect(&a).unwrap())]);
        assert!(ed.findings().iter().any(|x| x.text.ends_with("overlap")));
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

    #[test]
    fn layers_hide_and_lock_their_shapes() {
        let mut ed = Editor::new(Doc::default());
        let a = ed.add(preset("Service"), (0., 0.));
        let b = ed.add(preset("Database"), (0., 300.));
        let r = ed.connect(&a, &b).unwrap();
        ed.sel = Sel {
            nodes: vec![b.clone()],
            edges: vec![],
        };
        let layer = ed.add_layer("Data");
        assert_eq!(
            ed.cur().node(&b).unwrap().layer.as_deref(),
            Some(layer.as_str())
        );
        ed.layer_flags(0, Some(true), None);
        assert!(!ed.sight().shows(&b));
        assert!(
            ed.wires().is_empty(),
            "a connector to a hidden shape hides too"
        );
        assert!(ed.sel.nodes.is_empty());
        assert_eq!(ed.hit((0., 300.), 2.), Hit::None);
        assert!(ed.find("database").is_empty());
        ed.layer_flags(0, Some(false), Some(true));
        assert_eq!(ed.wires().len(), 1);
        ed.sel = Sel {
            nodes: vec![b.clone()],
            edges: vec![],
        };
        assert!(ed.movable().is_empty(), "a locked layer's shapes stay put");
        ed.rename_layer(0, "Stores");
        assert_eq!(ed.cur().node(&b).unwrap().layer.as_deref(), Some("Stores"));
        ed.delete_layer(0);
        assert!(ed.cur().node(&b).unwrap().layer.is_none());
        let _ = r;
    }

    #[test]
    fn collapsed_boundaries_stand_in_for_what_they_hold() {
        let mut ed = Editor::new(Doc::default());
        let g = ed.add(preset("VPC / network"), (300., 300.));
        let a = ed.add(preset("Service"), (300., 300.));
        let out = ed.add(preset("Database"), (900., 300.));
        let r = ed.connect(&a, &out).unwrap();
        let open = ed.rect(&g).unwrap();
        ed.toggle_collapse(&g);
        let n = ed.cur().node(&g).unwrap();
        assert!(n.collapsed);
        assert_eq!((n.w, n.h), COLLAPSED);
        assert!(!ed.sight().shows(&a));
        // The connector now runs from the collapsed boundary.
        let wires = ed.wires();
        let (_, w) = wires.iter().find(|(id, _)| *id == r).unwrap();
        assert!(ed.rect(&g).unwrap().inflate(1.).contains(w.points[0]));
        assert_eq!(ed.find("service"), vec![a.clone()]);
        ed.reveal(&a);
        assert!(ed.sight().shows(&a));
        assert_eq!(ed.rect(&g).unwrap(), open);
    }

    #[test]
    fn hand_placed_bends_move_with_both_ends() {
        let mut ed = Editor::new(Doc::default());
        let a = ed.add(preset("Service"), (0., 0.));
        let b = ed.add(preset("Database"), (400., 300.));
        let r = ed.connect(&a, &b).unwrap();
        ed.set_waypoints(&r, vec![(200., 50.)]);
        ed.sel = Sel {
            nodes: vec![a.clone(), b.clone()],
            edges: vec![],
        };
        ed.nudge((10., 20.));
        assert_eq!(ed.cur().edge(&r).unwrap().waypoints, vec![(210., 70.)]);
        ed.sel.nodes = vec![a.clone()];
        ed.nudge((10., 0.));
        assert_eq!(ed.cur().edge(&r).unwrap().waypoints, vec![(210., 70.)]);
        ed.sel = Sel {
            nodes: vec![],
            edges: vec![r.clone()],
        };
        ed.reset_paths();
        assert!(ed.cur().edge(&r).unwrap().waypoints.is_empty());
    }

    #[test]
    fn changes_between_versions_are_counted() {
        let mut ed = Editor::new(Doc::default());
        let a = ed.add(preset("Service"), (0., 0.));
        let b = ed.add(preset("Database"), (0., 300.));
        ed.connect(&a, &b).unwrap();
        let before = ed.doc.clone();
        let c = ed.add(preset("Service"), (400., 0.));
        ed.sel = Sel {
            nodes: vec![b.clone()],
            edges: vec![],
        };
        ed.delete();
        ed.sel = Sel {
            nodes: vec![a.clone()],
            edges: vec![],
        };
        ed.nudge((30., 0.));
        ed.doc.element_mut(&a).unwrap().name = "Orders".into();
        let view = ed.view.clone();
        let ch = changes(&before, &ed.doc, &view);
        assert_eq!(ch.added, vec![c]);
        assert_eq!(ch.removed.len(), 1);
        assert_eq!(ch.moved.len(), 1);
        assert_eq!(ch.edited, vec![a]);
        assert_eq!(ch.edges_removed.len(), 1);
        assert_eq!(
            ch.summary(),
            "1 added, 1 removed, 1 moved, 1 edited, 1 connector changed"
        );
        let mut undo = Editor::new(before.clone());
        undo.replace_doc(ed.doc.clone());
        assert!(!undo.dirty);
        assert!(undo.undo());
        assert_eq!(undo.doc, before);
    }
}
