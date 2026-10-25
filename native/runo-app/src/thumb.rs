//! Pictures attached to messages, shown small in the chat. A thumbnail's size comes from
//! the picture's header before the picture itself loads, so the chat lays its row out at
//! the right height at once instead of the picture spilling over its neighbours later.

use std::cell::RefCell;
use std::collections::HashMap;

/// Neither side of a thumbnail is shown smaller than this, so a very tall or very wide
/// picture stays visible, cropped to fit.
const MIN_SIDE: f32 = 32.;

thread_local! {
    /// Picture sizes read so far, by path; `None` when the header couldn't be read.
    static SIZES: RefCell<HashMap<String, Option<(u32, u32)>>> = RefCell::new(HashMap::new());
}

/// Width and height in pixels of the picture at `path`, from its header (PNG and JPEG).
/// Read once per path.
pub(crate) fn dimensions(path: &str) -> Option<(u32, u32)> {
    if let Some(known) = SIZES.with_borrow(|s| s.get(path).copied()) {
        return known;
    }
    let size = image::ImageReader::open(path)
        .ok()
        .and_then(|r| r.with_guessed_format().ok())
        .and_then(|r| r.into_dimensions().ok())
        .filter(|(w, h)| *w > 0 && *h > 0);
    SIZES.with_borrow_mut(|s| s.insert(path.to_string(), size));
    size
}

/// The size a picture of `dims` is shown at within `max_w` × `max_h`: never larger than
/// the picture, its shape kept, and each side at least `MIN_SIDE`. A picture of unknown
/// size gets a square as tall as allowed.
pub(crate) fn fit(dims: Option<(u32, u32)>, max_w: f32, max_h: f32) -> (f32, f32) {
    let Some((w, h)) = dims.filter(|(w, h)| *w > 0 && *h > 0) else {
        let side = max_w.min(max_h);
        return (side, side);
    };
    let (w, h) = (w as f32, h as f32);
    let scale = (max_w / w).min(max_h / h).min(1.);
    let side = |v: f32, max: f32| (v * scale).round().max(MIN_SIDE.min(max)).min(max);
    (side(w, max_w), side(h, max_h))
}

/// `text` with attachment paths cut to the file name people chose, for step lines such as
/// "Read .runo/attachments/20261025-173501495-Screenshot.png" → "Read Screenshot.png".
pub(crate) fn short_paths(text: &str) -> std::borrow::Cow<'_, str> {
    let marker = format!("{}/", crate::turns::ATTACHMENTS);
    let Some(at) = text.find(&marker) else {
        return text.into();
    };
    // The path starts after the last space before the folder, or at the start.
    let start = text[..at].rfind(char::is_whitespace).map_or(0, |i| i + 1);
    let rest = &text[at + marker.len()..];
    let mut out = String::with_capacity(text.len());
    out.push_str(&text[..start]);
    out.push_str(without_stamp(rest));
    out.into()
}

/// `name` without the "20261025-173501495-" Runo puts before an attachment's name.
pub(crate) fn without_stamp(name: &str) -> &str {
    let b = name.as_bytes();
    let stamped = b.len() > 19
        && b[..8].iter().all(u8::is_ascii_digit)
        && b[8] == b'-'
        && b[9..18].iter().all(u8::is_ascii_digit)
        && b[18] == b'-';
    if stamped {
        &name[19..]
    } else {
        name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wide_screenshot_fits_the_width() {
        assert_eq!(fit(Some((2880, 1800)), 220., 160.), (220., 138.));
    }

    #[test]
    fn a_tall_screenshot_fits_the_height() {
        assert_eq!(fit(Some((718, 870)), 220., 160.), (132., 160.));
    }

    #[test]
    fn a_small_picture_isnt_enlarged() {
        assert_eq!(fit(Some((100, 50)), 220., 160.), (100., 50.));
    }

    #[test]
    fn very_long_pictures_keep_a_visible_side() {
        assert_eq!(fit(Some((100, 20000)), 220., 160.), (32., 160.));
        assert_eq!(fit(Some((20000, 100)), 220., 160.), (220., 32.));
    }

    #[test]
    fn an_unknown_size_is_a_square() {
        assert_eq!(fit(None, 220., 160.), (160., 160.));
        assert_eq!(fit(Some((0, 10)), 220., 160.), (160., 160.));
    }

    #[test]
    fn a_missing_file_has_no_size() {
        assert_eq!(dimensions("/nonexistent/picture.png"), None);
    }

    #[test]
    fn a_png_header_gives_its_size() {
        let dir = std::env::temp_dir().join(format!("runo-thumb-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("p.png");
        image::RgbImage::new(7, 3).save(&path).unwrap();
        assert_eq!(dimensions(path.to_str().unwrap()), Some((7, 3)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn step_lines_show_the_attachment_name() {
        assert_eq!(
            short_paths(
                "Read .runo/attachments/20261025-173501495-Screenshot 2026-10-25 at 5.34.56 PM.png"
            ),
            "Read Screenshot 2026-10-25 at 5.34.56 PM.png"
        );
        assert_eq!(
            short_paths("Read /Users/a/p/.runo/attachments/paste-20261025-173501495.png"),
            "Read paste-20261025-173501495.png"
        );
        assert_eq!(short_paths("Read src/main.rs"), "Read src/main.rs");
    }

    #[test]
    fn only_runo_stamps_are_cut() {
        assert_eq!(without_stamp("20261025-173501495-a.png"), "a.png");
        assert_eq!(without_stamp("2026-report.png"), "2026-report.png");
        assert_eq!(without_stamp("20261025-173501495-"), "20261025-173501495-");
    }
}
