//! Image generation. One MCP tool, `generate_image`, backed by one of four providers
//! the person picks in Settings > Skills: OpenAI (`gpt-image-1` / DALL·E 3), Stability
//! (`sd3`, `sd3.5`, …), Replicate (any model id), or a local command the person has
//! already installed (Stable Diffusion CLI, ComfyUI, Draw Things CLI, …).
//!
//! Pictures land under the session's `.runo/attachments/generated/` folder (same place
//! pasted images go, see `turns::ATTACHMENTS`) so the chat image viewer renders them
//! inline and the person can click into the lightbox. Nothing is written outside the
//! session folder.
//!
//! HTTP calls go through `curl`, matching the rest of Runo (see `applog::fetch_text`),
//! so no new TLS dependency comes with this skill and the same code paths work on
//! macOS, Linux and Windows.

use super::{failure, image_reply, BuiltinSkill, SkillSettings};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Backend ids as they live in settings. Short and stable.
pub const BACKENDS: &[(&str, &str)] = &[
    ("openai", "OpenAI (gpt-image-1 / DALL·E 3)"),
    ("stability", "Stability AI"),
    ("replicate", "Replicate"),
    (
        "local",
        "Local command (Stable Diffusion / ComfyUI / Draw Things)",
    ),
];

/// Per-skill settings shown on the Settings > Skills page. API keys sit next to the
/// rest of the settings: `settings.json` is already written 0600 by
/// `store::write_private`, and `applog::hide_secrets` masks any field whose name says
/// `key` in diagnostics. One file is simpler to back up and migrate than two.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImageSettings {
    /// Which backend runs the tool. One of `BACKENDS`.
    pub backend: String,
    /// Model id for the hosted backends. Empty uses the backend's default.
    pub openai_model: String,
    pub stability_engine: String,
    pub replicate_model: String,
    /// Shell command for the Local backend. Receives the prompt on stdin and must write
    /// a PNG to the path given as `$1` (first arg).
    pub local_command: String,
    /// Default image size when the agent doesn't give one. One of `SIZES`.
    pub default_size: String,
    /// Spending ceiling for hosted backends, in US cents per day. 0 is no cap. Each
    /// backend reports what it charges; the server adds them up over the day.
    pub daily_cost_limit_cents: u32,
    /// API keys for the hosted backends. Field names contain `key` so they are hidden
    /// by `applog::hide_secrets` in any diagnostic bundle.
    pub openai_key: String,
    pub stability_key: String,
    pub replicate_key: String,
}

impl Default for ImageSettings {
    fn default() -> Self {
        Self {
            backend: "openai".into(),
            openai_model: "gpt-image-1".into(),
            stability_engine: "sd3".into(),
            replicate_model: "stability-ai/stable-diffusion-3".into(),
            local_command: String::new(),
            default_size: "1024x1024".into(),
            daily_cost_limit_cents: 0,
            openai_key: String::new(),
            stability_key: String::new(),
            replicate_key: String::new(),
        }
    }
}

impl ImageSettings {
    /// Trimmed key for a backend, or None when the field is empty.
    pub fn key(&self, backend: &str) -> Option<&str> {
        let value = match backend {
            "openai" => &self.openai_key,
            "stability" => &self.stability_key,
            "replicate" => &self.replicate_key,
            _ => return None,
        };
        Some(value.trim()).filter(|s| !s.is_empty())
    }
}

/// Image sizes offered in Settings. Backends map these onto their own names.
pub const SIZES: &[&str] = &["512x512", "1024x1024", "1024x1792", "1792x1024"];

/// The one and only image-generation skill.
pub struct ImageGeneration;

impl BuiltinSkill for ImageGeneration {
    fn id(&self) -> &'static str {
        "image"
    }
    fn name(&self) -> &'static str {
        "Image generation"
    }
    fn description(&self) -> &'static str {
        "Give every agent a `generate_image` tool. Pictures land in the session's attachments and show inline."
    }
    fn tools(&self) -> Vec<Value> {
        vec![json!({
            "name": "generate_image",
            "description": "Generate one or more images from a text prompt. The images are saved under the session's attachments and shown inline in the chat; the reply gives you their paths.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "prompt": { "type": "string", "description": "What to draw. Be specific: subject, style, lighting, composition." },
                    "size": { "type": "string", "description": "WIDTHxHEIGHT, one of 512x512, 1024x1024, 1024x1792 or 1792x1024. Default from Settings.", "enum": SIZES },
                    "style": { "type": "string", "description": "A short style hint prepended to the prompt (e.g. \"watercolor\", \"photoreal\", \"isometric\"). Optional." },
                    "n": { "type": "integer", "description": "How many images (1-4). Default 1.", "minimum": 1, "maximum": 4 }
                },
                "required": ["prompt"]
            }
        })]
    }

    fn call(&self, dir: &str, settings: &SkillSettings, tool: &str, args: &Value) -> Value {
        if tool != "generate_image" {
            return failure(format!("Unknown tool {tool}"));
        }
        let req = match parse_request(args, &settings.image) {
            Ok(req) => req,
            Err(e) => return failure(e),
        };
        let folder = match attachments_folder(dir) {
            Ok(f) => f,
            Err(e) => return failure(e),
        };
        let image = &settings.image;
        let backend = image.backend.as_str();
        let outcome = match backend {
            "openai" => run_openai(&req, image, &folder, &save),
            "stability" => run_stability(&req, image, &folder, &save),
            "replicate" => run_replicate(&req, image, &folder, &save),
            "local" => run_local(&req, &image.local_command, &folder, &save),
            other => Err(format!(
                "Unknown image backend \"{other}\". Pick one in Settings > Skills."
            )),
        };
        match outcome {
            Ok(images) if images.is_empty() => failure("The backend returned no images."),
            Ok(images) => images_as_reply(&images, backend),
            Err(e) => failure(e),
        }
    }
}

/// Parsed and validated `generate_image` arguments.
#[derive(Debug, PartialEq)]
pub struct Request {
    pub prompt: String,
    pub size: String,
    pub style: String,
    pub n: u32,
}

/// Image bytes with the chosen file extension and an optional per-image cost in cents.
#[derive(Debug)]
pub struct Produced {
    pub bytes: Vec<u8>,
    pub ext: &'static str,
    pub cost_cents: Option<u32>,
}

pub fn parse_request(args: &Value, defaults: &ImageSettings) -> Result<Request, String> {
    let prompt = args
        .get("prompt")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or("Give a prompt to draw from.")?
        .to_string();
    let style = args
        .get("style")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    let size = args
        .get("size")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| defaults.default_size.clone());
    if !SIZES.contains(&size.as_str()) {
        return Err(format!(
            "Size \"{size}\" isn't supported. Pick one of {}.",
            SIZES.join(", ")
        ));
    }
    let n = args
        .get("n")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .clamp(1, 4) as u32;
    Ok(Request {
        prompt,
        size,
        style,
        n,
    })
}

/// Prompt with the (optional) style hint prepended, the way most backends want it.
pub fn full_prompt(req: &Request) -> String {
    if req.style.is_empty() {
        req.prompt.clone()
    } else {
        format!("{}, {}", req.style, req.prompt)
    }
}

/// Where generated images for the session in `dir` go. The parent is the same folder
/// pasted attachments go to; nothing new is opened outside the session.
pub fn attachments_folder(dir: &str) -> Result<PathBuf, String> {
    if dir.trim().is_empty() {
        return Err("The session has no folder to save pictures into.".into());
    }
    let folder = Path::new(dir)
        .join(crate::turns::ATTACHMENTS)
        .join("generated");
    std::fs::create_dir_all(&folder)
        .map_err(|e| format!("couldn't make the attachments folder: {e}"))?;
    crate::turns::exclude_runo_folder(dir);
    Ok(folder)
}

/// Save `data` into `folder` with a stamped name and the right extension.
fn save(folder: &Path, index: u32, ext: &str, data: &[u8]) -> Result<PathBuf, String> {
    let name = format!(
        "generated-{}-{index}.{ext}",
        chrono::Local::now().format("%Y%m%d-%H%M%S%3f")
    );
    let path = folder.join(name);
    std::fs::write(&path, data).map_err(|e| format!("couldn't save {}: {e}", path.display()))?;
    Ok(path)
}

/// What the HTTP backends return once bytes are on disk.
type Save = dyn Fn(&Path, u32, &str, &[u8]) -> Result<PathBuf, String>;

/// Information about one saved image.
#[derive(Debug)]
pub struct Saved {
    pub path: PathBuf,
    pub cost_cents: Option<u32>,
}

fn images_as_reply(images: &[Saved], backend: &str) -> Value {
    let total: Option<u32> = images
        .iter()
        .map(|s| s.cost_cents)
        .try_fold(0u32, |a, c| c.map(|c| a + c));
    let cost = match total {
        Some(c) if c > 0 => format!(" (~${:.2} on {backend})", c as f32 / 100.0),
        _ => String::new(),
    };
    if images.len() == 1 {
        image_reply(&images[0].path, &format!("Generated 1 image{cost}."))
    } else {
        // Several images: text summary + each as a resource.
        let paths: Vec<String> = images
            .iter()
            .map(|s| s.path.to_string_lossy().into_owned())
            .collect();
        let mut content = vec![json!({
            "type": "text",
            "text": format!(
                "Generated {} images{cost}.\n\nSaved:\n{}",
                images.len(),
                paths.iter().map(|p| format!("- {p}")).collect::<Vec<_>>().join("\n")
            )
        })];
        for saved in images {
            content.push(json!({
                "type": "resource",
                "resource": {
                    "uri": format!("file://{}", saved.path.to_string_lossy()),
                    "mimeType": "image/png"
                }
            }));
        }
        json!({ "content": content })
    }
}

// ---------- OpenAI ----------
//
// `POST https://api.openai.com/v1/images/generations`, bearer-authenticated. The
// response has one `{b64_json}` per image. `gpt-image-1` reports `usage.cost_cents`
// on some tiers; DALL·E 3 doesn't, so we fall back to the published prices.

/// Published price per image, in cents. Rough; backends that report their own override.
fn openai_fallback_cost(model: &str, size: &str) -> Option<u32> {
    match (model, size) {
        ("gpt-image-1", _) => Some(4),
        ("dall-e-3", "1024x1024") => Some(4),
        ("dall-e-3", _) => Some(8),
        _ => None,
    }
}

pub fn openai_request_body(model: &str, req: &Request) -> Value {
    json!({
        "model": model,
        "prompt": full_prompt(req),
        "n": req.n,
        "size": req.size,
        "response_format": "b64_json"
    })
}

pub fn parse_openai_response(body: &str) -> Result<Vec<Produced>, String> {
    let value: Value = serde_json::from_str(body)
        .map_err(|e| format!("OpenAI returned something that isn't JSON: {e}"))?;
    if let Some(err) = value.get("error") {
        return Err(err["message"]
            .as_str()
            .unwrap_or("OpenAI refused the request.")
            .to_string());
    }
    let data = value
        .get("data")
        .and_then(Value::as_array)
        .ok_or("OpenAI's reply had no `data` array.")?;
    let mut out = Vec::new();
    for entry in data {
        let b64 = entry["b64_json"]
            .as_str()
            .ok_or("OpenAI's reply had no base64 image.")?;
        let bytes = decode_base64(b64)?;
        out.push(Produced {
            bytes,
            ext: "png",
            cost_cents: None,
        });
    }
    Ok(out)
}

fn run_openai(
    req: &Request,
    image: &ImageSettings,
    folder: &Path,
    save_fn: &Save,
) -> Result<Vec<Saved>, String> {
    let key = image
        .key("openai")
        .ok_or("OpenAI needs an API key: set one in Settings > Skills > Image generation.")?;
    let model = default_or("gpt-image-1", &image.openai_model);
    let body = openai_request_body(&model, req);
    let text = post_json(
        "https://api.openai.com/v1/images/generations",
        &[
            ("Authorization", &format!("Bearer {key}")),
            ("Content-Type", "application/json"),
        ],
        &body.to_string(),
    )?;
    let produced = parse_openai_response(&text)?;
    let fallback = openai_fallback_cost(&model, &req.size);
    save_produced(produced, fallback, folder, save_fn)
}

// ---------- Stability ----------
//
// `POST https://api.stability.ai/v2beta/stable-image/generate/sd3` with
// `multipart/form-data`. Response is the image bytes themselves (image/*), or a JSON
// error.

pub fn stability_endpoint(engine: &str) -> String {
    let engine = if engine.trim().is_empty() {
        "sd3"
    } else {
        engine
    };
    format!("https://api.stability.ai/v2beta/stable-image/generate/{engine}")
}

/// Rough per-image cost in cents.
fn stability_fallback_cost(engine: &str) -> u32 {
    match engine {
        "sd3" => 6,
        "sd3.5" => 7,
        "core" => 3,
        _ => 5,
    }
}

fn run_stability(
    req: &Request,
    image: &ImageSettings,
    folder: &Path,
    save_fn: &Save,
) -> Result<Vec<Saved>, String> {
    let key = image
        .key("stability")
        .ok_or("Stability needs an API key: set one in Settings > Skills > Image generation.")?;
    let engine = image.stability_engine.clone();
    let endpoint = stability_endpoint(&engine);
    let mut out = Vec::new();
    for i in 0..req.n {
        let bytes = post_multipart(
            &endpoint,
            &[
                ("Authorization", &format!("Bearer {key}")),
                ("Accept", "image/*"),
            ],
            &[
                ("prompt", &full_prompt(req)),
                ("output_format", "png"),
                ("aspect_ratio", &stability_aspect(&req.size)),
            ],
        )?;
        if bytes.starts_with(b"{") {
            let txt = String::from_utf8_lossy(&bytes);
            return Err(stability_error(&txt));
        }
        let path = save_fn(folder, i + 1, "png", &bytes)?;
        out.push(Saved {
            path,
            cost_cents: Some(stability_fallback_cost(&engine)),
        });
    }
    Ok(out)
}

pub fn stability_aspect(size: &str) -> String {
    match size {
        "1024x1792" => "9:16".into(),
        "1792x1024" => "16:9".into(),
        _ => "1:1".into(),
    }
}

pub fn stability_error(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| {
            v["errors"][0]
                .as_str()
                .map(String::from)
                .or_else(|| v["message"].as_str().map(String::from))
        })
        .unwrap_or_else(|| "Stability refused the request.".into())
}

// ---------- Replicate ----------
//
// `POST https://api.replicate.com/v1/predictions`, then poll
// `GET https://api.replicate.com/v1/predictions/<id>` until the prediction is done.
// `output` is an array of image URLs; fetched one by one.

pub fn replicate_create_body(model: &str, req: &Request) -> Value {
    let (w, h) = size_dims(&req.size);
    json!({
        "version": model,
        "input": {
            "prompt": full_prompt(req),
            "width": w,
            "height": h,
            "num_outputs": req.n
        }
    })
}

fn size_dims(size: &str) -> (u32, u32) {
    size.split_once('x')
        .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
        .unwrap_or((1024, 1024))
}

pub fn parse_replicate_output(body: &str) -> Result<Vec<String>, String> {
    let value: Value = serde_json::from_str(body)
        .map_err(|e| format!("Replicate returned something that isn't JSON: {e}"))?;
    if let Some(detail) = value.get("detail").and_then(Value::as_str) {
        return Err(detail.to_string());
    }
    match value.get("output") {
        Some(Value::Array(urls)) => Ok(urls
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect()),
        Some(Value::String(url)) => Ok(vec![url.clone()]),
        _ => Err("Replicate's prediction had no output.".into()),
    }
}

fn run_replicate(
    req: &Request,
    image: &ImageSettings,
    folder: &Path,
    save_fn: &Save,
) -> Result<Vec<Saved>, String> {
    let key = image
        .key("replicate")
        .ok_or("Replicate needs an API token: set one in Settings > Skills > Image generation.")?;
    let model = image.replicate_model.clone();
    let auth = format!("Token {key}");
    let created = post_json(
        "https://api.replicate.com/v1/predictions",
        &[
            ("Authorization", &auth),
            ("Content-Type", "application/json"),
        ],
        &replicate_create_body(&model, req).to_string(),
    )?;
    let value: Value = serde_json::from_str(&created)
        .map_err(|e| format!("Replicate returned something that isn't JSON: {e}"))?;
    let id = value["id"]
        .as_str()
        .ok_or("Replicate didn't give the prediction an id.")?;
    let urls = poll_replicate(&auth, id)?;
    let mut out = Vec::new();
    for (i, url) in urls.iter().enumerate() {
        let bytes = fetch_bytes(url, &[])?;
        let path = save_fn(folder, i as u32 + 1, "png", &bytes)?;
        out.push(Saved {
            path,
            cost_cents: None,
        });
    }
    Ok(out)
}

fn poll_replicate(auth: &str, id: &str) -> Result<Vec<String>, String> {
    let url = format!("https://api.replicate.com/v1/predictions/{id}");
    for _ in 0..60 {
        let body = get_text(&url, &[("Authorization", auth)])?;
        let value: Value = serde_json::from_str(&body)
            .map_err(|e| format!("Replicate returned something that isn't JSON: {e}"))?;
        match value["status"].as_str().unwrap_or("") {
            "succeeded" => return parse_replicate_output(&body),
            "failed" | "canceled" => {
                return Err(value["error"]
                    .as_str()
                    .unwrap_or("Replicate failed.")
                    .to_string())
            }
            _ => std::thread::sleep(std::time::Duration::from_secs(2)),
        }
    }
    Err("Replicate took too long.".into())
}

// ---------- Local ----------
//
// The person's own CLI. The command runs with the prompt on stdin and the output
// path as `$1`. The usual pattern for Draw Things / sdcli / ComfyUI wrappers.

fn run_local(
    req: &Request,
    command: &str,
    folder: &Path,
    save_fn: &Save,
) -> Result<Vec<Saved>, String> {
    let cmd = command.trim();
    if cmd.is_empty() {
        return Err(
            "The Local backend needs a shell command: set one in Settings > Skills > Image generation."
                .into(),
        );
    }
    let mut out = Vec::new();
    for i in 0..req.n {
        let placeholder = save_fn(folder, i + 1, "png", &[])?;
        run_local_shell(cmd, &full_prompt(req), &placeholder, &req.size)?;
        let bytes = std::fs::read(&placeholder).map_err(|e| {
            format!(
                "the local command didn't leave a file at {}: {e}",
                placeholder.display()
            )
        })?;
        if bytes.is_empty() {
            return Err(format!(
                "the local command left no bytes at {}; the command may have failed silently",
                placeholder.display()
            ));
        }
        out.push(Saved {
            path: placeholder,
            cost_cents: Some(0),
        });
    }
    Ok(out)
}

/// Spawn the user's shell command. Prompt on stdin, out-path and size as arguments,
/// so the person's wrapper can read them without any Runo-specific shape.
fn run_local_shell(command: &str, prompt: &str, out: &Path, size: &str) -> Result<(), String> {
    use std::io::Write as _;
    use std::process::Stdio;
    let (shell, flag) = if cfg!(windows) {
        ("cmd", "/C")
    } else {
        ("sh", "-c")
    };
    let full = format!(
        "{command} {} {}",
        quote_arg(&out.to_string_lossy()),
        quote_arg(size)
    );
    let mut child = Command::new(shell)
        .args([flag, &full])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("couldn't start the local command: {e}"))?;
    if let Some(stdin) = child.stdin.as_mut() {
        let _ = stdin.write_all(prompt.as_bytes());
    }
    let output = child
        .wait_with_output()
        .map_err(|e| format!("the local command failed: {e}"))?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(format!(
            "the local command exited {}: {}",
            output.status, err
        ));
    }
    Ok(())
}

/// Shell-safe quote of one argument. Spaces, quotes and backslashes are handled.
fn quote_arg(arg: &str) -> String {
    if cfg!(windows) {
        format!("\"{}\"", arg.replace('"', "\\\""))
    } else {
        format!("'{}'", arg.replace('\'', "'\\''"))
    }
}

// ---------- HTTP via curl ----------
//
// Hosted backends go over HTTPS via `curl`. Keys never land in a log line (see
// `applog::redact_text`): the headers list only reaches curl's argv, never a log sink.

fn post_json(url: &str, headers: &[(&str, &str)], body: &str) -> Result<String, String> {
    let out = curl_cmd()
        .args(["-sS", "--fail-with-body", "--max-time", "180"])
        .args(header_flags(headers))
        .args(["-X", "POST", "-d", body])
        .arg(url)
        .output()
        .map_err(|e| format!("couldn't run curl: {e}"))?;
    let body = String::from_utf8_lossy(&out.stdout).into_owned();
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!(
            "the request failed ({}): {}",
            out.status,
            err.trim()
        ));
    }
    Ok(body)
}

fn post_multipart(
    url: &str,
    headers: &[(&str, &str)],
    fields: &[(&str, &str)],
) -> Result<Vec<u8>, String> {
    let mut cmd = curl_cmd();
    cmd.args(["-sS", "--fail-with-body", "--max-time", "180"])
        .args(header_flags(headers));
    for (k, v) in fields {
        cmd.arg("-F").arg(format!("{k}={v}"));
    }
    cmd.arg(url);
    let out = cmd
        .output()
        .map_err(|e| format!("couldn't run curl: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!(
            "the request failed ({}): {}",
            out.status,
            err.trim()
        ));
    }
    Ok(out.stdout)
}

fn get_text(url: &str, headers: &[(&str, &str)]) -> Result<String, String> {
    let out = curl_cmd()
        .args(["-sS", "--fail-with-body", "--max-time", "60"])
        .args(header_flags(headers))
        .arg(url)
        .output()
        .map_err(|e| format!("couldn't run curl: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "the request failed ({}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn fetch_bytes(url: &str, headers: &[(&str, &str)]) -> Result<Vec<u8>, String> {
    let out = curl_cmd()
        .args(["-sSL", "--max-time", "180"])
        .args(header_flags(headers))
        .arg(url)
        .output()
        .map_err(|e| format!("couldn't run curl: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "the download failed ({}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(out.stdout)
}

fn curl_cmd() -> Command {
    let mut cmd = Command::new("curl");
    cmd.stdin(std::process::Stdio::null());
    cmd
}

fn header_flags(headers: &[(&str, &str)]) -> Vec<String> {
    let mut flags = Vec::new();
    for (k, v) in headers {
        flags.push("-H".to_string());
        flags.push(format!("{k}: {v}"));
    }
    flags
}

fn save_produced(
    produced: Vec<Produced>,
    fallback_cost: Option<u32>,
    folder: &Path,
    save_fn: &Save,
) -> Result<Vec<Saved>, String> {
    let mut out = Vec::new();
    for (i, p) in produced.into_iter().enumerate() {
        let path = save_fn(folder, i as u32 + 1, p.ext, &p.bytes)?;
        out.push(Saved {
            path,
            cost_cents: p.cost_cents.or(fallback_cost),
        });
    }
    Ok(out)
}

fn default_or(fallback: &str, chosen: &str) -> String {
    if chosen.trim().is_empty() {
        fallback.to_string()
    } else {
        chosen.trim().to_string()
    }
}

/// Decode a standard base64 string. The `base64` crate is already a dependency; we go
/// through its runtime API so no feature flag is needed.
fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine as _;
    // Trim whitespace that some providers wrap base64 with.
    let clean: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    STANDARD
        .decode(clean.as_bytes())
        .map_err(|e| format!("couldn't decode base64 image: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn req(prompt: &str, size: &str, n: u32) -> Request {
        Request {
            prompt: prompt.into(),
            size: size.into(),
            style: String::new(),
            n,
        }
    }

    #[test]
    fn parse_request_takes_defaults_and_clamps_n() {
        let defaults = ImageSettings {
            default_size: "1024x1024".into(),
            ..Default::default()
        };
        let r = parse_request(&json!({ "prompt": "a cat" }), &defaults).unwrap();
        assert_eq!(r.size, "1024x1024");
        assert_eq!(r.n, 1);
        let r = parse_request(&json!({ "prompt": "a cat", "n": 99 }), &defaults).unwrap();
        assert_eq!(r.n, 4);
        assert!(parse_request(&json!({ "prompt": "" }), &defaults).is_err());
        assert!(parse_request(&json!({ "prompt": "x", "size": "7x7" }), &defaults).is_err());
    }

    #[test]
    fn full_prompt_prepends_the_style() {
        let r = Request {
            prompt: "a cat".into(),
            size: "1024x1024".into(),
            style: "watercolor".into(),
            n: 1,
        };
        assert_eq!(full_prompt(&r), "watercolor, a cat");
        let plain = req("a cat", "1024x1024", 1);
        assert_eq!(full_prompt(&plain), "a cat");
    }

    #[test]
    fn openai_body_shape() {
        let body =
            openai_request_body("gpt-image-1", &req("a cat on a skateboard", "1024x1024", 2));
        assert_eq!(body["model"], "gpt-image-1");
        assert_eq!(body["prompt"], "a cat on a skateboard");
        assert_eq!(body["n"], 2);
        assert_eq!(body["size"], "1024x1024");
        assert_eq!(body["response_format"], "b64_json");
    }

    #[test]
    fn openai_response_decodes_base64() {
        use base64::engine::general_purpose::STANDARD;
        use base64::Engine as _;
        let bytes = b"\x89PNG\r\n\x1a\n fake".to_vec();
        let b64 = STANDARD.encode(&bytes);
        let body = json!({ "data": [{ "b64_json": b64 }] }).to_string();
        let produced = parse_openai_response(&body).unwrap();
        assert_eq!(produced.len(), 1);
        assert_eq!(produced[0].bytes, bytes);
        assert_eq!(produced[0].ext, "png");
        // Errors come through.
        let err = parse_openai_response("{\"error\":{\"message\":\"nope\"}}").unwrap_err();
        assert_eq!(err, "nope");
    }

    #[test]
    fn stability_endpoint_defaults_and_aspect() {
        assert_eq!(
            stability_endpoint(""),
            "https://api.stability.ai/v2beta/stable-image/generate/sd3"
        );
        assert_eq!(
            stability_endpoint("sd3.5"),
            "https://api.stability.ai/v2beta/stable-image/generate/sd3.5"
        );
        assert_eq!(stability_aspect("1024x1024"), "1:1");
        assert_eq!(stability_aspect("1024x1792"), "9:16");
        assert_eq!(stability_aspect("1792x1024"), "16:9");
    }

    #[test]
    fn stability_error_pulls_a_message() {
        assert_eq!(
            stability_error("{\"errors\":[\"bad prompt\"]}"),
            "bad prompt"
        );
        assert_eq!(
            stability_error("{\"message\":\"invalid key\"}"),
            "invalid key"
        );
        assert_eq!(
            stability_error("not json"),
            "Stability refused the request."
        );
    }

    #[test]
    fn replicate_body_names_dimensions() {
        let body = replicate_create_body("ver-abc", &req("a cat", "1792x1024", 3));
        assert_eq!(body["version"], "ver-abc");
        assert_eq!(body["input"]["prompt"], "a cat");
        assert_eq!(body["input"]["width"], 1792);
        assert_eq!(body["input"]["height"], 1024);
        assert_eq!(body["input"]["num_outputs"], 3);
    }

    #[test]
    fn replicate_output_handles_strings_arrays_and_errors() {
        let urls =
            parse_replicate_output(r#"{"output":["https://a/1.png","https://a/2.png"]}"#).unwrap();
        assert_eq!(urls.len(), 2);
        let one = parse_replicate_output(r#"{"output":"https://x/1.png"}"#).unwrap();
        assert_eq!(one, vec!["https://x/1.png"]);
        let err = parse_replicate_output(r#"{"detail":"model not found"}"#).unwrap_err();
        assert_eq!(err, "model not found");
    }

    #[test]
    fn images_as_reply_includes_resources_and_cost() {
        let dir = std::env::temp_dir().join(format!("runo-img-reply-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p1 = dir.join("a.png");
        let p2 = dir.join("b.png");
        std::fs::write(&p1, b"a").unwrap();
        std::fs::write(&p2, b"b").unwrap();
        let reply = images_as_reply(
            &[
                Saved {
                    path: p1.clone(),
                    cost_cents: Some(4),
                },
                Saved {
                    path: p2.clone(),
                    cost_cents: Some(4),
                },
            ],
            "openai",
        );
        let text = reply["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("Generated 2 images"));
        assert!(text.contains("$0.08"));
        assert_eq!(
            reply["content"][1]["resource"]["uri"],
            format!("file://{}", p1.display())
        );
        assert_eq!(
            reply["content"][2]["resource"]["uri"],
            format!("file://{}", p2.display())
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn attachments_folder_lives_under_the_session() {
        let dir = std::env::temp_dir().join(format!("runo-img-att-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let folder = attachments_folder(dir.to_str().unwrap()).unwrap();
        assert!(folder.ends_with(".runo/attachments/generated"));
        assert!(folder.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn size_dims_round_trip() {
        assert_eq!(size_dims("1024x1024"), (1024, 1024));
        assert_eq!(size_dims("1792x1024"), (1792, 1024));
        assert_eq!(size_dims("junk"), (1024, 1024));
    }

    #[test]
    fn quote_arg_escapes_quotes() {
        if cfg!(windows) {
            assert_eq!(quote_arg("he said \"hi\""), "\"he said \\\"hi\\\"\"");
        } else {
            assert_eq!(quote_arg("it's"), "'it'\\''s'");
            assert_eq!(quote_arg("plain"), "'plain'");
        }
    }

    #[test]
    fn local_backend_requires_a_command() {
        let folder = std::env::temp_dir().join(format!("runo-img-local-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let saver = |f: &Path, i: u32, ext: &str, b: &[u8]| -> Result<PathBuf, String> {
            let p = f.join(format!("x-{i}.{ext}"));
            std::fs::write(&p, b).map_err(|e| e.to_string())?;
            Ok(p)
        };
        let err = run_local(&req("hi", "1024x1024", 1), "", &folder, &saver).unwrap_err();
        assert!(err.contains("needs a shell command"));
        let _ = std::fs::remove_dir_all(folder);
    }

    #[test]
    fn skill_reports_itself() {
        let s = ImageGeneration;
        assert_eq!(s.id(), "image");
        assert!(s.owns("generate_image"));
        assert!(!s.owns("read_memory"));
        let tools = s.tools();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "generate_image");
    }
}
