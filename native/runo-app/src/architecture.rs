//! The Architecture page: the project's structure read from its manifests and imports,
//! architecture notes and decisions kept in the repository, and agent sessions that start
//! with both attached.

use super::*;
use crate::archdraw::view::{ArchEditor, ArchEvent};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Folders never read for the map: build output, dependencies and tool state.
const SKIP_DIRS: [&str; 14] = [
    "node_modules",
    "target",
    "dist",
    "build",
    "out",
    "vendor",
    "venv",
    "__pycache__",
    "Pods",
    "DerivedData",
    "coverage",
    "site-packages",
    "third_party",
    "worktrees",
];

/// Files read at most, so a huge repository still maps quickly.
const MAX_FILES: usize = 6000;
/// Modules shown per package, largest first.
const MAX_MODULES: usize = 60;

/// A buildable unit found from its manifest: a crate, an npm package, a Python project, a
/// Go module, a Swift package, a Maven or Gradle project, a Ruby gem, a .NET project, a
/// Composer package or a Dart package.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Package {
    pub name: String,
    /// Its folder, relative to the project; empty for the project root.
    pub path: String,
    pub kind: &'static str,
    /// Dependencies from outside the repository.
    pub deps: Vec<String>,
    /// Other packages of this repository it depends on.
    pub internal: Vec<String>,
    pub files: usize,
    pub modules: Vec<Module>,
}

/// A part of a package: a top-level Rust module, a source folder or a Python package.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Module {
    pub name: String,
    pub files: usize,
    pub lines: usize,
    /// Other modules of the same package it refers to.
    pub uses: Vec<String>,
}

/// What the Architecture page shows for a project.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ProjectMap {
    pub packages: Vec<Package>,
    /// Source files by language, most first.
    pub languages: Vec<(String, usize)>,
    /// Top-level folders and how many files each holds.
    pub folders: Vec<(String, usize)>,
    pub files: usize,
    /// The file limit was reached; the map covers part of the project.
    pub partial: bool,
}

/// The language of a source file, by extension.
fn language(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "rs" => "Rust",
        "ts" | "tsx" | "mts" | "cts" => "TypeScript",
        "js" | "jsx" | "mjs" | "cjs" => "JavaScript",
        "vue" => "Vue",
        "svelte" => "Svelte",
        "py" => "Python",
        "go" => "Go",
        "swift" => "Swift",
        "kt" | "kts" => "Kotlin",
        "java" => "Java",
        "rb" => "Ruby",
        "php" => "PHP",
        "c" | "h" => "C",
        "cc" | "cpp" | "hpp" | "cxx" => "C++",
        "cs" => "C#",
        "ex" | "exs" => "Elixir",
        "dart" => "Dart",
        "scala" => "Scala",
        "css" | "scss" | "sass" | "less" => "CSS",
        "html" => "HTML",
        "sql" => "SQL",
        "sh" | "bash" | "zsh" => "Shell",
        _ => return None,
    })
}

fn is_js(path: &Path) -> bool {
    matches!(
        language(path),
        Some("TypeScript" | "JavaScript" | "Vue" | "Svelte")
    )
}

/// Every file under `root` worth mapping, as paths relative to it, and whether the limit
/// cut the walk short. Hidden folders, build output and dependencies are skipped.
pub(crate) fn walk(root: &Path) -> (Vec<PathBuf>, bool) {
    fn go(root: &Path, dir: &Path, depth: usize, out: &mut Vec<PathBuf>) -> bool {
        if depth > 8 {
            return false;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return false;
        };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            let path = entry.path();
            if kind.is_dir() {
                if name.starts_with('.') || SKIP_DIRS.contains(&name.as_str()) {
                    continue;
                }
                if go(root, &path, depth + 1, out) {
                    return true;
                }
            } else if kind.is_file() {
                if out.len() >= MAX_FILES {
                    return true;
                }
                if let Ok(rel) = path.strip_prefix(root) {
                    out.push(rel.to_path_buf());
                }
            }
        }
        false
    }
    let mut out = Vec::new();
    let partial = go(root, root, 0, &mut out);
    (out, partial)
}

fn rel_dir(file: &Path) -> String {
    file.parent()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default()
}

/// A package read from a manifest: (name, kind, external deps, deps by path or workspace).
type Manifest = (String, &'static str, Vec<String>, Vec<String>);

/// Read a `Cargo.toml`. A workspace root without a `[package]` is not a package.
pub(crate) fn read_cargo(text: &str) -> Option<Manifest> {
    let doc: toml::Table = text.parse().ok()?;
    let name = doc.get("package")?.get("name")?.as_str()?.to_string();
    let mut deps = BTreeSet::new();
    let mut local = BTreeSet::new();
    let mut tables: Vec<&toml::Table> = Vec::new();
    for key in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(t) = doc.get(key).and_then(|v| v.as_table()) {
            tables.push(t);
        }
    }
    if let Some(targets) = doc.get("target").and_then(|v| v.as_table()) {
        for target in targets.values() {
            for key in ["dependencies", "dev-dependencies", "build-dependencies"] {
                if let Some(t) = target.get(key).and_then(|v| v.as_table()) {
                    tables.push(t);
                }
            }
        }
    }
    for table in tables {
        for (dep, spec) in table {
            let dep = spec
                .get("package")
                .and_then(|p| p.as_str())
                .unwrap_or(dep)
                .to_string();
            if spec.get("path").is_some() {
                local.insert(dep);
            } else {
                deps.insert(dep);
            }
        }
    }
    Some((
        name,
        "Rust crate",
        deps.into_iter().collect(),
        local.into_iter().collect(),
    ))
}

/// Read a `package.json`.
pub(crate) fn read_package_json(text: &str, dir_name: &str) -> Option<Manifest> {
    let doc: serde_json::Value = serde_json::from_str(text).ok()?;
    let name = doc["name"]
        .as_str()
        .filter(|n| !n.is_empty())
        .unwrap_or(dir_name)
        .to_string();
    let mut deps = BTreeSet::new();
    let mut local = BTreeSet::new();
    for key in ["dependencies", "devDependencies", "peerDependencies"] {
        if let Some(map) = doc[key].as_object() {
            for (dep, version) in map {
                let v = version.as_str().unwrap_or_default();
                if v.starts_with("workspace:") || v.starts_with("file:") || v.starts_with("link:") {
                    local.insert(dep.clone());
                } else {
                    deps.insert(dep.clone());
                }
            }
        }
    }
    Some((
        name,
        "npm package",
        deps.into_iter().collect(),
        local.into_iter().collect(),
    ))
}

/// The package name in a Python requirement such as `requests[socks]>=2.0; python_version<"4"`.
fn requirement_name(req: &str) -> Option<String> {
    let name: String = req
        .trim()
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .collect();
    (!name.is_empty()).then_some(name)
}

/// Read a `pyproject.toml`: PEP 621 `[project]` or Poetry.
pub(crate) fn read_pyproject(text: &str, dir_name: &str) -> Option<Manifest> {
    let doc: toml::Table = text.parse().ok()?;
    let project = doc.get("project");
    let poetry = doc.get("tool").and_then(|t| t.get("poetry"));
    let name = project
        .or(poetry)
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or(dir_name)
        .to_string();
    let mut deps = BTreeSet::new();
    if let Some(list) = project
        .and_then(|p| p.get("dependencies"))
        .and_then(|d| d.as_array())
    {
        deps.extend(list.iter().filter_map(|d| requirement_name(d.as_str()?)));
    }
    if let Some(table) = poetry
        .and_then(|p| p.get("dependencies"))
        .and_then(|d| d.as_table())
    {
        deps.extend(table.keys().filter(|k| *k != "python").cloned());
    }
    Some((
        name,
        "Python project",
        deps.into_iter().collect(),
        Vec::new(),
    ))
}

/// Read a `requirements.txt`, for Python projects without a `pyproject.toml`.
pub(crate) fn read_requirements(text: &str, dir_name: &str) -> Manifest {
    let deps: BTreeSet<String> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with('-'))
        .filter_map(requirement_name)
        .collect();
    (
        dir_name.to_string(),
        "Python project",
        deps.into_iter().collect(),
        Vec::new(),
    )
}

/// Read a `go.mod`: the module path and what it requires.
pub(crate) fn read_go_mod(text: &str) -> Option<Manifest> {
    let mut name = None;
    let mut deps = BTreeSet::new();
    let mut block = false;
    for line in text.lines() {
        let line = line.split("//").next().unwrap_or("").trim();
        if let Some(m) = line.strip_prefix("module ") {
            name = Some(m.trim().trim_matches('"').to_string());
        } else if line.starts_with("require (") || line == "require(" {
            block = true;
        } else if block && line == ")" {
            block = false;
        } else if let Some(dep) = line.strip_prefix("require ") {
            deps.extend(dep.split_whitespace().next().map(str::to_string));
        } else if block {
            deps.extend(line.split_whitespace().next().map(str::to_string));
        }
    }
    Some((name?, "Go module", deps.into_iter().collect(), Vec::new()))
}

/// The quoted strings in `text`, in order, with either quote.
fn quoted_strings(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '"' || c == '\'' {
            let inner: String = chars.by_ref().take_while(|&d| d != c).collect();
            out.push(inner);
        }
    }
    out
}

/// The last part of a path or URL, without `.git` or a project file's extension.
fn last_part(path: &str) -> String {
    let part = path
        .trim_end_matches('/')
        .rsplit(['/', '\\', ':'])
        .next()
        .unwrap_or(path);
    let part = part.strip_suffix(".git").unwrap_or(part);
    match part.rsplit_once('.') {
        Some((stem, "csproj" | "fsproj" | "vbproj")) => stem.to_string(),
        _ => part.to_string(),
    }
}

/// Read a `Package.swift`: the package's name and the packages it depends on, by URL or
/// by local path.
pub(crate) fn read_swift_package(text: &str, dir_name: &str) -> Option<Manifest> {
    let body = &text[text.find("Package(")?..];
    let name = body
        .find("name:")
        .and_then(|at| quoted_strings(&body[at..]).into_iter().next())
        .unwrap_or_else(|| dir_name.to_string());
    let mut deps = BTreeSet::new();
    let mut local = BTreeSet::new();
    for call in body.split(".package(").skip(1) {
        let args = call.split(')').next().unwrap_or("");
        let value = |key: &str| {
            args.find(key)
                .and_then(|at| quoted_strings(&args[at..]).into_iter().next())
        };
        if let Some(path) = value("path:") {
            local.insert(last_part(&path));
        } else if let Some(name) = value("name:") {
            deps.insert(name);
        } else if let Some(url) = value("url:") {
            deps.insert(last_part(&url));
        }
    }
    Some((
        name,
        "Swift package",
        deps.into_iter().collect(),
        local.into_iter().collect(),
    ))
}

/// The text inside each `<tag>…</tag>` in `xml`.
fn xml_values<'a>(xml: &'a str, tag: &str) -> Vec<&'a str> {
    let (open, close) = (format!("<{tag}>"), format!("</{tag}>"));
    xml.split(open.as_str())
        .skip(1)
        .filter_map(|rest| rest.split_once(close.as_str()).map(|(v, _)| v.trim()))
        .collect()
}

/// Read a Maven `pom.xml`: its artifactId and the artifacts it depends on.
pub(crate) fn read_pom(text: &str, dir_name: &str) -> Option<Manifest> {
    if !text.contains("<project") {
        return None;
    }
    // The parent's and the dependencies' ids are not the project's own.
    let own = match (text.find("<parent>"), text.find("</parent>")) {
        (Some(a), Some(b)) if a < b => format!("{}{}", &text[..a], &text[b..]),
        _ => text.to_string(),
    };
    let own = own.split("<dependencies>").next().unwrap_or("");
    let own = own.split("<dependencyManagement>").next().unwrap_or("");
    let name = xml_values(own, "artifactId")
        .first()
        .map(|n| n.to_string())
        .unwrap_or_else(|| dir_name.to_string());
    let deps: BTreeSet<String> = xml_values(text, "dependency")
        .into_iter()
        .filter_map(|d| xml_values(d, "artifactId").first().map(|a| a.to_string()))
        .collect();
    Some((
        name,
        "Maven project",
        deps.into_iter().collect(),
        Vec::new(),
    ))
}

/// Read a Gradle build script (Groovy or Kotlin), or a settings script for the name.
/// Dependencies are `group:artifact:version` strings, version catalog entries and
/// `project(":sub")` references to other projects of the build.
pub(crate) fn read_gradle(text: &str, dir_name: &str) -> Manifest {
    let mut name = dir_name.to_string();
    let mut deps = BTreeSet::new();
    let mut local = BTreeSet::new();
    for line in text.lines() {
        let line = line.split("//").next().unwrap_or("").trim();
        if let Some(rest) = line.strip_prefix("rootProject.name") {
            if let Some(n) = quoted_strings(rest).into_iter().next() {
                name = n;
            }
            continue;
        }
        let Some(call) = line.split(['(', ' ']).next() else {
            continue;
        };
        let configuration = call.ends_with("mplementation")
            || call.ends_with("Only")
            || matches!(
                call,
                "api" | "kapt" | "ksp" | "annotationProcessor" | "classpath"
            );
        if !configuration {
            continue;
        }
        if let Some(at) = line.find("project(") {
            if let Some(p) = quoted_strings(&line[at..]).into_iter().next() {
                local.insert(last_part(&p));
            }
        } else if let Some(at) = line.find("libs.") {
            let alias: String = line[at + 5..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
                .collect();
            deps.insert(alias.replace('.', "-"));
        } else if let Some(coords) = quoted_strings(line).into_iter().next() {
            let mut parts = coords.split(':');
            if let (Some(_), Some(artifact)) = (parts.next(), parts.next()) {
                deps.insert(artifact.to_string());
            }
        }
    }
    (
        name,
        "Gradle project",
        deps.into_iter().collect(),
        local.into_iter().collect(),
    )
}

/// Read a `Gemfile` or a `.gemspec`: the gem's name and the gems it uses.
pub(crate) fn read_ruby(text: &str, dir_name: &str) -> Manifest {
    let mut name = dir_name.to_string();
    let mut deps = BTreeSet::new();
    let mut local = BTreeSet::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let first = || quoted_strings(line).into_iter().next();
        if line.contains(".name") && line.contains('=') {
            if let Some(n) = first() {
                name = n;
            }
        } else if line.starts_with("gem ") || line.starts_with("gem(") {
            if let Some(gem) = first() {
                if line.contains("path:") || line.contains(":path") {
                    local.insert(gem);
                } else {
                    deps.insert(gem);
                }
            }
        } else if line.contains("add_dependency")
            || line.contains("add_runtime_dependency")
            || line.contains("add_development_dependency")
        {
            deps.extend(first());
        }
    }
    (
        name,
        "Ruby project",
        deps.into_iter().collect(),
        local.into_iter().collect(),
    )
}

/// The value of attribute `name` in each `<tag …>` element of `xml`.
fn xml_attrs(xml: &str, tag: &str, name: &str) -> Vec<String> {
    let open = format!("<{tag} ");
    let key = format!("{name}=\"");
    xml.split(open.as_str())
        .skip(1)
        .filter_map(|rest| {
            let element = rest.split('>').next()?;
            let at = element.find(&key)? + key.len();
            Some(element[at..].split('"').next()?.to_string())
        })
        .collect()
}

/// Read a .NET project file (`.csproj`, `.fsproj`, `.vbproj`): NuGet packages and the
/// projects it references.
pub(crate) fn read_dotnet_project(text: &str, file_stem: &str) -> Manifest {
    let name = xml_values(text, "AssemblyName")
        .first()
        .map(|n| n.to_string())
        .unwrap_or_else(|| file_stem.to_string());
    let deps: BTreeSet<String> = xml_attrs(text, "PackageReference", "Include")
        .into_iter()
        .collect();
    let local: BTreeSet<String> = xml_attrs(text, "ProjectReference", "Include")
        .iter()
        .map(|p| last_part(p))
        .collect();
    (
        name,
        ".NET project",
        deps.into_iter().collect(),
        local.into_iter().collect(),
    )
}

/// Read a `.sln`: the solution and the projects it holds.
pub(crate) fn read_sln(text: &str, file_stem: &str) -> Manifest {
    let local: BTreeSet<String> = text
        .lines()
        .filter(|l| l.starts_with("Project("))
        .filter_map(|l| {
            let (_, rest) = l.split_once('=')?;
            let path = quoted_strings(rest).into_iter().nth(1)?;
            let project = last_part(&path);
            // Solution folders list themselves as the path.
            (project != path || path.contains('.')).then_some(project)
        })
        .collect();
    (
        file_stem.to_string(),
        ".NET solution",
        Vec::new(),
        local.into_iter().collect(),
    )
}

/// Read a `composer.json`.
pub(crate) fn read_composer(text: &str, dir_name: &str) -> Option<Manifest> {
    let doc: serde_json::Value = serde_json::from_str(text).ok()?;
    let name = doc["name"]
        .as_str()
        .filter(|n| !n.is_empty())
        .unwrap_or(dir_name)
        .to_string();
    let mut deps = BTreeSet::new();
    for key in ["require", "require-dev"] {
        if let Some(map) = doc[key].as_object() {
            deps.extend(
                map.keys()
                    .filter(|d| *d != "php" && !d.starts_with("ext-"))
                    .cloned(),
            );
        }
    }
    Some((
        name,
        "Composer package",
        deps.into_iter().collect(),
        Vec::new(),
    ))
}

/// Read a Dart or Flutter `pubspec.yaml`: its name and the packages under
/// `dependencies` and `dev_dependencies`, with `path:` ones as local.
pub(crate) fn read_pubspec(text: &str, dir_name: &str) -> Manifest {
    let mut name = dir_name.to_string();
    let mut deps = BTreeSet::new();
    let mut local = BTreeSet::new();
    let mut section = "";
    let mut current = String::new();
    // How far a section's entries are indented, from its first one.
    let mut entry = 0;
    for line in text.lines() {
        let content = line.split('#').next().unwrap_or("").trim_end();
        if content.trim().is_empty() {
            continue;
        }
        let indent = content.len() - content.trim_start().len();
        let content = content.trim();
        if indent == 0 {
            section = content.split(':').next().unwrap_or("");
            entry = 0;
            if section == "name" {
                name = content["name:".len()..]
                    .trim()
                    .trim_matches(['"', '\''])
                    .to_string();
            }
            continue;
        }
        if !matches!(section, "dependencies" | "dev_dependencies") {
            continue;
        }
        if entry == 0 {
            entry = indent;
        }
        if indent <= entry {
            if let Some((dep, _)) = content.split_once(':') {
                current = dep.trim().to_string();
                deps.insert(current.clone());
            }
        } else if content.starts_with("path:") && deps.remove(&current) {
            local.insert(current.clone());
        }
    }
    (
        name,
        "Dart package",
        deps.into_iter().collect(),
        local.into_iter().collect(),
    )
}

/// Whether `text` refers to Rust module `name` as a path: `name::`, `crate::name::` or
/// `super::name::`, but not `std::name::`.
fn mentions_rust_module(text: &str, name: &str) -> bool {
    let needle = format!("{name}::");
    let bytes = text.as_bytes();
    let mut from = 0;
    while let Some(at) = text[from..].find(&needle) {
        let at = from + at;
        from = at + needle.len();
        let before = &text[..at];
        if before.ends_with("crate::") || before.ends_with("super::") {
            return true;
        }
        let prev = at.checked_sub(1).map(|i| bytes[i]);
        match prev {
            Some(c) if c.is_ascii_alphanumeric() || c == b'_' || c == b':' => continue,
            _ => {
                // Skip comments.
                let line = before.rsplit('\n').next().unwrap_or("");
                if !line.trim_start().starts_with("//") {
                    return true;
                }
            }
        }
    }
    false
}

/// The paths imported by a JavaScript or TypeScript file: `from '…'`, `import '…'`,
/// `require('…')` and `import('…')`.
pub(crate) fn js_imports(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let t = line.trim_start();
        if t.starts_with("//") || t.starts_with('*') {
            continue;
        }
        for key in ["from ", "import ", "require(", "import("] {
            let mut rest = line;
            while let Some(at) = rest.find(key) {
                rest = &rest[at + key.len()..];
                let r = rest.trim_start();
                let Some(quote) = r.chars().next().filter(|c| matches!(c, '\'' | '"' | '`')) else {
                    continue;
                };
                if let Some(end) = r[1..].find(quote) {
                    let spec = &r[1..1 + end];
                    if !spec.is_empty() && !out.iter().any(|s| s == spec) {
                        out.push(spec.to_string());
                    }
                }
            }
        }
    }
    out
}

/// The top-level Python modules a file imports: `import a.b` and `from a.b import c`.
pub(crate) fn py_imports(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        let names: Vec<&str> = if let Some(rest) = t.strip_prefix("import ") {
            rest.split(',').collect()
        } else if let Some(rest) = t.strip_prefix("from ") {
            rest.split_whitespace().next().into_iter().collect()
        } else {
            continue;
        };
        for name in names {
            let top = name
                .trim()
                .split([' ', '.'])
                .next()
                .unwrap_or("")
                .to_string();
            if !top.is_empty() && !out.contains(&top) {
                out.push(top);
            }
        }
    }
    out
}

/// Normalise `a/b/../c/./d` to `a/c/d`.
fn normalize(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    parts.join("/")
}

/// The module a JavaScript file belongs to, from its path inside the package: the first
/// folder under `src/` (or the package root), or the file's own name at that level.
pub(crate) fn js_unit(inside: &str) -> String {
    let inside = inside.strip_prefix("src/").unwrap_or(inside);
    match inside.split_once('/') {
        Some((dir, _)) => dir.to_string(),
        None => inside
            .rsplit_once('.')
            .map(|(stem, _)| stem)
            .unwrap_or(inside)
            .to_string(),
    }
}

/// A module named by the first folder under the package's source folder, or by the
/// file itself when it sits at the top of it.
fn folder_unit(inside: &str, roots: &[&str]) -> String {
    let inside = roots
        .iter()
        .find_map(|r| inside.strip_prefix(r))
        .unwrap_or(inside);
    match inside.split_once('/') {
        Some((dir, _)) => dir.to_string(),
        None => Path::new(inside)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "(root)".into()),
    }
}

/// The module a Rust file belongs to, from its path under `src/`.
fn rust_unit(inside_src: &str) -> String {
    match inside_src.split_once('/') {
        Some((dir, _)) => dir.to_string(),
        None => inside_src.trim_end_matches(".rs").to_string(),
    }
}

/// Two manifests of one folder as one package. Of the same kind (a Gemfile and a
/// gemspec, a Gradle build and settings script) their dependencies add up and a real
/// name beats the folder's; otherwise the later one stands, as a project file does over
/// its solution.
fn merge_manifests(old: Manifest, new: Manifest, dir_name: &str) -> Manifest {
    if old.1 != new.1 {
        return if old.1 == ".NET solution" || new.1 != ".NET solution" {
            new
        } else {
            old
        };
    }
    let name = if new.0 != dir_name { new.0 } else { old.0 };
    let union = |a: Vec<String>, b: Vec<String>| {
        a.into_iter()
            .chain(b)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
    };
    (name, new.1, union(old.2, new.2), union(old.3, new.3))
}

/// Build the map of the project at `root`.
pub(crate) fn scan(root: &Path) -> ProjectMap {
    let (files, partial) = walk(root);
    let mut map = ProjectMap {
        files: files.len(),
        partial,
        ..Default::default()
    };
    let mut languages: BTreeMap<&str, usize> = BTreeMap::new();
    let mut folders: BTreeMap<String, usize> = BTreeMap::new();
    for file in &files {
        if let Some(lang) = language(file) {
            *languages.entry(lang).or_default() += 1;
        }
        let top = match file.components().count() {
            1 => "(root)".to_string(),
            _ => file
                .components()
                .next()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .unwrap_or_default(),
        };
        *folders.entry(top).or_default() += 1;
    }
    map.languages = languages
        .into_iter()
        .map(|(l, n)| (l.to_string(), n))
        .collect();
    map.languages
        .sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    map.folders = folders.into_iter().collect();
    map.folders
        .sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    // Packages, from their manifests. A folder with a pyproject ignores its requirements.
    let mut found: BTreeMap<String, (Manifest, String)> = BTreeMap::new();
    for file in &files {
        let Some(name) = file.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let dir = rel_dir(file);
        let dir_name = Path::new(&dir)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .or_else(|| root.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_default();
        let read = || std::fs::read_to_string(root.join(file)).unwrap_or_default();
        let stem = file
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("");
        let manifest = match (name, ext) {
            ("Cargo.toml", _) => read_cargo(&read()),
            ("package.json", _) => read_package_json(&read(), &dir_name),
            ("pyproject.toml", _) => read_pyproject(&read(), &dir_name),
            ("requirements.txt", _) if !found.contains_key(&dir) => {
                Some(read_requirements(&read(), &dir_name))
            }
            ("go.mod", _) => read_go_mod(&read()),
            ("Package.swift", _) => read_swift_package(&read(), &dir_name),
            ("pom.xml", _) => read_pom(&read(), &dir_name),
            (
                "build.gradle" | "build.gradle.kts" | "settings.gradle" | "settings.gradle.kts",
                _,
            ) => Some(read_gradle(&read(), &dir_name)),
            ("Gemfile", _) | (_, "gemspec") => Some(read_ruby(&read(), &dir_name)),
            (_, "csproj" | "fsproj" | "vbproj") => Some(read_dotnet_project(&read(), &stem)),
            // A solution beside a project file is that project's folder.
            (_, "sln") if !found.contains_key(&dir) => Some(read_sln(&read(), &stem)),
            ("composer.json", _) => read_composer(&read(), &dir_name),
            ("pubspec.yaml", _) => Some(read_pubspec(&read(), &dir_name)),
            _ => None,
        };
        if let Some(m) = manifest {
            let m = match found.remove(&dir) {
                Some((old, _)) => merge_manifests(old, m, &dir_name),
                None => m,
            };
            found.insert(dir.clone(), (m, name.to_string()));
        }
    }
    let names: BTreeSet<String> = found.values().map(|((n, ..), _)| n.clone()).collect();
    let dirs: Vec<String> = found.keys().cloned().collect();
    // The package a file belongs to: the deepest one whose folder holds it.
    let owner = |file: &Path| -> Option<String> {
        let path = file.to_string_lossy().replace('\\', "/");
        dirs.iter()
            .filter(|d| d.is_empty() || path.starts_with(&format!("{d}/")))
            .max_by_key(|d| d.len())
            .cloned()
    };
    let mut owned: BTreeMap<String, Vec<&PathBuf>> = BTreeMap::new();
    for file in &files {
        if let Some(dir) = owner(file) {
            owned.entry(dir).or_default().push(file);
        }
    }
    for (dir, ((name, kind, deps, local), _)) in found {
        // A dependency on another package of this repository is internal, however it
        // was declared.
        let mut internal: BTreeSet<String> = local.into_iter().collect();
        let mut external = Vec::new();
        for dep in deps {
            if names.contains(&dep) && dep != name {
                internal.insert(dep);
            } else {
                external.push(dep);
            }
        }
        let mine = owned.remove(&dir).unwrap_or_default();
        let modules = modules_of(root, &dir, kind, &mine);
        map.packages.push(Package {
            name,
            path: dir,
            kind,
            deps: external,
            internal: internal.into_iter().collect(),
            files: mine.len(),
            modules,
        });
    }
    map
}

/// The modules of one package and which of them refer to which.
fn modules_of(root: &Path, dir: &str, kind: &str, files: &[&PathBuf]) -> Vec<Module> {
    let prefix = if dir.is_empty() {
        String::new()
    } else {
        format!("{dir}/")
    };
    // (unit, path inside the package, text) for each source file of its language.
    let mut sources: Vec<(String, String, String)> = Vec::new();
    for file in files {
        let path = file.to_string_lossy().replace('\\', "/");
        let inside = path.strip_prefix(&prefix).unwrap_or(&path).to_string();
        let unit = match kind {
            "Rust crate" if file.extension().is_some_and(|e| e == "rs") => {
                let Some(src) = inside.strip_prefix("src/") else {
                    continue;
                };
                rust_unit(src)
            }
            "npm package" if is_js(file) => js_unit(&inside),
            "Python project" if file.extension().is_some_and(|e| e == "py") => {
                let inside = inside.strip_prefix("src/").unwrap_or(&inside);
                match inside.split_once('/') {
                    Some((d, _)) => d.to_string(),
                    None => inside.trim_end_matches(".py").to_string(),
                }
            }
            "Swift package"
                if file.extension().is_some_and(|e| e == "swift") && inside != "Package.swift" =>
            {
                folder_unit(&inside, &["Sources/"])
            }
            "Maven project" | "Gradle project"
                if file.extension().is_some_and(|e| {
                    matches!(e.to_str(), Some("java" | "kt" | "scala" | "groovy"))
                }) =>
            {
                // By Java package, under the source set's folder.
                let inside = inside
                    .split_once("/java/")
                    .or_else(|| inside.split_once("/kotlin/"))
                    .or_else(|| inside.split_once("/scala/"))
                    .map(|(_, rest)| rest)
                    .unwrap_or(&inside);
                let d = rel_dir(Path::new(inside));
                if d.is_empty() {
                    "(root)".to_string()
                } else {
                    d
                }
            }
            "Ruby project" if file.extension().is_some_and(|e| e == "rb") => {
                folder_unit(&inside, &["lib/", "app/"])
            }
            ".NET project" | ".NET solution"
                if file
                    .extension()
                    .is_some_and(|e| matches!(e.to_str(), Some("cs" | "fs" | "vb"))) =>
            {
                folder_unit(&inside, &[])
            }
            "Composer package" if file.extension().is_some_and(|e| e == "php") => {
                folder_unit(&inside, &["src/", "app/"])
            }
            "Dart package" if file.extension().is_some_and(|e| e == "dart") => {
                folder_unit(&inside, &["lib/src/", "lib/"])
            }
            "Go module" if file.extension().is_some_and(|e| e == "go") => {
                let d = rel_dir(Path::new(&inside));
                if d.is_empty() {
                    "(root)".to_string()
                } else {
                    d
                }
            }
            _ => continue,
        };
        let text = std::fs::read_to_string(root.join(file)).unwrap_or_default();
        sources.push((unit, inside, text));
    }
    let mut modules: BTreeMap<String, Module> = BTreeMap::new();
    for (unit, _, text) in &sources {
        let m = modules.entry(unit.clone()).or_insert_with(|| Module {
            name: unit.clone(),
            ..Default::default()
        });
        m.files += 1;
        m.lines += text.lines().count();
    }
    let units: Vec<String> = modules.keys().cloned().collect();
    let mut uses: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (unit, inside, text) in &sources {
        let found = uses.entry(unit.clone()).or_default();
        match kind {
            "Rust crate" => {
                for other in &units {
                    if other != unit
                        && !matches!(other.as_str(), "main" | "lib")
                        && mentions_rust_module(text, other)
                    {
                        found.insert(other.clone());
                    }
                }
            }
            "npm package" => {
                let from = rel_dir(Path::new(inside));
                for spec in js_imports(text) {
                    let target = if spec.starts_with('.') {
                        normalize(&format!("{from}/{spec}"))
                    } else if let Some(rest) = spec.strip_prefix("@/").or(spec.strip_prefix("~/")) {
                        format!("src/{rest}")
                    } else {
                        continue;
                    };
                    let other = js_unit(&target);
                    if &other != unit && units.contains(&other) {
                        found.insert(other);
                    }
                }
            }
            "Python project" => {
                for name in py_imports(text) {
                    if &name != unit && units.contains(&name) {
                        found.insert(name);
                    }
                }
            }
            "Go module" => {
                for other in &units {
                    let tail = format!("/{other}\"");
                    if other != unit && text.contains(&tail) {
                        found.insert(other.clone());
                    }
                }
            }
            _ => {}
        }
    }
    for (unit, set) in uses {
        if let Some(m) = modules.get_mut(&unit) {
            m.uses = set.into_iter().collect();
        }
    }
    let mut list: Vec<Module> = modules.into_values().collect();
    list.sort_by(|a, b| b.lines.cmp(&a.lines).then(a.name.cmp(&b.name)));
    list.truncate(MAX_MODULES);
    list
}

/// A Mermaid graph of the map: packages when there are several, else the modules of the
/// one package that refer to each other.
pub(crate) fn mermaid(map: &ProjectMap) -> String {
    let id = |s: &str| {
        s.chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect::<String>()
    };
    let mut out = String::from("```mermaid\ngraph LR\n");
    let mut edges = 0;
    if map.packages.len() > 1 {
        for p in &map.packages {
            out.push_str(&format!("  {}[\"{}\"]\n", id(&p.name), p.name));
        }
        for p in &map.packages {
            for dep in &p.internal {
                out.push_str(&format!("  {} --> {}\n", id(&p.name), id(dep)));
            }
        }
    } else if let Some(p) = map.packages.first() {
        for m in &p.modules {
            for dep in &m.uses {
                if edges >= 80 {
                    break;
                }
                out.push_str(&format!("  {} --> {}\n", id(&m.name), id(dep)));
                edges += 1;
            }
        }
        if edges == 0 {
            for m in p.modules.iter().take(20) {
                out.push_str(&format!("  {}[\"{}\"]\n", id(&m.name), m.name));
            }
        }
    }
    out.push_str("```\n");
    out
}

/// The map as Markdown, for an agent to start from.
pub(crate) fn summary(map: &ProjectMap) -> String {
    let mut out = String::new();
    let langs: Vec<String> = map
        .languages
        .iter()
        .take(6)
        .map(|(l, n)| format!("{l} ({n})"))
        .collect();
    out.push_str(&format!(
        "{} files{}. Languages: {}.\n",
        map.files,
        if map.partial { ", partly read" } else { "" },
        if langs.is_empty() {
            "none recognised".to_string()
        } else {
            langs.join(", ")
        }
    ));
    for p in &map.packages {
        let at = if p.path.is_empty() { "." } else { &p.path };
        out.push_str(&format!("\n### {} ({}, {at})\n", p.name, p.kind));
        if !p.internal.is_empty() {
            out.push_str(&format!("Depends on: {}\n", p.internal.join(", ")));
        }
        if !p.deps.is_empty() {
            let shown: Vec<&str> = p.deps.iter().take(30).map(String::as_str).collect();
            let more = p.deps.len().saturating_sub(shown.len());
            out.push_str(&format!(
                "External: {}{}\n",
                shown.join(", "),
                if more > 0 {
                    format!(" and {more} more")
                } else {
                    String::new()
                }
            ));
        }
        for m in &p.modules {
            out.push_str(&format!("- {} ({} lines)", m.name, m.lines));
            if !m.uses.is_empty() {
                out.push_str(&format!(" uses {}", m.uses.join(", ")));
            }
            out.push('\n');
        }
    }
    out
}

/// Where the project keeps its architecture notes: an existing `ARCHITECTURE.md` or
/// `docs/architecture.md`, else `docs/architecture.md` once something is written.
pub(crate) fn notes_path(root: &Path) -> PathBuf {
    for name in [
        "ARCHITECTURE.md",
        "architecture.md",
        "docs/ARCHITECTURE.md",
        "docs/architecture.md",
    ] {
        let path = root.join(name);
        if path.is_file() {
            return path;
        }
    }
    root.join("docs").join("architecture.md")
}

/// Write the notes file, making its folder. Empty notes don't create a file, but empty
/// out one that exists.
pub(crate) fn save_notes(file: &Path, text: &str) -> std::io::Result<()> {
    if text.trim().is_empty() && !file.exists() {
        return Ok(());
    }
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(file, text)
}

/// What a session asking about the architecture starts with.
pub(crate) fn context(project: &str, map: Option<&ProjectMap>, notes: &str, file: &str) -> String {
    let mut out = format!("## Architecture of {project}\n");
    if let Some(map) = map {
        out.push_str("Runo mapped the project from its manifests and imports:\n\n");
        out.push_str(&summary(map));
    }
    if notes.trim().is_empty() {
        out.push_str(&format!(
            "\nThere are no architecture notes yet. Keep any you write in {file}.\n"
        ));
    } else {
        out.push_str(&format!(
            "\n## Architecture notes and decisions ({file})\nFollow these, and update the file when a decision changes.\n\n{}\n",
            notes.trim()
        ));
    }
    out
}

/// Questions offered as one-click starts.
const QUICK: [(&str, &str); 4] = [
    (
        "Explain the architecture",
        "Explain this project's architecture: the main parts, how data flows between them and the external services it relies on.",
    ),
    (
        "Write the notes",
        "Read the code and write the architecture notes file: the main parts, how they talk to each other, a Mermaid diagram and the decisions worth recording. Keep it short.",
    ),
    (
        "Check the code against the notes",
        "Check the code against the architecture notes. List every place it breaks a rule or a decision, and anything the notes leave out.",
    ),
    (
        "Find risky coupling",
        "Find the riskiest coupling in this codebase: modules that know too much about each other, cycles and god objects. Suggest small, safe steps to untangle them.",
    ),
];

/// The page's own state: the notes editor, the question box and the last map.
pub(crate) struct ArchPage {
    pub notes: Entity<TextareaState>,
    pub question: Entity<InputState>,
    /// The project whose notes the editor holds.
    pub project: Option<u64>,
    /// The page was on screen last frame; coming back to it reads the notes again.
    pub shown: bool,
    /// The notes file, and whether it held anything when it was read.
    pub file: Option<PathBuf>,
    pub loading: bool,
    pub saved: Option<std::time::Instant>,
    pub error: Option<String>,
    /// The last map of each project, and the projects being mapped now.
    pub maps: HashMap<u64, ProjectMap>,
    pub scanning: std::collections::HashSet<u64>,
    /// Packages shown the other way from how they start (only the first starts open).
    pub open: std::collections::HashSet<String>,
    /// The diagram editor of each project, and whether the page shows it or the code map.
    pub editors: HashMap<u64, Entity<ArchEditor>>,
    pub code_map: bool,
    pub editor_subs: Vec<Subscription>,
}

impl ArchPage {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<RunoApp>) -> (Self, Vec<Subscription>) {
        let notes = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("How the system fits together, and the decisions behind it.")
        });
        let question = cx.new(|cx| {
            InputState::new(window, cx).placeholder(
                "Ask about the architecture, e.g. How does a request reach the database?",
            )
        });
        let subs = vec![
            cx.subscribe_in(&notes, window, |this, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.arch_notes_changed(cx);
                    cx.notify();
                }
            }),
            cx.subscribe_in(&question, window, |this, _, event, window, cx| {
                match event {
                    InputEvent::PressEnter { .. } => {
                        let q = this.arch.question.read(cx).value().trim().to_string();
                        if !q.is_empty() {
                            this.ask_architecture(q, window, cx);
                        }
                    }
                    InputEvent::Change => {}
                    _ => return,
                }
                cx.notify();
            }),
        ];
        (
            Self {
                notes,
                question,
                project: None,
                shown: false,
                file: None,
                loading: false,
                saved: None,
                error: None,
                maps: HashMap::new(),
                scanning: Default::default(),
                open: Default::default(),
                editors: HashMap::new(),
                code_map: false,
                editor_subs: Vec::new(),
            },
            subs,
        )
    }
}

impl RunoApp {
    /// Load the current project's notes and map, when the page shows another project.
    pub(crate) fn prepare_architecture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.all_scope() {
            // The chooser shows each project's size, so map the ones not read yet.
            let ids: Vec<u64> = self.state.projects.iter().map(|p| p.id).collect();
            for id in ids {
                if !self.arch.maps.contains_key(&id) {
                    self.map_project(id);
                }
            }
            return;
        }
        let Some(project) = self
            .current_project()
            .and_then(|id| self.state.projects.iter().find(|p| p.id == id))
            .cloned()
        else {
            return;
        };
        let back = !std::mem::replace(&mut self.arch.shown, true);
        self.ensure_arch_editor(&project, back, window, cx);
        if self.arch.project == Some(project.id) && !back {
            return;
        }
        self.arch.project = Some(project.id);
        let file = notes_path(Path::new(&project.path));
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        self.arch.file = Some(file);
        self.arch.saved = None;
        self.arch.error = None;
        self.arch.loading = true;
        self.arch
            .notes
            .update(cx, |input, cx| input.set_value(text, window, cx));
        self.arch.loading = false;
        // The last map shows while the project is read again.
        self.map_project(project.id);
    }

    /// Make the project's diagram editor, or read its files again when the page comes back.
    fn ensure_arch_editor(
        &mut self,
        project: &Project,
        back: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(editor) = self.arch.editors.get(&project.id) {
            if back {
                editor.update(cx, |e, cx| e.reload_if_clean(cx));
            }
            return;
        }
        let root = PathBuf::from(&project.path);
        let name = project.name.clone();
        let editor = cx.new(|cx| ArchEditor::new(root, name, window, cx));
        let pid = project.id;
        let sub = cx.subscribe_in(&editor, window, move |this, _, event, window, cx| {
            match event {
                ArchEvent::Ask {
                    task,
                    context,
                    edit,
                    images,
                    agent,
                } => this.ask_from_canvas(
                    pid,
                    View::Architecture,
                    task,
                    shell::StartContext {
                        label: "Architecture diagram".into(),
                        text: context.clone(),
                        images: images.clone(),
                        in_place: *edit,
                    },
                    *agent,
                    window,
                    cx,
                ),
                ArchEvent::Open(path) => {
                    this.view = View::Session;
                    this.show_panel = true;
                    this.open_preview(&path.to_string_lossy());
                    this.right_tab = RightTab::Files;
                }
            }
            cx.notify();
        });
        self.arch.editor_subs.push(sub);
        if let Some(agent) = self.canvas_agent(project.id, View::Architecture) {
            editor.update(cx, |e, _| e.use_agent(agent));
        }
        self.arch.editors.insert(project.id, editor);
    }

    /// Read the project's structure off the UI thread.
    pub(crate) fn map_project(&mut self, pid: u64) {
        let Some(path) = self
            .state
            .projects
            .iter()
            .find(|p| p.id == pid)
            .map(|p| p.path.clone())
        else {
            return;
        };
        if self.demo || !self.arch.scanning.insert(pid) {
            return;
        }
        self.bg(
            move || scan(Path::new(&path)),
            move |this, map, cx| {
                this.arch.scanning.remove(&pid);
                this.arch.maps.insert(pid, map);
                cx.notify();
            },
        );
    }

    /// Write the notes back to their file as they change.
    fn arch_notes_changed(&mut self, cx: &mut Context<Self>) {
        if self.arch.loading || self.demo {
            return;
        }
        let Some(file) = self.arch.file.clone() else {
            return;
        };
        let text = self.arch.notes.read(cx).value().to_string();
        match save_notes(&file, &text) {
            Ok(()) => {
                self.arch.saved = Some(std::time::Instant::now());
                self.arch.error = None;
            }
            Err(e) => {
                let why = format!("Couldn't save {}: {e}", file.display());
                crate::applog::error("save", &why);
                self.arch.error = Some(why)
            }
        }
    }

    /// Start a session about the architecture, with the map and notes attached.
    pub(crate) fn ask_architecture(
        &mut self,
        question: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self
            .arch
            .project
            .and_then(|id| self.state.projects.iter().find(|p| p.id == id))
            .cloned()
        else {
            return;
        };
        let notes = self.arch.notes.read(cx).value().to_string();
        let file = self
            .arch
            .file
            .as_ref()
            .and_then(|f| f.strip_prefix(&project.path).ok())
            .map(|f| f.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|| "docs/architecture.md".into());
        let text = context(
            &project.name,
            self.arch.maps.get(&project.id),
            &notes,
            &file,
        );
        self.arch
            .question
            .update(cx, |q, cx| q.set_value("", window, cx));
        self.start_with_context(
            project.id,
            &question,
            shell::StartContext {
                label: "Architecture map and notes".into(),
                text,
                images: Vec::new(),
                in_place: false,
            },
            window,
            cx,
        );
    }
}

/// A small rounded label.
fn chip(text: String, color: u32) -> Div {
    div()
        .h(px(22.))
        .px_2()
        .flex()
        .items_center()
        .rounded_full()
        .border_1()
        .border_color(rgb(c_border()))
        .text_size(px(11.5))
        .text_color(rgb(color))
        .child(text)
}

impl RunoApp {
    /// Add the generated diagram to the end of the notes.
    fn insert_diagram(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(map) = self.arch.project.and_then(|pid| self.arch.maps.get(&pid)) else {
            return;
        };
        let block = format!("## Structure\n\n{}", mermaid(map));
        self.arch.notes.update(cx, |input, cx| {
            let text = input.value().to_string();
            let lead = if text.trim().is_empty() || text.ends_with("\n\n") {
                ""
            } else if text.ends_with('\n') {
                "\n"
            } else {
                "\n\n"
            };
            input.set_value(format!("{text}{lead}{block}"), window, cx);
        });
        self.arch_notes_changed(cx);
    }

    fn package_card(&self, i: usize, p: &Package, cx: &mut Context<Self>) -> AnyElement {
        let key = format!("{}/{}", p.path, p.name);
        // The first package starts open; a click flips any package from how it started.
        let open = (i == 0) != self.arch.open.contains(&key);
        let lines: usize = p.modules.iter().map(|m| m.lines).sum();
        let mut card = div()
            .v_flex()
            .rounded_lg()
            .border_1()
            .border_color(rgb(c_border()))
            .bg(rgb(c_canvas()))
            .overflow_hidden()
            .child(
                div()
                    .id(("arch-pkg", i))
                    .px_3()
                    .py_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .cursor_pointer()
                    .hover(|el| el.bg(tint(c_subtle())))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.arch.open.remove(&key) {
                            this.arch.open.insert(key.clone());
                        }
                        cx.notify();
                    }))
                    .child(icon(
                        if open {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        },
                        12.,
                        c_muted(),
                    ))
                    .child(icon(IconName::Package, 14., c_blue()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .v_flex()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(p.name.clone()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(11.5))
                                    .text_color(rgb(c_muted()))
                                    .child(format!(
                                        "{} · {} · {} files{}",
                                        p.kind,
                                        if p.path.is_empty() { "." } else { &p.path },
                                        p.files,
                                        if lines > 0 {
                                            format!(" · {lines} lines")
                                        } else {
                                            String::new()
                                        }
                                    )),
                            ),
                    ),
            );
        if !open {
            return card.into_any_element();
        }
        let mut body = div().px_3().pb_3().v_flex().gap_2();
        if !p.internal.is_empty() {
            let mut row = div().flex().flex_wrap().gap_1().items_center().child(
                div()
                    .text_size(px(11.5))
                    .text_color(rgb(c_muted()))
                    .mr_1()
                    .child("Depends on"),
            );
            for dep in &p.internal {
                row = row.child(chip(dep.clone(), c_blue()));
            }
            body = body.child(row);
        }
        if !p.deps.is_empty() {
            let mut row = div().flex().flex_wrap().gap_1().items_center().child(
                div()
                    .text_size(px(11.5))
                    .text_color(rgb(c_muted()))
                    .mr_1()
                    .child(format!("{} external", p.deps.len())),
            );
            for dep in p.deps.iter().take(24) {
                row = row.child(chip(dep.clone(), c_secondary()));
            }
            if p.deps.len() > 24 {
                row = row.child(chip(format!("+{}", p.deps.len() - 24), c_muted()));
            }
            body = body.child(row);
        }
        if !p.modules.is_empty() {
            let mut list = div().v_flex().mt_1();
            for m in &p.modules {
                list = list.child(
                    div()
                        .py(px(3.))
                        .flex()
                        .items_start()
                        .gap_2()
                        .text_size(px(12.))
                        .border_t_1()
                        .border_color(rgb(c_border()))
                        .child(
                            div()
                                .w(px(150.))
                                .flex_none()
                                .truncate()
                                .font_family(self.mono_font.clone())
                                .child(m.name.clone()),
                        )
                        .child(
                            div()
                                .w(px(70.))
                                .flex_none()
                                .text_color(rgb(c_muted()))
                                .child(format!("{} lines", m.lines)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_color(rgb(c_secondary()))
                                .child(if m.uses.is_empty() {
                                    String::new()
                                } else {
                                    format!("→ {}", m.uses.join(", "))
                                }),
                        ),
                );
            }
            body = body.child(list);
        }
        card = card.child(body);
        card.into_any_element()
    }

    pub(crate) fn architecture_view(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.all_scope() {
            return shell::mode_frame(
                Mode::Architecture,
                "Architecture",
                Some(self.mode_project_chip(cx)),
                "Pick a project to see its structure and notes.",
                self.project_chooser(Mode::Architecture, cx),
                None,
            );
        }
        let Some(project) = self
            .arch
            .project
            .and_then(|id| self.state.projects.iter().find(|p| p.id == id))
            .cloned()
        else {
            return shell::mode_frame(
                Mode::Architecture,
                "Architecture",
                None,
                "See how the project fits together and keep its decisions.",
                div().child(empty_hint(
                    "arch-empty",
                    IconName::FolderPlus,
                    "No project yet",
                    "Add a project to map its architecture.",
                )),
                None,
            );
        };
        let pid = project.id;
        if !self.arch.code_map {
            if let Some(editor) = self.arch.editors.get(&pid) {
                return self.arch_diagram(pid, editor.clone(), cx);
            }
        }
        let scanning = self.arch.scanning.contains(&pid);
        let map = self.arch.maps.get(&pid);

        // Structure: languages, folders and packages with their modules.
        let mut structure = div()
            .v_flex()
            .gap_3()
            .p_4()
            .rounded_xl()
            .border_1()
            .border_color(rgb(c_border()))
            .bg(tint(c_raised()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .v_flex()
                            .child(
                                div()
                                    .text_size(px(14.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Structure"),
                            )
                            .child(div().text_size(px(12.)).text_color(rgb(c_muted())).child(
                                match map {
                                    _ if scanning => "Reading the project…".to_string(),
                                    Some(m) => format!(
                                        "{} packages, {} files{}. From the manifests and imports.",
                                        m.packages.len(),
                                        m.files,
                                        if m.partial { " (partly read)" } else { "" }
                                    ),
                                    None => "Not read yet.".to_string(),
                                },
                            )),
                    )
                    .child(
                        self.ghost_button_sized(
                            "arch-rescan",
                            IconName::RefreshCw,
                            "Read again",
                            28.,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.map_project(pid);
                            cx.notify();
                        })),
                    ),
            );
        if let Some(map) = map {
            if !map.languages.is_empty() {
                let mut row = div().flex().flex_wrap().gap_1();
                for (lang, n) in map.languages.iter().take(8) {
                    row = row.child(chip(format!("{lang} {n}"), c_secondary()));
                }
                structure = structure.child(row);
            }
            if !map.folders.is_empty() {
                let mut row = div().flex().flex_wrap().gap_1().items_center().child(
                    div()
                        .mr_1()
                        .text_size(px(11.5))
                        .text_color(rgb(c_muted()))
                        .child("Folders"),
                );
                for (folder, n) in map.folders.iter().take(12) {
                    row = row.child(chip(format!("{folder} {n}"), c_secondary()));
                }
                structure = structure.child(row);
            }
            if map.packages.is_empty() {
                structure = structure.child(
                    div()
                        .text_size(px(12.5))
                        .text_color(rgb(c_muted()))
                        .child("No package manifest found (Cargo.toml, package.json, pyproject.toml, go.mod, Package.swift, pom.xml, build.gradle, Gemfile, .csproj, composer.json or pubspec.yaml)."),
                );
            }
            for (i, p) in map.packages.iter().enumerate() {
                structure = structure.child(self.package_card(i, p, cx));
            }
        }

        let mut quick = div().flex().flex_wrap().gap_1();
        for (j, (label, question)) in QUICK.iter().enumerate() {
            quick = quick.child(
                div()
                    .id(("arch-quick", j))
                    .h(px(26.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_1()
                    .rounded_full()
                    .border_1()
                    .border_color(rgb(c_border()))
                    .cursor_pointer()
                    .text_size(px(12.))
                    .text_color(rgb(c_secondary()))
                    .hover(|el| el.bg(tint(c_subtle())).text_color(rgb(c_text())))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.ask_architecture(question.to_string(), window, cx);
                        cx.notify();
                    }))
                    .child(icon(IconName::Sparkles, 11., c_muted()))
                    .child(*label),
            );
        }
        let ask = div()
            .v_flex()
            .gap_2()
            .p_4()
            .rounded_xl()
            .border_1()
            .border_color(rgb(c_border()))
            .bg(tint(c_raised()))
            .child(
                div()
                    .text_size(px(14.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Ask an agent"),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(c_muted()))
                    .child("The session starts with the structure above and your notes."),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&self.arch.question).bordered(true).h(px(32.))),
                    )
                    .child(
                        self.button("arch-ask", "Ask", true)
                            .child(icon(IconName::ArrowRight, 13., c_canvas()))
                            .on_click(cx.listener(|this, _, window, cx| {
                                let q = this.arch.question.read(cx).value().trim().to_string();
                                if !q.is_empty() {
                                    this.ask_architecture(q, window, cx);
                                }
                                cx.notify();
                            })),
                    ),
            )
            .child(quick);

        let file = self
            .arch
            .file
            .as_ref()
            .and_then(|f| f.strip_prefix(&project.path).ok())
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default();
        let status = match (&self.arch.error, self.arch.saved) {
            (Some(e), _) => e.clone(),
            (None, Some(at)) if at.elapsed() < Duration::from_secs(3) => "Saved".into(),
            _ => "Saved as you type".into(),
        };
        let notes = div()
            .v_flex()
            .rounded_xl()
            .border_1()
            .border_color(rgb(c_border()))
            .bg(tint(c_raised()))
            .overflow_hidden()
            .child(
                div()
                    .px_4()
                    .py_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .border_b_1()
                    .border_color(rgb(c_border()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .v_flex()
                            .child(
                                div()
                                    .text_size(px(14.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Notes and decisions"),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(11.5))
                                    .font_family(self.mono_font.clone())
                                    .text_color(rgb(c_muted()))
                                    .child(file),
                            ),
                    )
                    .when(map.is_some(), |el| {
                        el.child(
                            self.button("arch-diagram", "Insert diagram", false)
                                .child(icon(IconName::Network, 13., c_secondary()))
                                .tooltip(|window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(
                                        "Add a Mermaid diagram of the structure to the notes.",
                                    )
                                    .build(window, cx)
                                })
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.insert_diagram(window, cx);
                                    cx.notify();
                                })),
                        )
                    }),
            )
            .child(
                div().px_4().py_3().text_size(px(13.5)).child(
                    Textarea::new(&self.arch.notes)
                        .appearance(false)
                        .bordered(false)
                        .h(px(360.)),
                ),
            )
            .child(
                div()
                    .px_4()
                    .py_2()
                    .border_t_1()
                    .border_color(rgb(c_border()))
                    .text_size(px(11.5))
                    .text_color(rgb(if self.arch.error.is_some() {
                        c_red()
                    } else {
                        c_muted()
                    }))
                    .child(status),
            );

        let body = div()
            .flex()
            .items_start()
            .gap_4()
            .child(div().flex_1().min_w(px(360.)).child(structure))
            .child(
                div()
                    .flex_1()
                    .min_w(px(360.))
                    .v_flex()
                    .gap_4()
                    .child(ask)
                    .child(notes),
            );
        shell::mode_frame(
            Mode::Architecture,
            "Architecture",
            Some(self.mode_project_chip(cx)),
            "The project's structure from its manifests and imports, notes and decisions kept in the repository, and agents that start with both.",
            body,
            Some(self.arch_tabs(cx)),
        )
    }

    /// Diagram or code map.
    fn arch_tabs(&self, cx: &mut Context<Self>) -> AnyElement {
        let code_map = self.arch.code_map;
        crate::github::segmented()
            .child(
                crate::github::segment(("arch-tab", 0), "Diagram", !code_map).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.arch.code_map = false;
                        cx.notify();
                    }),
                ),
            )
            .child(
                crate::github::segment(("arch-tab", 1), "Code map", code_map).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.arch.code_map = true;
                        cx.notify();
                    }),
                ),
            )
            .into_any_element()
    }

    /// The diagram editor, edge to edge under a slim header.
    fn arch_diagram(
        &self,
        pid: u64,
        editor: Entity<ArchEditor>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .v_flex()
            .child(
                div()
                    .flex_none()
                    .h(px(48.))
                    .px_4()
                    .flex()
                    .items_center()
                    .gap_3()
                    .border_b_1()
                    .border_color(rgb(c_border()))
                    .child(icon(Mode::Architecture.glyph(), 16., c_blue()))
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Architecture"),
                    )
                    .child(self.mode_project_chip(cx))
                    .child(div().flex_1())
                    .children(self.canvas_chats(pid, View::Architecture, cx))
                    .child(self.arch_tabs(cx)),
            )
            .child(div().flex_1().min_h_0().child(editor))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("runo-arch-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(root: &Path, path: &str, text: &str) {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn cargo_manifests_split_path_and_registry_deps() {
        let text = r#"
[package]
name = "app"

[dependencies]
serde = "1"
core = { path = "../core" }
renamed = { package = "real-name", version = "2" }

[target.'cfg(unix)'.dependencies]
libc = "0.2"

[dev-dependencies]
tempfile = "3"
"#;
        let (name, kind, deps, local) = read_cargo(text).unwrap();
        assert_eq!((name.as_str(), kind), ("app", "Rust crate"));
        assert_eq!(deps, vec!["libc", "real-name", "serde", "tempfile"]);
        assert_eq!(local, vec!["core"]);
        // A virtual workspace is not a package.
        assert!(read_cargo("[workspace]\nmembers = [\"a\"]\n").is_none());
    }

    #[test]
    fn other_manifests_are_read() {
        let pkg = r#"{"name":"web","dependencies":{"react":"^18","ui":"workspace:*"},"devDependencies":{"vite":"5"}}"#;
        let (name, _, deps, local) = read_package_json(pkg, "dir").unwrap();
        assert_eq!(name, "web");
        assert_eq!(deps, vec!["react", "vite"]);
        assert_eq!(local, vec!["ui"]);

        let py =
            "[project]\nname = \"svc\"\ndependencies = [\"requests[socks]>=2\", \"fastapi\"]\n";
        let (name, _, deps, _) = read_pyproject(py, "dir").unwrap();
        assert_eq!(name, "svc");
        assert_eq!(deps, vec!["fastapi", "requests"]);
        let poetry = "[tool.poetry]\nname = \"p\"\n[tool.poetry.dependencies]\npython = \"^3.11\"\nclick = \"*\"\n";
        assert_eq!(read_pyproject(poetry, "d").unwrap().2, vec!["click"]);

        let (_, _, deps, _) = read_requirements("# deps\nflask==3.0\n-r other.txt\n\nrich\n", "d");
        assert_eq!(deps, vec!["flask", "rich"]);

        let go = "module example.com/api\n\ngo 1.22\n\nrequire (\n\tgithub.com/a/b v1.0.0 // indirect\n\tgolang.org/x/net v0.1.0\n)\nrequire github.com/c/d v2.0.0\n";
        let (name, _, deps, _) = read_go_mod(go).unwrap();
        assert_eq!(name, "example.com/api");
        assert_eq!(
            deps,
            vec!["github.com/a/b", "github.com/c/d", "golang.org/x/net"]
        );
    }

    #[test]
    fn swift_maven_and_gradle_manifests_are_read() {
        let swift = r#"// swift-tools-version:5.9
import PackageDescription

let package = Package(
    name: "Kit",
    dependencies: [
        .package(url: "https://github.com/apple/swift-argument-parser.git", from: "1.3.0"),
        .package(path: "../Core"),
    ],
    targets: [.target(name: "Kit")]
)
"#;
        let (name, kind, deps, local) = read_swift_package(swift, "dir").unwrap();
        assert_eq!((name.as_str(), kind), ("Kit", "Swift package"));
        assert_eq!(deps, vec!["swift-argument-parser"]);
        assert_eq!(local, vec!["Core"]);

        let pom = r#"<project>
  <parent><groupId>com.acme</groupId><artifactId>acme-parent</artifactId></parent>
  <artifactId>billing</artifactId>
  <dependencies>
    <dependency><groupId>com.acme</groupId><artifactId>core</artifactId></dependency>
    <dependency><groupId>junit</groupId><artifactId>junit</artifactId><scope>test</scope></dependency>
  </dependencies>
</project>"#;
        let (name, kind, deps, _) = read_pom(pom, "dir").unwrap();
        assert_eq!((name.as_str(), kind), ("billing", "Maven project"));
        assert_eq!(deps, vec!["core", "junit"]);

        let gradle = r#"plugins { id("org.jetbrains.kotlin.jvm") }
dependencies {
    implementation("com.squareup.okhttp3:okhttp:4.12.0")
    implementation(project(":core"))
    testImplementation 'org.junit.jupiter:junit-jupiter:5.10.0'
    api(libs.kotlinx.coroutines)
    // implementation("old:thing:1")
}
"#;
        let (name, kind, deps, local) = read_gradle(gradle, "app");
        assert_eq!((name.as_str(), kind), ("app", "Gradle project"));
        assert_eq!(deps, vec!["junit-jupiter", "kotlinx-coroutines", "okhttp"]);
        assert_eq!(local, vec!["core"]);
        let settings = "rootProject.name = \"shop\"\ninclude(\":app\", \":core\")\n";
        assert_eq!(read_gradle(settings, "dir").0, "shop");
    }

    #[test]
    fn ruby_dotnet_php_and_dart_manifests_are_read() {
        let gemfile = "source 'https://rubygems.org'\ngem 'rails', '~> 7.1'\ngem \"pg\"\ngem 'engine', path: 'engines/engine'\n# gem 'old'\n";
        let (_, kind, deps, local) = read_ruby(gemfile, "shop");
        assert_eq!(kind, "Ruby project");
        assert_eq!(deps, vec!["pg", "rails"]);
        assert_eq!(local, vec!["engine"]);
        let gemspec = "Gem::Specification.new do |spec|\n  spec.name = \"tiny\"\n  spec.add_dependency \"rack\", \">= 2\"\n  spec.add_development_dependency 'rspec'\nend\n";
        let (name, _, deps, _) = read_ruby(gemspec, "dir");
        assert_eq!(name, "tiny");
        assert_eq!(deps, vec!["rack", "rspec"]);

        let csproj = r#"<Project Sdk="Microsoft.NET.Sdk">
  <ItemGroup>
    <PackageReference Include="Newtonsoft.Json" Version="13.0.3" />
    <ProjectReference Include="..\Core\Core.csproj" />
  </ItemGroup>
</Project>"#;
        let (name, kind, deps, local) = read_dotnet_project(csproj, "Api");
        assert_eq!((name.as_str(), kind), ("Api", ".NET project"));
        assert_eq!(deps, vec!["Newtonsoft.Json"]);
        assert_eq!(local, vec!["Core"]);
        let sln = "Microsoft Visual Studio Solution File\nProject(\"{FAE04EC0}\") = \"Api\", \"src\\Api\\Api.csproj\", \"{1}\"\nEndProject\nProject(\"{2150E333}\") = \"docs\", \"docs\", \"{2}\"\nEndProject\n";
        let (name, kind, _, local) = read_sln(sln, "Shop");
        assert_eq!((name.as_str(), kind), ("Shop", ".NET solution"));
        assert_eq!(local, vec!["Api"]);

        let composer = r#"{"name":"acme/site","require":{"php":">=8.2","ext-json":"*","laravel/framework":"^11"},"require-dev":{"phpunit/phpunit":"^11"}}"#;
        let (name, kind, deps, _) = read_composer(composer, "dir").unwrap();
        assert_eq!((name.as_str(), kind), ("acme/site", "Composer package"));
        assert_eq!(deps, vec!["laravel/framework", "phpunit/phpunit"]);

        let pubspec = "name: app\nenvironment:\n  sdk: '>=3.0.0 <4.0.0'\ndependencies:\n  flutter:\n    sdk: flutter\n  http: ^1.2.0\n  shared:\n    path: ../shared\ndev_dependencies:\n    lints: ^3.0.0\nflutter:\n  uses-material-design: true\n";
        let (name, kind, deps, local) = read_pubspec(pubspec, "dir");
        assert_eq!((name.as_str(), kind), ("app", "Dart package"));
        assert_eq!(deps, vec!["flutter", "http", "lints"]);
        assert_eq!(local, vec!["shared"]);
    }

    #[test]
    fn scan_reads_other_ecosystems() {
        let root = temp("scan-more");
        write(
            &root,
            "settings.gradle.kts",
            "rootProject.name = \"shop\"\n",
        );
        write(
            &root,
            "build.gradle.kts",
            "dependencies {\n    implementation(project(\":core\"))\n}\n",
        );
        write(
            &root,
            "src/main/kotlin/com/shop/App.kt",
            "package com.shop\n",
        );
        write(
            &root,
            "core/build.gradle.kts",
            "dependencies {\n    api(\"com.google.guava:guava:33.0.0-jre\")\n}\n",
        );
        write(
            &root,
            "core/src/main/kotlin/com/shop/core/Money.kt",
            "package com.shop.core\n",
        );
        write(
            &root,
            "ios/Package.swift",
            "let package = Package(name: \"ShopKit\")\n",
        );
        write(&root, "ios/Sources/ShopKit/Cart.swift", "struct Cart {}\n");
        write(
            &root,
            "api/Api.csproj",
            "<Project><ItemGroup><PackageReference Include=\"Dapper\" /></ItemGroup></Project>",
        );
        write(&root, "api/Shop.sln", "");
        write(&root, "api/Controllers/Home.cs", "class Home {}\n");
        let map = scan(&root);
        let find = |name: &str| map.packages.iter().find(|p| p.name == name).unwrap();
        let shop = find("shop");
        assert_eq!((shop.kind, shop.path.as_str()), ("Gradle project", ""));
        assert_eq!(shop.internal, vec!["core"]);
        assert_eq!(shop.modules[0].name, "com/shop");
        assert_eq!(find("core").deps, vec!["guava"]);
        let kit = find("ShopKit");
        assert_eq!(kit.modules[0].name, "ShopKit");
        let api = find("Api");
        assert_eq!(api.kind, ".NET project");
        assert_eq!(api.deps, vec!["Dapper"]);
        assert_eq!(api.modules[0].name, "Controllers");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn rust_module_mentions_skip_other_paths() {
        assert!(mentions_rust_module("let x = git::status();", "git"));
        assert!(mentions_rust_module("use crate::git::Change;", "git"));
        assert!(mentions_rust_module("super::git::x()", "git"));
        assert!(!mentions_rust_module("std::fs::read(p)", "fs"));
        assert!(!mentions_rust_module("legit::x()", "git"));
        assert!(!mentions_rust_module("    // git::x() is old", "git"));
    }

    #[test]
    fn imports_are_found() {
        let js = "import a from './a';\nimport { b } from \"../lib/b\";\nconst c = require('c');\nimport './side.css';\n// import x from './x'\nconst d = await import(`./d`);\n";
        assert_eq!(
            js_imports(js),
            vec!["./a", "../lib/b", "c", "./side.css", "./d"]
        );
        let py = "import os, app.models\nfrom app.views import home\nfrom . import x\n";
        assert_eq!(py_imports(py), vec!["os", "app"]);
        assert_eq!(js_unit("src/components/Button.tsx"), "components");
        assert_eq!(js_unit("src/main.ts"), "main");
        assert_eq!(js_unit("lib/x.js"), "lib");
        assert_eq!(normalize("src/a/../b/./c"), "src/b/c");
    }

    #[test]
    fn scan_maps_a_small_workspace() {
        let root = temp("scan");
        write(
            &root,
            "Cargo.toml",
            "[workspace]\nmembers = [\"app\", \"core\"]\n",
        );
        write(&root, "app/Cargo.toml", "[package]\nname = \"app\"\n[dependencies]\ncore = { path = \"../core\" }\nserde = \"1\"\n");
        write(
            &root,
            "app/src/main.rs",
            "mod ui;\nmod db;\nfn main() { ui::run(); db::open(); }\n",
        );
        write(
            &root,
            "app/src/ui.rs",
            "pub fn run() { crate::db::open(); }\n",
        );
        write(
            &root,
            "app/src/db/mod.rs",
            "pub fn open() { std::fs::read(\"x\"); }\n",
        );
        write(&root, "core/Cargo.toml", "[package]\nname = \"core\"\n");
        write(&root, "core/src/lib.rs", "pub fn f() {}\n");
        write(
            &root,
            "web/package.json",
            "{\"name\":\"web\",\"dependencies\":{\"react\":\"18\"}}",
        );
        write(
            &root,
            "web/src/main.tsx",
            "import App from './app/App';\nimport { api } from '@/api/client';\n",
        );
        write(
            &root,
            "web/src/app/App.tsx",
            "import { api } from '../api/client';\n",
        );
        write(&root, "web/src/api/client.ts", "export const api = 1;\n");
        write(
            &root,
            "web/node_modules/react/package.json",
            "{\"name\":\"react\"}",
        );
        write(&root, ".git/config", "");

        let map = scan(&root);
        assert!(!map.partial);
        let names: Vec<&str> = map.packages.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["app", "core", "web"]);
        let app = &map.packages[0];
        assert_eq!(app.internal, vec!["core"]);
        assert_eq!(app.deps, vec!["serde"]);
        let uses = |p: &Package, m: &str| {
            p.modules
                .iter()
                .find(|x| x.name == m)
                .map(|x| x.uses.clone())
                .unwrap_or_default()
        };
        assert_eq!(uses(app, "main"), vec!["db", "ui"]);
        assert_eq!(uses(app, "ui"), vec!["db"]);
        assert!(uses(app, "db").is_empty());
        let web = &map.packages[2];
        assert_eq!(web.files, 4);
        assert_eq!(uses(web, "main"), vec!["api", "app"]);
        assert_eq!(uses(web, "app"), vec!["api"]);
        assert!(map.languages.iter().any(|(l, _)| l == "Rust"));

        let graph = mermaid(&map);
        assert!(graph.contains("app --> core"));
        let text = summary(&map);
        assert!(text.contains("### app (Rust crate, app)"));
        assert!(text.contains("Depends on: core"));
        assert!(text.contains("uses db, ui"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn notes_live_in_the_repository() {
        let root = temp("notes");
        let file = notes_path(&root);
        assert_eq!(file, root.join("docs/architecture.md"));
        // Nothing written yet: no file appears.
        save_notes(&file, "  ").unwrap();
        assert!(!file.exists());
        save_notes(&file, "# Arch\n").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "# Arch\n");
        write(&root, "ARCHITECTURE.md", "top");
        assert_eq!(notes_path(&root), root.join("ARCHITECTURE.md"));

        let ctx = context("runo", None, "", "docs/architecture.md");
        assert!(ctx.contains("no architecture notes yet"));
        let ctx = context(
            "runo",
            None,
            "Keep UI off the agent thread.",
            "docs/architecture.md",
        );
        assert!(ctx.contains("## Architecture notes and decisions (docs/architecture.md)"));
        assert!(ctx.contains("Keep UI off the agent thread."));
        let _ = std::fs::remove_dir_all(&root);
    }
}
