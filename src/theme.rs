//! Light / dark palettes that follow the OS setting, with a user-chosen
//! accent colour. Every widget is drawn with sharp (square) corners.

use iced::theme::{Mode, Palette};
use iced::widget::{button, checkbox, container, pick_list, scrollable, text_editor, text_input};
use iced::{Background, Border, Color, Theme, color};

pub const ACCENTS: [(&str, &str); 8] = [
    ("Blue", "#2f6bed"),
    ("Indigo", "#5b5bd6"),
    ("Violet", "#8e4ec6"),
    ("Pink", "#d6409f"),
    ("Red", "#d93f3f"),
    ("Orange", "#e5751f"),
    ("Green", "#2b9a66"),
    ("Teal", "#0f8f8f"),
];

pub fn accent_color(hex: &str) -> Color {
    let (r, g, b) = crate::config::parse_hex(hex).unwrap_or((0x2f, 0x6b, 0xed));
    Color::from_rgb8(r, g, b)
}

pub fn build(mode: Mode, accent: &str) -> Theme {
    let primary = accent_color(accent);
    match mode {
        Mode::Dark => Theme::custom(
            "Mailix Dark".to_string(),
            Palette {
                background: color!(0x1b1c1f),
                text: color!(0xe4e5e9),
                primary: iced::theme::palette::lighten(primary, 0.06),
                success: color!(0x3fb27f),
                warning: color!(0xe0a43a),
                danger: color!(0xf0605d),
            },
        ),
        _ => Theme::custom(
            "Mailix Light".to_string(),
            Palette {
                background: color!(0xffffff),
                text: color!(0x1d1e22),
                primary,
                success: color!(0x1f8a5b),
                warning: color!(0xb7791f),
                danger: color!(0xd14343),
            },
        ),
    }
}

const SHARP: Border = Border {
    color: Color::TRANSPARENT,
    width: 0.0,
    radius: iced::border::Radius { top_left: 0.0, top_right: 0.0, bottom_right: 0.0, bottom_left: 0.0 },
};

pub fn muted(theme: &Theme) -> Color {
    Color { a: 0.6, ..theme.extended_palette().background.base.text }
}

/// Text colour for a background highlight in HTML mail (`<mark>`,
/// `background-color`): it keeps the original hue but is pushed far enough
/// from the theme background to stay readable. `None` for neutral
/// backgrounds (white/grey/black) that carry no visible marking.
pub fn marked_text(rgb: [u8; 3], theme: &Theme) -> Option<Color> {
    let (h, s, l) = rgb_to_hsl(rgb);
    if s < 0.2 {
        return None;
    }
    let s = s.max(0.55);
    let bg = luminance(theme.palette().background).clamp(0.05, 0.95);
    let mut l = l;
    let mut out = hsl_to_rgb(h, s, l);
    for _ in 0..24 {
        if (luminance(out) - bg).abs() >= 0.35 {
            break;
        }
        l += if bg > 0.5 { -0.04 } else { 0.04 };
        if !(0.02..=0.98).contains(&l) {
            break;
        }
        out = hsl_to_rgb(h, s, l);
    }
    Some(out)
}

fn luminance(c: Color) -> f32 {
    0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b
}

fn rgb_to_hsl(rgb: [u8; 3]) -> (f32, f32, f32) {
    let [r, g, b] = rgb.map(|c| c as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let (d, l) = (max - min, (max + min) / 2.0);
    if d < 1e-6 {
        return (0.0, 0.0, l);
    }
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if (max - r).abs() < 1e-6 {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if (max - g).abs() < 1e-6 {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h / 6.0, s, l)
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> Color {
    if s <= 1e-6 {
        return Color { r: l, g: l, b: l, a: 1.0 };
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let f = |mut t: f32| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        let v = if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        };
        v.clamp(0.0, 1.0)
    };
    Color { r: f(h + 1.0 / 3.0), g: f(h), b: f(h - 1.0 / 3.0), a: 1.0 }
}

// ---- containers -------------------------------------------------------------

/// Sidebar, toolbars, status bar.
pub fn pane(theme: &Theme) -> container::Style {
    let p = theme.extended_palette();
    container::Style {
        background: Some(Background::Color(p.background.weakest.color)),
        text_color: Some(p.background.weakest.text),
        ..Default::default()
    }
}

pub fn base(theme: &Theme) -> container::Style {
    let p = theme.extended_palette();
    container::Style {
        background: Some(Background::Color(p.background.base.color)),
        text_color: Some(p.background.base.text),
        ..Default::default()
    }
}

/// Bordered box (attachments, suggestions, settings groups).
pub fn boxed(theme: &Theme) -> container::Style {
    let p = theme.extended_palette();
    container::Style {
        background: Some(Background::Color(p.background.base.color)),
        border: Border { color: p.background.strong.color, width: 1.0, radius: 0.0.into() },
        ..Default::default()
    }
}

pub fn note(theme: &Theme) -> container::Style {
    let p = theme.extended_palette();
    container::Style {
        background: Some(Background::Color(p.primary.weak.color)),
        text_color: Some(p.primary.weak.text),
        ..Default::default()
    }
}

/// Quoted original in the composer: a barely tinted panel close to the
/// window background (very dark grey in dark mode, off-white in light).
pub fn quote(theme: &Theme) -> container::Style {
    let p = theme.extended_palette();
    let (bg, fg) = (p.background.base.color, p.background.base.text);
    let mix = |a: f32, b: f32| a + (b - a) * 0.045;
    container::Style {
        background: Some(Background::Color(Color { r: mix(bg.r, fg.r), g: mix(bg.g, fg.g), b: mix(bg.b, fg.b), a: 1.0 })),
        border: Border { color: p.background.weak.color, width: 1.0, radius: 0.0.into() },
        ..Default::default()
    }
}

pub fn error_note(theme: &Theme) -> container::Style {
    let p = theme.extended_palette();
    container::Style {
        background: Some(Background::Color(p.danger.weak.color)),
        text_color: Some(p.danger.weak.text),
        ..Default::default()
    }
}

/// Square "avatar" with the sender's initials.
pub fn avatar(theme: &Theme) -> container::Style {
    let p = theme.extended_palette();
    container::Style {
        background: Some(Background::Color(p.primary.base.color)),
        text_color: Some(p.primary.base.text),
        ..Default::default()
    }
}

pub fn swatch(c: Color, selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let p = theme.extended_palette();
        button::Style {
            background: Some(Background::Color(c)),
            border: Border {
                color: if selected || status == button::Status::Hovered { p.background.base.text } else { Color::TRANSPARENT },
                width: 2.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        }
    }
}

// ---- buttons ----------------------------------------------------------------

pub fn primary(theme: &Theme, status: button::Status) -> button::Style {
    button::Style { border: SHARP, ..button::primary(theme, status) }
}

pub fn secondary(theme: &Theme, status: button::Status) -> button::Style {
    let p = theme.extended_palette();
    let bg = match status {
        button::Status::Hovered | button::Status::Pressed => p.background.weak.color,
        _ => p.background.base.color,
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: if status == button::Status::Disabled { muted(theme) } else { p.background.base.text },
        border: Border { color: p.background.strong.color, width: 1.0, radius: 0.0.into() },
        ..Default::default()
    }
}

pub fn danger(theme: &Theme, status: button::Status) -> button::Style {
    button::Style { border: SHARP, ..button::danger(theme, status) }
}

/// Flat list row. Selected rows get the accent tint plus a bar on the left
/// (drawn by the caller).
pub fn row(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let p = theme.extended_palette();
        let bg = if selected {
            Some(Color { a: 0.14, ..p.primary.base.color })
        } else {
            match status {
                button::Status::Hovered | button::Status::Pressed => Some(p.background.weaker.color),
                _ => None,
            }
        };
        button::Style {
            background: bg.map(Background::Color),
            text_color: p.background.base.text,
            border: SHARP,
            ..Default::default()
        }
    }
}

/// Quiet toolbar button.
pub fn tool(theme: &Theme, status: button::Status) -> button::Style {
    let p = theme.extended_palette();
    let bg = match status {
        button::Status::Hovered | button::Status::Pressed => Some(Background::Color(p.background.weak.color)),
        _ => None,
    };
    button::Style {
        background: bg,
        text_color: if status == button::Status::Disabled { muted(theme) } else { p.background.base.text },
        border: SHARP,
        ..Default::default()
    }
}

/// Tab / segmented control.
pub fn tab(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let p = theme.extended_palette();
        let mut s = tool(theme, status);
        if active {
            s.background = Some(Background::Color(p.background.weak.color));
            s.text_color = p.primary.base.color;
        }
        s
    }
}

pub fn link(theme: &Theme, status: button::Status) -> button::Style {
    let p = theme.extended_palette();
    button::Style {
        background: None,
        text_color: match status {
            button::Status::Hovered => p.primary.strong.color,
            _ => p.primary.base.color,
        },
        border: SHARP,
        ..Default::default()
    }
}

// ---- inputs -----------------------------------------------------------------

pub fn input(theme: &Theme, status: text_input::Status) -> text_input::Style {
    let mut s = text_input::default(theme, status);
    s.border.radius = 0.0.into();
    s
}

/// Borderless input for the composer header rows.
pub fn bare_input(theme: &Theme, status: text_input::Status) -> text_input::Style {
    let mut s = text_input::default(theme, status);
    s.border = Border { color: Color::TRANSPARENT, width: 0.0, radius: 0.0.into() };
    s.background = Background::Color(Color::TRANSPARENT);
    s
}

pub fn editor(theme: &Theme, status: text_editor::Status) -> text_editor::Style {
    let mut s = text_editor::default(theme, status);
    s.border.radius = 0.0.into();
    s
}

pub fn bare_editor(theme: &Theme, status: text_editor::Status) -> text_editor::Style {
    let mut s = text_editor::default(theme, status);
    s.border = Border { color: Color::TRANSPARENT, width: 0.0, radius: 0.0.into() };
    s.background = Background::Color(Color::TRANSPARENT);
    s
}

/// Read-only mail body: no border, no background, selection in accent.
pub fn reader(theme: &Theme, status: text_editor::Status) -> text_editor::Style {
    let mut s = bare_editor(theme, status);
    s.selection = Color { a: 0.35, ..theme.extended_palette().primary.base.color };
    s
}

pub fn pick(theme: &Theme, status: pick_list::Status) -> pick_list::Style {
    let mut s = pick_list::default(theme, status);
    s.border.radius = 0.0.into();
    s
}

pub fn check(theme: &Theme, status: checkbox::Status) -> checkbox::Style {
    let mut s = checkbox::primary(theme, status);
    s.border.radius = 0.0.into();
    s
}

pub fn scroll(theme: &Theme, status: scrollable::Status) -> scrollable::Style {
    let mut s = scrollable::default(theme, status);
    for rail in [&mut s.vertical_rail, &mut s.horizontal_rail] {
        rail.border.radius = 0.0.into();
        rail.scroller.border.radius = 0.0.into();
        rail.background = None;
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_tint_keeps_hue_and_adapts() {
        let yellow = [0xff, 0xff, 0x00];
        let light = marked_text(yellow, &build(Mode::Light, "#000000")).unwrap();
        assert!(light.b < 0.2 && light.r > 0.55, "{light:?}");
        assert!((0.4..0.8).contains(&luminance(light)), "{light:?}");
        let dark = marked_text(yellow, &build(Mode::Dark, "#000000")).unwrap();
        assert!(luminance(dark) >= 0.55, "{dark:?}");
    }

    #[test]
    fn mark_tint_skips_neutrals() {
        assert!(marked_text([0xf2, 0xf2, 0xf2], &build(Mode::Light, "#000000")).is_none());
        assert!(marked_text([17, 17, 17], &build(Mode::Light, "#000000")).is_none());
        // A pale yellow highlight still counts.
        assert!(marked_text([0xfb, 0xed, 0x7c], &build(Mode::Light, "#000000")).is_some());
    }

    #[test]
    fn hsl_roundtrip() {
        for rgb in [[255u8, 0, 0], [0, 255, 0], [34, 170, 221], [255, 255, 0], [128, 128, 128]] {
            let (h, s, l) = rgb_to_hsl(rgb);
            let c = hsl_to_rgb(h, s, l);
            assert!((c.r - rgb[0] as f32 / 255.0).abs() < 1e-3, "{rgb:?} -> {c:?}");
            assert!((c.g - rgb[1] as f32 / 255.0).abs() < 1e-3, "{rgb:?} -> {c:?}");
            assert!((c.b - rgb[2] as f32 / 255.0).abs() < 1e-3, "{rgb:?} -> {c:?}");
        }
    }
}
