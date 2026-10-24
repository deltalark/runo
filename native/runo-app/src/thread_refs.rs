//! Thread numbers in Crew's chat, such as "#34", as links to the thread. Before a message
//! is parsed, each `#N` naming one of the project's threads becomes a Markdown link to
//! `runo-thread:N`; a plugin then draws that link in the accent with the thread's title
//! and status on hover, and opens the thread when clicked. Numbers in code, links and
//! URLs, numbers that aren't threads, and ones that follow "PR" or "issue" stay as written.
//! Copying keeps the plain `#34`.

use super::*;
use crate::crew_ui::thread_status;
use gpui_kit::component::text::{
    markdown_ast, InlineElement, InlineRenderContext, MarkdownNode, MarkdownParseContext,
    MarkdownPlugin,
};
use std::ops::Range;
use std::sync::Arc;

/// Name of the Markdown nodes the plugin makes.
const NAME: &str = "thread-ref";
/// Link scheme the rewrite points thread numbers at.
const SCHEME: &str = "runo-thread:";
/// Marks a reference with no title written next to it, which may show one.
const BARE: &str = "+title";
/// Words that make the number after them something other than a thread.
const NOT_THREADS: &[&str] = &[
    "pr", "prs", "pull", "request", "requests", "issue", "issues", "mr", "ticket", "bug",
];
/// Longest title shown next to a number.
const TITLE_CHARS: usize = 32;

/// A thread a reference can name: its title and how it stands.
#[derive(Clone)]
pub(crate) struct Thread {
    pub title: String,
    pub status: &'static str,
}

/// A thread number found in a message: where it is and which thread.
#[derive(Debug, PartialEq)]
pub(crate) struct Found {
    pub range: Range<usize>,
    pub thread: u64,
}

/// The `#N` references in Markdown `text` that `known` says are threads, in order. Fenced
/// code, code spans, links, autolinks, URLs, escaped `\#N` and numbers after "PR",
/// "issue" and the like are left out.
pub(crate) fn find(text: &str, known: impl Fn(u64) -> bool) -> Vec<Found> {
    let mut out = Vec::new();
    // The open fence's character and length, while inside one.
    let mut fence: Option<(u8, usize)> = None;
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        let start = at;
        at += line.len();
        let body = line.trim_start_matches(' ');
        if line.len() - body.len() <= 3 {
            if let Some((ch, n)) = fence_run(body) {
                match fence {
                    None => {
                        fence = Some((ch, n));
                        continue;
                    }
                    Some((open, len)) if open == ch && n >= len && body[n..].trim().is_empty() => {
                        fence = None;
                        continue;
                    }
                    _ => {}
                }
            }
        }
        if fence.is_none() {
            scan_line(line, start, &known, &mut out);
        }
    }
    out
}

/// `text` with each thread reference turned into a link the plugin draws. A reference
/// that already has its thread's title or a bracket after it is marked so no second
/// title shows.
pub(crate) fn link(text: &str, threads: &HashMap<u64, Thread>) -> String {
    let found = find(text, |n| threads.contains_key(&n));
    if found.is_empty() {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len() + found.len() * 24);
    let mut last = 0;
    for f in found {
        let after = text[f.range.end..].trim_start_matches([' ', '\t']);
        let title = crew::shown_title(&threads[&f.thread].title);
        let titled = after.starts_with('(')
            || (!title.is_empty() && after.to_lowercase().starts_with(&title.to_lowercase()));
        out.push_str(&text[last..f.range.start]);
        out.push_str(&format!(
            "[#{n}]({SCHEME}{n}{})",
            if titled { "" } else { BARE },
            n = f.thread
        ));
        last = f.range.end;
    }
    out.push_str(&text[last..]);
    out
}

/// The thread a `runo-thread:` link names, and whether it may show the title.
fn parse_url(url: &str) -> Option<(u64, bool)> {
    let rest = url.strip_prefix(SCHEME)?;
    let (num, bare) = match rest.strip_suffix(BARE) {
        Some(num) => (num, true),
        None => (rest, false),
    };
    Some((num.parse().ok()?, bare))
}

/// A fence opening or closing a code block: three or more backticks or tildes.
fn fence_run(body: &str) -> Option<(u8, usize)> {
    let ch = *body.as_bytes().first()?;
    if ch != b'`' && ch != b'~' {
        return None;
    }
    let n = body.bytes().take_while(|b| *b == ch).count();
    (n >= 3).then_some((ch, n))
}

/// Where the run of `ch` starting at `from` ends.
fn run_end(bytes: &[u8], from: usize, ch: u8) -> usize {
    from + bytes[from..].iter().take_while(|b| **b == ch).count()
}

/// The index of the bracket closing the one at `open`, on the same line.
fn closing(bytes: &[u8], open: usize, left: u8, right: u8) -> Option<usize> {
    let mut depth = 0;
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 1,
            b if b == left => depth += 1,
            b if b == right => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// The thread references on one line outside a code block; `start` is where it begins.
fn scan_line(line: &str, start: usize, known: &impl Fn(u64) -> bool, out: &mut Vec<Found>) {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            // A code span runs to the next run of as many backticks.
            b'`' => {
                let end = run_end(bytes, i, b'`');
                let n = end - i;
                let mut j = end;
                i = end;
                while j < bytes.len() {
                    if bytes[j] == b'`' {
                        let close = run_end(bytes, j, b'`');
                        if close - j == n {
                            i = close;
                            break;
                        }
                        j = close;
                    } else {
                        j += 1;
                    }
                }
            }
            // An autolink such as <https://…>.
            b'<' => {
                i = match line[i..].find('>') {
                    Some(end)
                        if !line[i..i + end].contains(' ')
                            && line[i..i + end].contains([':', '@']) =>
                    {
                        i + end + 1
                    }
                    _ => i + 1,
                };
            }
            // A link's text and target are already a link; plain brackets are read.
            b'[' => {
                i = match closing(bytes, i, b'[', b']') {
                    Some(end) if bytes.get(end + 1) == Some(&b'(') => {
                        closing(bytes, end + 1, b'(', b')').map_or(i + 1, |e| e + 1)
                    }
                    Some(end) if bytes.get(end + 1) == Some(&b'[') => {
                        closing(bytes, end + 1, b'[', b']').map_or(i + 1, |e| e + 1)
                    }
                    _ => i + 1,
                };
            }
            b'#' => {
                let end = i
                    + 1
                    + bytes[i + 1..]
                        .iter()
                        .take_while(|b| b.is_ascii_digit())
                        .count();
                if let Some(thread) = reference(line, i, end) {
                    if known(thread) {
                        out.push(Found {
                            range: start + i..start + end,
                            thread,
                        });
                    }
                }
                i = end.max(i + 1);
            }
            _ => i += 1,
        }
    }
}

/// The thread number `line[at..end]` stands for, when it reads as a thread reference:
/// `#` then digits, not part of a word or URL, and not after "PR" and the like.
fn reference(line: &str, at: usize, end: usize) -> Option<u64> {
    let bytes = line.as_bytes();
    if end == at + 1 || end - at > 10 {
        return None;
    }
    // An underscore is emphasis, as in _#34_, unless it joins a word, as in #34_x.
    let word_char = |b: Option<&u8>| b.is_some_and(|b| b.is_ascii_alphanumeric());
    let after = bytes.get(end);
    if word_char(after) || (after == Some(&b'_') && word_char(bytes.get(end + 1))) {
        return None;
    }
    let before = &line[..at];
    let mut back = before.chars().rev();
    if back.next().is_some_and(|c| {
        c.is_alphanumeric()
            || matches!(c, '/' | '&' | '#' | '-' | ':' | '.' | '=' | '?' | '@')
            || (c == '_' && back.next().is_some_and(char::is_alphanumeric))
    }) {
        return None;
    }
    // The word it sits in, for URLs such as https://example.com/?x #34 don't count.
    let word = before
        .rsplit(|c: char| c.is_whitespace() || c == '(' || c == '<')
        .next()
        .unwrap_or("");
    if word.contains("://") || word.starts_with("www.") {
        return None;
    }
    let prev = before
        .trim_end_matches(|c: char| c.is_whitespace() || matches!(c, ':' | '*' | '_'))
        .rsplit(|c: char| !c.is_alphanumeric())
        .next()
        .unwrap_or("")
        .to_lowercase();
    if NOT_THREADS.contains(&prev.as_str()) {
        return None;
    }
    line[at + 1..end].parse().ok()
}

/// Draws the links `link` made: the number in the accent, underlined on hover, with the
/// thread's title and status in a tooltip, and its title beside it when `titles` is on.
/// A click opens the thread beside the chat; ⌘-click opens it in the main view.
#[derive(Clone)]
pub(crate) struct ThreadLinks {
    pub threads: Arc<HashMap<u64, Thread>>,
    pub titles: bool,
    pub app: WeakEntity<RunoApp>,
}

impl ThreadLinks {
    /// `text` with its thread references as links; see [`link`].
    pub(crate) fn link(&self, text: &str) -> String {
        link(text, &self.threads)
    }
}

/// What the parser keeps of a reference.
struct Ref {
    thread: u64,
    bare: bool,
}

impl MarkdownPlugin for ThreadLinks {
    fn name(&self) -> &str {
        NAME
    }

    fn parse(
        &self,
        node: &markdown_ast::Node,
        _cx: &MarkdownParseContext<'_>,
    ) -> Option<MarkdownNode> {
        let markdown_ast::Node::Link(link) = node else {
            return None;
        };
        let (thread, bare) = parse_url(&link.url)?;
        let text = format!("#{thread}");
        Some(
            MarkdownNode::new(NAME, Ref { thread, bare })
                .text(text.clone())
                .markdown(text),
        )
    }

    fn render_inline(
        &self,
        node: &MarkdownNode,
        _context: &InlineRenderContext,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Option<InlineElement> {
        let r = node.data::<Ref>()?;
        // A thread deleted since reads as its plain number.
        let info = self.threads.get(&r.thread)?;
        let thread = r.thread;
        let title = crew::shown_title(&info.title);
        let tip = if title.is_empty() {
            info.status.to_string()
        } else {
            format!("{title} · {}", info.status)
        };
        let chip = (self.titles && r.bare && !title.is_empty()).then(|| {
            if title.chars().count() > TITLE_CHARS {
                let cut: String = title.chars().take(TITLE_CHARS - 1).collect();
                format!("{}…", cut.trim_end())
            } else {
                title.clone()
            }
        });
        let app = self.app.clone();
        let key = node.source_range().map_or(thread as usize, |r| r.start);
        Some(InlineElement::new(
            div()
                .id(("thread-ref", key))
                .flex()
                .gap(px(4.))
                .cursor_pointer()
                .text_color(rgb(c_blue()))
                .hover(|el| el.underline())
                .tooltip(move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(tip.clone()).build(window, cx)
                })
                .on_click(move |event, _, cx| {
                    let main = event.modifiers().platform;
                    app.update(cx, |this, cx| {
                        this.open_thread_ref(thread, main);
                        cx.notify();
                    })
                    .ok();
                })
                .child(format!("#{thread}"))
                .children(chip),
        ))
    }
}

impl RunoApp {
    /// The links for thread numbers in the open chat, when it is a Crew chat.
    pub(crate) fn thread_links(&self, cx: &mut Context<Self>) -> Option<ThreadLinks> {
        let chat = self.current().filter(|s| s.canvas == crew::CANVAS)?;
        let threads = self
            .crew_threads(chat.project_id, true)
            .into_iter()
            .map(|t| {
                let thread = Thread {
                    title: t.title.clone(),
                    status: thread_status(self.thread_working(t.id), t),
                };
                (t.id, thread)
            })
            .collect();
        Some(ThreadLinks {
            threads: Arc::new(threads),
            titles: cx.global::<AppSettings>().thread_ref_titles,
            app: cx.entity().downgrade(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn threads(text: &str) -> Vec<u64> {
        find(text, |n| [3, 24, 34].contains(&n))
            .into_iter()
            .map(|f| f.thread)
            .collect()
    }

    #[test]
    fn finds_real_thread_numbers() {
        assert_eq!(threads("#34 and #24 are done."), [34, 24]);
        assert_eq!(threads("See #3, then (#24)."), [3, 24]);
        assert_eq!(threads("- **#34** finished\n- #24: needs you"), [34, 24]);
        assert_eq!(threads("_#24_ and *#3* and __#34__"), [24, 3, 34]);
        let found = find("ask #34 now", |n| n == 34);
        assert_eq!(
            found,
            [Found {
                range: 4..7,
                thread: 34
            }]
        );
    }

    #[test]
    fn skips_numbers_that_are_not_threads() {
        assert_eq!(threads("#99 and #1 aren't threads"), Vec::<u64>::new());
        assert_eq!(threads("#340 #34a #34_x a#34 foo_#34"), Vec::<u64>::new());
        assert_eq!(threads("# 34 and #"), Vec::<u64>::new());
    }

    #[test]
    fn skips_code() {
        assert_eq!(threads("run `grep #34` first"), Vec::<u64>::new());
        assert_eq!(threads("``a ` #34`` then #24"), [24]);
        assert_eq!(threads("```\n#34\n```\n#24"), [24]);
        assert_eq!(threads("~~~~rust\n#34\n~~~\n#3\n~~~~\n#24"), [24]);
    }

    #[test]
    fn skips_pull_requests_and_issues() {
        assert_eq!(
            threads("PR #34, pr #24, PR#3, issue #34, pull request #24"),
            Vec::<u64>::new()
        );
        assert_eq!(threads("Opened PR: #34. Thread #24 helped."), [24]);
    }

    #[test]
    fn skips_urls_and_links() {
        assert_eq!(
            threads("https://github.com/o/r/pull/34#34 and https://x.com/?a=1 #24x"),
            Vec::<u64>::new()
        );
        assert_eq!(threads("see https://x.com/page,#34"), Vec::<u64>::new());
        assert_eq!(
            threads("<https://example.com/#34> [see #34](https://x.io) #24"),
            [24]
        );
        assert_eq!(threads("[#34][ref] and [plain #24] and \\#3"), [24]);
    }

    #[test]
    fn links_threads_and_marks_ones_already_titled() {
        let map: HashMap<u64, Thread> = [
            (
                34,
                Thread {
                    title: "Review X post".into(),
                    status: "done",
                },
            ),
            (
                24,
                Thread {
                    title: "Fix login".into(),
                    status: "working",
                },
            ),
        ]
        .into();
        assert_eq!(
            link("#34 is done; #24 (Fix login) too. `#34`", &map),
            "[#34](runo-thread:34+title) is done; [#24](runo-thread:24) (Fix login) too. `#34`"
        );
        assert_eq!(
            link("#34 review x post", &map),
            "[#34](runo-thread:34) review x post"
        );
        assert_eq!(link("no refs #7", &map), "no refs #7");
    }

    #[test]
    fn reads_link_targets() {
        assert_eq!(parse_url("runo-thread:34"), Some((34, false)));
        assert_eq!(parse_url("runo-thread:34+title"), Some((34, true)));
        assert_eq!(parse_url("https://x.io"), None);
        assert_eq!(parse_url("runo-thread:x"), None);
    }

    /// The link targets in `node` and under it, in order.
    fn urls(node: &markdown::mdast::Node, out: &mut Vec<String>) {
        if let markdown::mdast::Node::Link(link) = node {
            out.push(link.url.clone());
        }
        for child in node.children().into_iter().flatten() {
            urls(child, out);
        }
    }

    #[test]
    fn rewritten_references_parse_as_links_the_plugin_reads() {
        let map: HashMap<u64, Thread> = [(34, "Review"), (24, "Fix"), (3, "Ship")]
            .map(|(n, title)| {
                let thread = Thread {
                    title: title.into(),
                    status: "done",
                };
                (n, thread)
            })
            .into();
        let text = link(
            "**#34** done, _#24_ too\n\n| a | b |\n|---|---|\n| #3 | x |\n\n> #34 again",
            &map,
        );
        let tree = markdown::to_mdast(&text, &markdown::ParseOptions::gfm()).unwrap();
        let mut found = Vec::new();
        urls(&tree, &mut found);
        let threads: Vec<u64> = found
            .iter()
            .filter_map(|u| parse_url(u))
            .map(|(n, _)| n)
            .collect();
        assert_eq!(threads, [34, 24, 3, 34]);
    }
}
