//! The architecture model and its diagrams, kept as readable TOML in `architecture/`.
//!
//! The model (`model.toml`) holds the elements and the relationships between them. Each
//! diagram (`views/<id>.toml`) says which elements it shows, where, and how they look, so
//! renaming an element once updates every diagram it appears on.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The folder in the project that holds the model and its diagrams.
pub(crate) const FOLDER: &str = "architecture";
const MODEL: &str = "model.toml";
const VIEWS: &str = "views";
const README_FILE: &str = "README.md";
/// The guide to the format written next to the files.
pub(crate) const README: &str = include_str!("README.md");
const README_MARK: &str = "<!-- Written by Runo.";

/// What an element is. Decides its default shape, colour and size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Kind {
    Person,
    System,
    #[default]
    Container,
    Component,
    Database,
    Queue,
    External,
    Boundary,
    Box,
    Rounded,
    Ellipse,
    Diamond,
    Cloud,
    Note,
    Text,
    /// A logo or picture with its name under it, like a cloud provider's service icon.
    Icon,
    /// A standing figure with the name under it.
    Actor,
    /// A group of people.
    Users,
    /// A browser window.
    Browser,
    /// A phone.
    Phone,
    /// A desktop computer's screen on its stand.
    Desktop,
    /// A server.
    Server,
    /// A page with a wavy foot.
    Document,
    /// A folder.
    Folder,
    Hexagon,
    Triangle,
    /// Data in or out, as flowcharts draw it.
    Parallelogram,
    /// A flowchart's start or end.
    Terminator,
    /// Security, a firewall or a policy.
    Shield,
    /// A speech bubble.
    Callout,
    /// A block arrow pointing right.
    Arrow,
}

impl Kind {
    pub(crate) const ALL: [Kind; 31] = [
        Kind::Person,
        Kind::System,
        Kind::Container,
        Kind::Component,
        Kind::Database,
        Kind::Queue,
        Kind::External,
        Kind::Boundary,
        Kind::Box,
        Kind::Rounded,
        Kind::Ellipse,
        Kind::Diamond,
        Kind::Cloud,
        Kind::Note,
        Kind::Text,
        Kind::Icon,
        Kind::Actor,
        Kind::Users,
        Kind::Browser,
        Kind::Phone,
        Kind::Desktop,
        Kind::Server,
        Kind::Document,
        Kind::Folder,
        Kind::Hexagon,
        Kind::Triangle,
        Kind::Parallelogram,
        Kind::Terminator,
        Kind::Shield,
        Kind::Callout,
        Kind::Arrow,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Kind::Person => "Person",
            Kind::System => "System",
            Kind::Container => "Container",
            Kind::Component => "Component",
            Kind::Database => "Database",
            Kind::Queue => "Queue",
            Kind::External => "External system",
            Kind::Boundary => "Boundary",
            Kind::Box => "Box",
            Kind::Rounded => "Rounded box",
            Kind::Ellipse => "Ellipse",
            Kind::Diamond => "Decision",
            Kind::Cloud => "Cloud",
            Kind::Note => "Sticky note",
            Kind::Text => "Text",
            Kind::Icon => "Icon",
            Kind::Actor => "Actor",
            Kind::Users => "Users",
            Kind::Browser => "Browser",
            Kind::Phone => "Phone",
            Kind::Desktop => "Desktop",
            Kind::Server => "Server",
            Kind::Document => "Document",
            Kind::Folder => "Folder",
            Kind::Hexagon => "Hexagon",
            Kind::Triangle => "Triangle",
            Kind::Parallelogram => "Data",
            Kind::Terminator => "Start or end",
            Kind::Shield => "Shield",
            Kind::Callout => "Callout",
            Kind::Arrow => "Arrow",
        }
    }

    /// The size a new element of this kind gets.
    pub(crate) fn size(self) -> (f32, f32) {
        match self {
            Kind::Person => (180., 150.),
            Kind::Boundary => (420., 280.),
            Kind::Database | Kind::Queue => (180., 100.),
            Kind::Ellipse | Kind::Diamond => (160., 100.),
            Kind::Cloud => (200., 110.),
            Kind::Note => (180., 140.),
            Kind::Text => (180., 40.),
            Kind::Icon => (96., 104.),
            Kind::Component => (180., 80.),
            Kind::Actor => (90., 140.),
            Kind::Users => (130., 120.),
            Kind::Browser => (220., 140.),
            Kind::Phone => (110., 190.),
            Kind::Desktop => (190., 150.),
            Kind::Server => (140., 150.),
            Kind::Document | Kind::Folder => (170., 110.),
            Kind::Hexagon => (170., 100.),
            Kind::Triangle => (150., 120.),
            Kind::Parallelogram => (190., 80.),
            Kind::Terminator => (170., 64.),
            Kind::Shield => (130., 150.),
            Kind::Callout => (190., 110.),
            Kind::Arrow => (190., 90.),
            _ => (200., 90.),
        }
    }

    /// Kinds drawn as a picture of what they are, which need no icon on top to say so.
    pub(crate) fn pictured(self) -> bool {
        matches!(
            self,
            Kind::Person
                | Kind::Actor
                | Kind::Users
                | Kind::Browser
                | Kind::Phone
                | Kind::Desktop
                | Kind::Server
                | Kind::Shield
        )
    }

    /// Kinds whose name sits under the picture instead of inside it.
    pub(crate) fn name_below(self) -> bool {
        matches!(self, Kind::Icon | Kind::Actor | Kind::Users)
    }

    /// Kinds that stand for real parts of the system, as opposed to drawing aids.
    pub(crate) fn is_part(self) -> bool {
        !matches!(
            self,
            Kind::Note | Kind::Text | Kind::Boundary | Kind::Callout | Kind::Arrow
        )
    }

    /// The C4 word shown under the name, e.g. "[Container: Go]".
    pub(crate) fn c4(self) -> Option<&'static str> {
        Some(match self {
            Kind::Person => "Person",
            Kind::System => "Software system",
            Kind::Container => "Container",
            Kind::Component => "Component",
            Kind::Database => "Database",
            Kind::Queue => "Queue",
            Kind::External => "External system",
            _ => return None,
        })
    }
}

/// A part of the system: a service, a store, a person, an external system, or a drawing
/// aid such as a note or a boundary.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Element {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub kind: Kind,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tech: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub owner: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Paths or globs of the code that implements it, relative to the project.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub code: Vec<String>,
    /// A glyph shown on the shape (a Lucide icon name), e.g. "database" or "zap".
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub icon: String,
    /// A picture shown on the shape, relative to the diagram folder (`icons/docker.svg`):
    /// the whole shape for an icon, a badge in the corner for others.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub image: String,
    /// Another diagram that zooms into this element; double-click opens it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub link: String,
}

/// A relationship: "web calls api over HTTPS".
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Relation {
    pub id: String,
    pub from: String,
    pub to: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tech: String,
}

/// How an element or connector looks on one diagram. Unset fields use the kind's default.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Style {
    /// Colours as `#rrggbb`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f32>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dashed: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bold: bool,
}

impl Style {
    pub(crate) fn is_empty(&self) -> bool {
        *self == Style::default()
    }
}

/// Where an element sits on one diagram.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Node {
    /// The element's id.
    pub id: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// The boundary it sits in on this diagram; moving the boundary moves it too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub locked: bool,
    #[serde(default, skip_serializing_if = "Style::is_empty")]
    pub style: Style,
    #[serde(default, skip_serializing_if = "is_default")]
    pub effect: Effect,
    /// The layer it is on; none is the base layer, which is always shown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    /// A boundary folded to one box; what it holds is hidden until it opens again.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub collapsed: bool,
    /// A collapsed boundary's size when open.
    #[serde(default, rename = "open-size", skip_serializing_if = "Option::is_none")]
    pub open_size: Option<(f32, f32)>,
}

/// A set of shapes that can be hidden or locked together, such as infra or data flows.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Layer {
    pub name: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub locked: bool,
}

fn is_default<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}

/// How a shape draws the eye while motion is on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Effect {
    #[default]
    None,
    /// A soft glow that breathes, for a hot path or a new part.
    Pulse,
    /// A ring that blinks, for an alert.
    Blink,
    /// The icon turns, for a worker, a sync or a job.
    Spin,
}

impl Effect {
    pub(crate) const ALL: [Effect; 4] = [Effect::None, Effect::Pulse, Effect::Blink, Effect::Spin];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Effect::None => "None",
            Effect::Pulse => "Pulse",
            Effect::Blink => "Blink",
            Effect::Spin => "Spin",
        }
    }
}

/// How a connector moves while motion is on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Motion {
    /// Dashes marching in the arrow's direction.
    #[default]
    Flow,
    /// Dots travelling along the line, both ways on a two-way connector.
    Packets,
    /// The line glows now and then.
    Pulse,
    None,
}

impl Motion {
    pub(crate) const ALL: [Motion; 4] =
        [Motion::Flow, Motion::Packets, Motion::Pulse, Motion::None];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Motion::Flow => "Flow",
            Motion::Packets => "Packets",
            Motion::Pulse => "Pulse",
            Motion::None => "None",
        }
    }
}

/// How fast a connector moves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Speed {
    Slow,
    #[default]
    Normal,
    Fast,
}

impl Speed {
    pub(crate) const ALL: [Speed; 3] = [Speed::Slow, Speed::Normal, Speed::Fast];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Speed::Slow => "Slow",
            Speed::Normal => "Normal",
            Speed::Fast => "Fast",
        }
    }

    /// Diagram units per second.
    pub(crate) fn rate(self) -> f32 {
        match self {
            Speed::Slow => 24.,
            Speed::Normal => 48.,
            Speed::Fast => 110.,
        }
    }
}

/// A named walk through the diagram, one connector at a time, played as an animation.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Story {
    pub name: String,
    #[serde(default, rename = "step", skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<Step>,
}

/// One step of a story: a connector, and what to say while it plays.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Step {
    /// The connector's (relation's) id.
    pub edge: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub caption: String,
}

impl Node {
    pub(crate) fn rect(&self) -> Rect {
        Rect::new(self.x, self.y, self.w, self.h)
    }
}

/// The path a connector takes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Route {
    Straight,
    #[default]
    Elbow,
    Curved,
}

/// Which ends of a connector carry an arrowhead.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Arrow {
    #[default]
    End,
    Both,
    None,
}

/// A relationship drawn on one diagram.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Edge {
    /// The relation's id.
    pub id: String,
    #[serde(default)]
    pub route: Route,
    #[serde(default)]
    pub arrow: Arrow,
    #[serde(default, skip_serializing_if = "Style::is_empty")]
    pub style: Style,
    #[serde(default, skip_serializing_if = "is_default")]
    pub motion: Motion,
    #[serde(default, skip_serializing_if = "is_default")]
    pub speed: Speed,
    /// The sides it is pinned to; otherwise the sides facing each other.
    #[serde(default, rename = "from-side", skip_serializing_if = "Option::is_none")]
    pub from_side: Option<super::geom::Side>,
    #[serde(default, rename = "to-side", skip_serializing_if = "Option::is_none")]
    pub to_side: Option<super::geom::Side>,
    /// Bends placed by hand, in diagram space; otherwise the route is worked out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub waypoints: Vec<(f32, f32)>,
}

/// One diagram: a view over the model.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct View {
    /// The file stem in `views/`; not stored in the file.
    #[serde(skip)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub order: u32,
    #[serde(default, rename = "node", skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<Node>,
    #[serde(default, rename = "edge", skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<Edge>,
    /// Motion is off on this diagram.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub still: bool,
    #[serde(default, rename = "layer", skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<Layer>,
    #[serde(default, rename = "story", skip_serializing_if = "Vec::is_empty")]
    pub stories: Vec<Story>,
}

impl View {
    pub(crate) fn node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub(crate) fn node_mut(&mut self, id: &str) -> Option<&mut Node> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }

    pub(crate) fn edge(&self, id: &str) -> Option<&Edge> {
        self.edges.iter().find(|e| e.id == id)
    }

    pub(crate) fn edge_mut(&mut self, id: &str) -> Option<&mut Edge> {
        self.edges.iter_mut().find(|e| e.id == id)
    }

    /// Ids of the nodes inside `id`, at any depth.
    pub(crate) fn descendants(&self, id: &str) -> Vec<String> {
        let kids = self.children();
        let mut out: Vec<String> = Vec::new();
        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::from([id]);
        let mut queue = vec![id];
        while let Some(p) = queue.pop() {
            for k in kids.get(p).into_iter().flatten() {
                if seen.insert(k) {
                    out.push(k.to_string());
                    queue.push(k);
                }
            }
        }
        out
    }

    /// Each boundary's direct contents.
    pub(crate) fn children(&self) -> std::collections::HashMap<&str, Vec<&str>> {
        let mut kids: std::collections::HashMap<&str, Vec<&str>> = Default::default();
        for n in &self.nodes {
            if let Some(p) = n.parent.as_deref() {
                kids.entry(p).or_default().push(n.id.as_str());
            }
        }
        kids
    }

    pub(crate) fn layer(&self, name: &str) -> Option<&Layer> {
        self.layers.iter().find(|l| l.name == name)
    }
}

#[derive(Default, Serialize, Deserialize)]
struct ModelFile {
    #[serde(default, rename = "element", skip_serializing_if = "Vec::is_empty")]
    elements: Vec<Element>,
    #[serde(default, rename = "relation", skip_serializing_if = "Vec::is_empty")]
    relations: Vec<Relation>,
}

/// The whole model with all its diagrams.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Doc {
    pub elements: Vec<Element>,
    pub relations: Vec<Relation>,
    pub views: Vec<View>,
}

impl Doc {
    pub(crate) fn element(&self, id: &str) -> Option<&Element> {
        self.elements.iter().find(|e| e.id == id)
    }

    pub(crate) fn element_mut(&mut self, id: &str) -> Option<&mut Element> {
        self.elements.iter_mut().find(|e| e.id == id)
    }

    pub(crate) fn relation(&self, id: &str) -> Option<&Relation> {
        self.relations.iter().find(|r| r.id == id)
    }

    pub(crate) fn relation_mut(&mut self, id: &str) -> Option<&mut Relation> {
        self.relations.iter_mut().find(|r| r.id == id)
    }

    /// A new id from a name: lowercase words joined by dashes, unique in the model.
    pub(crate) fn new_id(&self, name: &str) -> String {
        let taken = |id: &str| {
            self.elements.iter().any(|e| e.id == id) || self.relations.iter().any(|r| r.id == id)
        };
        unique(&slug(name, "element"), taken)
    }

    /// A new diagram id, unique among the diagrams.
    pub(crate) fn new_view_id(&self, name: &str) -> String {
        unique(&slug(name, "diagram"), |id| {
            self.views.iter().any(|v| v.id == id)
        })
    }

    /// Drop relations whose ends are gone and diagram entries for missing parts.
    pub(crate) fn tidy(&mut self) {
        let ids: std::collections::HashSet<String> =
            self.elements.iter().map(|e| e.id.clone()).collect();
        self.relations
            .retain(|r| ids.contains(&r.from) && ids.contains(&r.to));
        let rels: std::collections::HashMap<&str, &Relation> =
            self.relations.iter().map(|r| (r.id.as_str(), r)).collect();
        for v in &mut self.views {
            v.nodes.retain(|n| ids.contains(&n.id));
            let shown: std::collections::HashSet<String> =
                v.nodes.iter().map(|n| n.id.clone()).collect();
            let layers: std::collections::HashSet<String> =
                v.layers.iter().map(|l| l.name.clone()).collect();
            for n in &mut v.nodes {
                if n.parent.as_ref().is_some_and(|p| !shown.contains(p)) {
                    n.parent = None;
                }
                if n.layer.as_ref().is_some_and(|l| !layers.contains(l)) {
                    n.layer = None;
                }
            }
            v.edges.retain(|e| {
                rels.get(e.id.as_str())
                    .is_some_and(|r| shown.contains(&r.from) && shown.contains(&r.to))
            });
            let drawn: std::collections::HashSet<String> =
                v.edges.iter().map(|e| e.id.clone()).collect();
            for story in &mut v.stories {
                story.steps.retain(|s| drawn.contains(&s.edge));
            }
        }
    }

    /// The model file's text.
    pub(crate) fn model_text(&self) -> String {
        let file = ModelFile {
            elements: self.elements.clone(),
            relations: self.relations.clone(),
        };
        toml::to_string_pretty(&file).unwrap_or_default()
    }

    /// A diagram file's text.
    pub(crate) fn view_text(view: &View) -> String {
        toml::to_string_pretty(view).unwrap_or_default()
    }

    /// Read the model and a diagram from text, as edited in the text view.
    pub(crate) fn parse(model: &str, view_id: &str, view: &str) -> Result<(Self, View), String> {
        let file: ModelFile = toml::from_str(model).map_err(|e| format!("model.toml: {e}"))?;
        let mut v: View = toml::from_str(view).map_err(|e| format!("{view_id}.toml: {e}"))?;
        v.id = view_id.to_string();
        Ok((
            Doc {
                elements: file.elements,
                relations: file.relations,
                views: Vec::new(),
            },
            v,
        ))
    }
}

/// Lowercase ASCII words joined by dashes.
pub(crate) fn slug(name: &str, fallback: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        fallback.to_string()
    } else {
        out
    }
}

fn unique(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|id| !taken(id))
        .unwrap_or_default()
}

pub(crate) fn folder(root: &Path) -> PathBuf {
    root.join(FOLDER)
}

/// Whether the project has a model to load.
pub(crate) fn exists(root: &Path) -> bool {
    folder(root).join(MODEL).is_file()
}

/// Read the model and every diagram. A project without one gets an empty model.
pub(crate) fn load(root: &Path) -> Result<Doc, String> {
    let dir = folder(root);
    let mut doc = Doc::default();
    if let Ok(text) = std::fs::read_to_string(dir.join(MODEL)) {
        let file: ModelFile =
            toml::from_str(&text).map_err(|e| format!("{FOLDER}/{MODEL}: {e}"))?;
        doc.elements = file.elements;
        doc.relations = file.relations;
    }
    let mut views = Vec::new();
    for entry in std::fs::read_dir(dir.join(VIEWS))
        .into_iter()
        .flatten()
        .flatten()
    {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let Some(stem) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
            continue;
        };
        let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let mut view: View =
            toml::from_str(&text).map_err(|e| format!("{FOLDER}/{VIEWS}/{stem}.toml: {e}"))?;
        view.id = stem;
        views.push(view);
    }
    views.sort_by(|a, b| a.order.cmp(&b.order).then(a.name.cmp(&b.name)));
    doc.views = views;
    doc.tidy();
    Ok(doc)
}

/// Write the model and every diagram, and remove files of diagrams that were deleted.
pub(crate) fn save(root: &Path, doc: &Doc) -> std::io::Result<()> {
    let dir = folder(root);
    let views_dir = dir.join(VIEWS);
    std::fs::create_dir_all(&views_dir)?;
    write_if_changed(&dir.join(MODEL), &doc.model_text())?;
    // The guide to these files, for people and agents; kept current unless it's been
    // replaced by one of the project's own.
    let readme = dir.join(README_FILE);
    let ours = std::fs::read_to_string(&readme).map_or(true, |t| t.starts_with(README_MARK));
    if ours {
        write_if_changed(&readme, README)?;
    }
    let mut keep = std::collections::HashSet::new();
    for (i, v) in doc.views.iter().enumerate() {
        let mut v = v.clone();
        v.order = i as u32;
        let file = views_dir.join(format!("{}.toml", v.id));
        write_if_changed(&file, &Doc::view_text(&v))?;
        keep.insert(file);
    }
    for entry in std::fs::read_dir(&views_dir)
        .into_iter()
        .flatten()
        .flatten()
    {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("toml") && !keep.contains(&path) {
            std::fs::remove_file(path)?;
        }
    }
    Ok(())
}

/// Every file under the diagram folder with its bytes, to put back exactly later.
pub(crate) type Snapshot = Vec<(PathBuf, Vec<u8>)>;

fn files_under(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            files_under(&path, out);
        } else {
            out.push(path);
        }
    }
}

pub(crate) fn snapshot(root: &Path) -> Snapshot {
    let mut files = Vec::new();
    files_under(&folder(root), &mut files);
    files.sort();
    files
        .into_iter()
        .filter_map(|p| Some((p.clone(), std::fs::read(&p).ok()?)))
        .collect()
}

/// Put the diagram folder back as it was in `snap`, removing files made since.
pub(crate) fn restore(root: &Path, snap: &Snapshot) -> std::io::Result<()> {
    let keep: std::collections::HashSet<&PathBuf> = snap.iter().map(|(p, _)| p).collect();
    let mut now = Vec::new();
    files_under(&folder(root), &mut now);
    for p in now {
        if !keep.contains(&p) {
            std::fs::remove_file(&p)?;
        }
    }
    for (p, bytes) in snap {
        if std::fs::read(p).ok().as_deref() != Some(bytes.as_slice()) {
            if let Some(dir) = p.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(p, bytes)?;
        }
    }
    Ok(())
}

/// The model and one diagram as the project's last commit has them: an empty model when
/// the commit has none, and no diagram when it didn't have that one.
pub(crate) fn committed(root: &Path, view_id: &str) -> Result<(Doc, Option<View>), String> {
    let dir = root.to_string_lossy();
    let mut doc = Doc::default();
    if let Some(text) = crate::git::committed_text(&dir, &format!("{FOLDER}/{MODEL}")) {
        let file: ModelFile =
            toml::from_str(&text).map_err(|e| format!("{FOLDER}/{MODEL}: {e}"))?;
        doc.elements = file.elements;
        doc.relations = file.relations;
    }
    let view = match crate::git::committed_text(&dir, &format!("{FOLDER}/{VIEWS}/{view_id}.toml")) {
        Some(text) => {
            let mut v: View = toml::from_str(&text)
                .map_err(|e| format!("{FOLDER}/{VIEWS}/{view_id}.toml: {e}"))?;
            v.id = view_id.to_string();
            Some(v)
        }
        None => None,
    };
    Ok((doc, view))
}

/// `doc` with one diagram put back as the last commit had it (`was`, None when the commit
/// didn't have it), along with the parts and connections only that diagram shows: they go
/// back to how the commit had them, or go when it didn't have them. What other diagrams
/// show stays as it is now.
pub(crate) fn revert_view(doc: &Doc, view_id: &str, committed: &Doc, was: Option<View>) -> Doc {
    use std::collections::HashSet;
    let now = doc.views.iter().find(|v| v.id == view_id);
    let others = doc.views.iter().filter(|v| v.id != view_id);
    let mut shown_elsewhere = HashSet::new();
    let mut drawn_elsewhere = HashSet::new();
    for v in others {
        shown_elsewhere.extend(v.nodes.iter().map(|n| n.id.as_str()));
        drawn_elsewhere.extend(v.edges.iter().map(|e| e.id.as_str()));
    }
    let both = now.into_iter().chain(was.as_ref());
    let mut own_parts = HashSet::new();
    let mut own_links = HashSet::new();
    for v in both {
        own_parts.extend(
            v.nodes
                .iter()
                .map(|n| n.id.as_str())
                .filter(|id| !shown_elsewhere.contains(id)),
        );
        own_links.extend(
            v.edges
                .iter()
                .map(|e| e.id.as_str())
                .filter(|id| !drawn_elsewhere.contains(id)),
        );
    }
    // In the commit's order, so a model put fully back reads as it did.
    fn merge<T: Clone>(
        now: &[T],
        then: &[T],
        own: &HashSet<&str>,
        id: impl Fn(&T) -> &str,
    ) -> Vec<T> {
        let mut out: Vec<T> = then
            .iter()
            .filter_map(|t| {
                if own.contains(id(t)) {
                    Some(t.clone())
                } else {
                    now.iter().find(|n| id(n) == id(t)).cloned()
                }
            })
            .collect();
        out.extend(
            now.iter()
                .filter(|n| !own.contains(id(n)) && !then.iter().any(|t| id(t) == id(n)))
                .cloned(),
        );
        out
    }
    let mut out = Doc {
        elements: merge(&doc.elements, &committed.elements, &own_parts, |e| &e.id),
        relations: merge(&doc.relations, &committed.relations, &own_links, |r| &r.id),
        views: Vec::new(),
    };
    for v in &doc.views {
        if v.id != view_id {
            out.views.push(v.clone());
        } else if let Some(was) = &was {
            out.views.push(was.clone());
        }
    }
    out.tidy();
    out
}

/// After a diagram was put back: files that hold what the last commit has, written in
/// another way (by hand, or by an agent), get the commit's own text, so git shows no
/// change in them; and the guide to the files goes when it is all that's left changed.
pub(crate) fn match_commit(root: &Path, doc: &Doc) -> std::io::Result<()> {
    let dir = root.to_string_lossy();
    let model = format!("{FOLDER}/{MODEL}");
    if let Some(text) = crate::git::committed_text(&dir, &model) {
        let same = toml::from_str::<ModelFile>(&text)
            .is_ok_and(|f| f.elements == doc.elements && f.relations == doc.relations);
        if same {
            write_if_changed(&root.join(&model), &text)?;
        }
    }
    for now in &doc.views {
        let file = format!("{FOLDER}/{VIEWS}/{}.toml", now.id);
        let Some(text) = crate::git::committed_text(&dir, &file) else {
            continue;
        };
        let same = toml::from_str::<View>(&text).is_ok_and(|mut then| {
            let mut now = now.clone();
            then.id = now.id.clone();
            then.order = 0;
            now.order = 0;
            then == now
        });
        if same {
            write_if_changed(&root.join(&file), &text)?;
        }
    }
    let readme = format!("{FOLDER}/{README_FILE}");
    if crate::git::changed_under(&dir, FOLDER) == [readme.clone()] {
        crate::git::discard_under(&dir, &readme).map_err(std::io::Error::other)?;
    }
    Ok(())
}

/// A fingerprint of the diagram folder's files (names, sizes and times), to notice edits.
pub(crate) fn stamp(root: &Path) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut files = Vec::new();
    files_under(&folder(root), &mut files);
    files.sort();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for p in files {
        p.hash(&mut h);
        if let Ok(m) = std::fs::metadata(&p) {
            m.len().hash(&mut h);
            if let Ok(t) = m.modified() {
                t.hash(&mut h);
            }
        }
    }
    h.finish()
}

fn write_if_changed(path: &Path, text: &str) -> std::io::Result<()> {
    if std::fs::read_to_string(path).is_ok_and(|old| old == text) {
        return Ok(());
    }
    std::fs::write(path, text)
}

/// An axis-aligned rectangle in diagram space.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub(crate) fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    /// The rectangle spanned by two corners, in any order.
    pub(crate) fn span(a: (f32, f32), b: (f32, f32)) -> Self {
        Self::new(
            a.0.min(b.0),
            a.1.min(b.1),
            (a.0 - b.0).abs(),
            (a.1 - b.1).abs(),
        )
    }

    pub(crate) fn right(&self) -> f32 {
        self.x + self.w
    }

    pub(crate) fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub(crate) fn center(&self) -> (f32, f32) {
        (self.x + self.w / 2., self.y + self.h / 2.)
    }

    pub(crate) fn contains(&self, p: (f32, f32)) -> bool {
        p.0 >= self.x && p.0 <= self.right() && p.1 >= self.y && p.1 <= self.bottom()
    }

    pub(crate) fn contains_rect(&self, r: &Rect) -> bool {
        r.x >= self.x && r.y >= self.y && r.right() <= self.right() && r.bottom() <= self.bottom()
    }

    pub(crate) fn intersects(&self, r: &Rect) -> bool {
        self.x < r.right() && r.x < self.right() && self.y < r.bottom() && r.y < self.bottom()
    }

    pub(crate) fn union(&self, r: &Rect) -> Rect {
        let x = self.x.min(r.x);
        let y = self.y.min(r.y);
        Rect::new(
            x,
            y,
            self.right().max(r.right()) - x,
            self.bottom().max(r.bottom()) - y,
        )
    }

    pub(crate) fn inflate(&self, by: f32) -> Rect {
        Rect::new(self.x - by, self.y - by, self.w + 2. * by, self.h + 2. * by)
    }

    /// The smallest rectangle around all of them.
    pub(crate) fn bounds(rects: impl IntoIterator<Item = Rect>) -> Option<Rect> {
        rects.into_iter().reduce(|a, b| a.union(&b))
    }
}

/// Parse `#rrggbb` (or `rrggbb`) into a colour.
pub(crate) fn parse_hex(text: &str) -> Option<u32> {
    let t = text.trim().trim_start_matches('#');
    if t.len() != 6 {
        return None;
    }
    u32::from_str_radix(t, 16).ok()
}

pub(crate) fn hex(color: u32) -> String {
    format!("#{:06x}", color & 0xff_ffff)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("runo-archdraw-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample() -> Doc {
        Doc {
            elements: vec![
                Element {
                    id: "web".into(),
                    name: "Web app".into(),
                    kind: Kind::Container,
                    tech: "React".into(),
                    code: vec!["web/**".into()],
                    ..Default::default()
                },
                Element {
                    id: "db".into(),
                    name: "Orders DB".into(),
                    kind: Kind::Database,
                    ..Default::default()
                },
            ],
            relations: vec![Relation {
                id: "web-db".into(),
                from: "web".into(),
                to: "db".into(),
                label: "reads".into(),
                ..Default::default()
            }],
            views: vec![View {
                id: "containers".into(),
                name: "Containers".into(),
                order: 0,
                nodes: vec![
                    Node {
                        id: "web".into(),
                        x: 0.,
                        y: 0.,
                        w: 200.,
                        h: 90.,
                        style: Style {
                            fill: Some("#112233".into()),
                            ..Default::default()
                        },
                        effect: Effect::Pulse,
                        layer: Some("Apps".into()),
                        ..Default::default()
                    },
                    Node {
                        id: "db".into(),
                        x: 0.,
                        y: 200.,
                        w: 180.,
                        h: 100.,
                        ..Default::default()
                    },
                ],
                edges: vec![Edge {
                    id: "web-db".into(),
                    route: Route::Curved,
                    motion: Motion::Packets,
                    speed: Speed::Fast,
                    from_side: Some(super::super::geom::Side::Bottom),
                    waypoints: vec![(100., 150.)],
                    ..Default::default()
                }],
                still: false,
                layers: vec![Layer {
                    name: "Apps".into(),
                    locked: true,
                    ..Default::default()
                }],
                stories: vec![Story {
                    name: "Read".into(),
                    steps: vec![Step {
                        edge: "web-db".into(),
                        caption: "Reads the orders".into(),
                    }],
                }],
            }],
        }
    }

    #[test]
    fn saves_and_loads_the_model_and_diagrams() {
        let root = temp("roundtrip");
        let doc = sample();
        save(&root, &doc).unwrap();
        assert!(exists(&root));
        assert!(root.join("architecture/views/containers.toml").is_file());
        let back = load(&root).unwrap();
        assert_eq!(back, doc);
        let text = std::fs::read_to_string(root.join("architecture/model.toml")).unwrap();
        assert!(text.contains("[[element]]") && text.contains("kind = \"database\""));
    }

    #[test]
    fn deleted_diagrams_lose_their_files() {
        let root = temp("delete");
        let mut doc = sample();
        let mut other = doc.views[0].clone();
        other.id = "deploy".into();
        doc.views.push(other);
        save(&root, &doc).unwrap();
        doc.views.pop();
        save(&root, &doc).unwrap();
        assert!(!root.join("architecture/views/deploy.toml").exists());
        assert_eq!(load(&root).unwrap().views.len(), 1);
    }

    #[test]
    fn tidy_drops_dangling_parts() {
        let mut doc = sample();
        doc.elements.retain(|e| e.id != "db");
        doc.tidy();
        assert!(doc.relations.is_empty());
        assert_eq!(doc.views[0].nodes.len(), 1);
        assert!(doc.views[0].edges.is_empty());
    }

    #[test]
    fn ids_are_slugs_and_unique() {
        let doc = sample();
        assert_eq!(doc.new_id("Payments API!"), "payments-api");
        assert_eq!(doc.new_id("Web"), "web-2");
        assert_eq!(doc.new_id("   "), "element");
        assert_eq!(doc.new_view_id("Containers"), "containers-2");
    }

    #[test]
    fn rects_and_colours() {
        let a = Rect::new(0., 0., 10., 10.);
        let b = Rect::new(5., 5., 10., 10.);
        assert!(a.intersects(&b));
        assert_eq!(a.union(&b), Rect::new(0., 0., 15., 15.));
        assert_eq!(Rect::span((10., 0.), (0., 5.)), Rect::new(0., 0., 10., 5.));
        assert_eq!(parse_hex("#ff8800"), Some(0xff8800));
        assert_eq!(hex(0xff8800), "#ff8800");
        assert_eq!(parse_hex("nope"), None);
    }

    #[test]
    fn descendants_are_found_at_any_depth() {
        let mut v = View::default();
        for (id, parent) in [
            ("vpc", None),
            ("subnet", Some("vpc")),
            ("api", Some("subnet")),
        ] {
            v.nodes.push(Node {
                id: id.into(),
                parent: parent.map(String::from),
                ..Default::default()
            });
        }
        let mut d = v.descendants("vpc");
        d.sort();
        assert_eq!(d, vec!["api", "subnet"]);
    }

    #[test]
    fn the_format_guide_matches_the_format() {
        assert!(README.starts_with(README_MARK));
        let blocks: Vec<&str> = README
            .split("```toml")
            .skip(1)
            .map(|b| b.split("```").next().unwrap())
            .collect();
        assert_eq!(blocks.len(), 2);
        let model: ModelFile = toml::from_str(blocks[0]).expect("model example reads");
        assert_eq!(model.elements[0].kind, Kind::Container);
        assert_eq!(model.relations[0].to, "orders-api");
        let view: View = toml::from_str(blocks[1]).expect("view example reads");
        let n = &view.nodes[0];
        assert_eq!(n.effect, Effect::Pulse);
        assert_eq!(n.open_size, Some((620., 420.)));
        assert_eq!(n.style.radius, Some(8.));
        let e = &view.edges[0];
        assert_eq!(e.to_side, Some(super::super::geom::Side::Top));
        assert_eq!(e.waypoints, vec![(600., 380.)]);
        assert_eq!(view.layers[0].name, "Services");
        assert_eq!(view.stories[0].steps[0].edge, "web-to-orders-api");
        for k in Kind::ALL {
            let name = toml::to_string(&ModelFile {
                elements: vec![Element {
                    kind: k,
                    ..Default::default()
                }],
                ..Default::default()
            })
            .unwrap();
            let word = name
                .split("kind = \"")
                .nth(1)
                .unwrap()
                .split('"')
                .next()
                .unwrap();
            assert!(README.contains(&format!("`{word}`")), "{word} is described");
        }
    }

    #[test]
    fn saving_writes_the_guide_but_keeps_a_projects_own() {
        let dir = std::env::temp_dir().join(format!("runo-guide-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        save(&dir, &sample()).unwrap();
        let readme = folder(&dir).join(README_FILE);
        assert_eq!(std::fs::read_to_string(&readme).unwrap(), README);
        std::fs::write(&readme, "# Our notes").unwrap();
        save(&dir, &sample()).unwrap();
        assert_eq!(std::fs::read_to_string(&readme).unwrap(), "# Our notes");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn restore_puts_files_back_exactly() {
        let dir = std::env::temp_dir().join(format!("runo-restore-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        save(&dir, &sample()).unwrap();
        let model = folder(&dir).join(MODEL);
        std::fs::write(
            &model,
            "# hand formatted\n[[element]]\nid = \"web\"\nname = \"Web\"\n",
        )
        .unwrap();
        let snap = snapshot(&dir);
        let before = stamp(&dir);
        std::fs::write(&model, "broken = [").unwrap();
        std::fs::write(folder(&dir).join(VIEWS).join("new.toml"), "name = \"New\"").unwrap();
        assert_ne!(stamp(&dir), before);
        restore(&dir, &snap).unwrap();
        assert!(std::fs::read_to_string(&model)
            .unwrap()
            .starts_with("# hand formatted"));
        assert!(!folder(&dir).join(VIEWS).join("new.toml").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn one_diagram_goes_back_to_the_commit() {
        let then = sample();
        let mut now = then.clone();
        // On the committed diagram: a part renamed and a new part added.
        now.elements[0].name = "Storefront".into();
        now.elements.push(Element {
            id: "cache".into(),
            name: "Cache".into(),
            ..Default::default()
        });
        now.views[0].nodes.push(Node {
            id: "cache".into(),
            ..Default::default()
        });
        // A new diagram showing the database and a part of its own.
        now.elements.push(Element {
            id: "queue".into(),
            name: "Queue".into(),
            ..Default::default()
        });
        now.views.push(View {
            id: "extra".into(),
            name: "Extra".into(),
            nodes: ["db", "queue"]
                .map(|id| Node {
                    id: id.into(),
                    ..Default::default()
                })
                .into(),
            ..Default::default()
        });
        let was = then.views[0].clone();
        let back = revert_view(&now, "containers", &then, Some(was));
        assert_eq!(back.element("web").unwrap().name, "Web app");
        assert!(back.element("cache").is_none());
        assert!(back.element("queue").is_some());
        assert_eq!(back.views[0], then.views[0]);
        assert_eq!(back.views.len(), 2);
        // The new diagram goes, with the part only it showed; the shared one stays.
        let gone = revert_view(&back, "extra", &then, None);
        assert_eq!(gone, then);
        assert_eq!(gone.model_text(), then.model_text());
    }

    #[test]
    fn diagrams_put_back_one_by_one_leave_git_with_no_changes() {
        let root = temp("revert-git");
        let dir = root.to_str().unwrap().to_string();
        let sh = |args: &[&str]| {
            assert!(std::process::Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(args)
                .output()
                .unwrap()
                .status
                .success());
        };
        sh(&["init", "-q"]);
        sh(&["config", "user.email", "t@t"]);
        sh(&["config", "user.name", "t"]);
        let then = sample();
        save(&root, &then).unwrap();
        // Written by hand, or by an agent: the same diagram in other words.
        let view = folder(&root).join(VIEWS).join("containers.toml");
        let text = std::fs::read_to_string(&view).unwrap();
        std::fs::write(&view, format!("# Drawn by hand\n{text}")).unwrap();
        sh(&["add", "-A"]);
        sh(&["commit", "-q", "-m", "diagrams"]);
        let mut now = load(&root).unwrap();
        now.elements[0].name = "Storefront".into();
        now.elements.push(Element {
            id: "queue".into(),
            name: "Queue".into(),
            ..Default::default()
        });
        now.views.push(View {
            id: "extra".into(),
            name: "Extra".into(),
            nodes: vec![Node {
                id: "queue".into(),
                ..Default::default()
            }],
            ..Default::default()
        });
        save(&root, &now).unwrap();
        // The model, the new diagram, and the hand-written one saved in Runo's words.
        assert_eq!(crate::git::changed_under(&dir, FOLDER).len(), 3);
        for id in ["containers", "extra"] {
            let (committed_doc, was) = committed(&root, id).unwrap();
            now = revert_view(&now, id, &committed_doc, was);
            save(&root, &now).unwrap();
            match_commit(&root, &now).unwrap();
        }
        assert!(crate::git::changed_under(&dir, FOLDER).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }
}
