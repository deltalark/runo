//! The parts the Insert section offers, by kind: the pieces of `components.css` put together
//! the way an app uses them, so a whole app can be built by hand.

/// A part: what it's called and its HTML.
pub(crate) type Part = (&'static str, &'static str);

pub(crate) const PARTS: [(&str, &[Part]); 6] = [
    (
        "Basics",
        &[
            ("Heading", "<h2>Heading</h2>"),
            ("Text", "<p>Some text.</p>"),
            ("Caption", "<p class=\"small muted\">A short note.</p>"),
            ("Button", "<button class=\"button primary\">Button</button>"),
            (
                "Wide button",
                "<button class=\"button primary block\">Continue</button>",
            ),
            (
                "Secondary",
                "<button class=\"button secondary\">Button</button>",
            ),
            (
                "Icon button",
                "<button class=\"icon-button\"><i class=\"icon i-heart\"></i></button>",
            ),
            ("Icon", "<i class=\"icon i-star\"></i>"),
            ("Link", "<a class=\"link\">Link</a>"),
            ("Divider", "<hr class=\"divider\">"),
        ],
    ),
    (
        "Navigation",
        &[
            (
                "Top bar",
                "<header class=\"bar\"><button class=\"icon-button\" data-goto=\"back\"><i class=\"icon i-chevron-left\"></i></button><strong class=\"bar-title\">Title</strong><button class=\"icon-button\"><i class=\"icon i-ellipsis\"></i></button></header>",
            ),
            (
                "Large title",
                "<header class=\"bar plain\"><h1>Title</h1><div class=\"avatar sm\">A</div></header>",
            ),
            (
                "Tab bar",
                "<nav class=\"tabbar\"><a class=\"tab on\"><i class=\"icon i-house\"></i>Home</a><a class=\"tab\"><i class=\"icon i-search\"></i>Search</a><a class=\"tab\"><i class=\"icon i-heart\"></i>Saved</a><a class=\"tab\"><i class=\"icon i-user\"></i>Profile</a></nav>",
            ),
            (
                "Segmented",
                "<div class=\"segmented\"><span class=\"on\">Day</span><span>Week</span><span>Month</span></div>",
            ),
            (
                "Chips",
                "<div class=\"scroll-x\"><span class=\"chip on\">All</span><span class=\"chip\">Popular</span><span class=\"chip\">New</span><span class=\"chip\">Nearby</span><span class=\"chip\">Saved</span></div>",
            ),
            (
                "Floating button",
                "<button class=\"fab\"><i class=\"icon i-plus\"></i></button>",
            ),
        ],
    ),
    (
        "Inputs",
        &[
            (
                "Input",
                "<label class=\"field\"><span>Label</span><input placeholder=\"Placeholder\"></label>",
            ),
            (
                "Search",
                "<div class=\"search-box\"><i class=\"icon i-search\"></i><input class=\"search\" type=\"search\" placeholder=\"Search\"></div>",
            ),
            (
                "Text area",
                "<label class=\"field\"><span>Message</span><textarea placeholder=\"Write something\"></textarea></label>",
            ),
            (
                "Switch",
                "<div class=\"row\"><span class=\"grow\">Setting</span><span class=\"switch on\"></span></div>",
            ),
            (
                "Checkbox",
                "<label class=\"check\"><input type=\"checkbox\" checked> Remember me</label>",
            ),
            (
                "Select",
                "<label class=\"field\"><span>Choose</span><select><option>First</option><option>Second</option></select></label>",
            ),
        ],
    ),
    (
        "Content",
        &[
            (
                "Card",
                "<div class=\"card stack gap-sm\"><strong>Card title</strong><p class=\"muted\">Card text.</p></div>",
            ),
            (
                "List",
                "<ul class=\"list card\"><li class=\"row\"><span class=\"icon-tile\"><i class=\"icon i-bell\"></i></span><div class=\"grow\"><strong>First</strong><p class=\"small muted\">Subtitle</p></div><i class=\"icon i-chevron-right\"></i></li><li class=\"row\"><span class=\"icon-tile\"><i class=\"icon i-lock\"></i></span><div class=\"grow\"><strong>Second</strong><p class=\"small muted\">Subtitle</p></div><i class=\"icon i-chevron-right\"></i></li></ul>",
            ),
            (
                "Row",
                "<div class=\"row\"><div class=\"avatar\">A</div><div class=\"grow\"><strong>Title</strong><p class=\"small muted\">Subtitle</p></div><span class=\"badge\">New</span></div>",
            ),
            ("Image", "<div class=\"image\"></div>"),
            (
                "Media card",
                "<div class=\"stack gap-sm\"><div class=\"image warm\"></div><strong>Title</strong><p class=\"small muted\">A line about it</p></div>",
            ),
            (
                "Carousel",
                "<div class=\"scroll-x\"><div class=\"stack gap-xs\" style=\"width: 220px\"><div class=\"image cool\"></div><strong>First</strong></div><div class=\"stack gap-xs\" style=\"width: 220px\"><div class=\"image warm\"></div><strong>Second</strong></div><div class=\"stack gap-xs\" style=\"width: 220px\"><div class=\"image green\"></div><strong>Third</strong></div></div>",
            ),
            (
                "Grid",
                "<div class=\"grid-2\"><div class=\"stack gap-xs\"><div class=\"image square warm\"></div><strong>Item</strong><span class=\"muted\">$24</span></div><div class=\"stack gap-xs\"><div class=\"image square cool\"></div><strong>Item</strong><span class=\"muted\">$32</span></div></div>",
            ),
            ("Avatar", "<div class=\"avatar\">A</div>"),
            ("Badge", "<span class=\"badge\">New</span>"),
            (
                "Stat",
                "<div class=\"card stat\"><span class=\"muted small\">Balance</span><span class=\"value\">$4,120</span><span class=\"delta up\"><i class=\"icon icon-sm i-trending-up\"></i>12%</span></div>",
            ),
            (
                "Progress",
                "<div class=\"progress\"><span style=\"width: 60%\"></span></div>",
            ),
            (
                "Empty state",
                "<div class=\"empty\"><div class=\"illustration\"><i class=\"icon i-sparkles\"></i></div><h2>Nothing here yet</h2><p>When you add something, it shows up here.</p><button class=\"button primary\">Add one</button></div>",
            ),
        ],
    ),
    (
        "Overlays",
        &[
            (
                "Bottom sheet",
                "<div class=\"overlay\"><div class=\"sheet\"><h2>Sheet title</h2><p class=\"muted\">Choose what to do.</p><button class=\"button primary block\">Confirm</button><button class=\"button secondary block\">Cancel</button></div></div>",
            ),
            (
                "Dialog",
                "<div class=\"overlay\" style=\"align-items: center\"><div class=\"modal\"><h2>Are you sure?</h2><p class=\"muted\">This can't be undone.</p><div class=\"cluster\"><button class=\"button secondary\">Cancel</button><button class=\"button danger\">Delete</button></div></div></div>",
            ),
            (
                "Toast",
                "<div class=\"toast\"><i class=\"icon i-circle-check\"></i>Saved</div>",
            ),
            (
                "Callout",
                "<div class=\"callout\"><i class=\"icon i-info\"></i><p>Something worth knowing.</p></div>",
            ),
        ],
    ),
    (
        "Layout",
        &[
            ("Stack", "<div class=\"stack gap-sm\"><p>First</p><p>Second</p></div>"),
            (
                "Side by side",
                "<div class=\"row-between\"><strong>Left</strong><span class=\"muted\">Right</span></div>",
            ),
            (
                "Section",
                "<section class=\"section\"><div class=\"section-head\"><h3>Section</h3><a class=\"link small\">See all</a></div><p class=\"muted\">Content goes here.</p></section>",
            ),
            (
                "Two columns",
                "<div class=\"grid-2\"><div class=\"card\">One</div><div class=\"card\">Two</div></div>",
            ),
        ],
    ),
];

/// Every part, with its kind.
#[cfg(test)]
pub(crate) fn all() -> impl Iterator<Item = (&'static str, &'static str, &'static str)> {
    PARTS
        .iter()
        .flat_map(|(kind, parts)| parts.iter().map(move |(n, h)| (*kind, *n, *h)))
}

/// The classes `html` uses that the kit's CSS doesn't define.
#[cfg(test)]
pub(crate) fn missing_classes(html: &str) -> Vec<String> {
    let css = format!(
        "{}{}",
        include_str!("assets/components.css"),
        include_str!("assets/icons.css")
    );
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(i) = rest.find("class=\"") {
        let after = &rest[i + 7..];
        let end = after.find('"').unwrap_or(after.len());
        for c in after[..end].split_whitespace() {
            let known = [" ", ",", ":", ".", " {", "{", "::"]
                .iter()
                .any(|t| css.contains(&format!(".{c}{t}")));
            if !known && !out.contains(&c.to_string()) {
                out.push(c.to_string());
            }
        }
        rest = &after[end..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_part_uses_classes_the_kit_has() {
        let mut names = std::collections::HashSet::new();
        for (kind, name, html) in all() {
            assert!(names.insert(name), "{name} is offered twice");
            assert_eq!(
                missing_classes(html),
                Vec::<String>::new(),
                "{kind} / {name}"
            );
        }
    }
}
