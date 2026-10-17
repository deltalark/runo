//! What an agent is told when it designs: how the design folder works, what the design
//! system offers, how to design well, and the tasks the canvas's buttons ask for.
//!
//! The context is split into paragraphs on blank lines, so a later ask in the same chat
//! repeats only the ones that changed (see `shell::lean_context`).

use super::checks::{self, Finding};
use super::model::{Device, COMPONENTS_CSS, FOLDER, ICONS_CSS};

/// The states "Add states" designs, as the suffix of each new screen's id.
pub(crate) const STATES: [&str; 3] = ["empty", "loading", "error"];

/// What the canvas shows, for the context of an ask.
#[derive(Default)]
pub(crate) struct Canvas<'a> {
    /// Each screen's name, id and device.
    pub screens: Vec<(&'a str, &'a str, Device)>,
    /// The selected screen's name and id.
    pub selected: Option<(&'a str, &'a str)>,
    /// The selected element, said the way the agent can find it.
    pub element: Option<String>,
    /// The project's design rules file, when it says something.
    pub rules: Option<String>,
    /// `components.css` and `tokens.css` as they are now.
    pub components: &'a str,
    pub tokens: &'a str,
    /// A picture of the selected screen as it looks now, attached to the ask.
    pub picture: Option<String>,
    /// More selected elements than `element`, said the same way.
    pub also: Vec<String>,
    /// The components there are to reuse: id, name and the names of its variants.
    pub library: Vec<(String, String, Vec<String>)>,
    /// How the prototype's screens link up: from one screen id to another.
    pub flow: Vec<(String, String)>,
    /// The comments people left that are still open, one line each.
    pub comments: Vec<String>,
}

/// The most open comments an ask carries.
const MOST_COMMENTS: usize = 12;

/// Everything an agent needs to change the design.
pub(crate) fn context(c: &Canvas) -> String {
    let mut parts: Vec<String> = vec![format!(
        "## Change the design\n\
         The design is in {FOLDER}/. Each screen is a standalone HTML file, \
         {FOLDER}/screens/<id>.html, that links ../tokens.css, ../components.css and \
         ../prototype.js (copy the <head> of an existing screen). Name a screen with <title>, \
         set its device with <meta name=\"runo:device\" content=\"phone\"> (or tablet, \
         desktop), and link screens for the prototype with data-goto=\"<other screen id>\" on \
         what's clicked (add data-transition=\"sheet\" for a screen that opens as a modal \
         sheet; push, fade, axis and none also work, and the platform's own is used \
         otherwise; data-goto=\"back\" goes back to the screen before). Pictures people \
         added are in {FOLDER}/assets/, used as ../assets/<file>. Make controls work when tapped in Play: a .switch flips its .on class, a \
         .tab, or any child of an element with data-select, becomes the one with .on, \
         data-toggle=\"class\" toggles that class on the element, data-show=\"#id\" shows or \
         hides another element (menus, accordions, sheets), and checkboxes, radios and inputs \
         work as they are. Reusable pieces are components in {FOLDER}/components/<id>.html, \
         one <template data-variant=\"…\"> per variant; an element marked \
         data-component=\"<id>\" data-variant=\"…\" on a screen is an instance of one, so \
         build with them, keep the marks, and when you change a component's file, change its \
         instances on every screen the same way, keeping their own text. People's comments \
         on the screens are in {FOLDER}/comments.toml, each pinned at x, y on a screen. \
         data-name is a layer's name and data-locked marks a layer people locked: keep both, \
         and leave locked layers as they are unless asked. Text styles are classes: the kit's \
         (display, big, small, tiny, eyebrow) and any listed after \"Text styles made in \
         Design\" in components.css; use them for type rather than setting sizes. \
         New screen files show up on the canvas by themselves. Make the \
         change by editing these files directly: Runo shows it live and the user keeps or \
         undoes it, so don't ask before editing. Don't change files outside {FOLDER}/ for this \
         request."
    )];
    parts.push(system(c.components));
    parts.push(CRAFT.to_string());
    parts.push(FINISH.to_string());
    if c.screens.is_empty() {
        let mut p = String::from("There are no screens yet.");
        if super::model::var(c.tokens, "--primary").as_deref() == Some("#2563eb") {
            p.push_str(
                " The theme is still Runo's default. If this project already has an app or a \
                 site, first look through its code for its brand (a Tailwind config, CSS \
                 variables, theme files, the logo's colours, the fonts it loads) and set \
                 tokens.css to match, so the design looks like this product. If it has none, \
                 choose a primary colour and font that suit what's being designed.",
            );
        }
        parts.push(p);
    } else {
        let mut p = String::from("Screens now:");
        for (name, id, device) in &c.screens {
            p.push_str(&format!(
                "\n- {name} ({FOLDER}/screens/{id}.html, {})",
                device.label().to_lowercase()
            ));
        }
        if !c.flow.is_empty() {
            p.push_str("\nThe prototype's links (data-goto):");
            for (from, to) in &c.flow {
                p.push_str(&format!(" {from} → {to};"));
            }
            p.pop();
            p.push('.');
        }
        parts.push(p);
    }
    if !c.library.is_empty() {
        let mut p = String::from(
            "Components to reuse rather than rebuild. Put one on a screen by copying a \
             variant's element from its file as it is, marks and all, then change its text:",
        );
        for (id, name, variants) in &c.library {
            p.push_str(&format!(
                "\n- {name} ({FOLDER}/components/{id}.html): {}",
                variants.join(", ")
            ));
        }
        parts.push(p);
    }
    if !c.comments.is_empty() {
        let mut p = String::from(
            "Open comments people left on the design. Bear them in mind; address them when \
             the request is about them:",
        );
        for line in c.comments.iter().take(MOST_COMMENTS) {
            p.push_str(&format!("\n- {line}"));
        }
        if c.comments.len() > MOST_COMMENTS {
            p.push_str(&format!(
                "\n- and {} more in {FOLDER}/comments.toml",
                c.comments.len() - MOST_COMMENTS
            ));
        }
        parts.push(p);
    }
    match c.selected {
        Some((name, id)) => {
            let mut p = format!(
                "Selected on the canvas (\"this\" means it): the screen {name} \
                 ({FOLDER}/screens/{id}.html)"
            );
            match (&c.element, c.also.is_empty()) {
                (Some(el), true) => p.push_str(&format!(", and on it the element {el}")),
                (Some(el), false) => {
                    p.push_str(&format!(
                        ", and on it {} elements:\n- {el}",
                        c.also.len() + 1
                    ));
                    for more in &c.also {
                        p.push_str(&format!("\n- {more}"));
                    }
                }
                (None, _) => {}
            }
            if !(c.element.is_some() && !c.also.is_empty()) {
                p.push('.');
            }
            if c.element.is_some() && c.also.is_empty() {
                p.push_str(
                    "\nChange only that element unless asked otherwise. If what's asked for \
                     isn't on the screen yet, add it inside that element when it's a \
                     container, else right after it.",
                );
            } else if c.element.is_some() {
                p.push_str(
                    "\nChange only those elements unless asked otherwise, treating them as \
                     one group: make them consistent with each other.",
                );
            }
            if let Some(pic) = &c.picture {
                p.push_str(&format!(
                    "\nA picture of how the screen looks now is attached ({pic}): look at it \
                     before changing the file."
                ));
            }
            parts.push(p);
        }
        None => parts.push("Nothing is selected on the canvas.".into()),
    }
    if let Some(file) = &c.rules {
        parts.push(format!(
            "Follow the project's design rules in {file}: who it's for, the look and feel, and \
             its rules for spacing, type and colour. They win over the general advice above."
        ));
    }
    let mut out = parts.join("\n\n");
    out.push('\n');
    out
}

/// What the design system offers: the components, the tokens and the icons.
fn system(components: &str) -> String {
    let ours = components == COMPONENTS_CSS || components.contains(".segmented");
    let mut out = String::from("## The design system\n");
    if ours {
        out.push_str(CATALOG);
    } else {
        out.push_str(
            "components.css has this project's own classes: read it before designing and use \
             what it has. Icons are in icons.css; a screen that uses them links \
             ../icons.css in its <head> unless components.css imports it.",
        );
    }
    out.push_str(&format!(
        "\nIcons: <i class=\"icon i-NAME\"></i> takes the text colour and size (.icon-sm, \
         .icon-lg; .icon-tile sets one on a tinted square). NAME is one of: {}. Use the \
         closest one; inline a 24×24 stroke SVG in the same style only when none fits. Never \
         use emoji as icons.",
        icon_names().join(", ")
    ));
    out
}

/// The icons `icons.css` has.
pub(crate) fn icon_names() -> Vec<&'static str> {
    ICONS_CSS
        .lines()
        .filter_map(|l| l.strip_prefix(".i-")?.split_once(' ').map(|(n, _)| n))
        .collect()
}

/// The classes and tokens of Runo's components.css, by what they're for.
const CATALOG: &str = "\
Build from the classes in components.css and the variables in tokens.css rather than raw \
colours and sizes, so the theme keeps working. A screen can add its own styles in a <style> \
block, using the variables; move anything two screens share into components.css.
- Layout: .screen (phone padding), .page (centred desktop column), .stack, .cluster \
(wrapping row), .between, .grow, .gap-xs/-sm/-md/-lg/-xl, .grid (auto-fill), .grid-2/-3/-4, \
.scroll-x (sideways carousel), .section with .section-head.
- Frames: .bar (top bar; .plain, .bar-title); phone: .tabbar of .tab (icon over label, .on); \
desktop: .app holding .sidebar (.brand, .nav-item with .on and .count) and .main.
- Text: h1-h4, .display, .big, .eyebrow, .muted, .small, .tiny, .strong, .num (tabular \
figures), .truncate, .primary-text, .success-text, .warning-text, .danger-text.
- Actions: .button with .primary, .secondary, .ghost or .danger, and .sm, .lg or .block; \
.icon-button (.filled); .fab; .link.
- Forms: .field (a label holding a span, the input and a .hint; .error), .input, .search-box \
(an icon then input.search), .check, .switch (.on), .segmented (children, one .on), .chip \
(.on).
- Content: .card (.raised, .outline, .flat, .accent), .list of .row, .divider, .table, .stat \
(label, .value, .delta.up or .down), .progress > span (style its width), .badge (.success, \
.warning, .danger, .neutral), .dot, .avatar (initials; .sm, .lg), .avatars (overlapping).
- Pictures (there are no photos, so these stand in): .image (.square, .tall; tints .warm, \
.cool, .green, .dusk), .hero, .illustration with an icon inside.
- Overlays and states: .overlay holding a .sheet or .modal, .toast, .callout (.warning, \
.danger), .empty, .skeleton (give it a size), .spinner.
- Tokens: colours --primary, --on-primary, --primary-soft, --bg, --surface, --surface-2, \
--text, --muted, --border, --success, --warning, --danger; type --text-xs, -sm, -md, -lg, \
-xl, -2xl, -3xl, --font, --font-display; spacing on an 8px grid, calc(var(--space) * n); \
--radius, --radius-sm, --radius-lg, --radius-full; --shadow-sm, --shadow, --shadow-lg.";

/// How to design well, the way a strong product designer would.
const CRAFT: &str = "\
## How to design well
Aim for a screen a top product team would ship, not a template or a wireframe.
- Purpose first: decide the one thing each screen is for and make it the most prominent \
thing on it. One primary button per screen; other actions are secondary, ghost or icons.
- Hierarchy comes from size, weight and colour, not from boxes around everything. Use the \
type scale, two or three sizes per area, and muted text for what's secondary.
- Spacing follows the 8px grid: related things sit close together and groups are clearly \
apart. Line things up on a few shared edges and keep padding the same on every card and row.
- Colour with restraint: neutrals for most of the screen, the primary colour for the main \
action and the current selection, status colours only for status.
- Real content: names, numbers, dates, prices and copy that fit the product and its users, \
with varied lengths. Never lorem ipsum, \"Item 1\", \"Title\" or \"Description here\".
- Icons beside labels in navigation, rows and buttons where they help people scan, the same \
size throughout.
- Fit the device: a phone is 390px wide, one column, with actions in thumb reach, a bottom \
tab bar for top-level places and 44px tap targets. A desktop uses its width: a sidebar or \
top navigation, several columns, tables for dense data and a sensible maximum line length.
- When asked for an app or a flow rather than one screen, design its key screens (usually \
three to six), each in its own file, linked with data-goto in the order people move through \
them.";

/// What to check before finishing.
const FINISH: &str = "\
## Before you finish
Read each screen you changed from top to bottom as its user would. Everything that can be \
tapped looks tappable, the selected tab or item is shown, nothing overflows or is cut off, \
every data-goto names a screen that exists, and the screens look like one product. Then say \
in a line or two what you made or changed.";

/// The task for AI to design a new screen from a description.
pub(crate) fn describe_task(what: &str, device: Device) -> String {
    let device = device.label().to_lowercase();
    format!(
        "Design a new {device} screen: {what}\n\nAdd it as a new file in {FOLDER}/screens/ with \
         a short id, a <title> naming it and <meta name=\"runo:device\" content=\"{device}\">. \
         Make it complete and realistic, finished to the standard in \"How to design well\". \
         If this describes an app or a flow rather than one screen, design its key screens, \
         each its own file. Link it to the screens it naturally leads to and from with \
         data-goto where that fits."
    )
}

/// The task for AI to make variants of a screen.
pub(crate) fn variants_task(id: &str, name: &str) -> String {
    format!(
        "Make three alternative designs of the screen {name} ({FOLDER}/screens/{id}.html) as \
         new screens {FOLDER}/screens/{id}-v1.html, {id}-v2.html and {id}-v3.html. Keep its \
         purpose, content and links, but make each a clearly different take: a different \
         layout, hierarchy or density, not just other colours. Finish each one as fully as a \
         real screen. Title each \"{name} · variant N\". Don't change {id}.html itself; Runo \
         shows the variants under it to pick from."
    )
}

/// The task for AI to design a screen's empty, loading and error states.
pub(crate) fn states_task(id: &str, name: &str) -> String {
    format!(
        "Design the other states of the screen {name} ({FOLDER}/screens/{id}.html) as new \
         screens beside it, each keeping its frame, navigation and device:\n\
         - {id}-empty.html, titled \"{name} · empty\": nothing here yet. Say why in a friendly \
         line and offer the one action that fixes it.\n\
         - {id}-loading.html, titled \"{name} · loading\": skeletons in the shape of the real \
         content, not a lone spinner.\n\
         - {id}-error.html, titled \"{name} · error\": what went wrong in plain words and a \
         way to try again.\n\
         Don't change {id}.html itself."
    )
}

/// The task for AI to make a version of a screen for another device.
pub(crate) fn adapt_task(id: &str, name: &str, from: Device, to: Device) -> String {
    let (from, to_label) = (from.label().to_lowercase(), to.label().to_lowercase());
    let how = match to {
        Device::Desktop => {
            "Use the width: a sidebar or top navigation in place of a bottom tab bar, \
             several columns, content side by side rather than stacked, tables where \
             there are lists of data, and a sensible line length."
        }
        Device::Tablet => {
            "Use the extra room: two columns or a list beside its detail where that \
             fits, larger touch targets kept, navigation in a sidebar or a top bar."
        }
        Device::Phone => {
            "Fit one column 390px wide: stack what sat side by side, a bottom tab bar \
             for top-level places, actions in thumb reach and 44px tap targets."
        }
    };
    format!(
        "Make a {to_label} version of the {from} screen {name} ({FOLDER}/screens/{id}.html) \
         as a new screen {FOLDER}/screens/{id}-{to_label}.html, titled \"{name} · \
         {to_label}\" with <meta name=\"runo:device\" content=\"{to_label}\">. Keep its \
         purpose, content, components and links, but lay it out the way people expect on a \
         {to_label}, not a stretched copy. {how} Don't change {id}.html itself."
    )
}

/// The task for AI to add the usual interaction states to a component as variants.
pub(crate) fn component_states_task(id: &str, name: &str, have: &[String]) -> String {
    format!(
        "Add variants to the component {name} ({FOLDER}/components/{id}.html), which has \
         {}. Add the states it needs that it doesn't have yet, each a new <template \
         data-variant=\"…\"> holding the same element: for something tapped, Hover, Pressed \
         and Disabled; for a field, Focused, Filled, Error and Disabled; for a row, card or \
         item, Selected and Disabled. Keep the root element's data-component=\"{id}\" and \
         set its data-variant to the variant's name. Make each state clearly different using \
         the theme's tokens, keep the same size and structure so instances can switch, and \
         don't change the existing variants or any screen.",
        have.join(", ")
    )
}

/// The screen `id` is a state of: `inbox-empty` is a state of `inbox`, and which one.
pub(crate) fn state_of(id: &str) -> Option<(String, usize)> {
    STATES.iter().enumerate().find_map(|(i, s)| {
        let base = id.strip_suffix(s)?.strip_suffix('-')?;
        (!base.is_empty()).then(|| (base.to_string(), i + 1))
    })
}

/// One screen to look at again after an agent changed it.
pub(crate) struct Looked {
    pub id: String,
    pub name: String,
    /// Where its picture is, when it could be drawn.
    pub picture: Option<String>,
    pub findings: Vec<Finding>,
}

/// The task for the agent to look at what it made and polish it: the pictures of the
/// screens it changed and what Runo's checks found on them.
pub(crate) fn review_task(asked: &str, screens: &[Looked]) -> String {
    let mut out = format!(
        "## Check your work\n\
         Here is how the screens you just changed look as Runo draws them. Look at each \
         picture closely, as a demanding design lead would before it ships, against what was \
         asked: \"{}\".\n",
        asked.trim()
    );
    for s in screens {
        match &s.picture {
            Some(p) => out.push_str(&format!(
                "- {} ({FOLDER}/screens/{}.html): {p}\n",
                s.name, s.id
            )),
            None => out.push_str(&format!("- {} ({FOLDER}/screens/{}.html)\n", s.name, s.id)),
        }
    }
    out.push_str(
        "\nLook for anything that doesn't do what was asked; no clear main thing, or too many \
         competing sizes and colours; uneven spacing and edges that don't line up; text that's \
         cramped, cut off, overflowing or too faint; placeholder or repetitive content; empty \
         or lopsided areas; parts that look like a wireframe rather than a finished product; \
         and screens that don't look like one product.\n",
    );
    let found: Vec<(String, String, Vec<Finding>)> = screens
        .iter()
        .filter(|s| !s.findings.is_empty())
        .map(|s| (s.id.clone(), s.name.clone(), s.findings.clone()))
        .collect();
    if !found.is_empty() {
        out.push_str(
            "\nRuno's design checks also found these (elements are CSS paths from <body>):\n",
        );
        out.push_str(&checks::listed(&found));
    }
    out.push_str(
        "\nFix what you find by editing the files. Make real improvements rather than churn: \
         leave a screen that's already right as it is. End with one line on what you \
         improved, or that it already looked right.",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn the_icons_are_listed_from_the_stylesheet() {
        let names = icon_names();
        assert!(names.len() > 100);
        assert!(names.contains(&"search") && names.contains(&"chevron-right"));
        assert!(names.iter().all(|n| !n.is_empty() && !n.contains(' ')));
    }

    #[test]
    fn a_new_design_is_told_to_find_the_brand_and_how_to_design() {
        let c = Canvas {
            components: COMPONENTS_CSS,
            tokens: super::super::model::TOKENS_CSS,
            ..Default::default()
        };
        let text = context(&c);
        assert!(text.contains("## How to design well"));
        assert!(text.contains(".segmented"));
        assert!(text.contains("one of: search, plus,"));
        assert!(text.contains("look through its code for its brand"));
        // Its own theme: no hunting for the brand.
        let c = Canvas {
            tokens: ":root { --primary: #db2777; }",
            components: "/* mine */",
            ..c
        };
        let text = context(&c);
        assert!(!text.contains("for its brand"));
        assert!(text.contains("this project's own classes"));
    }

    #[test]
    fn a_selection_and_its_picture_are_described() {
        let c = Canvas {
            screens: vec![("Inbox", "inbox", Device::Phone)],
            selected: Some(("Inbox", "inbox")),
            element: Some("button \"Send\" at `body > :nth-child(2)`".into()),
            picture: Some("/tmp/inbox.png".into()),
            components: COMPONENTS_CSS,
            tokens: "",
            ..Default::default()
        };
        let text = context(&c);
        assert!(text.contains("- Inbox (design/screens/inbox.html, phone)"));
        assert!(text.contains("the element button \"Send\""));
        assert!(text.contains("Change only that element"));
        assert!(text.contains("attached (/tmp/inbox.png)"));
        // Parts are split on blank lines, so a later ask repeats only what changed.
        let rest = context(&Canvas {
            selected: None,
            element: None,
            picture: None,
            ..c
        });
        let lean = crate::shell::lean_context(&text, &rest);
        assert!(lean.contains("Nothing is selected"));
        assert!(!lean.contains("How to design well"));
    }

    #[test]
    fn the_library_flow_comments_and_a_group_are_described() {
        let c = Canvas {
            screens: vec![
                ("Inbox", "inbox", Device::Phone),
                ("Mail", "mail", Device::Phone),
            ],
            selected: Some(("Inbox", "inbox")),
            element: Some("div.row at `body > :nth-child(1)`".into()),
            also: vec!["div.row at `body > :nth-child(2)`".into()],
            library: vec![(
                "row".into(),
                "Setting row".into(),
                vec!["Default".into(), "Off".into()],
            )],
            flow: vec![("inbox".into(), "mail".into())],
            comments: (0..14).map(|i| format!("comment {i}")).collect(),
            components: COMPONENTS_CSS,
            ..Default::default()
        };
        let text = context(&c);
        assert!(text.contains("links (data-goto): inbox → mail."));
        assert!(text.contains("- Setting row (design/components/row.html): Default, Off"));
        assert!(text.contains("2 elements:\n- div.row at `body > :nth-child(1)`\n- div.row"));
        assert!(text.contains("treating them as one group"));
        assert!(text.contains("- comment 11\n- and 2 more"));
        assert!(!text.contains("comment 12"));
        let t = adapt_task("inbox", "Inbox", Device::Phone, Device::Desktop);
        assert!(t.contains("design/screens/inbox-desktop.html"));
        assert!(t.contains("sidebar or top navigation"));
        let t = component_states_task("row", "Setting row", &["Default".into()]);
        assert!(t.contains("which has Default"));
    }

    #[test]
    fn states_sit_beside_their_screen() {
        assert_eq!(state_of("inbox-empty"), Some(("inbox".into(), 1)));
        assert_eq!(state_of("inbox-error"), Some(("inbox".into(), 3)));
        assert_eq!(state_of("empty"), None);
        assert_eq!(state_of("inbox"), None);
        assert!(states_task("inbox", "Inbox").contains("inbox-loading.html"));
    }

    #[test]
    fn a_review_shows_the_pictures_and_what_the_checks_found() {
        let task = review_task(
            "A calmer inbox",
            &[
                Looked {
                    id: "inbox".into(),
                    name: "Inbox".into(),
                    picture: Some("/p/inbox.png".into()),
                    findings: vec![Finding {
                        kind: checks::Kind::Contrast,
                        path: Some(vec![0, 1]),
                        rect: None,
                        text: "\u{201c}Sent\u{201d} is hard to read".into(),
                    }],
                },
                Looked {
                    id: "detail".into(),
                    name: "Detail".into(),
                    picture: None,
                    findings: vec![],
                },
            ],
        );
        assert!(task.contains("\"A calmer inbox\""));
        assert!(task.contains("- Inbox (design/screens/inbox.html): /p/inbox.png"));
        assert!(task.contains("- Detail (design/screens/detail.html)\n"));
        assert!(task.contains("is hard to read at `body > :nth-child(1) > :nth-child(2)`"));
    }
}
