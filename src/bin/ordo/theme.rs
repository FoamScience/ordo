// ---------------------------------------------------------------------- themes
// The reviewer's whole palette, in one place.
//
// Two kinds of theme live here. A **plain** theme (`dark`, `light`) names only its tints and leaves every text colour to the page. A
// **truecolor** theme (catppuccin, tokyonight, …) names every colour itself,
// for a reviewer who wants ordo to look like their editor.

/// A theme colour: one the theme names, or `Default`, which leaves the role
/// to the page's own colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Color {
    Default,
    Rgb(u8, u8, u8),
}

/// `0x89b4fa` → `Color::Rgb(0x89, 0xb4, 0xfa)`.
pub(super) const fn hex(v: u32) -> Color {
    Color::Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

#[derive(Clone, Copy)]
pub(super) struct Syntax {
    pub(super) comment: Color,
    pub(super) keyword: Color,
    pub(super) string: Color,
    pub(super) number: Color,
    pub(super) function: Color,
    pub(super) type_: Color,
    pub(super) property: Color,
    pub(super) operator: Color,
    pub(super) variable: Color,
    pub(super) builtin: Color,
    pub(super) param: Color,
    pub(super) attribute: Color,
}

#[derive(Clone, Copy)]
pub(super) struct Theme {
    pub(super) name: &'static str,
    // ---- chrome
    /// default text; `Default` on a plain theme, so the page's own
    /// foreground shows through
    pub(super) fg: Color,
    /// secondary text — present but recessive
    pub(super) dim: Color,
    pub(super) border: Color,
    /// the focus ring
    pub(super) border_focus: Color,
    /// the selected row and the active tab
    pub(super) accent: Color,
    /// a reviewed row's marker
    pub(super) reviewed: Color,
    /// warning-level text
    pub(super) warn: Color,
    // ---- diff
    pub(super) add_bg: Color,
    pub(super) del_bg: Color,
    /// the *changed part* of a line whose counterpart was identified — the
    /// line keeps the quiet add/del tint, and only what actually changed gets
    /// these (Neovim's DiffText over DiffChange)
    pub(super) add_strong_bg: Color,
    pub(super) del_strong_bg: Color,
    /// the selected row's tint — a neutral slate, subtle next to
    /// add_bg/del_bg
    pub(super) select_bg: Color,
    /// search-match backgrounds — current match brighter than the rest, so
    /// it reads as "here" among however many others are also highlighted
    pub(super) match_bg: Color,
    pub(super) match_cur_bg: Color,
    // ---- state
    /// the top band
    pub(super) band_bg: Color,
    /// A hunk's state is its row's background, never a glyph: the highest
    /// finding level on it, or reviewed. A verdict is plum rather than red so
    /// it cannot read as a deleted line. Each `_strong` shade is the same
    /// state selected, or as a chip.
    pub(super) verdict_bg: Color,
    pub(super) verdict_strong_bg: Color,
    pub(super) warn_bg: Color,
    pub(super) warn_strong_bg: Color,
    pub(super) note_bg: Color,
    pub(super) note_strong_bg: Color,
    pub(super) reviewed_bg: Color,
    // ---- code
    pub(super) syn: Syntax,
}

impl Theme {
    /// The page's own text colours, with tints for a dark or a light page.
    pub(super) fn plain(name: &'static str, light: bool) -> Theme {
        let d = Color::Default;
        let chrome = Theme {
            name,
            fg: d,
            dim: d,
            border: d,
            border_focus: d,
            accent: d,
            reviewed: d,
            warn: d,
            // filled in per lightness below
            add_bg: d,
            del_bg: d,
            add_strong_bg: d,
            del_strong_bg: d,
            select_bg: d,
            match_bg: d,
            match_cur_bg: d,
            band_bg: d,
            verdict_bg: d,
            verdict_strong_bg: d,
            warn_bg: d,
            warn_strong_bg: d,
            note_bg: d,
            note_strong_bg: d,
            reviewed_bg: d,
            syn: Syntax {
                comment: d,
                keyword: d,
                string: d,
                number: d,
                function: d,
                type_: d,
                property: d,
                operator: d,
                variable: d,
                builtin: d,
                param: d,
                attribute: d,
            },
        };
        if light {
            // pale tints of the same hues, at light-page weight — not the
            // dark values inverted, which would read as loud on a light page
            Theme {
                add_bg: hex(0xd6f0da),
                del_bg: hex(0xf8d6d6),
                add_strong_bg: hex(0xa0dcaf),
                del_strong_bg: hex(0xf5aeae),
                select_bg: hex(0xccd0da),
                match_bg: hex(0xffecaa),
                match_cur_bg: hex(0xffca44),
                band_bg: hex(0xdce0e8),
                verdict_bg: hex(0xecd5f4),
                verdict_strong_bg: hex(0xd9aeea),
                warn_bg: hex(0xf6e6bd),
                warn_strong_bg: hex(0xefd28a),
                note_bg: hex(0xe4ecf7),
                note_strong_bg: hex(0xa9c7f0),
                reviewed_bg: hex(0xd6efd9),
                ..chrome
            }
        } else {
            Theme {
                add_bg: hex(0x142819),
                del_bg: hex(0x32181c),
                add_strong_bg: hex(0x22542e),
                del_strong_bg: hex(0x68282e),
                select_bg: hex(0x45475a),
                match_bg: hex(0x463c0a),
                match_cur_bg: hex(0x8c6e0f),
                band_bg: hex(0x313244),
                verdict_bg: hex(0x45244f),
                verdict_strong_bg: hex(0x66307a),
                warn_bg: hex(0x4d3b14),
                warn_strong_bg: hex(0x6b5219),
                note_bg: hex(0x232b3e),
                note_strong_bg: hex(0x2b4870),
                reviewed_bg: hex(0x1d3326),
                ..chrome
            }
        }
    }
}

/// A truecolor theme, built from the eleven colours these palettes all publish.
/// Every field of `Theme` is derived here, so a palette is eleven lines rather
/// than twenty-five, and two themes can't disagree about which colour plays
/// which role.
struct Palette {
    name: &'static str,
    fg: u32,
    dim: u32,
    /// the palette's own "surface"/"current line" tone — the selection tint
    surface: u32,
    red: u32,
    green: u32,
    yellow: u32,
    blue: u32,
    magenta: u32,
    cyan: u32,
    orange: u32,
    /// how far to lift a tint off the background: dark palettes need a floor,
    /// light ones need to stay pale
    light: bool,
}

impl Palette {
    /// Mix `a` toward `b` by `w`/256 — how a diff tint is derived from a
    /// palette colour rather than guessed at per theme.
    const fn mix(a: u32, b: u32, w: u32) -> Color {
        hex(Palette::mix32(a, b, w))
    }

    const fn mix32(a: u32, b: u32, w: u32) -> u32 {
        const fn ch(a: u32, b: u32, w: u32, sh: u32) -> u32 {
            let (x, y) = ((a >> sh) & 0xff, (b >> sh) & 0xff);
            ((x * (256 - w) + y * w) / 256) << sh
        }
        ch(a, b, w, 16) | ch(a, b, w, 8) | ch(a, b, w, 0)
    }

    fn theme(&self) -> Theme {
        // a tint is the accent mixed into the page: toward black on a dark
        // palette, toward white on a light one
        self.on(if self.light { 0xffffff } else { 0x000000 })
    }

    /// This palette with every tint mixed toward `ground`, black or white.
    fn on(&self, ground: u32) -> Theme {
        // how far the tint sits from the page: quiet enough to read a whole
        // line over, strong enough that the refined span stands out inside it
        let quiet = if self.light { 200 } else { 210 };
        let strong = 130;
        // state tints sit apart from the diff and match weights so no two
        // roles share a colour; a warn is orange because yellow is the search
        // match's, and a note is fainter than the other two levels
        let (state, state_strong, faint) = if self.light {
            (190, 140, 220)
        } else {
            (180, 130, 215)
        };
        Theme {
            name: self.name,
            fg: hex(self.fg),
            dim: hex(self.dim),
            border: hex(self.surface),
            border_focus: hex(self.blue),
            accent: hex(self.blue),
            reviewed: hex(self.green),
            warn: hex(self.orange),
            add_bg: Palette::mix(self.green, ground, quiet),
            del_bg: Palette::mix(self.red, ground, quiet),
            add_strong_bg: Palette::mix(self.green, ground, strong),
            del_strong_bg: Palette::mix(self.red, ground, strong),
            // the band is the surface itself, so the selection lifts off it
            // toward the text colour
            select_bg: Palette::mix(self.surface, self.fg, 40),
            match_bg: Palette::mix(self.yellow, ground, quiet),
            match_cur_bg: Palette::mix(self.yellow, ground, strong),
            band_bg: hex(self.surface),
            verdict_bg: Palette::mix(self.magenta, ground, state),
            verdict_strong_bg: Palette::mix(self.magenta, ground, state_strong),
            warn_bg: Palette::mix(self.orange, ground, state),
            warn_strong_bg: Palette::mix(self.orange, ground, state_strong),
            note_bg: Palette::mix(self.blue, ground, faint),
            note_strong_bg: Palette::mix(self.blue, ground, state_strong),
            reviewed_bg: Palette::mix(self.green, ground, faint),
            syn: Syntax {
                comment: hex(self.dim),
                keyword: hex(self.magenta),
                string: hex(self.green),
                number: hex(self.orange),
                function: hex(self.blue),
                type_: hex(self.yellow),
                property: hex(self.cyan),
                operator: hex(self.dim),
                variable: hex(self.fg),
                builtin: hex(self.red),
                param: hex(self.orange),
                attribute: hex(self.cyan),
            },
        }
    }
}

/// The built-in truecolor palettes, as each project publishes them.
const PALETTES: &[Palette] = &[
    Palette {
        name: "catppuccin-mocha",
        fg: 0xcdd6f4,
        dim: 0x6c7086,
        surface: 0x313244,
        red: 0xf38ba8,
        green: 0xa6e3a1,
        yellow: 0xf9e2af,
        blue: 0x89b4fa,
        magenta: 0xcba6f7,
        cyan: 0x94e2d5,
        orange: 0xfab387,
        light: false,
    },
    Palette {
        name: "catppuccin-macchiato",
        fg: 0xcad3f5,
        dim: 0x6e738d,
        surface: 0x363a4f,
        red: 0xed8796,
        green: 0xa6da95,
        yellow: 0xeed49f,
        blue: 0x8aadf4,
        magenta: 0xc6a0f6,
        cyan: 0x8bd5ca,
        orange: 0xf5a97f,
        light: false,
    },
    Palette {
        name: "catppuccin-frappe",
        fg: 0xc6d0f5,
        dim: 0x737994,
        surface: 0x414559,
        red: 0xe78284,
        green: 0xa6d189,
        yellow: 0xe5c890,
        blue: 0x8caaee,
        magenta: 0xca9ee6,
        cyan: 0x81c8be,
        orange: 0xef9f76,
        light: false,
    },
    Palette {
        name: "catppuccin-latte",
        fg: 0x4c4f69,
        dim: 0x8c8fa1,
        surface: 0xccd0da,
        red: 0xd20f39,
        green: 0x40a02b,
        yellow: 0xdf8e1d,
        blue: 0x1e66f5,
        magenta: 0x8839ef,
        cyan: 0x179299,
        orange: 0xfe640b,
        light: true,
    },
    Palette {
        name: "tokyonight-night",
        fg: 0xc0caf5,
        dim: 0x565f89,
        surface: 0x292e42,
        red: 0xf7768e,
        green: 0x9ece6a,
        yellow: 0xe0af68,
        blue: 0x7aa2f7,
        magenta: 0xbb9af7,
        cyan: 0x7dcfff,
        orange: 0xff9e64,
        light: false,
    },
    Palette {
        name: "tokyonight-storm",
        fg: 0xc0caf5,
        dim: 0x565f89,
        surface: 0x2f334d,
        red: 0xf7768e,
        green: 0x9ece6a,
        yellow: 0xe0af68,
        blue: 0x7aa2f7,
        magenta: 0xbb9af7,
        cyan: 0x7dcfff,
        orange: 0xff9e64,
        light: false,
    },
    Palette {
        name: "tokyonight-moon",
        fg: 0xc8d3f5,
        dim: 0x636da6,
        surface: 0x2f334d,
        red: 0xff757f,
        green: 0xc3e88d,
        yellow: 0xffc777,
        blue: 0x82aaff,
        magenta: 0xc099ff,
        cyan: 0x86e1fc,
        orange: 0xff966c,
        light: false,
    },
    Palette {
        name: "tokyonight-day",
        fg: 0x3760bf,
        dim: 0x848cb5,
        surface: 0xc4c8da,
        red: 0xf52a65,
        green: 0x587539,
        yellow: 0x8c6c3e,
        blue: 0x2e7de9,
        magenta: 0x9854f1,
        cyan: 0x007197,
        orange: 0xb15c00,
        light: true,
    },
    Palette {
        name: "gruvbox-dark",
        fg: 0xebdbb2,
        dim: 0x928374,
        surface: 0x3c3836,
        red: 0xfb4934,
        green: 0xb8bb26,
        yellow: 0xfabd2f,
        blue: 0x83a598,
        magenta: 0xd3869b,
        cyan: 0x8ec07c,
        orange: 0xfe8019,
        light: false,
    },
    Palette {
        name: "gruvbox-light",
        fg: 0x3c3836,
        dim: 0x7c6f64,
        surface: 0xebdbb2,
        red: 0x9d0006,
        green: 0x79740e,
        yellow: 0xb57614,
        blue: 0x076678,
        magenta: 0x8f3f71,
        cyan: 0x427b58,
        orange: 0xaf3a03,
        light: true,
    },
    Palette {
        name: "nord",
        fg: 0xd8dee9,
        dim: 0x4c566a,
        surface: 0x3b4252,
        red: 0xbf616a,
        green: 0xa3be8c,
        yellow: 0xebcb8b,
        blue: 0x81a1c1,
        magenta: 0xb48ead,
        cyan: 0x88c0d0,
        orange: 0xd08770,
        light: false,
    },
    Palette {
        name: "dracula",
        fg: 0xf8f8f2,
        dim: 0x6272a4,
        surface: 0x44475a,
        red: 0xff5555,
        green: 0x50fa7b,
        yellow: 0xf1fa8c,
        blue: 0xbd93f9,
        magenta: 0xff79c6,
        cyan: 0x8be9fd,
        orange: 0xffb86c,
        light: false,
    },
    Palette {
        name: "solarized-dark",
        fg: 0x93a1a1,
        dim: 0x586e75,
        surface: 0x073642,
        red: 0xdc322f,
        green: 0x859900,
        yellow: 0xb58900,
        blue: 0x268bd2,
        magenta: 0xd33682,
        cyan: 0x2aa198,
        orange: 0xcb4b16,
        light: false,
    },
    Palette {
        name: "solarized-light",
        fg: 0x586e75,
        dim: 0x93a1a1,
        surface: 0xeee8d5,
        red: 0xdc322f,
        green: 0x859900,
        yellow: 0xb58900,
        blue: 0x268bd2,
        magenta: 0xd33682,
        cyan: 0x2aa198,
        orange: 0xcb4b16,
        light: true,
    },
];

/// Every theme name, plain ones first — the order `--theme` reports, and
/// the order `:config` cycles in.
pub(super) fn theme_names() -> Vec<String> {
    ["dark".to_string(), "light".to_string()]
        .into_iter()
        .chain(PALETTES.iter().map(|p| p.name.to_string()))
        .collect()
}

pub(super) fn theme(name: &str) -> Option<Theme> {
    match name {
        "dark" => Some(Theme::plain("dark", false)),
        "light" => Some(Theme::plain("light", true)),
        n => PALETTES.iter().find(|p| p.name == n).map(Palette::theme),
    }
}

/// Declares the theme roles a config file may set, each the name of a `Theme`
/// field, generating the write side (`apply_theme_colors`) and read side
/// (`theme_role_color`) from one list — so a role can't drift between the
/// two, which is how "match-bg" once read back the wrong field.
macro_rules! theme_roles {
    ($($role:literal => $($seg:ident).+),* $(,)?) => {
        pub(super) const THEME_ROLES: &[&str] = &[$($role),*];

        /// Overlay a config's colour overrides onto a theme. Unknown roles are
        /// rejected at parse time, so everything reaching here names a field.
        pub(super) fn apply_theme_colors(mut t: Theme, colors: &[(String, Color)]) -> Theme {
            for (role, c) in colors {
                match role.as_str() {
                    $($role => t.$($seg).+ = *c,)*
                    _ => {}
                }
            }
            t
        }

        /// A theme role's current colour, by the name a config file uses. The
        /// read side of `apply_theme_colors`, so `--init-config` prints what
        /// the program would actually read back.
        pub(super) fn theme_role_color(t: &Theme, role: &str) -> Color {
            match role {
                $($role => t.$($seg).+,)*
                _ => Color::Default,
            }
        }
    };
}

theme_roles! {
    "fg" => fg,
    "dim" => dim,
    "border" => border,
    "border-focus" => border_focus,
    "accent" => accent,
    "reviewed" => reviewed,
    "warn" => warn,
    "add-bg" => add_bg,
    "del-bg" => del_bg,
    "add-strong-bg" => add_strong_bg,
    "del-strong-bg" => del_strong_bg,
    "select-bg" => select_bg,
    "match-bg" => match_bg,
    "match-current-bg" => match_cur_bg,
    "band-bg" => band_bg,
    "verdict-bg" => verdict_bg,
    "verdict-strong-bg" => verdict_strong_bg,
    "warn-bg" => warn_bg,
    "warn-strong-bg" => warn_strong_bg,
    "note-bg" => note_bg,
    "note-strong-bg" => note_strong_bg,
    "reviewed-bg" => reviewed_bg,
    "syntax-comment" => syn.comment,
    "syntax-keyword" => syn.keyword,
    "syntax-string" => syn.string,
    "syntax-number" => syn.number,
    "syntax-function" => syn.function,
    "syntax-type" => syn.type_,
    "syntax-property" => syn.property,
    "syntax-operator" => syn.operator,
    "syntax-variable" => syn.variable,
    "syntax-builtin" => syn.builtin,
    "syntax-parameter" => syn.param,
    "syntax-attribute" => syn.attribute,
}
