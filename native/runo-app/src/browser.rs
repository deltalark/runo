//! The Browser tab: find the session's dev servers, check a page, open it.
//!
//! GPUI has no web view, so pages open in the system browser; this tab does the
//! finding and checking around that.

use super::*;
use std::net::{SocketAddr, TcpStream};

/// Ports dev servers commonly listen on.
const DEV_PORTS: [u16; 14] = [
    3000, 3001, 4000, 4200, 4321, 5000, 5173, 5174, 6006, 8000, 8080, 8081, 8888, 9000,
];

pub(crate) struct PageCheck {
    pub url: String,
    pub status: String,
    pub title: String,
    pub ok: bool,
}

/// Turn what was typed into a URL: `localhost:3000` and `:5173` work too.
pub(crate) fn normalize_url(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() || raw.contains(char::is_whitespace) {
        return None;
    }
    if raw.starts_with("http://") || raw.starts_with("https://") {
        return Some(raw.to_string());
    }
    if let Some(port) = raw.strip_prefix(':') {
        return port
            .parse::<u16>()
            .ok()
            .map(|p| format!("http://localhost:{p}"));
    }
    if raw.parse::<u16>().is_ok() {
        return Some(format!("http://localhost:{raw}"));
    }
    let local =
        raw.starts_with("localhost") || raw.starts_with("127.") || raw.starts_with("0.0.0.0");
    if local {
        return Some(format!("http://{raw}"));
    }
    raw.contains('.').then(|| format!("https://{raw}"))
}

/// http(s) links printed in terminal output, newest first, without duplicates.
pub(crate) fn urls_in(lines: &[String]) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for line in lines.iter().rev() {
        for (i, _) in line.match_indices("http") {
            let rest = &line[i..];
            if !(rest.starts_with("http://") || rest.starts_with("https://")) {
                continue;
            }
            let url: String = rest
                .chars()
                .take_while(|c| !c.is_whitespace() && !"\"'<>`)]}".contains(*c))
                .collect();
            let url = url.trim_end_matches(['.', ',', ';', ':']).to_string();
            if url.len() > 10 && !found.contains(&url) {
                found.push(url);
            }
        }
    }
    found.truncate(8);
    found
}

/// `<title>` of an HTML page, whitespace collapsed.
pub(crate) fn html_title(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let Some(start) = lower.find("<title") else {
        return String::new();
    };
    let Some(open_end) = lower[start..].find('>').map(|i| start + i + 1) else {
        return String::new();
    };
    let end = lower[open_end..]
        .find("</title")
        .map(|i| open_end + i)
        .unwrap_or(open_end);
    html[open_end..end]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

impl RunoApp {
    /// Look for listening dev-server ports on this machine, off the UI thread.
    pub(crate) fn scan_ports(&mut self) {
        if self.browser_scanning {
            return;
        }
        self.browser_scanning = true;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let open: Vec<String> = DEV_PORTS
                .iter()
                .filter(|p| {
                    let addr = SocketAddr::from(([127, 0, 0, 1], **p));
                    TcpStream::connect_timeout(&addr, Duration::from_millis(120)).is_ok()
                })
                .map(|p| p.to_string())
                .collect();
            let _ = tx.send(Event::Output("ports".into(), Ok(open.join(","))));
        });
    }

    /// Fetch the page with curl for its status and title.
    pub(crate) fn check_page(&mut self, url: String) {
        self.remember_url(&url);
        self.browser_check = Some(PageCheck {
            url: url.clone(),
            status: "Checking…".into(),
            title: String::new(),
            ok: true,
        });
        let mut cmd = std::process::Command::new("curl");
        cmd.args([
            "-sL",
            "-m",
            "6",
            "-w",
            "\n__RUNO_STATUS__%{http_code}",
            &url,
        ]);
        agent::oneshot(format!("page:{url}"), cmd, ".", self.tx.clone());
    }

    pub(crate) fn remember_url(&mut self, url: &str) {
        self.browser_recent.retain(|u| u != url);
        self.browser_recent.insert(0, url.to_string());
        self.browser_recent.truncate(6);
    }

    pub(crate) fn open_in_browser(&mut self, url: String, cx: &mut Context<Self>) {
        self.remember_url(&url);
        cx.open_url(&url);
    }

    /// Results of the port scan and page checks.
    pub(crate) fn handle_browser_job(&mut self, key: &str, result: Result<String, String>) -> bool {
        if key == "ports" {
            self.browser_scanning = false;
            self.browser_ports = result
                .unwrap_or_default()
                .split(',')
                .filter_map(|p| p.parse().ok())
                .collect();
            return true;
        }
        let Some(url) = key.strip_prefix("page:") else {
            return false;
        };
        if self.browser_check.as_ref().is_none_or(|c| c.url != url) {
            return true;
        }
        let check = match result {
            Ok(out) => {
                let (body, code) = out
                    .rsplit_once("__RUNO_STATUS__")
                    .unwrap_or((out.as_str(), ""));
                let code = code.trim().to_string();
                let ok = code.starts_with('2') || code.starts_with('3');
                PageCheck {
                    url: url.to_string(),
                    status: if code == "000" || code.is_empty() {
                        "Not reachable".into()
                    } else {
                        format!("HTTP {code}")
                    },
                    title: html_title(body),
                    ok,
                }
            }
            Err(e) => PageCheck {
                url: url.to_string(),
                status: if e.contains("No such file") || e.contains("not found") {
                    "curl is not installed".into()
                } else {
                    "Not reachable".into()
                },
                title: String::new(),
                ok: false,
            },
        };
        self.browser_check = Some(check);
        true
    }

    fn url_row(
        &self,
        id: (&'static str, usize),
        url: &str,
        lead: IconName,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (open, check) = (url.to_string(), url.to_string());
        stagger(
            div()
                .id(id)
                .group("url-row")
                .h(px(32.))
                .px_2()
                .flex()
                .items_center()
                .gap_2()
                .rounded_md()
                .hover(|el| el.bg(tint(c_subtle())))
                .child(icon(lead, 13., c_muted()))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_family(self.mono_font.clone())
                        .text_size(px(12.))
                        .child(url.to_string()),
                )
                .child(
                    self.ghost_button_sized(
                        (id.0, id.1 + 1000),
                        IconName::Activity,
                        "Check the page",
                        24.,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.check_page(check.clone());
                        cx.notify();
                    })),
                )
                .child(
                    self.ghost_button_sized(
                        (id.0, id.1 + 2000),
                        IconName::ExternalLink,
                        "Open in your browser",
                        24.,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.open_in_browser(open.clone(), cx);
                        cx.notify();
                    })),
                ),
            SharedString::from(format!("{}-{}-in", id.0, id.1)),
            id.1,
        )
    }

    pub(crate) fn submit_address(&mut self, open: bool, cx: &mut Context<Self>) {
        let raw = self.address.read(cx).value().to_string();
        match normalize_url(&raw) {
            Some(url) if open => self.open_in_browser(url, cx),
            Some(url) => self.check_page(url),
            None if raw.trim().is_empty() => {}
            None => self.notice = Some("Enter a URL such as localhost:3000 or example.com.".into()),
        }
        cx.notify();
    }

    pub(crate) fn browser_tab(&self, cx: &mut Context<Self>) -> AnyElement {
        let id = self.selected.unwrap_or(0);
        let mut printed: Vec<String> = self.terminal.get(&id).cloned().unwrap_or_default();
        for shell in self.session_shells(id) {
            printed.extend(shell.text().lines().map(str::to_string));
        }
        let detected = urls_in(&printed);
        let section = |title: &'static str| {
            div()
                .mt_2()
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(c_muted()))
                .child(title)
        };
        let mut body = div()
            .id("browser-body")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_4()
            .v_flex()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&self.address).bordered(true).h(px(34.))),
                    )
                    .child(
                        self.button("check-url", "Check", false)
                            .h(px(34.))
                            .child(icon(IconName::Activity, 12., c_secondary()))
                            .on_click(cx.listener(|this, _, _, cx| this.submit_address(false, cx))),
                    )
                    .child(
                        self.button("open-url", "Open", true)
                            .h(px(34.))
                            .child(icon(IconName::ExternalLink, 12., c_canvas()))
                            .on_click(cx.listener(|this, _, _, cx| this.submit_address(true, cx))),
                    ),
            )
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(rgb(c_muted()))
                    .child("Enter opens the page in your browser. Bare ports like :5173 work."),
            );
        if let Some(check) = &self.browser_check {
            let url = check.url.clone();
            let color = if check.status == "Checking…" {
                c_secondary()
            } else if check.ok {
                c_green()
            } else {
                c_red()
            };
            body = body.child(rise(
                div()
                    .mt_2()
                    .p_3()
                    .v_flex()
                    .gap_1()
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(c_border()))
                    .bg(tint(c_subtle()))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(icon(
                                if check.status == "Checking…" {
                                    IconName::Loader
                                } else if check.ok {
                                    IconName::CircleCheck
                                } else {
                                    IconName::CircleX
                                },
                                14.,
                                color,
                            ))
                            .child(
                                div()
                                    .text_size(px(12.5))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(color))
                                    .child(check.status.clone()),
                            )
                            .child(div().flex_1())
                            .child(
                                self.ghost_button_sized(
                                    "check-open",
                                    IconName::ExternalLink,
                                    "Open in your browser",
                                    24.,
                                )
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.open_in_browser(url.clone(), cx);
                                        cx.notify();
                                    },
                                )),
                            ),
                    )
                    .when(check.ok && check.status != "Checking…", |el| {
                        el.child(div().text_size(px(13.)).child(if check.title.is_empty() {
                            "No page title".to_string()
                        } else {
                            check.title.clone()
                        }))
                    })
                    .child(
                        div()
                            .truncate()
                            .font_family(self.mono_font.clone())
                            .text_size(px(11.5))
                            .text_color(rgb(c_muted()))
                            .child(check.url.clone()),
                    ),
                SharedString::from(format!("check-{}-{}", check.url, check.status)),
                260,
            ));
        }
        body = body.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(section("RUNNING ON THIS MACHINE"))
                .child(
                    self.ghost_button_sized("scan-ports", IconName::RefreshCw, "Scan again", 24.)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.scan_ports();
                            cx.notify();
                        })),
                ),
        );
        if self.browser_ports.is_empty() {
            body = body.child(
                div()
                    .px_2()
                    .text_size(px(12.5))
                    .text_color(rgb(c_muted()))
                    .child(if self.browser_scanning {
                        "Looking for dev servers…"
                    } else {
                        "No dev server found on the usual ports. Start one in the Terminal."
                    }),
            );
        }
        for (i, port) in self.browser_ports.iter().enumerate() {
            body = body.child(self.url_row(
                ("port-url", i),
                &format!("http://localhost:{port}"),
                IconName::Globe,
                cx,
            ));
        }
        if !detected.is_empty() {
            body = body.child(section("PRINTED IN THE TERMINAL"));
            for (i, url) in detected.iter().enumerate() {
                body = body.child(self.url_row(("term-url", i), url, IconName::SquareTerminal, cx));
            }
        }
        if !self.browser_recent.is_empty() {
            body = body.child(section("RECENT"));
            for (i, url) in self.browser_recent.iter().enumerate() {
                body = body.child(self.url_row(("recent-url", i), url, IconName::Clock, cx));
            }
        }
        div()
            .flex_1()
            .min_h_0()
            .v_flex()
            .child(body)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // `super::*` brings in gpui's own `test` attribute; use the built-in one.
    use std::prelude::v1::test;

    #[test]
    fn urls_are_normalized_found_and_titled() {
        assert_eq!(
            normalize_url("localhost:3000").as_deref(),
            Some("http://localhost:3000")
        );
        assert_eq!(
            normalize_url(":5173").as_deref(),
            Some("http://localhost:5173")
        );
        assert_eq!(
            normalize_url("8080").as_deref(),
            Some("http://localhost:8080")
        );
        assert_eq!(
            normalize_url("example.com").as_deref(),
            Some("https://example.com")
        );
        assert_eq!(
            normalize_url("https://a.b/c").as_deref(),
            Some("https://a.b/c")
        );
        assert_eq!(normalize_url("not a url"), None);
        assert_eq!(normalize_url("word"), None);
        let lines = vec![
            "  ➜  Local:   http://localhost:5173/".to_string(),
            "see (https://docs.rs/x)".to_string(),
            "again http://localhost:5173/.".to_string(),
        ];
        assert_eq!(
            urls_in(&lines),
            ["http://localhost:5173/", "https://docs.rs/x"]
        );
        assert_eq!(html_title("<html><TITLE>\n My  App </title>"), "My App");
        assert_eq!(html_title("<p>none</p>"), "");
    }
}
