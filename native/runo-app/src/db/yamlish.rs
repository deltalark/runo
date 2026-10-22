//! Just enough YAML to read Compose files and Rails' `database.yml`: block maps, block
//! lists, scalars and one-line flow lists. Anchors, multi-line strings and the rest of
//! YAML are skipped; a file that uses them is still read as far as it goes.

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Node {
    Map(Vec<(String, Node)>),
    List(Vec<Node>),
    Scalar(String),
}

impl Node {
    pub(crate) fn get(&self, key: &str) -> Option<&Node> {
        match self {
            Node::Map(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub(crate) fn str(&self) -> Option<&str> {
        match self {
            Node::Scalar(s) => Some(s),
            _ => None,
        }
    }

    pub(crate) fn entries(&self) -> &[(String, Node)] {
        match self {
            Node::Map(entries) => entries,
            _ => &[],
        }
    }

    pub(crate) fn items(&self) -> &[Node] {
        match self {
            Node::List(items) => items,
            _ => &[],
        }
    }
}

/// One line that holds something: its indent and its text without the comment.
struct Line {
    indent: usize,
    text: String,
}

fn strip_comment(line: &str) -> &str {
    let mut quote = None;
    for (i, c) in line.char_indices() {
        match (c, quote) {
            ('"' | '\'', None) => quote = Some(c),
            (q, Some(open)) if q == open => quote = None,
            ('#', None) if i == 0 || line[..i].ends_with(char::is_whitespace) => return &line[..i],
            _ => {}
        }
    }
    line
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

fn scalar(text: &str) -> Node {
    let t = text.trim();
    if t.starts_with('[') && t.ends_with(']') {
        let inner = &t[1..t.len() - 1];
        return Node::List(
            inner
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| Node::Scalar(unquote(s)))
                .collect(),
        );
    }
    // An anchor or tag in front of a value is dropped.
    let t = t
        .strip_prefix('&')
        .map(|r| r.split_once(' ').map_or("", |(_, v)| v))
        .unwrap_or(t);
    Node::Scalar(unquote(t))
}

/// `key: value` split at the first colon that is followed by a space or the end.
fn key_value(text: &str) -> Option<(String, &str)> {
    let mut quote = None;
    for (i, c) in text.char_indices() {
        match (c, quote) {
            ('"' | '\'', None) if i == 0 => quote = Some(c),
            (q, Some(open)) if q == open => quote = None,
            (':', None) => {
                let rest = &text[i + 1..];
                if rest.is_empty() || rest.starts_with(' ') {
                    return Some((unquote(&text[..i]), rest.trim()));
                }
            }
            _ => {}
        }
    }
    None
}

pub(crate) fn parse(text: &str) -> Node {
    let lines: Vec<Line> = text
        .lines()
        .filter(|l| !l.trim_start().starts_with("---"))
        .map(|l| {
            let body = strip_comment(l).trim_end();
            Line {
                indent: body.len() - body.trim_start().len(),
                text: body.trim_start().to_string(),
            }
        })
        .filter(|l| !l.text.is_empty())
        .collect();
    let mut at = 0;
    block(&lines, &mut at, 0)
}

/// The block that starts at `lines[*at]`, whose lines are indented at least `min`.
fn block(lines: &[Line], at: &mut usize, min: usize) -> Node {
    let Some(first) = lines.get(*at) else {
        return Node::Scalar(String::new());
    };
    if first.indent < min {
        return Node::Scalar(String::new());
    }
    let indent = first.indent;
    if first.text.starts_with("- ") || first.text == "-" {
        let mut items = Vec::new();
        while let Some(line) = lines.get(*at) {
            if line.indent != indent || !(line.text.starts_with("- ") || line.text == "-") {
                break;
            }
            let rest = line.text[1..].trim_start().to_string();
            *at += 1;
            if rest.is_empty() {
                items.push(block(lines, at, indent + 1));
            } else if let Some((key, value)) = key_value(&rest) {
                // A map that starts on the dash line; its other keys sit under the key.
                let mut entries = vec![(key, entry(value, lines, at, indent + 1))];
                if let Some(Node::Map(more)) = lines
                    .get(*at)
                    .filter(|l| l.indent > indent)
                    .map(|_| block(lines, at, indent + 1))
                {
                    entries.extend(more);
                }
                items.push(Node::Map(entries));
            } else {
                items.push(scalar(&rest));
            }
        }
        return Node::List(items);
    }
    let mut entries = Vec::new();
    while let Some(line) = lines.get(*at) {
        if line.indent != indent {
            if line.indent < indent {
                break;
            }
            // Deeper than expected and not under a key: skip it.
            *at += 1;
            continue;
        }
        let Some((key, value)) = key_value(&line.text) else {
            *at += 1;
            continue;
        };
        *at += 1;
        entries.push((key, entry(value, lines, at, indent + 1)));
    }
    Node::Map(entries)
}

/// A key's value: on its own line, or the block under it.
fn entry(value: &str, lines: &[Line], at: &mut usize, min: usize) -> Node {
    let value = value.trim();
    let block_follows = value.is_empty() || value.starts_with('&') && !value.contains(' ');
    if block_follows {
        // Lists are often written at the same indent as their key.
        let same_indent_list = lines
            .get(*at)
            .is_some_and(|l| l.indent + 1 == min && l.text.starts_with("- "));
        if same_indent_list {
            return block(lines, at, min - 1);
        }
        return block(lines, at, min);
    }
    if value == "|" || value == ">" || value.starts_with("|-") || value.starts_with(">-") {
        while lines.get(*at).is_some_and(|l| l.indent >= min) {
            *at += 1;
        }
        return Node::Scalar(String::new());
    }
    scalar(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_compose_file() {
        let doc = parse(
            r#"
version: "3.9"
services:
  db:
    image: postgres:16   # the database
    ports:
      - "5433:5432"
    environment:
      POSTGRES_USER: app
      POSTGRES_PASSWORD: "p#ss"
  cache:
    image: 'redis:7'
    environment:
    - FOO=bar
    ports: ["6379:6379"]
  worker:
    command: |
      run --fast
    image: node
"#,
        );
        let services = doc.get("services").unwrap();
        let db = services.get("db").unwrap();
        assert_eq!(db.get("image").unwrap().str(), Some("postgres:16"));
        assert_eq!(db.get("ports").unwrap().items()[0].str(), Some("5433:5432"));
        let env = db.get("environment").unwrap();
        assert_eq!(env.get("POSTGRES_PASSWORD").unwrap().str(), Some("p#ss"));
        let cache = services.get("cache").unwrap();
        assert_eq!(
            cache.get("environment").unwrap().items()[0].str(),
            Some("FOO=bar")
        );
        assert_eq!(
            cache.get("ports").unwrap().items()[0].str(),
            Some("6379:6379")
        );
        assert_eq!(
            services.get("worker").unwrap().get("image").unwrap().str(),
            Some("node")
        );
    }

    #[test]
    fn reads_rails_database_yml() {
        let doc = parse(
            "default: &default\n  adapter: postgresql\n  host: localhost\n\ndevelopment:\n  <<: *default\n  database: shop_development\n",
        );
        assert_eq!(
            doc.get("default").unwrap().get("adapter").unwrap().str(),
            Some("postgresql")
        );
        assert_eq!(
            doc.get("development")
                .unwrap()
                .get("database")
                .unwrap()
                .str(),
            Some("shop_development")
        );
    }

    #[test]
    fn list_items_can_be_maps() {
        let doc = parse("ports:\n  - target: 5432\n    published: 5440\n");
        let item = &doc.get("ports").unwrap().items()[0];
        assert_eq!(item.get("published").unwrap().str(), Some("5440"));
        assert_eq!(item.get("target").unwrap().str(), Some("5432"));
    }
}
