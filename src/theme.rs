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
