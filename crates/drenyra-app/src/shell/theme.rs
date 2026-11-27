//! Dreamcoder design tokens for the native window, GPUI-free.
//!
//! Values are copied verbatim from the canonical Dreamcoder palette,
//! `dreamcoder-workbench/DreamcoderThemes/dreamcoder/tokens.json` (`modes.dark`, lines 8-66, and
//! `modes.light`, lines 91-143). Its design rules apply here too: generated surfaces never invent
//! colors, both modes expose the same semantic roles, meaningful borders use `border_ui`, and pure
//! black is reserved for the dark canvas. The contrast tests below enforce the WCAG minimums the
//! palette declares (`guardrails`, lines 201-218) for every text and status role the window uses.

/// A selectable theme. The default is [`ThemeId::DreamcoderDark`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeId {
    #[default]
    DreamcoderDark,
    DreamcoderLight,
}

impl ThemeId {
    /// Every theme, in the order the Settings › Themes section lists them.
    pub const ALL: [ThemeId; 2] = [ThemeId::DreamcoderDark, ThemeId::DreamcoderLight];

    /// Display name, exactly as the palette names it (`tokens.json` `name`).
    pub const fn label(self) -> &'static str {
        match self {
            ThemeId::DreamcoderDark => "Dreamcoder Dark",
            ThemeId::DreamcoderLight => "Dreamcoder Light",
        }
    }

    /// Stable identifier stored in the settings file.
    pub const fn slug(self) -> &'static str {
        match self {
            ThemeId::DreamcoderDark => "dreamcoder-dark",
            ThemeId::DreamcoderLight => "dreamcoder-light",
        }
    }

    /// Parses a [`Self::slug`]; returns `None` for anything else.
    pub fn from_slug(slug: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|theme| theme.slug() == slug)
    }

    /// The design tokens of this theme.
    pub const fn tokens(self) -> ThemeTokens {
        match self {
            ThemeId::DreamcoderDark => DREAMCODER_DARK,
            ThemeId::DreamcoderLight => DREAMCODER_LIGHT,
        }
    }
}

/// Semantic colors (`0xRRGGBB`) the window draws with. Field docs name the source token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemeTokens {
    /// `bg`: window canvas behind every surface.
    pub canvas: u32,
    /// `bg_soft`: top bar and footer.
    pub chrome: u32,
    /// `surface0`: panes and side panels (scrollable surfaces).
    pub surface: u32,
    /// `surface1`: raised elements inside a surface (inputs, cards, overlays).
    pub surface_raised: u32,
    /// `surface2`: chip and badge fill.
    pub surface_strong: u32,
    /// `surface3`: dividers between layout regions.
    pub divider: u32,
    /// `text`: body text.
    pub text: u32,
    /// `text_heading`: titles.
    pub text_heading: u32,
    /// `muted`: secondary text.
    pub text_muted: u32,
    /// `subtle`: hints and captions.
    pub text_subtle: u32,
    /// `border_ui`: borders that carry meaning (chips, controls).
    pub border_ui: u32,
    /// `focus`: focus ring of the focused pane or control.
    pub focus: u32,
    /// `accent`: primary accent (active workspace, links, primary actions).
    pub accent: u32,
    /// `on_accent`: text drawn on an `accent` fill.
    pub on_accent: u32,
    /// `selection_bg`: selected row background.
    pub selection: u32,
    /// `selection_fg`: text on `selection`.
    pub on_selection: u32,
    /// `success` (`sage`).
    pub success: u32,
    /// `warning`.
    pub warning: u32,
    /// `error`.
    pub error: u32,
    /// `info`.
    pub info: u32,
    /// `scrim` as `0xRRGGBBAA`: dims the window behind modal overlays.
    pub scrim: u32,
}

/// `tokens.json` `modes.dark` ("Dreamcoder Dark").
pub const DREAMCODER_DARK: ThemeTokens = ThemeTokens {
    canvas: 0x000000,
    chrome: 0x0B0B0B,
    surface: 0x0B0B0B,
    surface_raised: 0x0D0D0F,
    surface_strong: 0x1F1F1F,
    divider: 0x2E2E2E,
    text: 0xE6E6E6,
    text_heading: 0xF5F5F5,
    text_muted: 0xC7C7C7,
    text_subtle: 0xA7A7A7,
    border_ui: 0x767676,
    focus: 0x3B82F6,
    accent: 0xA5B4FC,
    on_accent: 0x000000,
    selection: 0x3A3A3A,
    on_selection: 0xE6E6E6,
    success: 0x34D399,
    warning: 0xFBBF24,
    error: 0xFB8585,
    info: 0x7DD3FC,
    // rgba(0, 0, 0, 0.78)
    scrim: 0x000000C7,
};

/// `tokens.json` `modes.light` ("Dreamcoder Light").
pub const DREAMCODER_LIGHT: ThemeTokens = ThemeTokens {
    canvas: 0xf9f6f2,
    chrome: 0xf2eee9,
    surface: 0xfefdfa,
    surface_raised: 0xeee9e2,
    surface_strong: 0xe3ddd4,
    divider: 0xd3ccc1,
    text: 0x1e1814,
    text_heading: 0x120d09,
    text_muted: 0x352f29,
    text_subtle: 0x5e574f,
    border_ui: 0x645c53,
    focus: 0x006973,
    accent: 0x8b440c,
    on_accent: 0xfefdfa,
    selection: 0xf1d7c2,
    on_selection: 0x1e1814,
    success: 0x1c6c31,
    warning: 0x795700,
    error: 0xa12623,
    info: 0x125f94,
    // rgba(30, 24, 20, 0.42)
    scrim: 0x1e18146b,
};

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG 2.x relative luminance of `0xRRGGBB`.
    fn luminance(color: u32) -> f64 {
        let channel = |shift: u32| {
            let c = f64::from((color >> shift) & 0xff) / 255.0;
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
    }

    fn contrast(a: u32, b: u32) -> f64 {
        let (la, lb) = (luminance(a), luminance(b));
        let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
        (hi + 0.05) / (lo + 0.05)
    }

    #[test]
    fn themes_round_trip_through_their_slugs_and_keep_palette_names() {
        for theme in ThemeId::ALL {
            assert_eq!(ThemeId::from_slug(theme.slug()), Some(theme));
        }
        assert_eq!(ThemeId::from_slug("solarized"), None);
        assert_eq!(ThemeId::default(), ThemeId::DreamcoderDark);
        assert_eq!(ThemeId::DreamcoderDark.label(), "Dreamcoder Dark");
        assert_eq!(ThemeId::DreamcoderLight.label(), "Dreamcoder Light");
    }

    #[test]
    fn every_text_and_status_role_meets_wcag_aa_on_every_surface_it_sits_on() {
        for theme in ThemeId::ALL {
            let t = theme.tokens();
            let backgrounds = [
                ("chrome", t.chrome),
                ("surface", t.surface),
                ("surface_raised", t.surface_raised),
                ("surface_strong", t.surface_strong),
            ];
            let foregrounds = [
                ("text", t.text),
                ("text_heading", t.text_heading),
                ("text_muted", t.text_muted),
                ("text_subtle", t.text_subtle),
                ("accent", t.accent),
                ("success", t.success),
                ("warning", t.warning),
                ("error", t.error),
                ("info", t.info),
            ];
            for (bg_name, bg) in backgrounds {
                for (fg_name, fg) in foregrounds {
                    let ratio = contrast(fg, bg);
                    assert!(
                        ratio >= 4.5,
                        "{}: {fg_name} on {bg_name} is {ratio:.2}:1, below 4.5:1",
                        theme.label()
                    );
                }
            }
            assert!(contrast(t.on_selection, t.selection) >= 4.5);
            assert!(contrast(t.on_accent, t.accent) >= 4.5);
        }
    }

    #[test]
    fn controls_and_focus_ring_meet_non_text_contrast_on_surfaces() {
        // WCAG 1.4.11: user-interface components need at least 3:1 against adjacent colors.
        for theme in ThemeId::ALL {
            let t = theme.tokens();
            for bg in [t.chrome, t.surface, t.surface_raised] {
                assert!(contrast(t.border_ui, bg) >= 3.0, "{}", theme.label());
                assert!(contrast(t.focus, bg) >= 3.0, "{}", theme.label());
            }
        }
    }

    #[test]
    fn modes_differ_in_every_role_and_only_the_dark_canvas_is_pure_black() {
        let (dark, light) = (DREAMCODER_DARK, DREAMCODER_LIGHT);
        assert_eq!(dark.canvas, 0x000000);
        for t in [dark, light] {
            for surface in [t.chrome, t.surface, t.surface_raised, t.surface_strong] {
                assert_ne!(surface, 0x000000);
                assert_ne!(surface, 0xffffff);
            }
        }
        assert_ne!(dark.text, light.text);
        assert_ne!(dark.accent, light.accent);
        assert_ne!(dark.surface, light.surface);
    }
}
