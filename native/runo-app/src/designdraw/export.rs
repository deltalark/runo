//! Exporting a screen, or part of one, as a picture or as a page that stands on its own.

use super::dom;
use std::path::{Path, PathBuf};

/// The formats a selection exports to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Format {
    #[default]
    Png,
    Jpg,
    Html,
    React,
    SwiftUi,
}

impl Format {
    pub(crate) const ALL: [Format; 5] = [
        Format::Png,
        Format::Jpg,
        Format::Html,
        Format::React,
        Format::SwiftUi,
    ];

    /// Whether it's a picture, drawn at a scale.
    pub(crate) fn picture(self) -> bool {
        matches!(self, Format::Png | Format::Jpg)
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Format::Png => "PNG",
            Format::Jpg => "JPG",
            Format::Html => "HTML",
            Format::React => "React",
            Format::SwiftUi => "SwiftUI",
        }
    }

    pub(crate) fn ext(self) -> &'static str {
        match self {
            Format::Png => "png",
            Format::Jpg => "jpg",
            Format::Html => "html",
            Format::React => "tsx",
            Format::SwiftUi => "swift",
        }
    }
}

/// Where exports go in the project.
pub(crate) fn folder(root: &Path) -> PathBuf {
    super::model::folder(root).join("exports")
}

/// The file name for an export: the screen, the part of it if any, and the scale.
pub(crate) fn file_name(screen: &str, part: Option<&str>, scale: u32, format: Format) -> String {
    let base = match part.map(super::model::slug).filter(|p| !p.is_empty()) {
        Some(p) => format!("{screen}-{p}"),
        None => screen.to_string(),
    };
    match format {
        Format::Html => format!("{base}.html"),
        Format::React | Format::SwiftUi => format!("{base}.{}", format.ext()),
        _ if scale == 1 => format!("{base}.{}", format.ext()),
        _ => format!("{base}@{scale}x.{}", format.ext()),
    }
}

/// A drawing, kept as BGRA at `density` pixels per CSS pixel, cut to `area` (CSS pixels,
/// or the whole drawing) and scaled to `scale` pixels per CSS pixel.
pub(crate) fn picture(
    bgra: &[u8],
    width: u32,
    height: u32,
    density: f32,
    area: Option<&dom::Box>,
    scale: u32,
) -> Option<image::RgbaImage> {
    let mut rgba = bgra.to_vec();
    for px in rgba.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
    }
    let mut pic = image::RgbaImage::from_raw(width, height, rgba)?;
    if let Some(a) = area {
        let x = ((a.x * density).round().max(0.) as u32).min(width.saturating_sub(1));
        let y = ((a.y * density).round().max(0.) as u32).min(height.saturating_sub(1));
        let w = ((a.w * density).round() as u32).clamp(1, width - x);
        let h = ((a.h * density).round() as u32).clamp(1, height - y);
        pic = image::imageops::crop_imm(&pic, x, y, w, h).to_image();
    }
    let f = scale as f32 / density;
    if (f - 1.).abs() > 0.01 {
        let w = ((pic.width() as f32 * f).round() as u32).max(1);
        let h = ((pic.height() as f32 * f).round() as u32).max(1);
        pic = image::imageops::resize(&pic, w, h, image::imageops::FilterType::Lanczos3);
    }
    Some(pic)
}

/// A picture as the bytes of a file of `format`.
pub(crate) fn encode(pic: &image::RgbaImage, format: Format) -> Result<Vec<u8>, String> {
    let mut out = std::io::Cursor::new(Vec::new());
    match format {
        Format::Jpg => image::DynamicImage::ImageRgba8(pic.clone())
            .to_rgb8()
            .write_to(&mut out, image::ImageFormat::Jpeg),
        _ => pic.write_to(&mut out, image::ImageFormat::Png),
    }
    .map_err(|e| e.to_string())?;
    Ok(out.into_inner())
}

/// A screen's page with the design's style sheets written into it, so it opens anywhere.
/// With `body`, that HTML replaces what's in the page's `<body>`.
pub(crate) fn standalone(html: &str, design: &Path, body: Option<&str>) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(i) = rest.find("<link") {
        let Some(end) = rest[i..].find('>') else {
            break;
        };
        let tag = &rest[i..i + end + 1];
        out.push_str(&rest[..i]);
        match sheet_of(tag).and_then(|href| std::fs::read_to_string(design.join(href)).ok()) {
            Some(css) => {
                out.push_str("<style>\n");
                out.push_str(&css);
                out.push_str("\n</style>");
            }
            None => out.push_str(tag),
        }
        rest = &rest[i + end + 1..];
    }
    out.push_str(rest);
    let mut out = with_pictures(&out, design);
    // The prototype's links go nowhere on their own.
    if let Some(i) = out.find("<script src=\"../prototype.js\"") {
        if let Some(end) = out[i..].find("</script>") {
            out.replace_range(i..i + end + "</script>".len(), "");
        }
    }
    match body {
        Some(b) => match (out.find("<body"), out.rfind("</body>")) {
            (Some(s), Some(e)) => match out[s..].find('>') {
                Some(open) if s + open < e => {
                    format!("{}\n{b}\n{}", &out[..s + open + 1], &out[e..])
                }
                _ => out,
            },
            _ => out,
        },
        None => out,
    }
}

/// `html` with the pictures it uses from the design's assets written into it.
fn with_pictures(html: &str, design: &Path) -> String {
    use base64::Engine;
    const AT: &str = "../assets/";
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(i) = rest.find(AT) {
        out.push_str(&rest[..i]);
        let name_len = rest[i + AT.len()..]
            .find(|c: char| matches!(c, '"' | '\'' | ')' | '&' | '<') || c.is_whitespace())
            .unwrap_or(rest.len() - i - AT.len());
        let name = &rest[i + AT.len()..i + AT.len() + name_len];
        let kind = match name
            .rsplit('.')
            .next()
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("jpg" | "jpeg") => "image/jpeg",
            Some("gif") => "image/gif",
            Some("webp") => "image/webp",
            Some("svg") => "image/svg+xml",
            _ => "image/png",
        };
        match std::fs::read(design.join("assets").join(name)) {
            Ok(bytes) if !name.contains('/') => out.push_str(&format!(
                "data:{kind};base64,{}",
                base64::engine::general_purpose::STANDARD.encode(bytes)
            )),
            _ => out.push_str(&rest[i..i + AT.len() + name_len]),
        }
        rest = &rest[i + AT.len() + name_len..];
    }
    out.push_str(rest);
    out
}

/// The design file a `<link rel="stylesheet">` points at, when it's one of the design's
/// own: `../tokens.css` gives `tokens.css`.
fn sheet_of(tag: &str) -> Option<&str> {
    if !tag.contains("stylesheet") {
        return None;
    }
    let start = tag.find("href=\"")? + 6;
    let href = &tag[start..start + tag[start..].find('"')?];
    let name = href.strip_prefix("../")?;
    (!name.contains('/') && name.ends_with(".css")).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_go_into_a_standalone_page() {
        let dir = std::env::temp_dir().join(format!("runo-export-pics-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::write(dir.join("assets/cat.png"), b"png").unwrap();
        let html = "<div style=\"background-image: url(&quot;../assets/cat.png&quot;)\"></div><img src=\"../assets/gone.jpg\">";
        let out = standalone(html, &dir, None);
        assert!(out.contains("url(&quot;data:image/png;base64,cG5n&quot;)"));
        assert!(out.contains("src=\"../assets/gone.jpg\""));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn exports_are_named_for_what_and_how_big() {
        assert_eq!(file_name("home", None, 2, Format::Png), "home@2x.png");
        assert_eq!(file_name("home", None, 1, Format::Jpg), "home.jpg");
        assert_eq!(
            file_name("home", Some("button.primary"), 3, Format::Png),
            "home-button-primary@3x.png"
        );
        assert_eq!(file_name("home", None, 2, Format::Html), "home.html");
    }

    #[test]
    fn pictures_are_cut_and_scaled() {
        // A 4 × 2 drawing at 2 pixels per CSS pixel: left half blue, right half red (BGRA).
        let mut bgra = Vec::new();
        for _y in 0..2 {
            for x in 0..4 {
                bgra.extend_from_slice(if x < 2 {
                    &[255, 0, 0, 255]
                } else {
                    &[0, 0, 255, 255]
                });
            }
        }
        let whole = picture(&bgra, 4, 2, 2., None, 1).unwrap();
        assert_eq!(whole.dimensions(), (2, 1));
        let area = dom::Box {
            x: 1.,
            y: 0.,
            w: 1.,
            h: 1.,
        };
        let right = picture(&bgra, 4, 2, 2., Some(&area), 2).unwrap();
        assert_eq!(right.dimensions(), (2, 2));
        assert_eq!(right.get_pixel(0, 0).0, [255, 0, 0, 255]);
        let png = encode(&right, Format::Png).unwrap();
        assert!(png.starts_with(&[0x89, b'P', b'N', b'G']));
        let jpg = encode(&right, Format::Jpg).unwrap();
        assert!(jpg.starts_with(&[0xff, 0xd8]));
    }

    #[test]
    fn a_standalone_page_carries_its_styles() {
        let dir = std::env::temp_dir().join(format!("runo-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("tokens.css"), ":root { --primary: red; }").unwrap();
        let html = "<html><head><link rel=\"stylesheet\" href=\"../tokens.css\">\
                    <link rel=\"icon\" href=\"x.png\"></head>\
                    <body><p>Hi</p><script src=\"../prototype.js\" defer=\"\"></script></body></html>";
        let out = standalone(html, &dir, None);
        assert!(out.contains("<style>\n:root { --primary: red; }\n</style>"));
        assert!(out.contains("<link rel=\"icon\" href=\"x.png\">"));
        assert!(!out.contains("prototype.js"));
        let part = standalone(html, &dir, Some("<button>Go</button>"));
        assert!(part.contains("<body>\n<button>Go</button>\n</body>"));
        assert!(!part.contains("<p>Hi</p>"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
