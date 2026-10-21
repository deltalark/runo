//! Dark palettes on trial: well-known editor themes mapped onto Runo's dark tokens, picked
//! under Settings > Appearance > Dark palette. Once one is chosen, its values become the
//! dark side of the `c_*` functions in `settings.rs` and this module goes.

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
    /// A step above the canvas, for the list beside the rail.
    pub sheet: u32,
    pub subtle: u32,
    pub border: u32,
    /// The selected row.
    pub active: u32,
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
    /// Terminal colours 0 to 15.
    pub ansi: [u32; 16],
}

impl Palette {
    /// Text levels for glass mode: a step lighter, as `settings::c_text` explains.
    pub fn glass_text(&self) -> u32 {
        if self.id == "runo" {
            0xf0f2f5
        } else {
            mix(0xffffff, self.text, 0.45)
        }
    }
    pub fn glass_secondary(&self) -> u32 {
        if self.id == "runo" {
            0xd0d5dd
        } else {
            mix(0xffffff, self.secondary, 0.4)
        }
    }
    pub fn glass_muted(&self) -> u32 {
        if self.id == "runo" {
            0xa8aeb9
        } else {
            mix(0xffffff, self.muted, 0.25)
        }
    }
}

pub const PALETTES: [Palette; 9] = [
    Palette {
        id: "runo",
        name: "Runo (default)",
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
        ansi: [
            0x1e1e22, 0xf87171, 0x4ade80, 0xfacc15, 0x60a5fa, 0xc084fc, 0x22d3ee, 0xd4d4d8,
            0x71717a, 0xfca5a5, 0x86efac, 0xfde047, 0x93c5fd, 0xd8b4fe, 0x67e8f9, 0xfafafa,
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
        ansi: [
            0x51576d, 0xe78284, 0xa6d189, 0xe5c890, 0x8caaee, 0xf4b8e4, 0x81c8be, 0xb5bfe2,
            0x626880, 0xe78284, 0xa6d189, 0xe5c890, 0x8caaee, 0xf4b8e4, 0x81c8be, 0xa5adce,
        ],
    },
    Palette {
        id: "tokyo-night",
        name: "Tokyo Night",
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
        ansi: [
            0x1d202f, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xa9b1d6,
            0x414868, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xc0caf5,
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
        ansi: [
            0x011627, 0xef5350, 0x22da6e, 0xaddb67, 0x82aaff, 0xc792ea, 0x21c7a8, 0xffffff,
            0x575656, 0xef5350, 0x22da6e, 0xffeb95, 0x82aaff, 0xc792ea, 0x7fdbca, 0xffffff,
        ],
    },
];

static CURRENT: AtomicU8 = AtomicU8::new(0);

/// Pick the palette by id; an unknown id falls back to Runo's own.
pub fn set(id: &str) {
    let i = PALETTES.iter().position(|p| p.id == id).unwrap_or(0);
    CURRENT.store(i as u8, Ordering::Relaxed);
}

/// The dark palette in use.
pub fn current() -> &'static Palette {
    &PALETTES[CURRENT.load(Ordering::Relaxed) as usize]
}
