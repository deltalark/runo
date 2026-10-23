//! Snapping raw values to the theme: by hand from the checks, every screen at once, and
//! on its own after an AI change, so what AI writes follows the theme like the rest.

use super::super::snap::{self, Theme};
use super::*;

/// Screens snapped, each with the HTML from before, how it's drawn now and how many
/// values snapped.
type Snapped = Vec<(String, String, Shot, usize)>;

impl DesignCanvas {
    /// Write every value on every screen that is, or is all but, a token as that token.
    /// Each screen changed is one step to undo.
    pub(super) fn snap_to_tokens(&mut self, cx: &mut Context<Self>) {
        let theme = Theme::new(&self.tokens);
        if theme.is_empty() {
            self.say("The theme has no tokens to snap to yet.");
            cx.notify();
            return;
        }
        let screens: Vec<(String, Device)> = self
            .canvas
            .screens
            .iter()
            .map(|s| (s.id.clone(), s.device))
            .collect();
        self.run(
            cx,
            move |r| {
                let mut out: Snapped = Vec::new();
                for (id, device) in screens {
                    let Ok(found) = r.eval(&id, device, &dom::call("inlineStyles", &[])) else {
                        continue;
                    };
                    let found: Vec<(dom::Path, String)> =
                        serde_json::from_value(found).unwrap_or_default();
                    let mut n = 0;
                    let list: Vec<(dom::Path, String)> = found
                        .into_iter()
                        .filter_map(|(p, css)| {
                            let (now, k) = snap::style(&theme, &css)?;
                            n += k;
                            Some((p, now))
                        })
                        .collect();
                    if list.is_empty() {
                        continue;
                    }
                    let js = dom::call("setStyles", &[json!(list)]);
                    if let Ok(Some((before, shot))) = r.edit(&id, device, &js) {
                        out.push((id, before, shot, n));
                    }
                }
                out
            },
            |this, out, cx| {
                let values: usize = out.iter().map(|s| s.3).sum();
                let screens = out.len();
                for (id, before, shot, _) in out {
                    this.landed(id, before, shot, After::Same, cx);
                }
                this.say(match (values, screens) {
                    (0, _) => "Every value close to a token already uses it.".to_string(),
                    (v, 1) => format!("Snapped {v} {} to the theme's tokens.", plural(v)),
                    (v, s) => format!(
                        "Snapped {v} {} to the theme's tokens, on {s} screens.",
                        plural(v)
                    ),
                });
            },
        );
    }

    /// After an AI change, write what it added that is all but a token as that token, in
    /// the screens' files. Gives how many values snapped.
    pub(super) fn snap_ai_change(&self, before: &HashMap<PathBuf, Vec<u8>>) -> usize {
        snap::screens(
            &folder(&self.root).join("screens"),
            before,
            &Theme::new(&self.tokens),
        )
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        "value"
    } else {
        "values"
    }
}
