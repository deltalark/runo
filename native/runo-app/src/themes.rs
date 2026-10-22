//! Colour themes, picked under Settings > Appearance: one for dark mode and one for light
//! mode. Each maps an editor theme onto Runo's colour tokens; the `c_*` functions in
//! `settings.rs` read the one in use.

use std::sync::atomic::{AtomicU8, Ordering};

use crate::settings::mix;

pub struct Palette {
    pub id: &'static str,
    pub name: &'static str,
    /// The page: the editor background in the source theme.
    pub canvas: u32,
    pub sidebar: u32,
    /// The deepest layer, under the mode rail.
    pub rail: u32,
    /// A step off the canvas, for the list beside the rail.
    pub sheet: u32,
    pub subtle: u32,
    pub border: u32,
    /// The selected row.
    pub active: u32,
    /// The composer, menus and cards.
    pub raised: u32,
    pub term_bg: u32,
    pub text: u32,
    pub secondary: u32,
    pub muted: u32,
    pub green: u32,
    pub amber: u32,
    pub red: u32,
    /// The theme's own accent, used while the accent setting is left at its default.
    pub accent: Option<u32>,
    /// Text, secondary and muted in glass mode, when tuned by hand; otherwise derived.
    pub glass: Option<[u32; 3]>,
    /// Terminal colours 0 to 15.
    pub ansi: [u32; 16],
}

impl Palette {
    /// Text levels for glass mode: a step stronger, as `settings::c_text` explains.
    fn glass_level(&self, i: usize, color: u32, toward: f32) -> u32 {
        match self.glass {
            Some(g) => g[i],
            None if crate::settings::is_dark() => mix(0xffffff, color, toward),
            None => mix(0x000000, color, toward),
        }
    }
    pub fn glass_text(&self) -> u32 {
        self.glass_level(0, self.text, 0.45)
    }
    pub fn glass_secondary(&self) -> u32 {
        self.glass_level(1, self.secondary, 0.4)
    }
    pub fn glass_muted(&self) -> u32 {
        self.glass_level(2, self.muted, 0.3)
    }
}

pub const DARK: [Palette; 9] = [
    Palette {
        id: "tokyo-night",
        name: "Tokyo Night (default)",
        canvas: 0x1a1b26,
        sidebar: 0x16161e,
        rail: 0x111118,
        sheet: 0x1f2030,
        subtle: 0x222436,
        border: 0x2a2e42,
        active: 0x2b3047,
        raised: 0x1f2335,
        term_bg: 0x16161e,
        text: 0xc0caf5,
        secondary: 0xa9b1d6,
        muted: 0x737aa2,
        green: 0x9ece6a,
        amber: 0xe0af68,
        red: 0xf7768e,
        accent: Some(0x7aa2f7),
        glass: Some([0xdce2fa, 0xcbd0e6, 0x969bb9]),
        ansi: [
            0x15161e, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xa9b1d6,
            0x414868, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xc0caf5,
        ],
    },
    Palette {
        id: "tokyo-night-storm",
        name: "Tokyo Night Storm",
        canvas: 0x24283b,
        sidebar: 0x1f2335,
        rail: 0x1b1e2e,
        sheet: 0x292e42,
        subtle: 0x2b3048,
        border: 0x3b4261,
        active: 0x363d59,
        raised: 0x292e42,
        term_bg: 0x1f2335,
        text: 0xc0caf5,
        secondary: 0xa9b1d6,
        muted: 0x737aa2,
        green: 0x9ece6a,
        amber: 0xe0af68,
        red: 0xf7768e,
        accent: Some(0x7aa2f7),
        glass: Some([0xdce2fa, 0xcbd0e6, 0x969bb9]),
        ansi: [
            0x1d202f, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xa9b1d6,
            0x414868, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xc0caf5,
        ],
    },
    Palette {
        id: "catppuccin-mocha",
        name: "Catppuccin Mocha",
        canvas: 0x1e1e2e,
        sidebar: 0x181825,
        rail: 0x11111b,
        sheet: 0x282839,
        subtle: 0x2b2c3e,
        border: 0x393a50,
        active: 0x393a52,
        raised: 0x2a2b3d,
        term_bg: 0x181825,
        text: 0xcdd6f4,
        secondary: 0xbac2de,
        muted: 0x7f849c,
        green: 0xa6e3a1,
        amber: 0xf9e2af,
        red: 0xf38ba8,
        accent: Some(0xcba6f7),
        glass: None,
        ansi: [
            0x45475a, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xbac2de,
            0x585b70, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xa6adc8,
        ],
    },
    Palette {
        id: "catppuccin-macchiato",
        name: "Catppuccin Macchiato",
        canvas: 0x24273a,
        sidebar: 0x1e2030,
        rail: 0x181926,
        sheet: 0x2d3045,
        subtle: 0x303349,
        border: 0x40445b,
        active: 0x3f435a,
        raised: 0x2f3247,
        term_bg: 0x1e2030,
        text: 0xcad3f5,
        secondary: 0xb8c0e0,
        muted: 0x8087a2,
        green: 0xa6da95,
        amber: 0xeed49f,
        red: 0xed8796,
        accent: Some(0xc6a0f6),
        glass: None,
        ansi: [
            0x494d64, 0xed8796, 0xa6da95, 0xeed49f, 0x8aadf4, 0xf5bde6, 0x8bd5ca, 0xb8c0e0,
            0x5b6078, 0xed8796, 0xa6da95, 0xeed49f, 0x8aadf4, 0xf5bde6, 0x8bd5ca, 0xa5adcb,
        ],
    },
    Palette {
        id: "catppuccin-frappe",
        name: "Catppuccin Frappé",
        canvas: 0x303446,
        sidebar: 0x292c3c,
        rail: 0x232634,
        sheet: 0x383c50,
        subtle: 0x3b3f53,
        border: 0x4a4f64,
        active: 0x494d63,
        raised: 0x3a3e52,
        term_bg: 0x292c3c,
        text: 0xc6d0f5,
        secondary: 0xb5bfe2,
        muted: 0x838ba7,
        green: 0xa6d189,
        amber: 0xe5c890,
        red: 0xe78284,
        accent: Some(0xca9ee6),
        glass: None,
        ansi: [
            0x51576d, 0xe78284, 0xa6d189, 0xe5c890, 0x8caaee, 0xf4b8e4, 0x81c8be, 0xb5bfe2,
            0x626880, 0xe78284, 0xa6d189, 0xe5c890, 0x8caaee, 0xf4b8e4, 0x81c8be, 0xa5adce,
        ],
    },
    Palette {
        id: "ayu-dark",
        name: "Ayu Dark",
        canvas: 0x0d1017,
        sidebar: 0x0b0e14,
        rail: 0x080a0f,
        sheet: 0x131721,
        subtle: 0x141922,
        border: 0x1f242d,
        active: 0x20262f,
        raised: 0x131721,
        term_bg: 0x0b0e14,
        text: 0xbfbdb6,
        secondary: 0xa6a49d,
        muted: 0x6c7380,
        green: 0xaad94c,
        amber: 0xffb454,
        red: 0xf07178,
        accent: Some(0xe6b450),
        glass: None,
        ansi: [
            0x01060e, 0xea6c73, 0x91b362, 0xf9af4f, 0x53bdfa, 0xfae994, 0x90e1c6, 0xc7c7c7,
            0x686868, 0xf07178, 0xc2d94c, 0xffb454, 0x59c2ff, 0xffee99, 0x95e6cb, 0xffffff,
        ],
    },
    Palette {
        id: "ayu-mirage",
        name: "Ayu Mirage",
        canvas: 0x242936,
        sidebar: 0x1f2430,
        rail: 0x1a1f29,
        sheet: 0x2a2f3c,
        subtle: 0x2c3240,
        border: 0x363c4a,
        active: 0x343b4a,
        raised: 0x2a2f3c,
        term_bg: 0x1f2430,
        text: 0xcccac2,
        secondary: 0xb3b1aa,
        muted: 0x707a8c,
        green: 0xd5ff80,
        amber: 0xffd173,
        red: 0xf28779,
        accent: Some(0xffcc66),
        glass: None,
        ansi: [
            0x191e2a, 0xed8274, 0xa6cc70, 0xfad07b, 0x6dcbfa, 0xcfbafa, 0x90e1c6, 0xc7c7c7,
            0x686868, 0xf28779, 0xbae67e, 0xffd580, 0x73d0ff, 0xd4bfff, 0x95e6cb, 0xffffff,
        ],
    },
    Palette {
        id: "night-owl",
        name: "Night Owl",
        canvas: 0x011627,
        sidebar: 0x01111d,
        rail: 0x010d17,
        sheet: 0x0b2236,
        subtle: 0x0b2942,
        border: 0x163852,
        active: 0x1d3b53,
        raised: 0x0b2942,
        term_bg: 0x01111d,
        text: 0xd6deeb,
        secondary: 0x9fb4c7,
        muted: 0x5f7e97,
        green: 0xc5e478,
        amber: 0xffcb8b,
        red: 0xef5350,
        accent: Some(0x82aaff),
        glass: None,
        ansi: [
            0x011627, 0xef5350, 0x22da6e, 0xaddb67, 0x82aaff, 0xc792ea, 0x21c7a8, 0xffffff,
            0x575656, 0xef5350, 0x22da6e, 0xffeb95, 0x82aaff, 0xc792ea, 0x7fdbca, 0xffffff,
        ],
    },
    Palette {
        id: "one-dark",
        name: "One Dark",
        canvas: 0x282c34,
        sidebar: 0x21252b,
        rail: 0x1b1d23,
        sheet: 0x30353f,
        subtle: 0x323842,
        border: 0x3e4451,
        active: 0x3a404b,
        raised: 0x30353f,
        term_bg: 0x21252b,
        text: 0xdcdfe4,
        secondary: 0xabb2bf,
        muted: 0x7f848e,
        green: 0x98c379,
        amber: 0xe5c07b,
        red: 0xe06c75,
        accent: None,
        glass: Some([0xf0f2f5, 0xd0d5dd, 0xa8aeb9]),
        ansi: [
            0x1e1e22, 0xf87171, 0x4ade80, 0xfacc15, 0x60a5fa, 0xc084fc, 0x22d3ee, 0xd4d4d8,
            0x71717a, 0xfca5a5, 0x86efac, 0xfde047, 0x93c5fd, 0xd8b4fe, 0x67e8f9, 0xfafafa,
        ],
    },
];

pub const LIGHT: [Palette; 5] = [
    Palette {
        id: "runo",
        name: "Runo (default)",
        canvas: 0xffffff,
        sidebar: 0xf7f7f8,
        rail: 0xe4e4e8,
        sheet: 0xf4f4f6,
        subtle: 0xf1f1f3,
        border: 0xe5e5e8,
        active: 0xe9e9ec,
        raised: 0xffffff,
        term_bg: 0xfafafa,
        text: 0x18181b,
        secondary: 0x55555e,
        muted: 0x8a8a94,
        green: 0x16a34a,
        amber: 0xf0a020,
        red: 0xdc2626,
        accent: None,
        glass: Some([0x0f0f12, 0x2c2c33, 0x4f4f58]),
        ansi: [
            0x18181b, 0xdc2626, 0x16a34a, 0xca8a04, 0x2563eb, 0x9333ea, 0x0891b2, 0x52525b,
            0x71717a, 0xef4444, 0x22c55e, 0xeab308, 0x3b82f6, 0xa855f7, 0x06b6d4, 0x27272a,
        ],
    },
    Palette {
        id: "catppuccin-latte",
        name: "Catppuccin Latte",
        canvas: 0xeff1f5,
        sidebar: 0xe6e9ef,
        rail: 0xdce0e8,
        sheet: 0xe9ecf2,
        subtle: 0xe6e9ef,
        border: 0xccd0da,
        active: 0xdce0e8,
        raised: 0xf5f6f9,
        term_bg: 0xe6e9ef,
        text: 0x4c4f69,
        secondary: 0x5c5f77,
        muted: 0x8c8fa1,
        green: 0x40a02b,
        amber: 0xdf8e1d,
        red: 0xd20f39,
        accent: Some(0x8839ef),
        glass: None,
        ansi: [
            0x5c5f77, 0xd20f39, 0x40a02b, 0xdf8e1d, 0x1e66f5, 0xea76cb, 0x179299, 0xacb0be,
            0x6c6f85, 0xd20f39, 0x40a02b, 0xdf8e1d, 0x1e66f5, 0xea76cb, 0x179299, 0xbcc0cc,
        ],
    },
    Palette {
        id: "tokyo-night-day",
        name: "Tokyo Night Day",
        canvas: 0xe1e2e7,
        sidebar: 0xd0d5e3,
        rail: 0xc1c9df,
        sheet: 0xd8dbe5,
        subtle: 0xd5d8e3,
        border: 0xc4c8da,
        active: 0xc4c8da,
        raised: 0xe9eaee,
        term_bg: 0xd0d5e3,
        text: 0x3760bf,
        secondary: 0x6172b0,
        muted: 0x848cb5,
        green: 0x587539,
        amber: 0x8c6c3e,
        red: 0xf52a65,
        accent: Some(0x2e7de9),
        glass: None,
        ansi: [
            0xe9e9ed, 0xf52a65, 0x587539, 0x8c6c3e, 0x2e7de9, 0x9854f1, 0x007197, 0x6172b0,
            0xa1a6c5, 0xf52a65, 0x587539, 0x8c6c3e, 0x2e7de9, 0x9854f1, 0x007197, 0x3760bf,
        ],
    },
    Palette {
        id: "ayu-light",
        name: "Ayu Light",
        canvas: 0xfcfcfc,
        sidebar: 0xf3f4f5,
        rail: 0xe7eaed,
        sheet: 0xf6f7f8,
        subtle: 0xf3f4f5,
        border: 0xe1e4e8,
        active: 0xe7eaed,
        raised: 0xfcfcfc,
        term_bg: 0xf3f4f5,
        text: 0x5c6166,
        secondary: 0x6c7680,
        muted: 0x8a9199,
        green: 0x86b300,
        amber: 0xf2ae49,
        red: 0xf07171,
        accent: Some(0xfa8d3e),
        glass: None,
        ansi: [
            0x000000, 0xea6c6d, 0x6cbf43, 0xeca944, 0x3199e1, 0x9e75c7, 0x46ba94, 0xbababa,
            0x686868, 0xf07171, 0x86b300, 0xf2ae49, 0x399ee6, 0xa37acc, 0x4cbf99, 0xd1d1d1,
        ],
    },
    Palette {
        id: "light-owl",
        name: "Light Owl",
        canvas: 0xfbfbfb,
        sidebar: 0xf0f0f0,
        rail: 0xe4e4e4,
        sheet: 0xf4f4f4,
        subtle: 0xf0f0f0,
        border: 0xd9d9d9,
        active: 0xe0e0e0,
        raised: 0xfbfbfb,
        term_bg: 0xf0f0f0,
        text: 0x403f53,
        secondary: 0x5f5e72,
        muted: 0x90a7b2,
        green: 0x08916a,
        amber: 0xdaaa01,
        red: 0xde3d3b,
        accent: Some(0x4876d6),
        glass: None,
        ansi: [
            0x403f53, 0xde3d3b, 0x08916a, 0xe0af02, 0x288ed7, 0xd6438a, 0x2aa298, 0xf0f0f0,
            0x989fb1, 0xde3d3b, 0x08916a, 0xdaaa01, 0x288ed7, 0xd6438a, 0x2aa298, 0xf0f0f0,
        ],
    },
];

static DARK_PICK: AtomicU8 = AtomicU8::new(0);
static LIGHT_PICK: AtomicU8 = AtomicU8::new(0);

/// Pick the themes by id; an unknown id falls back to the first in its list.
pub fn set(dark: &str, light: &str) {
    DARK_PICK.store(index(&DARK, dark) as u8, Ordering::Relaxed);
    LIGHT_PICK.store(index(&LIGHT, light) as u8, Ordering::Relaxed);
}

fn index(list: &[Palette], id: &str) -> usize {
    list.iter().position(|p| p.id == id).unwrap_or(0)
}

/// The theme for dark or light mode.
pub fn get(dark: bool) -> &'static Palette {
    if dark {
        &DARK[DARK_PICK.load(Ordering::Relaxed) as usize]
    } else {
        &LIGHT[LIGHT_PICK.load(Ordering::Relaxed) as usize]
    }
}

/// The theme in use now.
pub fn current() -> &'static Palette {
    get(crate::settings::is_dark())
}

/// Every theme, so tests can check them all.
#[cfg(test)]
fn all() -> impl Iterator<Item = (&'static Palette, bool)> {
    DARK.iter()
        .map(|p| (p, true))
        .chain(LIGHT.iter().map(|p| (p, false)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lum(c: u32) -> f32 {
        let ch = |s: u32| {
            let v = ((c >> s) & 0xff) as f32 / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * ch(16) + 0.7152 * ch(8) + 0.0722 * ch(0)
    }

    fn contrast(a: u32, b: u32) -> f32 {
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn ids_are_unique() {
        let mut ids: Vec<_> = all().map(|(p, _)| p.id).collect();
        ids.sort();
        let len = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), len);
    }

    #[test]
    fn text_reads_on_every_theme() {
        for (p, dark) in all() {
            assert_eq!(lum(p.canvas) < 0.2, dark, "{} is in the wrong list", p.id);
            assert!(contrast(p.text, p.canvas) >= 4.5, "{} text", p.id);
            assert!(contrast(p.secondary, p.canvas) >= 3.0, "{} secondary", p.id);
        }
    }

    #[test]
    fn unknown_ids_fall_back() {
        assert_eq!(DARK[index(&DARK, "nope")].id, "tokyo-night");
        assert_eq!(LIGHT[index(&LIGHT, "nope")].id, "runo");
        assert_eq!(DARK[index(&DARK, "night-owl")].id, "night-owl");
        assert_eq!(LIGHT[index(&LIGHT, "ayu-light")].id, "ayu-light");
    }
}
