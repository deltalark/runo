//! Design's own edit actions, offered to agents as tools of Runo's MCP server, so an agent
//! changes a layer the way a click in Design does: through the same operations, with the
//! same undo, ids kept and locked layers left alone. The server process and the app meet
//! in a folder of small files, as approvals do: a call is a request file, the open canvas
//! for that project carries it out and writes the answer.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// How long an agent waits for the canvas to carry out an action.
const WAIT: Duration = Duration::from_secs(45);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Request {
    pub id: String,
    /// The folder the agent works in.
    pub dir: String,
    pub tool: String,
    #[serde(default)]
    pub input: Value,
    /// When it was asked, in seconds since 1970.
    pub asked: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Answer {
    pub ok: bool,
    pub text: String,
}

pub(crate) fn folder() -> PathBuf {
    crate::store::data_dir().join("design-ops")
}

/// Whether the agent working in `dir` has a design to change.
pub(crate) fn offered(dir: &str) -> bool {
    super::model::folder(Path::new(dir))
        .join("screens")
        .is_dir()
}

fn screen_prop() -> Value {
    json!({ "type": "string", "description": "The screen's id: its file name in design/screens/ without .html." })
}

fn selector_prop() -> Value {
    json!({ "type": "string", "description": "A CSS selector for the layers to change on that screen, such as [data-rid=\"k3x9a1b2\"], [data-name=\"Hero\"] h1, or .card .price. Every match is changed." })
}

/// The tools, as MCP lists them.
pub(crate) fn tools() -> Vec<Value> {
    let edit = |name: &str, description: &str, extra: Value, required: &[&str]| {
        let mut props = json!({ "screen": screen_prop(), "selector": selector_prop() });
        if let (Some(p), Some(e)) = (props.as_object_mut(), extra.as_object()) {
            p.extend(e.clone());
        }
        let mut req = vec!["screen", "selector"];
        req.extend_from_slice(required);
        json!({
            "name": name,
            "description": description,
            "inputSchema": { "type": "object", "properties": props, "required": req }
        })
    };
    vec![
        json!({
            "name": "design_selection",
            "description": "What the person has selected in Design right now: the screen, and each selected layer with a selector for it. Use it when they say \"this\" or \"the selected\".",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        edit(
            "design_set_text",
            "Set the text of layers, as typing on the canvas does.",
            json!({ "text": { "type": "string" } }),
            &["text"],
        ),
        edit(
            "design_set_style",
            "Set CSS properties on layers, as the Design panel does. Use the theme's tokens, like var(--primary) or var(--space-4). An empty value removes the property.",
            json!({ "styles": { "type": "object", "additionalProperties": { "type": "string" }, "description": "Property to value, like {\"padding\": \"var(--space-4)\", \"color\": \"var(--on-surface)\"}." } }),
            &["styles"],
        ),
        edit(
            "design_set_text_style",
            "Give text layers one of the design's text styles (a class such as display, big, small, tiny or eyebrow, or one made in Design), as the Text style chips do. An empty style clears it.",
            json!({ "style": { "type": "string" } }),
            &["style"],
        ),
        edit(
            "design_set_attribute",
            "Set an attribute on layers, such as data-name (the layer's name), alt, href or data-goto (the screen a tap goes to). An empty value removes it. Ids and component marks are kept by Design and can't be set.",
            json!({ "name": { "type": "string" }, "value": { "type": "string" } }),
            &["name", "value"],
        ),
        edit(
            "design_swap_variant",
            "Show component instances as another of their component's variants, keeping their own text and overrides, as picking a variant does.",
            json!({ "variant": { "type": "string", "description": "The variant's name, like \"State=Pressed\"." } }),
            &["variant"],
        ),
        edit(
            "design_insert",
            "Add new layers, from HTML, inside the matching layer (at its end) or after it. New layers get their ids from Design.",
            json!({
                "html": { "type": "string" },
                "position": { "type": "string", "enum": ["inside", "after"] }
            }),
            &["html", "position"],
        ),
        edit(
            "design_move",
            "Move layers before or after another layer.",
            json!({
                "target": { "type": "string", "description": "A CSS selector for the layer to move next to." },
                "position": { "type": "string", "enum": ["before", "after"] }
            }),
            &["target", "position"],
        ),
        edit("design_remove", "Delete layers.", json!({}), &[]),
    ]
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn result(ok: bool, text: impl Into<String>) -> Value {
    json!({ "content": [{ "type": "text", "text": text.into() }], "isError": !ok })
}

/// Carry out a tool call from the agent working in `dir`, waiting for the canvas.
pub(crate) fn call(base: &Path, dir: &str, tool: &str, input: &Value) -> Value {
    let request = Request {
        id: crate::approvals::new_id(),
        dir: dir.to_string(),
        tool: tool.to_string(),
        input: input.clone(),
        asked: now(),
    };
    let path = base.join(format!("{}.json", request.id));
    let answered = base.join(format!("{}.answer", request.id));
    let written = serde_json::to_vec(&request)
        .map_err(std::io::Error::other)
        .and_then(|data| crate::approvals::write_atomic(&path, &data));
    if let Err(e) = written {
        return result(false, format!("Runo couldn't pass this to Design: {e}"));
    }
    let start = Instant::now();
    loop {
        if let Some(a) = std::fs::read(&answered)
            .ok()
            .and_then(|b| serde_json::from_slice::<Answer>(&b).ok())
        {
            let _ = std::fs::remove_file(&answered);
            return result(a.ok, a.text);
        }
        if start.elapsed() >= WAIT {
            let _ = std::fs::remove_file(&path);
            return result(
                false,
                "Design isn't open for this project in Runo, so its actions are unavailable. Edit the screen files instead.",
            );
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The calls waiting for the canvas whose design is at `root`, oldest first. Each is taken:
/// it won't be given again.
pub(crate) fn take(base: &Path, root: &Path) -> Vec<Request> {
    let Ok(entries) = std::fs::read_dir(base) else {
        return Vec::new();
    };
    let mut out: Vec<Request> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .filter_map(|p| {
            let r: Request = serde_json::from_slice(&std::fs::read(&p).ok()?).ok()?;
            if !Path::new(&r.dir).starts_with(root) {
                // Nobody waits for one this old any more.
                if now() > r.asked + WAIT.as_secs() + 60 {
                    let _ = std::fs::remove_file(&p);
                }
                return None;
            }
            std::fs::remove_file(&p).ok()?;
            Some(r)
        })
        .collect();
    out.sort_by(|a, b| (a.asked, &a.id).cmp(&(b.asked, &b.id)));
    out
}

pub(crate) fn answer(base: &Path, id: &str, ok: bool, text: impl Into<String>) {
    let a = Answer {
        ok,
        text: text.into(),
    };
    if let Ok(data) = serde_json::to_vec(&a) {
        let _ = crate::approvals::write_atomic(&base.join(format!("{id}.answer")), &data);
    }
}

/// Attributes Design keeps itself.
pub(crate) fn kept_attribute(name: &str) -> bool {
    matches!(
        name.trim().to_ascii_lowercase().as_str(),
        "data-rid" | "data-component" | "data-variant" | "data-library" | "data-locked"
    )
}

/// One action, read from a call: what to run on the matching layers.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Action {
    Selection,
    /// A Design operation by name, with its arguments.
    Op(&'static str, Vec<Value>),
    Swap(String),
}

pub(crate) struct Call {
    pub screen: String,
    pub selector: String,
    pub action: Action,
    /// What it does, for undo: "set the text".
    pub what: String,
    /// The same with its values, to remember when the person undoes it.
    pub detail: String,
}

/// Read a call's input, or say what's wrong with it.
pub(crate) fn parse(tool: &str, input: &Value, text_styles: &[String]) -> Result<Call, String> {
    let s = |k: &str| input[k].as_str().map(str::to_string);
    if tool == "design_selection" {
        return Ok(Call {
            screen: String::new(),
            selector: String::new(),
            action: Action::Selection,
            what: String::new(),
            detail: String::new(),
        });
    }
    let screen = s("screen")
        .filter(|v| !v.trim().is_empty())
        .ok_or("Name the screen.")?;
    let selector = s("selector")
        .filter(|v| !v.trim().is_empty())
        .ok_or("Give a CSS selector for the layers.")?;
    let need = |k: &str| s(k).ok_or(format!("{tool} needs \"{k}\"."));
    let sel = &selector;
    let (action, what, detail) = match tool {
        "design_set_text" => {
            let text = need("text")?;
            let detail = format!("set the text of {sel} to \"{text}\"");
            (
                Action::Op("text", vec![json!(text)]),
                "set the text",
                detail,
            )
        }
        "design_set_style" => {
            let map = input["styles"]
                .as_object()
                .filter(|m| !m.is_empty())
                .ok_or("design_set_style needs \"styles\": property to value.")?;
            let decls: Vec<String> = map
                .iter()
                .map(|(k, v)| format!("{k}: {}", v.as_str().unwrap_or_default()))
                .collect();
            let detail = format!("set {} on {sel}", decls.join("; "));
            (
                Action::Op("styles", vec![Value::Object(map.clone())]),
                "set the style",
                detail,
            )
        }
        "design_set_text_style" => {
            let style = need("style")?;
            let detail = format!("give {sel} the text style \"{style}\"");
            (
                Action::Op("textStyle", vec![json!(style), json!(text_styles)]),
                "set the text style",
                detail,
            )
        }
        "design_set_attribute" => {
            let name = need("name")?;
            if kept_attribute(&name) {
                return Err(format!(
                    "{name} is kept by Design: ids, component marks and locks follow the layer, so they can't be set."
                ));
            }
            let value = need("value")?;
            let detail = format!("set {name}=\"{value}\" on {sel}");
            (
                Action::Op("attr", vec![json!(name), json!(value)]),
                "set an attribute",
                detail,
            )
        }
        "design_swap_variant" => {
            let variant = need("variant")?;
            let detail = format!("show {sel} as the variant \"{variant}\"");
            (Action::Swap(variant), "swapped the variant", detail)
        }
        "design_insert" => {
            let inside = match need("position")?.as_str() {
                "inside" => true,
                "after" => false,
                p => return Err(format!("position is \"inside\" or \"after\", not \"{p}\".")),
            };
            let html = need("html")?;
            let detail = format!(
                "add {} {} {sel}",
                html.chars().take(120).collect::<String>(),
                if inside { "inside" } else { "after" }
            );
            (
                Action::Op("insertNew", vec![json!(html), json!(inside)]),
                "added layers",
                detail,
            )
        }
        "design_move" => {
            let after = match need("position")?.as_str() {
                "after" => true,
                "before" => false,
                p => return Err(format!("position is \"before\" or \"after\", not \"{p}\".")),
            };
            let target = need("target")?;
            let detail = format!(
                "move {sel} {} {target}",
                if after { "after" } else { "before" }
            );
            (
                Action::Op("moveBy", vec![json!(target), json!(after)]),
                "moved layers",
                detail,
            )
        }
        "design_remove" => (
            Action::Op("remove", Vec::new()),
            "deleted layers",
            format!("delete {sel}"),
        ),
        other => return Err(format!("Design has no action {other}.")),
    };
    Ok(Call {
        screen,
        selector,
        action,
        what: what.to_string(),
        detail,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calls_become_design_operations() {
        let c = parse(
            "design_set_style",
            &json!({ "screen": "home", "selector": ".card", "styles": { "padding": "var(--space-4)" } }),
            &[],
        )
        .unwrap();
        assert_eq!(c.screen, "home");
        assert_eq!(c.detail, "set padding: var(--space-4) on .card");
        assert_eq!(
            c.action,
            Action::Op("styles", vec![json!({ "padding": "var(--space-4)" })])
        );
        let c = parse(
            "design_insert",
            &json!({ "screen": "home", "selector": "main", "html": "<p>Hi</p>", "position": "inside" }),
            &[],
        )
        .unwrap();
        assert_eq!(
            c.action,
            Action::Op("insertNew", vec![json!("<p>Hi</p>"), json!(true)])
        );
        assert!(parse(
            "design_set_attribute",
            &json!({ "screen": "home", "selector": "a", "name": "data-rid", "value": "x" }),
            &[]
        )
        .is_err());
        assert!(parse("design_set_text", &json!({ "screen": "home" }), &[]).is_err());
        assert!(parse(
            "design_fly",
            &json!({ "screen": "a", "selector": "b" }),
            &[]
        )
        .is_err());
    }

    #[test]
    fn calls_meet_the_canvas_in_a_folder() {
        let base = std::env::temp_dir().join(format!("runo-ops-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let b2 = base.clone();
        let agent = std::thread::spawn(move || {
            call(
                &b2,
                "/work/app",
                "design_remove",
                &json!({ "screen": "home", "selector": ".ad" }),
            )
        });
        let mut taken = Vec::new();
        for _ in 0..50 {
            assert!(take(&base, Path::new("/work/other")).is_empty());
            taken = take(&base, Path::new("/work/app"));
            if !taken.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(taken.len(), 1);
        assert_eq!(taken[0].tool, "design_remove");
        answer(&base, &taken[0].id, true, "Deleted 2 layers.");
        let out = agent.join().unwrap();
        assert_eq!(out["content"][0]["text"], "Deleted 2 layers.");
        assert_eq!(out["isError"], false);
        let _ = std::fs::remove_dir_all(&base);
    }
}
