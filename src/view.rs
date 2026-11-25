//! UI layout. Three panes (folders · list · reader) with aligned header
//! bars, and a status bar at the bottom. Everything is square-cornered.

use crate::app::*;
use crate::config::{AiProvider, Preset, Security};
use crate::mime::Hl;
use crate::store::{FolderRole, Summary};
use crate::theme;
use iced::widget::{
    Column, Row, Space, button, checkbox, column, container, pick_list, row, rule, scrollable, text, text_editor,
    text_input, tooltip,
};
use iced::{Alignment, Background, Border, Color, Element, Fill, Font, Length};

type El<'a> = Element<'a, Msg>;

pub const UI: Font = Font::with_name("Inter");
const BOLD: Font = Font { weight: iced::font::Weight::Bold, ..UI };
const SEMIBOLD: Font = Font { weight: iced::font::Weight::Semibold, ..UI };

/// All pane headers share this height so their bottom lines align.
const HEADER: f32 = 52.0;
const SIDEBAR_W: f32 = 232.0;
const LIST_W: f32 = 370.0;
const LABEL_W: f32 = 76.0;

fn muted<'a>(s: impl text::IntoFragment<'a>) -> text::Text<'a> {
    text(s).style(|t: &iced::Theme| text::Style { color: Some(theme::muted(t)) })
}

fn hr<'a>() -> El<'a> {
    rule::horizontal(1).into()
}

fn vr<'a>() -> El<'a> {
    rule::vertical(1).into()
}

fn header<'a>(content: impl Into<El<'a>>) -> El<'a> {
    container(content).height(HEADER).padding([0, 12]).center_y(HEADER).width(Fill).into()
}

fn input<'a>(placeholder: &str, value: &str) -> text_input::TextInput<'a, Msg> {
    text_input(placeholder, value).padding([7, 9]).size(14).style(theme::input)
}

fn primary_btn<'a>(label: &'a str, msg: Option<Msg>) -> El<'a> {
    button(text(label).font(SEMIBOLD).size(14)).padding([8, 18]).style(theme::primary).on_press_maybe(msg).into()
}

fn secondary_btn<'a>(label: &'a str, msg: Option<Msg>) -> El<'a> {
    button(text(label).size(13)).padding([7, 14]).style(theme::secondary).on_press_maybe(msg).into()
}

fn initials(name: &str) -> String {
    let s: String = name
        .split(|c: char| c.is_whitespace() || c == '.' || c == '@')
        .filter(|w| !w.is_empty())
        .take(2)
        .filter_map(|w| w.chars().next())
        .collect();
    if s.is_empty() { "?".into() } else { s.to_uppercase() }
}

/// Lucide icon font (ISC licensed, embedded).
pub const ICONS: Font = Font::with_name("lucide");

pub mod ic {
    pub const REPLY: char = '\u{e22a}';
    pub const REPLY_ALL: char = '\u{e22b}';
    pub const FORWARD: char = '\u{e229}';
    pub const ARCHIVE: char = '\u{e041}';
    pub const DELETE: char = '\u{e18e}';
    pub const FLAG: char = '\u{e0d1}';
    pub const UNFLAG: char = '\u{e292}';
    pub const MAIL: char = '\u{e10f}';
    pub const MAIL_OPEN: char = '\u{e363}';
    pub const AI: char = '\u{e412}';
    pub const ATTACHMENT: char = '\u{e12d}';
    pub const SEND: char = '\u{e152}';
    pub const CLOSE: char = '\u{e1b2}';
    pub const SEARCH: char = '\u{e151}';
    pub const SYNC: char = '\u{e145}';
    pub const SETTINGS: char = '\u{e154}';
    pub const CONTACTS: char = '\u{e54d}';
    pub const COMPOSE: char = '\u{e172}';
    pub const CHEVRON_DOWN: char = '\u{e06d}';
    pub const CHEVRON_RIGHT: char = '\u{e06f}';
    pub const COLLAPSE_ALL: char = '\u{e228}';
    pub const OPEN: char = '\u{e0b9}';
    pub const IMAGE: char = '\u{e0f6}';
    pub const DOWNLOAD: char = '\u{e0b2}';
    pub const INBOX: char = '\u{e0f7}';
    pub const INBOXES: char = '\u{e10f}';
    pub const FILE: char = '\u{e0c0}';
    pub const FOLDER: char = '\u{e0d7}';
    pub const JUNK: char = '\u{e127}';
    pub const DRAFT: char = '\u{e0cc}';
    pub const USERS: char = '\u{e1a4}';
    pub const PALETTE: char = '\u{e1dd}';
    pub const BOT: char = '\u{e1bb}';
    pub const BOLD: char = '\u{e05d}';
    pub const ITALIC: char = '\u{e0fb}';
    pub const UNDERLINE: char = '\u{e19a}';
    pub const STRIKE: char = '\u{e177}';
    pub const LIST: char = '\u{e106}';
    pub const LIST_ORDERED: char = '\u{e1d1}';
    pub const QUOTE: char = '\u{e239}';
    pub const LINK: char = '\u{e102}';
    pub const STAR: char = '\u{e176}';
    pub const THREADS: char = '\u{e40d}';
    pub const REPLY_ARROW: char = '\u{e0a2}';
    pub const FOLDER_TREE: char = '\u{e33c}';
    pub const FOLDER_PLUS: char = '\u{e0d9}';
    pub const SPELL: char = '\u{e49a}';
    pub const COPY: char = '\u{e09e}';
    pub const CHEVRON_LEFT: char = '\u{e06e}';
}

fn icon<'a>(glyph: char, size: u16) -> text::Text<'a> {
    let size = f32::from(size);
    text(glyph.to_string()).font(ICONS).size(size).line_height(1.0)
}

/// Toolbar button: icon with optional label, tooltip.
fn tool<'a>(glyph: char, label: &'a str, msg: Option<Msg>, tip: &'a str) -> El<'a> {
    let content: El<'a> = if label.is_empty() {
        icon(glyph, 16).into()
    } else {
        row![icon(glyph, 15), text(label).size(13)].spacing(6).align_y(Alignment::Center).into()
    };
    let b = button(content).padding([7, if label.is_empty() { 9 } else { 10 }]).style(theme::tool).on_press_maybe(msg);
    tooltip(b, container(text(tip).size(12)).padding([4, 8]).style(theme::boxed), tooltip::Position::Bottom).into()
}

fn icon_btn<'a>(glyph: char, msg: Msg, tip: &'a str) -> El<'a> {
    tool(glyph, "", Some(msg), tip)
}

/// Reader toggle for one of the two tags: a coloured marker plus its name,
/// filled while the tag is set. Keys `1` and `2` do the same.
fn tag_toggle<'a>(name: &'a str, color: Color, on: bool, msg: Msg, tip: &'a str) -> El<'a> {
    let marker = container(Space::new().width(9).height(9)).style(move |_| container::Style {
        background: on.then_some(Background::Color(color)),
        border: Border { color, width: 1.0, radius: 0.0.into() },
        ..Default::default()
    });
    let content = row![marker, text(name).size(13)].spacing(7).align_y(Alignment::Center);
    let b = button(content).padding([6, 9]).style(theme::tab(on)).on_press(msg);
    tooltip(b, container(text(tip).size(12)).padding([4, 8]).style(theme::boxed), tooltip::Position::Bottom).into()
}

/// List marker for a tag, with the tag's name on hover.
fn tag_marker<'a>(color: Color, tip: &'a str) -> El<'a> {
    tooltip(
        tagchip(color),
        container(text(tip).size(12)).padding([4, 8]).style(theme::boxed),
        tooltip::Position::Right,
    )
    .into()
}

fn count_badge<'a>(n: i64) -> El<'a> {
    text(n.to_string())
        .size(12)
        .font(SEMIBOLD)
        .style(|t: &iced::Theme| text::Style { color: Some(t.palette().primary) })
        .into()
}

fn folder_icon(role: FolderRole) -> char {
    match role {
        FolderRole::Inbox => ic::INBOX,
        FolderRole::Sent => ic::SEND,
        FolderRole::Drafts => ic::DRAFT,
        FolderRole::Archive | FolderRole::All => ic::ARCHIVE,
        FolderRole::Junk => ic::JUNK,
        FolderRole::Trash => ic::DELETE,
        FolderRole::Other => ic::FOLDER,
    }
}

/// Message row that can be dragged onto a folder.
fn drag_source<'a>(row: impl Into<El<'a>>, id: i64) -> El<'a> {
    iced::widget::mouse_area(row).on_enter(Msg::HoverRow(id)).on_exit(Msg::UnhoverRow(id)).into()
}

pub fn folder_key(account: &str, folder: &str) -> String {
    format!("{account}\u{1f}{folder}")
}

/// "a/b/c" → "a", "a/b"
fn ancestors(name: &str) -> impl Iterator<Item = &str> {
    name.match_indices(['/', '.']).map(move |(i, _)| &name[..i])
}

fn is_child(name: &str, parent: &str) -> bool {
    name.len() > parent.len() + 1
        && name.starts_with(parent)
        && matches!(name.as_bytes()[parent.len()], b'/' | b'.')
}

// ---- selectable, formatted mail text ---------------------------------------

/// Formatting ranges for one reader text block. Compared by pointer so the
/// editor doesn't re-diff large documents every frame.
#[derive(Debug, Clone)]
pub struct HlSettings(pub std::sync::Arc<Vec<Vec<(std::ops::Range<usize>, Hl)>>>, pub bool);

impl PartialEq for HlSettings {
    fn eq(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.0, &other.0)
    }
}

impl HlSettings {
    pub fn has_links(&self) -> bool {
        self.1
    }
}

pub struct RunHighlighter {
    lines: std::sync::Arc<Vec<Vec<(std::ops::Range<usize>, Hl)>>>,
    current: usize,
}

impl iced::advanced::text::Highlighter for RunHighlighter {
    type Settings = HlSettings;
    type Highlight = Hl;
    type Iterator<'a> = std::vec::IntoIter<(std::ops::Range<usize>, Hl)>;

    fn new(settings: &Self::Settings) -> Self {
        RunHighlighter { lines: settings.0.clone(), current: 0 }
    }
    fn update(&mut self, settings: &Self::Settings) {
        self.lines = settings.0.clone();
        self.current = 0;
    }
    fn change_line(&mut self, line: usize) {
        self.current = line;
    }
    fn highlight_line(&mut self, _line: &str) -> Self::Iterator<'_> {
        let v = self.lines.get(self.current).cloned().unwrap_or_default();
        self.current += 1;
        v.into_iter()
    }
    fn current_line(&self) -> usize {
        self.current
    }
}

/// Live formatting of composer markup: formatted text is shown bold /
/// italic / linked, the markers themselves are dimmed.
pub struct ComposeHighlighter {
    current: usize,
    issues: std::sync::Arc<Vec<(usize, std::ops::Range<usize>, crate::grammar::IssueKind)>>,
}

/// Highlighter settings: edit revision plus grammar issues per line.
#[derive(Debug, Clone, PartialEq)]
pub struct ComposeHl {
    pub revision: u64,
    pub issues: std::sync::Arc<Vec<(usize, std::ops::Range<usize>, crate::grammar::IssueKind)>>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CHl {
    /// Dimmed marker, drawn in the font of the text it wraps: cosmic-text
    /// picks one font per word, and "**bold" is a single word.
    Marker(crate::markup::Style),
    Style(crate::markup::Style),
    Link,
    Quote,
    /// Grammar/spelling issue, in the font of the text it covers.
    Issue(crate::grammar::IssueKind, crate::markup::Style),
}

impl iced::advanced::text::Highlighter for ComposeHighlighter {
    /// Any change re-highlights the whole draft.
    type Settings = ComposeHl;
    type Highlight = CHl;
    type Iterator<'a> = std::vec::IntoIter<(std::ops::Range<usize>, CHl)>;

    fn new(s: &ComposeHl) -> Self {
        ComposeHighlighter { current: 0, issues: s.issues.clone() }
    }
    fn update(&mut self, s: &ComposeHl) {
        self.current = 0;
        self.issues = s.issues.clone();
    }
    fn change_line(&mut self, line: usize) {
        self.current = line;
    }
    fn highlight_line(&mut self, line: &str) -> Self::Iterator<'_> {
        use crate::markup::{LineKind, Seg, inline, line_kind};
        let index = self.current;
        self.current += 1;
        let (kind, n) = line_kind(line);
        let mut v = Vec::new();
        if n > 0 {
            v.push((0..n, CHl::Marker(Default::default())));
        }
        let segs = inline(line, n);
        let style_of = |s: Option<&Seg<'_>>| match s {
            Some(Seg::Text(_, _, st)) | Some(Seg::Link { style: st, .. }) => Some(*st),
            _ => None,
        };
        for (i, seg) in segs.iter().cloned().enumerate() {
            match seg {
                Seg::Marker(r) => {
                    // Opening marker: style of what follows; closing: what precedes.
                    let next = style_of(segs.get(i + 1)).filter(|s| *s != Default::default());
                    let prev = i.checked_sub(1).and_then(|j| style_of(segs.get(j)));
                    v.push((r, CHl::Marker(next.or(prev).unwrap_or_default())));
                }
                Seg::Text(r, _, st) if kind == LineKind::Quote && st == Default::default() => v.push((r, CHl::Quote)),
                Seg::Text(r, _, st) if st != Default::default() => v.push((r, CHl::Style(st))),
                Seg::Link { range, .. } => v.push((range, CHl::Link)),
                _ => {}
            }
        }
        let issues: Vec<_> = self.issues.iter().filter(|(l, ..)| *l == index).collect();
        if issues.is_empty() {
            return v.into_iter();
        }
        // Overlay issues: split the formatted ranges at issue boundaries.
        let mut cuts: Vec<usize> = vec![0, line.len()];
        for (r, _) in &v {
            cuts.extend([r.start, r.end]);
        }
        for (_, r, _) in &issues {
            cuts.extend([r.start.min(line.len()), r.end.min(line.len())]);
        }
        cuts.sort_unstable();
        cuts.dedup();
        let mut out = Vec::new();
        for w in cuts.windows(2) {
            let (a, b) = (w[0], w[1]);
            if a >= b || !line.is_char_boundary(a) || !line.is_char_boundary(b) {
                continue;
            }
            let base = v.iter().find(|(r, _)| r.start <= a && b <= r.end).map(|(_, h)| *h);
            let issue = issues.iter().find(|(_, r, _)| r.start <= a && b <= r.end);
            match (issue, base) {
                (Some((_, _, kind)), base) => {
                    let st = match base {
                        Some(CHl::Style(st) | CHl::Marker(st)) => st,
                        _ => Default::default(),
                    };
                    out.push((a..b, CHl::Issue(*kind, st)));
                }
                (None, Some(h)) => out.push((a..b, h)),
                (None, None) => {}
            }
        }
        out.into_iter()
    }
    fn current_line(&self) -> usize {
        self.current
    }
}

fn style_font(st: crate::markup::Style) -> Font {
    let mut font = UI;
    if st.bold {
        font.weight = iced::font::Weight::Semibold;
    }
    if st.italic {
        font.style = iced::font::Style::Italic;
    }
    font
}

fn compose_format(h: &CHl, t: &iced::Theme) -> iced::advanced::text::highlighter::Format<Font> {
    use iced::advanced::text::highlighter::Format;
    match h {
        CHl::Marker(st) => Format { color: Some(Color { a: 0.35, ..t.palette().text }), font: Some(style_font(*st)) },
        CHl::Quote => Format { color: Some(theme::muted(t)), font: None },
        CHl::Link => Format { color: Some(t.palette().primary), font: None },
        CHl::Issue(kind, st) => {
            let color = match kind {
                crate::grammar::IssueKind::Style => t.palette().warning,
                _ => t.palette().danger,
            };
            Format { color: Some(color), font: Some(style_font(*st)) }
        }
        CHl::Style(st) => {
            // Underline/strikethrough can't be drawn inside the editor; tint them instead.
            let color = (st.underline || st.strike).then(|| t.palette().primary);
            Format { color, font: Some(style_font(*st)) }
        }
    }
}

fn hl_format(h: &Hl, t: &iced::Theme) -> iced::advanced::text::highlighter::Format<Font> {
    use iced::advanced::text::highlighter::Format;
    match h {
        Hl::Strong => Format { color: None, font: Some(SEMIBOLD) },
        Hl::Link => Format { color: Some(t.palette().primary), font: None },
        Hl::Code => Format { color: None, font: Some(Font::MONOSPACE) },
        // Background highlights can't be drawn in the editor; tint the text.
        Hl::Mark(rgb) => Format { color: theme::text_ink(*rgb, t), font: None },
        // Foreground colours from the HTML: adapted for readability here.
        Hl::Colour(rgb) => Format { color: theme::text_ink(*rgb, t), font: None },
        Hl::Muted => Format { color: Some(theme::muted(t)), font: None },
    }
}

impl App {
    pub fn view(&self) -> El<'_> {
        let main: El<'_> = match &self.screen {
            Screen::Mail => row![self.list_pane(), vr(), self.reader()].into(),
            Screen::Compose(c) => self.composer(c),
            Screen::Settings(s) => self.settings(s),
            Screen::Contacts(c) => self.contacts(c),
        };
        let app: El<'_> = container(column![row![self.sidebar(), vr(), main].height(Fill), hr(), self.status_bar()])
            .style(theme::base)
            .into();
        // Same widget structure with or without a drag, so scroll positions
        // and text selection survive it.
        let compose = match &self.screen { Screen::Compose(c) => Some(&**c), _ => None };
        let mail = self.drag.as_ref().filter(|d| d.active);
        let addr = compose.and_then(|c| c.addr_drag.as_ref().filter(|d| d.active));
        let floating: El<'_> = if let Some(d) = mail {
            // Label following the cursor while mail is dragged onto a folder.
            let n = d.ids.len();
            let what = if n == 1 {
                self.list.iter().find(|s| Some(&s.id) == d.ids.first()).map(|s| s.subject.clone()).unwrap_or_default()
            } else {
                format!("{n} messages")
            };
            let target = self.hovered.as_deref().and_then(|k| k.split_once('\u{1f}')).map(|(_, f)| display_folder(f));
            let label = match target {
                Some(f) => format!("Move to {f}"),
                None => "Drop on a folder".to_string(),
            };
            app_drag_chip(label, Some(what), d.pos)
        } else if let Some(d) = addr {
            let c = compose.unwrap();
            let text = match d.field {
                Field::To => c.to.clone(),
                Field::Cc => c.cc.clone(),
                Field::Bcc => c.bcc.clone(),
            };
            let (chips, _) = crate::app::addr_parts(&text);
            let what = chips.get(d.idx).copied().unwrap_or_default().to_string();
            let label = match c.addr_drop {
                Some(f) => format!("Move to {}", match f { Field::To => "To", Field::Cc => "Cc", Field::Bcc => "Bcc" }),
                None => "Drop on a recipient line".to_string(),
            };
            app_drag_chip(label, Some(what), d.pos)
        } else {
            Space::new().into()
        };
        let mut root = iced::widget::mouse_area(iced::widget::stack![app, floating]);
        if mail.is_some() || addr.is_some() {
            root = root.interaction(iced::mouse::Interaction::Grabbing);
        }
        root.into()
    }

    // ---- status bar --------------------------------------------------------

    fn status_bar(&self) -> El<'_> {
        let mut left = Row::new().spacing(14).align_y(Alignment::Center);
        left = left.push(tooltip(
            button(icon(ic::SYNC, 12)).padding([3, 5]).style(theme::tool).on_press(Msg::SyncAll),
            container(text("Sync all accounts (F5)").size(12)).padding([4, 8]).style(theme::boxed),
            tooltip::Position::Top,
        ));
        for acc in &self.cfg.accounts {
            let live = self.live.contains(&acc.id);
            let state = if self.syncing.contains(&acc.id) {
                "syncing…"
            } else if live {
                "live"
            } else {
                "periodic"
            };
            left = left.push(
                row![
                    container(Space::new().width(7).height(7)).style(move |t: &iced::Theme| container::Style {
                        background: Some(if live { t.palette().success } else { theme::muted(t) }.into()),
                        ..Default::default()
                    }),
                    muted(format!("{} · {state}", acc.name)).size(12),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            );
        }
        if let Some(ts) = self.last_sync {
            left = left.push(muted(format!("Updated {}", short_date(ts))).size(12));
        }
        let right: El<'_> = match &self.status {
            None => Space::new().into(),
            Some(st) => {
                let (msg, is_err) = match st {
                    Ok(m) => (m.as_str(), false),
                    Err(e) => (e.as_str(), true),
                };
                row![
                    text(msg).size(12).wrapping(text::Wrapping::None).style(move |t: &iced::Theme| text::Style {
                        color: Some(if is_err { t.palette().danger } else { t.palette().text })
                    }),
                    button(text("Dismiss").size(12)).padding([2, 8]).style(theme::tool).on_press(Msg::DismissStatus),
                ]
                .spacing(8)
                .align_y(Alignment::Center)
                .into()
            }
        };
        container(row![left, Space::new().width(Fill), right].align_y(Alignment::Center))
            .height(28)
            .padding([0, 12])
            .center_y(28)
            .width(Fill)
            .style(theme::pane)
            .into()
    }

    // ---- sidebar -----------------------------------------------------------

    fn sidebar(&self) -> El<'_> {
        use crate::config::SidebarMode;
        let compose = button(
            container(
                row![icon(ic::COMPOSE, 15), text("New message").font(SEMIBOLD).size(14)]
                    .spacing(8)
                    .align_y(Alignment::Center),
            )
            .center_x(Fill),
        )
        .padding([8, 0])
        .width(Fill)
        .style(theme::primary)
        .on_press(Msg::NewMail);

        let mode = self.cfg.sidebar_mode;
        let (mode_icon, mode_tip) = match mode {
            SidebarMode::Collapsed => (ic::COLLAPSE_ALL, "Collapsed — click to show favorites"),
            SidebarMode::Favorites => (ic::STAR, "Favorites — click to show all folders"),
            SidebarMode::All => (ic::FOLDER_TREE, "All folders — click to collapse"),
        };
        let mode_label = match mode {
            SidebarMode::Collapsed => "Collapsed",
            SidebarMode::Favorites => "Favorites",
            SidebarMode::All => "All folders",
        };
        let mut c = Column::new().padding(iced::Padding { top: 4.0, bottom: 8.0, ..Default::default() });
        c = c.push(
            container(
                row![
                    muted("MAILBOXES").size(11).font(SEMIBOLD).width(Fill),
                    tooltip(
                        button(row![muted(mode_label).size(11), icon(mode_icon, 14)].spacing(6).align_y(Alignment::Center))
                            .padding([4, 6])
                            .style(theme::tool)
                            .on_press(Msg::CycleSidebar),
                        container(text(mode_tip).size(12)).padding([4, 8]).style(theme::boxed),
                        tooltip::Position::Bottom,
                    ),
                ]
                .align_y(Alignment::Center),
            )
            .padding(iced::Padding { top: 6.0, bottom: 2.0, left: 16.0, right: 10.0 }),
        );
        if self.cfg.accounts.len() > 1 {
            let unread: i64 = self
                .folders
                .iter()
                .flat_map(|(_, f)| f)
                .filter(|f| f.role == FolderRole::Inbox)
                .map(|f| f.unread)
                .sum();
            let sel = Selection::Unified(FolderRole::Inbox);
            c = c.push(self.nav_row(ic::INBOXES, "All inboxes".into(), unread, Some(Msg::Select(sel.clone())), self.is_selected(&sel), 0, None, None));
        }
        for (acc_id, folders) in &self.folders {
            let Some(acc) = self.account(acc_id) else { continue };
            let collapsed = self.collapsed.contains(acc_id);
            let inbox_unread: i64 = folders.iter().filter(|f| f.role == FolderRole::Inbox).map(|f| f.unread).sum();
            let badge: El<'_> = if collapsed && inbox_unread > 0 { count_badge(inbox_unread) } else { Space::new().into() };
            c = c.push(
                button(
                    row![
                        icon(if collapsed { ic::CHEVRON_RIGHT } else { ic::CHEVRON_DOWN }, 13)
                            .style(|t: &iced::Theme| text::Style { color: Some(theme::muted(t)) }),
                        muted(acc.name.to_uppercase()).size(11).font(SEMIBOLD).wrapping(text::Wrapping::None),
                        self.status_dot(acc_id),
                        Space::new().width(Fill),
                        badge,
                        button(icon(ic::FOLDER_PLUS, 13))
                            .padding([1, 5])
                            .style(theme::link)
                            .on_press(Msg::NewFolderStart(acc_id.clone())),
                    ]
                    .spacing(6)
                    .align_y(Alignment::Center),
                )
                .width(Fill)
                .padding(iced::Padding { top: 10.0, bottom: 6.0, left: 14.0, right: 22.0 })
                .style(theme::tool)
                .on_press(Msg::ToggleCollapsed(acc_id.clone())),
            );
            if collapsed {
                continue;
            }
            if let Some((id, name)) = &self.new_folder {
                if id == acc_id {
                    c = c.push(
                        row![
                            input("Folder name", name)
                                .id(iced::widget::Id::new("new-folder-input"))
                                .on_input(Msg::NewFolderInput)
                                .on_submit(Msg::NewFolderSubmit)
                                .padding([5, 8])
                                .size(13)
                                .style(theme::input)
                                .width(Fill),
                            button(icon(ic::CLOSE, 12))
                                .padding([1, 7])
                                .style(theme::link)
                                .on_press(Msg::NewFolderCancel),
                        ]
                        .spacing(6)
                        .align_y(Alignment::Center)
                        .padding(iced::Padding { top: 0.0, bottom: 6.0, left: 30.0, right: 16.0 }),
                    );
                }
            }
            for f in folders {
                let key = folder_key(acc_id, &f.name);
                let sel = Selection::Folder { account: acc_id.clone(), name: f.name.clone() };
                let selected = self.is_selected(&sel);
                let favorite = self.favorites.contains(&key);
                if mode == SidebarMode::Favorites
                    && !matches!(f.role, FolderRole::Inbox | FolderRole::Sent)
                    && !favorite
                    && !selected
                {
                    continue;
                }
                // Hide sub-folders of a collapsed parent (only in the full tree).
                let tree = mode == SidebarMode::All;
                if tree
                    && f.role == FolderRole::Other
                    && ancestors(&f.name).any(|p| self.collapsed.contains(&folder_key(acc_id, p)))
                {
                    continue;
                }
                let label = match f.role {
                    FolderRole::Inbox => "Inbox".to_string(),
                    FolderRole::Other if !tree => folder_path(&f.name),
                    _ => display_folder(&f.name),
                };
                let depth = if tree && f.role == FolderRole::Other { f.name.matches(['/', '.']).count() } else { 0 };
                let unread =
                    if matches!(f.role, FolderRole::Sent | FolderRole::Trash | FolderRole::Junk | FolderRole::Drafts) {
                        0
                    } else {
                        f.unread
                    };
                let has_children = tree && folders.iter().any(|g| g.role == FolderRole::Other && is_child(&g.name, &f.name));
                let toggle = has_children.then(|| (self.collapsed.contains(&key), key.clone()));
                // Star: always on favorites in the full tree, on hover otherwise.
                let hovered = self.hovered.as_deref() == Some(key.as_str());
                let star = (hovered || (favorite && tree)).then(|| (favorite, key.clone()));
                let drop_target = hovered && self.drag.as_ref().is_some_and(|d| d.active);
                let row_el = self.nav_row(
                    folder_icon(f.role),
                    label,
                    unread,
                    Some(Msg::Select(sel.clone())),
                    selected || drop_target,
                    depth,
                    toggle,
                    star,
                );
                c = c.push(
                    iced::widget::mouse_area(row_el)
                        .on_enter(Msg::HoverFolder(Some(key.clone())))
                        .on_exit(Msg::Unhover(key.clone()))
                        .on_release(Msg::DroppedOn(key.clone())),
                );
            }
            if mode == SidebarMode::Favorites && !folders.iter().any(|f| self.favorites.contains(&folder_key(acc_id, &f.name))) {
                c = c.push(
                    container(muted("Hover a folder in \u{201c}All folders\u{201d} and click the star to pin it here.").size(11))
                        .padding(iced::Padding { top: 2.0, bottom: 6.0, left: 44.0, right: 16.0 }),
                );
            }
        }
        if self.cfg.accounts.is_empty() {
            c = c.push(container(muted("No accounts yet.").size(13)).padding([8, 16]));
        }

        let footer = column![
            self.nav_row(ic::CONTACTS, "Address book".into(), 0, Some(Msg::ShowContacts), matches!(self.screen, Screen::Contacts(_)), 0, None, None),
            self.nav_row(ic::SETTINGS, "Settings".into(), 0, Some(Msg::ShowSettings), matches!(self.screen, Screen::Settings(_)), 0, None, None),
        ]
        .padding([6, 0]);

        container(column![
            header(compose),
            hr(),
            scrollable(c).height(Fill).style(theme::scroll),
            hr(),
            footer
        ])
        .width(SIDEBAR_W)
        .height(Fill)
        .style(theme::pane)
        .into()
    }

    /// Green: connected (live IDLE or last sync OK). Red: last attempt failed.
    /// Grey: not connected yet.
    fn status_dot(&self, acc_id: &str) -> El<'_> {
        let live = self.live.contains(acc_id);
        let status = self.acc_status.get(acc_id);
        let (tip, kind) = match status {
            Some(Err(e)) => (format!("Offline — {e}"), 2u8),
            Some(Ok(())) if live => ("Online · new mail arrives instantly".to_string(), 0),
            Some(Ok(())) => ("Online · checked periodically".to_string(), 0),
            None if self.syncing.contains(acc_id) => ("Connecting…".to_string(), 1),
            None => ("Not connected yet".to_string(), 1),
        };
        let dot = container(Space::new().width(7).height(7)).style(move |t: &iced::Theme| container::Style {
            background: Some(
                match kind {
                    0 => t.palette().success,
                    2 => t.palette().danger,
                    _ => theme::muted(t),
                }
                .into(),
            ),
            border: iced::Border { radius: 4.0.into(), ..Default::default() },
            ..Default::default()
        });
        tooltip(dot, container(text(tip).size(12)).padding([4, 8]).style(theme::boxed), tooltip::Position::Right).into()
    }

    fn is_selected(&self, sel: &Selection) -> bool {
        self.selection == *sel && !self.results && matches!(self.screen, Screen::Mail)
    }

    #[allow(clippy::too_many_arguments)]
    fn nav_row(
        &self,
        glyph: char,
        label: String,
        unread: i64,
        msg: Option<Msg>,
        selected: bool,
        depth: usize,
        // (collapsed, key) for folders with sub-folders.
        toggle: Option<(bool, String)>,
        // (is favorite, key): show the favorite star.
        star: Option<(bool, String)>,
    ) -> El<'_> {
        let star_btn: El<'_> = match star {
            None => Space::new().into(),
            Some((fav, key)) => tooltip(
                button(icon(ic::STAR, 12).style(move |t: &iced::Theme| text::Style {
                    color: Some(if fav { t.palette().warning } else { theme::muted(t) })
                }))
                .padding([4, 4])
                .style(theme::tool)
                .on_press(Msg::ToggleFavorite(key)),
                container(text(if fav { "Remove from favorites" } else { "Add to favorites" }).size(12))
                    .padding([4, 8])
                    .style(theme::boxed),
                tooltip::Position::Right,
            )
            .into(),
        };
        let count: El<'_> = if unread > 0 { count_badge(unread) } else { Space::new().into() };
        // Collapse chevron for parents: on the left (like the account header),
        // nested so it toggles without selecting — and clear of the scrollbar
        // that overlaps the row edges on the right.
        let head: El<'_> = match toggle {
            None => Space::new().width(4).into(),
            Some((collapsed, key)) => button(
                icon(if collapsed { ic::CHEVRON_RIGHT } else { ic::CHEVRON_DOWN }, 13)
                    .style(|t: &iced::Theme| text::Style { color: Some(theme::muted(t)) }),
            )
            .padding([2, 4])
            .style(theme::tool)
            .on_press(Msg::ToggleCollapsed(key))
            .into(),
        };
        let main = button(
            row![
                accent_bar(selected, 34.0),
                Space::new().width(13.0 + depth as f32 * 14.0),
                head,
                icon(glyph, 15).style(move |t: &iced::Theme| text::Style {
                    color: Some(if selected { t.palette().primary } else { theme::muted(t) })
                }),
                Space::new().width(10),
                text(label)
                    .size(14)
                    .width(Fill)
                    .wrapping(text::Wrapping::None)
                    .font(if unread > 0 || selected { SEMIBOLD } else { UI }),
                star_btn,
                Space::new().width(4),
                count,
                Space::new().width(16),
            ]
            .align_y(Alignment::Center),
        )
        .width(Fill)
        .padding(0)
        .style(theme::row(selected))
        .on_press_maybe(msg);
        main.into()
    }

    // ---- message list ------------------------------------------------------

    fn list_pane(&self) -> El<'_> {
        let mut search = row![
            input("Search all mail   /", &self.query)
                .icon(text_input::Icon {
                    font: ICONS,
                    code_point: ic::SEARCH,
                    size: Some(14.into()),
                    spacing: 8.0,
                    side: text_input::Side::Left,
                })
                .id(SEARCH_ID)
                .on_input(Msg::Query)
                .on_submit(Msg::SubmitQuery)
                .width(Fill)
        ]
        .spacing(4)
        .align_y(Alignment::Center);
        if self.cfg.ai.enabled {
            search = search.push(tool(
                ic::AI,
                if self.ai_busy { "…" } else { "Ask AI" },
                (!self.ai_busy && !self.query.is_empty()).then_some(Msg::AiSearch),
                "Turn your question into a search",
            ));
        }
        if self.results {
            search = search.push(tool(ic::CLOSE, "", Some(Msg::ClearQuery), "Clear search (Esc)"));
        }

        let title = if self.results {
            "Search results".to_string()
        } else {
            match &self.selection {
                Selection::Unified(_) => "All inboxes".into(),
                Selection::Folder { name, .. } if name.eq_ignore_ascii_case("INBOX") => "Inbox".into(),
                Selection::Folder { name, .. } => display_folder(name),
            }
        };
        let unread = self.list.iter().filter(|s| !s.seen).count();
        let sub = container(
            row![
                text(title).font(SEMIBOLD).size(14).width(Fill),
                muted(if unread > 0 {
                    format!("{unread} unread · {}", self.list.len())
                } else {
                    format!("{}", self.list.len())
                })
                .size(12),
                tooltip(
                    button(icon(ic::THREADS, 14).style({
                        let on = self.cfg.threads;
                        move |t: &iced::Theme| text::Style {
                            color: Some(if on { t.palette().primary } else { theme::muted(t) }),
                        }
                    }))
                    .padding([4, 6])
                    .style(theme::tool)
                    .on_press(Msg::ToggleThreads),
                    container(text(if self.cfg.threads { "Grouped by conversation — click to show a flat list" } else { "Flat list — click to group by conversation" }).size(12))
                        .padding([4, 8])
                        .style(theme::boxed),
                    tooltip::Position::Bottom,
                ),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .padding([0, 16])
        .height(34)
        .center_y(34);

        let mut rows = Column::new();
        for r in &self.rows {
            rows = rows.push(self.list_row(r));
            rows = rows.push(hr());
        }
        if self.list.is_empty() {
            rows = rows
                .push(container(muted(if self.results { "Nothing found." } else { "No messages." }).size(13)).padding(24));
        } else if self.list.len() >= self.limit {
            rows = rows.push(container(secondary_btn("Load more", Some(Msg::MoreRows))).center_x(Fill).padding(12));
        }

        column![header(search), hr(), sub, hr(), scrollable(rows).height(Fill).style(theme::scroll)]
            .width(LIST_W)
            .into()
    }

    fn list_row<'a>(&'a self, r: &'a ListRow) -> El<'a> {
        let s: &'a Summary = &self.list[r.idx];
        let selected = self.selected == Some(s.id);
        let from = if s.from_name.is_empty() { s.from_addr.as_str() } else { s.from_name.as_str() };
        let unread = r.any_unread;
        if r.depth > 0 {
            return drag_source(self.thread_child(s, selected), s.id);
        }
        let dot: El<'a> = if unread {
            container(Space::new().width(7).height(7))
                .style(|t: &iced::Theme| container::Style {
                    background: Some(t.palette().primary.into()),
                    ..Default::default()
                })
                .into()
        } else {
            Space::new().into()
        };

        let mut flags = Row::new().spacing(6).align_y(Alignment::Center);
        if s.has_attachments {
            flags = flags.push(icon(ic::ATTACHMENT, 13).style(|t: &iced::Theme| text::Style { color: Some(theme::muted(t)) }));
        }
        if s.answered {
            flags = flags.push(tooltip(
                icon(ic::REPLY, 13).style(|t: &iced::Theme| text::Style { color: Some(t.palette().primary) }),
                container(text("Answered").size(12)).padding([4, 8]).style(theme::boxed),
                tooltip::Position::Right,
            ));
        }
        if s.flagged {
            flags = flags.push(icon(ic::FLAG, 13).style(|t: &iced::Theme| text::Style { color: Some(t.palette().warning) }));
        }
        if s.tags & crate::store::TAG_TODO != 0 {
            flags = flags.push(tag_marker(theme::CHIP_TODO, "Todo (press 1)"));
        }
        if s.tags & crate::store::TAG_IMPORTANT != 0 {
            flags = flags.push(tag_marker(theme::CHIP_IMPORTANT, "Important (press 2)"));
        }
        if s.tags & crate::store::TAG_AGENT != 0 {
            flags = flags.push(tag_marker(theme::CHIP_AGENT, "AgentProcessed"));
        }
        // Single-line texts are clipped in their own box so they never run
        // under the date or the attachment/flag icons.
        let line = |t: text::Text<'a>| container(t.wrapping(text::Wrapping::None)).width(Fill).clip(true);
        let content = column![
            row![
                line(text(from).font(if unread { BOLD } else { UI }).size(14)),
                thread_badge(r, !self.folded.contains(&r.key)),
                muted(short_date(s.date)).size(12),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            line(text(&s.subject).size(13).font(if unread { SEMIBOLD } else { UI })),
            row![line(muted(&s.preview).size(12)), flags].spacing(8).align_y(Alignment::Center),
        ]
        .spacing(3);

        let row = button(
            row![
                accent_bar(selected, 74.0),
                container(dot).width(22).padding(iced::Padding { top: 6.0, left: 8.0, ..Default::default() }).height(Fill),
                content,
                Space::new().width(14),
            ]
            .height(74)
            .align_y(Alignment::Center),
        )
        .width(Fill)
        .padding(0)
        .style(theme::row(selected))
        .on_press(Msg::Open(s.id));
        drag_source(row, s.id)
    }

    /// Earlier message of a conversation, indented under its newest message.
    fn thread_child<'a>(&'a self, s: &'a Summary, selected: bool) -> El<'a> {
        let from = if s.from_name.is_empty() { s.from_addr.as_str() } else { s.from_name.as_str() };
        let unread = !s.seen;
        let line = |t: text::Text<'a>| container(t.wrapping(text::Wrapping::None)).width(Fill).clip(true);
        let dot: El<'a> = if unread {
            container(Space::new().width(6).height(6))
                .style(|t: &iced::Theme| container::Style {
                    background: Some(t.palette().primary.into()),
                    ..Default::default()
                })
                .into()
        } else {
            Space::new().into()
        };
        button(
            row![
                accent_bar(selected, 50.0),
                Space::new().width(22),
                icon(ic::REPLY_ARROW, 12).style(|t: &iced::Theme| text::Style { color: Some(theme::muted(t)) }),
                container(dot).width(16).center_x(16),
                column![
                    row![line(text(from).font(if unread { SEMIBOLD } else { UI }).size(13)), muted(short_date(s.date)).size(11)]
                        .spacing(8)
                        .align_y(Alignment::Center),
                    line(muted(&s.preview).size(12)),
                ]
                .spacing(2),
                Space::new().width(14),
            ]
            .height(50)
            .align_y(Alignment::Center),
        )
        .width(Fill)
        .padding(0)
        .style(theme::row(selected))
        .on_press(Msg::Open(s.id))
        .into()
    }

    // ---- reader ------------------------------------------------------------

    fn reader(&self) -> El<'_> {
        let Some(m) = &self.open else {
            let hint = if self.cfg.accounts.is_empty() {
                "Add an account in Settings to get started."
            } else {
                "No message selected"
            };
            return column![
                header(Space::new()),
                hr(),
                container(
                    column![
                        icon(ic::MAIL, 40).style(|t: &iced::Theme| text::Style { color: Some(theme::muted(t)) }),
                        muted(hint).size(14),
                        muted("j / k navigate · Ctrl+N new message · / search").size(12)
                    ]
                    .spacing(10)
                    .align_x(Alignment::Center)
                )
                .center(Fill)
            ]
            .into();
        };
        let s = &m.summary;
        let mut actions = row![];
        if self.is_draft(&m.summary) {
            actions = actions.push(tool(ic::COMPOSE, "Edit", Some(Msg::EditDraft), "Continue writing this draft"));
        }
        let mut actions = actions.extend([
            tool(ic::REPLY, "Reply", Some(Msg::Reply { all: false }), "Reply (Ctrl+R)"),
            tool(ic::REPLY_ALL, "Reply all", Some(Msg::Reply { all: true }), "Reply all (Ctrl+Shift+R)"),
            tool(ic::FORWARD, "Forward", Some(Msg::Forward), "Forward (Ctrl+L)"),
            container(vr()).height(20).padding([0, 6]).into(),
            tool(ic::ARCHIVE, "", Some(Msg::Archive), "Archive (e)"),
            tool(ic::DELETE, "", Some(Msg::Delete), "Delete (Del)"),
            tool(if s.flagged { ic::UNFLAG } else { ic::FLAG }, "", Some(Msg::ToggleFlag), if s.flagged { "Remove flag (s)" } else { "Flag (s)" }),
            tool(if s.seen { ic::MAIL } else { ic::MAIL_OPEN }, "", Some(Msg::ToggleSeen), if s.seen { "Mark as unread (u)" } else { "Mark as read (u)" }),
            container(vr()).height(20).padding([0, 6]).into(),
            tag_toggle(
                "Todo",
                theme::CHIP_TODO,
                s.tags & crate::store::TAG_TODO != 0,
                Msg::ToggleTag(crate::store::TAG_TODO),
                if s.tags & crate::store::TAG_TODO != 0 { "Remove the Todo tag (1)" } else { "Tag as Todo (1)" },
            ),
            tag_toggle(
                "Important",
                theme::CHIP_IMPORTANT,
                s.tags & crate::store::TAG_IMPORTANT != 0,
                Msg::ToggleTag(crate::store::TAG_IMPORTANT),
                if s.tags & crate::store::TAG_IMPORTANT != 0 { "Remove the Important tag (2)" } else { "Tag as Important (2)" },
            ),
        ])
        .spacing(2)
        .align_y(Alignment::Center);
        if self.cfg.ai.enabled {
            actions = actions.push(container(vr()).height(20).padding([0, 6]));
            actions = actions.push(tool(
                ic::AI,
                if self.ai_busy { "Summarizing…" } else { "Summary" },
                (!self.ai_busy).then_some(Msg::Summarize),
                "Summarize with AI",
            ));
        }

        let name = if s.from_name.is_empty() { &s.from_addr } else { &s.from_name };
        let avatar = container(text(initials(name)).font(SEMIBOLD).size(15))
            .width(42)
            .height(42)
            .center(42)
            .style(theme::avatar);
        let mut people = column![
            row![text(name).font(SEMIBOLD).size(15).width(Fill), muted(full_date(s.date)).size(12)]
                .align_y(Alignment::Center),
            muted(&s.from_addr).size(12),
            muted(format!("To: {}", m.to)).size(12),
        ]
        .spacing(2)
        .width(Fill);
        if !m.cc.is_empty() {
            people = people.push(muted(format!("Cc: {}", m.cc)).size(12));
        }

        let mut body = Column::new().spacing(18).padding([24, 28]).max_width(880);
        body = body.push(text(&s.subject).size(22).font(SEMIBOLD));
        body = body.push(row![avatar, people].spacing(12).align_y(Alignment::Start));

        // Attachments
        if let Some(p) = &self.parsed {
            let files: Vec<_> = p.files().collect();
            if !files.is_empty() {
                let mut chips = Row::new().spacing(8);
                for (i, a) in files {
                    chips = chips.push(
                        container(
                            row![
                                icon(ic::FILE, 18).style(|t: &iced::Theme| text::Style { color: Some(theme::muted(t)) }),
                                column![
                                    container(text(&a.name).size(13).wrapping(text::Wrapping::None)).width(Fill).clip(true),
                                    muted(format!("{} · {}", a.size_label(), a.mime)).size(11).wrapping(text::Wrapping::None),
                                ]
                                .spacing(2)
                                .width(Length::Fixed(170.0))
                                .clip(true),
                                icon_btn(ic::OPEN, Msg::OpenAttachment(i), "Open"),
                                icon_btn(ic::DOWNLOAD, Msg::SaveAttachment(i), "Save as…"),
                            ]
                            .spacing(8)
                            .align_y(Alignment::Center),
                        )
                        .padding([8, 10])
                        .style(theme::boxed),
                    );
                }
                body = body.push(chips.wrap().vertical_spacing(8));
            }
        } else if !m.attachments.is_empty() {
            body = body.push(muted(format!("Attachments: {}", m.attachments.join(", "))).size(12));
        }

        // View options
        let html = self.parsed.as_ref().filter(|p| p.html.is_some());
        if let Some(p) = html {
            let mut opts = row![
                button(text("Formatted").size(12))
                    .padding([4, 10])
                    .style(theme::tab(self.show_html))
                    .on_press_maybe((!self.show_html).then_some(Msg::ToggleHtml)),
                button(text("Plain text").size(12))
                    .padding([4, 10])
                    .style(theme::tab(!self.show_html))
                    .on_press_maybe(self.show_html.then_some(Msg::ToggleHtml)),
                Space::new().width(Fill),
            ]
            .spacing(2)
            .align_y(Alignment::Center);
            opts = opts.push(
                button(row![icon(ic::OPEN, 13), text("Open in browser").size(12)].spacing(6).align_y(Alignment::Center))
                    .padding([4, 6])
                    .style(theme::link)
                    .on_press(Msg::OpenInBrowser),
            );
            body = body.push(opts);
            let remote = p.remote_images().len();
            if self.show_html && remote > 0 && self.remote_images != Some(true) {
                let loading = self.remote_images == Some(false);
                body = body.push(
                    container(
                        row![
                            icon(ic::IMAGE, 15),
                            text(format!(
                                "{remote} remote image{} blocked to protect your privacy.",
                                if remote == 1 { "" } else { "s" }
                            ))
                            .size(13)
                            .width(Fill),
                            secondary_btn(if loading { "Loading…" } else { "Show images" }, (!loading).then_some(Msg::LoadRemoteImages)),
                        ]
                        .spacing(10)
                        .align_y(Alignment::Center),
                    )
                    .padding([6, 12])
                    .width(Fill)
                    .style(theme::note),
                );
            }
        }
        // The mail's raw source carries an AgentProcessed field: yellow badge.
        if let Some(tags) = self.open.as_ref().map(|m| m.summary.tags) {
            if tags & crate::store::TAG_AGENT != 0 {
                body = body.push(
                    container(
                        row![tagchip(theme::CHIP_AGENT), text("AgentProcessed").size(13)]
                            .spacing(8)
                            .align_y(Alignment::Center),
                    )
                    .padding([6, 12])
                    .width(Fill)
                    .style(theme::note),
                );
            }
        }
        if let Some(sum) = &self.summary {
            body = body.push(container(text(sum).size(13)).padding(14).width(Fill).style(theme::note));
        }
        body = body.push(hr());
        if self.reader.is_empty() && self.loading {
            body = body.push(muted("Loading…"));
        }
        let mut content = Column::new().spacing(12);
        for (i, b) in self.reader.iter().enumerate() {
            content = content.push(match b {
                ReaderBlock::Text { content, settings } => text_editor(content)
                    .on_action(move |a| Msg::ReaderAction(i, a))
                    .highlight_with::<RunHighlighter>(settings.clone(), hl_format)
                    .padding(0)
                    .size(14)
                    .line_height(1.5)
                    .style(theme::reader)
                    .into(),
                ReaderBlock::Image { src, link } => match self.images.get(src) {
                    None => El::from(Space::new()),
                    Some(h) => {
                        let img = iced::widget::image(h.clone()).content_fit(iced::ContentFit::ScaleDown);
                        // Right-click copies the image (the click stays with the
                        // link button below).
                        let img = iced::widget::mouse_area(img).on_right_press(Msg::CopyImage(src.clone()));
                        match link {
                            Some(url) => button(img).padding(0).style(theme::link).on_press(Msg::Link(url.clone())).into(),
                            None => img.into(),
                        }
                    }
                },
            });
        }
        body = body.push(content);
        let has_images =
            self.reader.iter().any(|b| matches!(b, ReaderBlock::Image { src, .. } if self.images.contains_key(src)));
        if has_images {
            body = body.push(muted("Right-click an image to copy it.").size(11));
        }
        if self.reader.iter().any(|b| matches!(b, ReaderBlock::Text { settings, .. } if settings.has_links())) {
            body = body.push(muted("Ctrl+click a link to open it").size(11));
        }

        column![header(actions.wrap()), hr(), scrollable(container(body).width(Fill)).height(Fill).style(theme::scroll)]
            .width(Fill)
            .into()
    }

    // ---- composer ----------------------------------------------------------

    /// Grammar check: the issue at the cursor with its suggestions.
    fn grammar_bar<'a>(&'a self, c: &'a Compose) -> El<'a> {
        let g = &c.grammar;
        let small = |s: String| muted(s).size(12);
        let content: El<'a> = if let Some(e) = &g.error {
            let hint = if self.cfg.grammar.server.contains("localhost") {
                " — start it with: docker run -d -p 8010:8010 erikvl87/languagetool, or pick the public server in Settings → General"
            } else {
                ""
            };
            row![
                icon(ic::SPELL, 13).style(|t: &iced::Theme| text::Style { color: Some(t.palette().danger) }),
                small(format!("Grammar check unavailable: {e}{hint}")).width(Fill),
            ]
            .spacing(8)
            .align_y(Alignment::Center)
            .into()
        } else if let Some(i) = current_issue(c) {
            let issue = &g.issues[i];
            let n = g.issues.len();
            let word = g.text.get(issue.range.clone()).unwrap_or("");
            let mut fixes = Row::new().spacing(4).align_y(Alignment::Center);
            for r in &issue.replacements {
                let label = if r.is_empty() { "(remove)".to_string() } else { r.clone() };
                fixes = fixes.push(
                    button(text(label).size(12).font(SEMIBOLD))
                        .padding([3, 8])
                        .style(theme::secondary)
                        .on_press(Msg::GrammarFix(i, r.clone())),
                );
            }
            let dot_color = issue.kind;
            row![
                container(Space::new().width(7).height(7)).style(move |t: &iced::Theme| container::Style {
                    background: Some(
                        match dot_color {
                            crate::grammar::IssueKind::Style => t.palette().warning,
                            _ => t.palette().danger,
                        }
                        .into()
                    ),
                    ..Default::default()
                }),
                text(format!("\u{201c}{word}\u{201d}")).size(12).font(SEMIBOLD),
                container(small(issue.message.clone()).wrapping(text::Wrapping::None)).width(Fill).clip(true),
                fixes,
                button(text("Ignore").size(12)).style(theme::link).on_press(Msg::GrammarIgnore(i)),
                Space::new().width(6),
                tool(ic::CHEVRON_LEFT, "", Some(Msg::GrammarGo((i + n - 1) % n)), "Previous issue"),
                small(format!("{} / {n}", i + 1)),
                tool(ic::CHEVRON_RIGHT, "", Some(Msg::GrammarGo((i + 1) % n)), "Next issue"),
            ]
            .spacing(8)
            .align_y(Alignment::Center)
            .into()
        } else {
            let note = if g.busy || g.due.is_some() { "Checking…" } else { "No grammar or spelling issues" };
            row![icon(ic::SPELL, 13).style(|t: &iced::Theme| text::Style { color: Some(theme::muted(t)) }), small(note.into())]
                .spacing(8)
                .align_y(Alignment::Center)
                .into()
        };
        container(content).padding([5, 16]).width(Fill).height(36).align_y(Alignment::Center).into()
    }

    fn composer<'a>(&'a self, c: &'a Compose) -> El<'a> {
        let acc = self.account(&c.account);
        let ids: Vec<(String, String)> = self.cfg.accounts.iter().map(|a| (a.name.clone(), a.id.clone())).collect();
        let names: Vec<String> = ids.iter().map(|(n, _)| n.clone()).collect();
        let from = pick_list(names, acc.map(|a| a.name.clone()), move |name| {
            Msg::ComposeAccount(ids.iter().find(|(n, _)| *n == name).map(|(_, id)| id.clone()).unwrap_or_default())
        })
        .text_size(14)
        .style(theme::pick);

        let mut sig_names = vec![NO_SIGNATURE.to_string()];
        if let Some(a) = acc {
            sig_names.extend(a.signatures.iter().map(|s| s.name.clone()));
        }
        let sig =
            pick_list(sig_names, Some(c.signature.clone().unwrap_or_else(|| NO_SIGNATURE.into())), Msg::ComposeSignature)
                .text_size(13)
                .style(theme::pick);

        let line = |label: &'a str, w: El<'a>| -> El<'a> {
            container(row![muted(label).size(13).width(LABEL_W), w].align_y(Alignment::Center))
                .padding([4, 16])
                .height(44)
                .center_y(44)
                .into()
        };
        let field = |value: &'a str, f: Field| -> El<'a> {
            let (chips, tail) = crate::app::addr_parts(value);
            let mut row_chips = Row::new().spacing(6);
            for (i, item) in chips.iter().enumerate() {
                row_chips = row_chips.push(
                    iced::widget::mouse_area(
                        container(
                            row![
                                container(text(item.trim()).size(13)).max_width(260).clip(true),
                                button(icon(ic::CLOSE, 11)).padding([0, 3]).style(theme::link).on_press(Msg::RemoveAddr(f, i)),
                            ]
                            .spacing(6)
                            .align_y(Alignment::Center),
                        )
                        .padding([3, 8])
                        .style(theme::boxed),
                    )
                    .on_enter(Msg::AddrChipHover(Some((f, i))))
                    .on_exit(Msg::AddrChipLeft(f, i)),
                );
            }
            row![
                row_chips,
                text_input("", tail)
                    .id(match f {
                        Field::To => COMPOSE_TO_ID,
                        Field::Cc => "compose-cc",
                        Field::Bcc => "compose-bcc",
                    })
                    .on_input(move |v| Msg::ComposeField(f, v))
                    .on_submit(Msg::CommitField(f))
                    .padding([6, 0])
                    .size(14)
                    .style(theme::bare_input)
                    .width(Fill),
            ]
            .align_y(Alignment::Center)
            .into()
        };
        // Recipient lines accept a dragged chip: highlight the one under the
        // cursor while a drag is going on.
        let drag_active = matches!(&c.addr_drag, Some(d) if d.active);
        let drop_line = |target: Field, label: &'a str, content: El<'a>| -> El<'a> {
            let hot = drag_active && c.addr_drop == Some(target);
            iced::widget::mouse_area(
                container(
                    row![muted(label).size(13).width(LABEL_W), content].align_y(Alignment::Center),
                )
                .padding([4, 16])
                .height(44)
                .align_y(iced::alignment::Vertical::Center)
                .style(move |t: &iced::Theme| {
                    if hot {
                        container::Style {
                            background: Some(t.extended_palette().background.weak.color.into()),
                            border: iced::Border { color: t.palette().primary, width: 1.0, radius: 0.0.into() },
                            ..Default::default()
                        }
                    } else {
                        container::Style::default()
                    }
                }),
            )
            .on_enter(Msg::AddrDropHover(target))
            .on_exit(Msg::AddrDropLeft(target))
            .on_release(Msg::AddrDropped(target))
            .into()
        };

        let mut form = column![
            line(
                "From",
                row![from, Space::new().width(Fill), muted("Signature").size(12), sig]
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .into()
            ),
            hr(),
            drop_line(
                Field::To,
                "To",
                row![
                    field(&c.to, Field::To),
                    button(text(if c.show_cc { "Hide Cc/Bcc" } else { "Cc/Bcc" }).size(12))
                        .padding([4, 8])
                        .style(theme::tool)
                        .on_press(Msg::ToggleCc)
                ]
                .width(Fill)
                .spacing(8)
                .align_y(Alignment::Center)
                .into()
            ),
            hr(),
        ];
        if c.show_cc {
            form = form.push(drop_line(Field::Cc, "Cc", field(&c.cc, Field::Cc)));
            form = form.push(hr());
            form = form.push(drop_line(Field::Bcc, "Bcc", field(&c.bcc, Field::Bcc)));
            form = form.push(hr());
        }
        if !c.suggestions.is_empty() {
            let target = if c.show_cc && !c.cc.is_empty() && c.cc.len() > c.to.len() { Field::Cc } else { Field::To };
            let mut sug = Column::new();
            for s in &c.suggestions {
                let label = if s.name.is_empty() { s.email.clone() } else { crate::smtp::mailbox_text(&s.name, &s.email) };
                sug = sug.push(
                    button(row![text(label.clone()).size(13).width(Fill), muted(&s.source).size(11)])
                        .width(Fill)
                        .padding([6, 10])
                        .style(theme::row(false))
                        .on_press(Msg::PickSuggestion(target, label)),
                );
            }
            form = form.push(container(container(sug).style(theme::boxed).width(Fill)).padding(iced::Padding {
                left: 16.0 + LABEL_W,
                right: 16.0,
                top: 4.0,
                bottom: 4.0,
            }));
        }
        form = form.push(line(
            "Subject",
            text_input("", &c.subject)
                .on_input(Msg::ComposeSubject)
                .padding([6, 0])
                .size(14)
                .style(theme::bare_input)
                .into(),
        ));
        form = form.push(hr());

        if !c.attachments.is_empty() {
            let mut chips = Row::new().spacing(8);
            for (i, a) in c.attachments.iter().enumerate() {
                chips = chips.push(
                    container(
                        row![
                            text(&a.name).size(13),
                            muted(a.size_label()).size(11),
                            button(text("Remove").size(12))
                                .padding([2, 6])
                                .style(theme::link)
                                .on_press(Msg::RemoveAttachment(i)),
                        ]
                        .spacing(8)
                        .align_y(Alignment::Center),
                    )
                    .padding([6, 10])
                    .style(theme::boxed),
                );
            }
            form = form.push(container(chips.wrap().vertical_spacing(8)).padding([10, 16]));
            form = form.push(hr());
        }
        if c.forward.is_some() {
            form = form.push(container(muted("The forwarded mail will be sent with its images and formatting.").size(12)).padding(iced::Padding { top: 8.0, left: 16.0, right: 16.0, ..Default::default() }));
        }

        let editor = text_editor(&c.body)
            .id(COMPOSE_BODY_ID)
            .on_action(Msg::ComposeBody)
            .key_binding(|kp| {
                use iced::keyboard::{Key, key::Named};
                use text_editor::Binding;
                if kp.modifiers.command() {
                    let fmt = match kp.key.as_ref() {
                        Key::Character("b") => Some(Fmt::Bold),
                        Key::Character("i") => Some(Fmt::Italic),
                        Key::Character("u") => Some(Fmt::Underline),
                        Key::Character("k") => Some(Fmt::Link),
                        _ => None,
                    };
                    if let Some(f) = fmt {
                        return Some(Binding::Custom(Msg::Format(f)));
                    }
                    if kp.key == Key::Named(Named::Enter) {
                        return Some(Binding::Custom(Msg::Send));
                    }
                }
                Binding::from_key_press(kp)
            })
            .highlight_with::<ComposeHighlighter>(
                ComposeHl { revision: c.revision, issues: c.grammar.lines.clone() },
                compose_format,
            )
            .placeholder("Write your message…")
            .padding([14, 16])
            .size(14)
            .line_height(1.5)
            .min_height(180)
            .style(theme::bare_editor);
        let fbtn = |glyph: char, f: Fmt, tip: &'a str| tool(glyph, "", Some(Msg::Format(f)), tip);
        let format_bar = container(
            row![
                fbtn(ic::BOLD, Fmt::Bold, "Bold (Ctrl+B)"),
                fbtn(ic::ITALIC, Fmt::Italic, "Italic (Ctrl+I)"),
                fbtn(ic::UNDERLINE, Fmt::Underline, "Underline (Ctrl+U)"),
                fbtn(ic::STRIKE, Fmt::Strike, "Strikethrough"),
                container(vr()).height(18).padding([0, 6]),
                fbtn(ic::LIST, Fmt::Bullets, "Bulleted list"),
                fbtn(ic::LIST_ORDERED, Fmt::Numbers, "Numbered list"),
                fbtn(ic::QUOTE, Fmt::Quote, "Quote"),
                fbtn(ic::LINK, Fmt::Link, "Link (Ctrl+K)"),
            ]
            .spacing(2)
            .align_y(Alignment::Center),
        )
        .padding([4, 12])
        .width(Fill)
        .style(theme::pane);

        // Signature and quoted original below the text, scrolling with it.
        let mut extra = Column::new().spacing(12).padding(iced::Padding { top: 4.0, right: 16.0, bottom: 16.0, left: 16.0 });
        if !c.sig_preview.is_empty() {
            let mut sig = Column::new().spacing(6).push(muted("-- ").size(12));
            for part in &c.sig_preview {
                let el: El<'a> = match part {
                    SigPart::Text(t) => muted(t.as_str()).size(12).into(),
                    SigPart::Image(h) => container(
                        iced::widget::image(h.clone()).content_fit(iced::ContentFit::ScaleDown),
                    )
                    .max_width(220)
                    .into(),
                };
                sig = sig.push(el);
            }
            extra = extra.push(sig);
        }
        if !c.quote.is_empty() {
            extra = extra.push(
                column![
                    row![
                        muted("Original message").size(12).font(SEMIBOLD).width(Fill),
                        button(text("Don't include").size(12)).style(theme::link).on_press(Msg::DropQuote),
                    ]
                    .align_y(Alignment::Center),
                    container(muted(c.quote.as_str()).size(13)).width(Fill).padding([10, 14]).style(theme::quote),
                ]
                .spacing(6),
            );
        }

        let title = if c.in_reply_to.is_some() {
            "Reply"
        } else if c.subject.starts_with("Fwd:") {
            "Forward"
        } else {
            "New message"
        };
        let top = header(
            row![
                text(title).font(SEMIBOLD).size(15).width(Fill),
                tool(ic::ATTACHMENT, "Attach", Some(Msg::Attach), "Attach files (or drop them on the window)"),
                tool(ic::DELETE, "Discard", Some(Msg::DiscardDraft), "Delete this message (Esc keeps it as a draft)"),
                button(
                    row![icon(ic::SEND, 14), text(if c.sending { "Sending…" } else { "Send" }).font(SEMIBOLD).size(14)]
                        .spacing(8)
                        .align_y(Alignment::Center)
                )
                .padding([8, 18])
                .style(theme::primary)
                .on_press_maybe((!c.sending).then_some(Msg::Send)),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        );

        let body = scrollable(column![editor, extra]).height(Fill).style(theme::scroll);
        let mut col = column![top, hr(), form, format_bar, hr(), body];
        if self.cfg.grammar.enabled {
            col = col.push(hr()).push(self.grammar_bar(c));
        }
        if self.cfg.ai.enabled {
            col = col.push(hr());
            col = col.push(
                container(
                    row![
                        muted("AI").size(12).font(SEMIBOLD).width(28),
                        text_input(
                            "e.g. \"reply that I'm in, but on Friday\" · \"shorter\" · \"translate to English\"",
                            &c.ai_prompt
                        )
                        .on_input(Msg::AiPrompt)
                        .on_submit(Msg::AiWrite)
                        .padding([6, 0])
                        .size(13)
                        .style(theme::bare_input),
                        secondary_btn(if c.ai_busy { "Writing…" } else { "Draft" }, (!c.ai_busy).then_some(Msg::AiWrite)),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                )
                .padding([6, 16])
                .style(theme::pane),
            );
        }
        container(col).width(Fill).height(Fill).into()
    }

    // ---- contacts ----------------------------------------------------------

    fn contacts<'a>(&'a self, v: &'a ContactsView) -> El<'a> {
        let f = v.filter.to_lowercase();
        let cell = |w: u16| Length::FillPortion(w);
        let head = container(
            row![
                muted("Name").size(12).font(SEMIBOLD).width(cell(20)),
                muted("Email").size(12).font(SEMIBOLD).width(cell(25)),
                muted("Phone").size(12).font(SEMIBOLD).width(cell(13)),
                muted("Organization").size(12).font(SEMIBOLD).width(cell(15)),
                muted("Source").size(12).font(SEMIBOLD).width(cell(12)),
                Space::new().width(70),
            ]
            .spacing(10),
        )
        .padding([8, 16]);
        let mut list = Column::new();
        let shown: Vec<_> =
            v.all.iter().filter(|c| f.is_empty() || c.name.to_lowercase().contains(&f) || c.email.contains(&f)).collect();
        for c in &shown {
            let action: El<'a> = if c.source == "local" {
                button(text("Remove").size(12))
                    .padding([2, 6])
                    .style(theme::link)
                    .on_press(Msg::ContactDelete(c.source.clone(), c.email.clone()))
                    .width(70)
                    .into()
            } else {
                Space::new().width(70).into()
            };
            list = list.push(
                container(
                    row![
                        text(&c.name).size(13).width(cell(20)).wrapping(text::Wrapping::None),
                        text(&c.email).size(13).width(cell(25)).wrapping(text::Wrapping::None),
                        muted(&c.phone).size(12).width(cell(13)),
                        muted(&c.org).size(12).width(cell(15)).wrapping(text::Wrapping::None),
                        muted(&c.source).size(11).width(cell(12)),
                        action,
                    ]
                    .spacing(10)
                    .align_y(Alignment::Center),
                )
                .padding([8, 16])
                .clip(true),
            );
            list = list.push(hr());
        }
        column![
            header(
                row![
                    text("Address book").font(SEMIBOLD).size(15).width(Fill),
                    tool(ic::SYNC, "Sync LDAP / Google", Some(Msg::SyncContacts), "Pull contacts from configured directories"),
                    tool(ic::CLOSE, "Close", Some(Msg::CloseScreen), "Esc"),
                ]
                .spacing(6)
                .align_y(Alignment::Center)
            ),
            hr(),
            container(
                row![
                    input("Filter", &v.filter).on_input(Msg::ContactsFilter).width(Fill),
                    container(vr()).height(24).padding([0, 6]),
                    input("Name", &v.new_name).on_input(Msg::ContactNewName).width(180),
                    input("email@example.com", &v.new_email)
                        .on_input(Msg::ContactNewEmail)
                        .on_submit(Msg::ContactAdd)
                        .width(220),
                    secondary_btn("Add contact", Some(Msg::ContactAdd)),
                ]
                .spacing(6)
                .align_y(Alignment::Center)
            )
            .padding([10, 16]),
            hr(),
            head,
            hr(),
            scrollable(list).height(Fill).style(theme::scroll),
            hr(),
            container(muted(format!("{} of {} contacts", shown.len(), v.all.len())).size(12)).padding([6, 16]),
        ]
        .width(Fill)
        .into()
    }

    // ---- settings ----------------------------------------------------------

    fn settings<'a>(&'a self, s: &'a Settings) -> El<'a> {
        let nav = |glyph: char, label: &'a str, t: SettingsTab| -> El<'a> {
            let active = s.tab == t;
            button(
                row![
                    accent_bar(active, 36.0),
                    Space::new().width(13),
                    icon(glyph, 15).style(move |th: &iced::Theme| text::Style {
                        color: Some(if active { th.palette().primary } else { theme::muted(th) })
                    }),
                    Space::new().width(10),
                    text(label).size(14).font(if active { SEMIBOLD } else { UI })
                ]
                .align_y(Alignment::Center),
            )
            .width(Fill)
            .padding(0)
            .style(theme::row(active))
            .on_press(Msg::Settings(SettingsMsg::Tab(t)))
            .into()
        };
        let menu = container(
            column![
                nav(ic::USERS, "Accounts", SettingsTab::Accounts),
                nav(ic::CONTACTS, "Address book", SettingsTab::AddressBook),
                nav(ic::BOT, "AI assistant", SettingsTab::Ai),
                nav(ic::PALETTE, "Appearance", SettingsTab::Appearance),
                nav(ic::SETTINGS, "General", SettingsTab::General),
            ]
            .padding([8, 0]),
        )
        .width(190)
        .height(Fill);

        let content: El<'a> = match s.tab {
            SettingsTab::Accounts => self.settings_accounts(s),
            SettingsTab::AddressBook => settings_contacts(s),
            SettingsTab::Ai => settings_ai(s),
            SettingsTab::Appearance => settings_appearance(s),
            SettingsTab::General => column![
                section("Sync"),
                labeled(
                    "Full sync every (minutes, 0 = manual). New inbox mail arrives instantly via IMAP IDLE.",
                    input("5", &s.draft.sync_interval_minutes.to_string())
                        .on_input(|v| Msg::Settings(SettingsMsg::SyncInterval(v)))
                        .width(120)
                        .into()
                ),
                checkbox(s.draft.notifications)
                    .label("Desktop notification for new mail")
                    .on_toggle(|b| Msg::Settings(SettingsMsg::Notifications(b)))
                    .text_size(14)
                    .style(theme::check),
                checkbox(s.draft.notify_preview)
                    .label("Show the first lines of the message in notifications (visible on the lock screen)")
                    .on_toggle(|b| Msg::Settings(SettingsMsg::NotifyPreview(b)))
                    .text_size(14)
                    .style(theme::check),
                section("Grammar & spelling"),
                checkbox(s.draft.grammar.enabled)
                    .label("Check grammar and spelling while writing (German and English, via LanguageTool)")
                    .on_toggle(|b| Msg::Settings(SettingsMsg::GrammarEnabled(b)))
                    .text_size(14)
                    .style(theme::check),
                column![
                    labeled(
                        "LanguageTool server. The public one receives what you write; a local one keeps \
                         your text on this machine.",
                        row![
                            input(crate::config::LANGUAGETOOL_LOCAL, &s.draft.grammar.server)
                                .on_input(|v| Msg::Settings(SettingsMsg::GrammarServer(v)))
                                .width(Fill),
                            button(text("Local").size(12)).style(theme::link).on_press(Msg::Settings(
                                SettingsMsg::GrammarServer(crate::config::LANGUAGETOOL_LOCAL.into())
                            )),
                            button(text("Public").size(12)).style(theme::link).on_press(Msg::Settings(
                                SettingsMsg::GrammarServer(crate::config::LANGUAGETOOL_PUBLIC.into())
                            )),
                            secondary_btn(if s.busy { "Testing…" } else { "Test" }, (!s.busy).then_some(Msg::Settings(SettingsMsg::GrammarTest))),
                        ]
                        .spacing(8)
                        .align_y(Alignment::Center)
                        .into()
                    ),
                    row![
                        muted(crate::config::LANGUAGETOOL_DOCKER).size(12),
                        tool(
                            ic::COPY,
                            "",
                            Some(Msg::Settings(SettingsMsg::GrammarCopyCmd)),
                            "Copy the run command for a local LanguageTool (Docker)",
                        ),
                    ]
                    .spacing(6)
                    .align_y(Alignment::Center),
                ]
                .spacing(5)
                .width(Fill),
                row![
                    labeled(
                        "Language",
                        pick_list(crate::config::GrammarLang::ALL, Some(s.draft.grammar.language), |l| {
                            Msg::Settings(SettingsMsg::GrammarLanguage(l))
                        })
                        .text_size(13)
                        .style(theme::pick)
                        .into()
                    ),
                    container(
                        checkbox(s.draft.grammar.english_gb)
                            .label("British English")
                            .on_toggle(|b| Msg::Settings(SettingsMsg::GrammarGb(b)))
                            .text_size(14)
                            .style(theme::check)
                    )
                    .padding(iced::Padding { top: 22.0, ..Default::default() }),
                ]
                .spacing(16),
                labeled(
                    "Ignored words (comma separated) — added by \u{201c}Ignore\u{201d} in the composer",
                    input("Mailix, Qwellcode", &s.grammar_ignored)
                        .on_input(|v| Msg::Settings(SettingsMsg::GrammarIgnored(v)))
                        .into()
                ),
                section("Files"),
                muted(format!(
                    "Config: {}",
                    crate::config::paths().map(|p| p.config_file.display().to_string()).unwrap_or_default()
                ))
                .size(12),
            ]
            .spacing(14)
            .into(),
        };

        let title = match s.tab {
            SettingsTab::Accounts => "Accounts",
            SettingsTab::AddressBook => "Address book",
            SettingsTab::Ai => "AI assistant",
            SettingsTab::Appearance => "Appearance",
            SettingsTab::General => "General",
        };
        let mut right = column![
            header(
                row![
                    text(title).font(SEMIBOLD).size(15).width(Fill),
                    tool(ic::CLOSE, "Close", Some(Msg::CloseScreen), "Esc"),
                    primary_btn("Save", Some(Msg::Settings(SettingsMsg::Save))),
                ]
                .spacing(6)
                .align_y(Alignment::Center)
            ),
            hr(),
        ];
        if let Some(note) = &s.note {
            let (t, st): (&str, fn(&iced::Theme) -> container::Style) = match note {
                Ok(t) => (t, theme::note),
                Err(e) => (e, theme::error_note),
            };
            right = right.push(container(text(t).size(13)).padding([8, 16]).width(Fill).style(st));
        }
        right = right.push(
            scrollable(container(container(content).max_width(860)).padding([20, 24]).width(Fill))
                .height(Fill)
                .style(theme::scroll),
        );
        row![menu, vr(), right.width(Fill)].into()
    }

    fn settings_accounts<'a>(&'a self, s: &'a Settings) -> El<'a> {
        let set = |m: SettingsMsg| Msg::Settings(m);
        let mut list = Column::new();
        for (i, a) in s.draft.accounts.iter().enumerate() {
            list = list.push(
                button(column![text(&a.name).size(14), muted(&a.email).size(11)].spacing(1))
                    .width(Fill)
                    .padding([7, 10])
                    .style(theme::row(s.account == Some(i)))
                    .on_press(set(SettingsMsg::EditAccount(i))),
            );
        }
        let add = container(
            column![
                text("Add account").font(SEMIBOLD).size(13),
                pick_list(&Preset::ALL[..], Some(s.new_preset), |p| Msg::Settings(SettingsMsg::NewPreset(p)))
                    .width(Fill)
                    .text_size(13)
                    .style(theme::pick),
                input("you@example.com", &s.new_email)
                    .on_input(|v| Msg::Settings(SettingsMsg::NewEmail(v)))
                    .on_submit(set(SettingsMsg::AddAccount)),
                secondary_btn("Add", Some(set(SettingsMsg::AddAccount))),
                muted(match s.new_preset {
                    Preset::ProtonBridge => "Start Proton Mail Bridge first and use the bridge password it shows.",
                    Preset::Gmail => "Needs 2-step verification and an app password (myaccount.google.com/apppasswords).",
                    Preset::Outlook => "Needs an app password; OAuth-only tenants aren't supported yet.",
                    Preset::Generic => "Server names are guessed — adjust them after adding.",
                })
                .size(11),
            ]
            .spacing(8),
        )
        .padding(12)
        .style(theme::boxed);
        let left = column![list, add].spacing(16).width(230);

        let right: El<'a> = match s.account.and_then(|i| s.draft.accounts.get(i)) {
            None => muted("Add an account to get started.").into(),
            Some(a) => {
                let field = |ph: &'a str, v: String, f: AccField| -> El<'a> {
                    input(ph, &v).on_input(move |x| Msg::Settings(SettingsMsg::Acc(f, x))).into()
                };
                let server = |imap: bool| -> El<'a> {
                    let srv = if imap { &a.imap } else { &a.smtp };
                    let (h, p, u, ins) = if imap {
                        (AccField::ImapHost, AccField::ImapPort, AccField::ImapUser, AccField::ImapInsecure)
                    } else {
                        (AccField::SmtpHost, AccField::SmtpPort, AccField::SmtpUser, AccField::SmtpInsecure)
                    };
                    column![
                        section(if imap { "Incoming mail (IMAP)" } else { "Outgoing mail (SMTP)" }),
                        row![
                            labeled("Server", field("imap.example.com", srv.host.clone(), h)),
                            container(labeled("Port", field("993", srv.port.to_string(), p))).width(90),
                            container(labeled(
                                "Security",
                                pick_list(&Security::ALL[..], Some(srv.security), move |sec| {
                                    Msg::Settings(SettingsMsg::Security(imap, sec))
                                })
                                .text_size(14)
                                .style(theme::pick)
                                .into()
                            ))
                            .width(130),
                        ]
                        .spacing(8),
                        labeled("Username", field("username", srv.username.clone(), u)),
                        checkbox(srv.accept_invalid_certs)
                            .label("Accept self-signed certificate (Proton Bridge)")
                            .on_toggle(move |b| Msg::Settings(SettingsMsg::AccBool(ins, b)))
                            .text_size(13)
                            .style(theme::check),
                    ]
                    .spacing(10)
                    .into()
                };

                let mut sigs = Column::new().spacing(10);
                for (i, sig) in a.signatures.iter().enumerate() {
                    if let Some(content) = s.signatures.get(i) {
                        sigs = sigs.push(
                            container(
                                column![
                                    row![
                                        input("Name", &sig.name)
                                            .on_input(move |v| Msg::Settings(SettingsMsg::SignatureName(i, v)))
                                            .width(Fill),
                                        button(text("Remove").size(12))
                                            .style(theme::link)
                                            .on_press(Msg::Settings(SettingsMsg::RemoveSignature(i))),
                                    ]
                                    .spacing(6)
                                    .align_y(Alignment::Center),
                                    if sig.html.is_some() {
                                        let n = crate::signature::images(sig.html.as_deref().unwrap_or("")).len();
                                        row![
                                            icon(ic::IMAGE, 13),
                                            muted(format!(
                                                "HTML signature with {n} image{} — sent formatted. Below: plain-text version.",
                                                if n == 1 { "" } else { "s" }
                                            ))
                                            .size(12)
                                            .width(Fill),
                                            button(text("Replace…").size(12))
                                                .style(theme::link)
                                                .on_press(Msg::Settings(SettingsMsg::ImportSignature(i))),
                                            button(text("Use plain text").size(12))
                                                .style(theme::link)
                                                .on_press(Msg::Settings(SettingsMsg::RemoveSignatureHtml(i))),
                                        ]
                                        .spacing(6)
                                        .align_y(Alignment::Center)
                                    } else {
                                        row![
                                            muted("Plain text — or import an HTML signature with images").size(12).width(Fill),
                                            button(text("Import HTML…").size(12))
                                                .style(theme::link)
                                                .on_press(Msg::Settings(SettingsMsg::ImportSignature(i))),
                                        ]
                                        .spacing(6)
                                        .align_y(Alignment::Center)
                                    },
                                    text_editor(content)
                                        .on_action(move |act| Msg::Settings(SettingsMsg::SignatureBody(i, act)))
                                        .height(Length::Fixed(96.0))
                                        .padding(8)
                                        .size(13)
                                        .style(theme::editor),
                                ]
                                .spacing(8),
                            )
                            .padding(12)
                            .style(theme::boxed),
                        );
                    }
                }
                let mut sig_names = vec![NO_SIGNATURE.to_string()];
                sig_names.extend(a.signatures.iter().map(|s| s.name.clone()));

                column![
                    section("Identity"),
                    row![
                        labeled("Account name", field("Work", a.name.clone(), AccField::Name)),
                        labeled("Your name", field("Jane Doe", a.display_name.clone(), AccField::DisplayName)),
                    ]
                    .spacing(8),
                    labeled("Email address", field("jane@example.com", a.email.clone(), AccField::Email)),
                    server(true),
                    labeled(
                        "Password — stored in the OS keyring; leave empty to keep the current one",
                        input("••••••••", &s.imap_password)
                            .secure(true)
                            .on_input(|v| Msg::Settings(SettingsMsg::ImapPassword(v)))
                            .into()
                    ),
                    server(false),
                    labeled(
                        "SMTP password — only if different from IMAP",
                        input("same as IMAP", &s.smtp_password)
                            .secure(true)
                            .on_input(|v| Msg::Settings(SettingsMsg::SmtpPassword(v)))
                            .into()
                    ),
                    checkbox(a.save_sent_copy)
                        .label("Save a copy of sent mail to the Sent folder")
                        .on_toggle(|b| Msg::Settings(SettingsMsg::AccBool(AccField::SaveSent, b)))
                        .text_size(13)
                        .style(theme::check),
                    row![
                        section("Signatures"),
                        Space::new().width(Fill),
                        muted("Default").size(12),
                        pick_list(
                            sig_names,
                            Some(a.default_signature.clone().unwrap_or_else(|| NO_SIGNATURE.into())),
                            |n| Msg::Settings(SettingsMsg::DefaultSignature(n))
                        )
                        .text_size(13)
                        .style(theme::pick),
                        secondary_btn("Add signature", Some(set(SettingsMsg::AddSignature))),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                    sigs,
                    hr(),
                    row![
                        secondary_btn(
                            if s.busy { "Testing…" } else { "Test connection" },
                            (!s.busy).then_some(set(SettingsMsg::Test))
                        ),
                        Space::new().width(Fill),
                        button(text("Remove account").size(13))
                            .padding([7, 14])
                            .style(theme::danger)
                            .on_press(set(SettingsMsg::RemoveAccount)),
                    ],
                ]
                .spacing(12)
                .into()
            }
        };
        row![left, vr(), right].spacing(20).into()
    }
}

/// Message count of a conversation with a fold/unfold chevron.
fn thread_badge<'a>(r: &ListRow, open: bool) -> El<'a> {
    if r.count < 2 {
        return Space::new().into();
    }
    button(
        row![
            text(r.count.to_string()).size(11).font(SEMIBOLD),
            icon(if open { ic::CHEVRON_DOWN } else { ic::CHEVRON_RIGHT }, 11),
        ]
        .spacing(3)
        .align_y(Alignment::Center),
    )
    .padding([1, 5])
    .style(theme::secondary)
    .on_press(Msg::FoldThread(r.key.clone()))
    .into()
}

/// Selected-state marker on the left edge of rows.
fn accent_bar<'a>(on: bool, height: f32) -> El<'a> {
    container(Space::new().width(3).height(height))
        .style(move |t: &iced::Theme| container::Style {
            background: on.then(|| t.palette().primary.into()),
            ..Default::default()
        })
        .into()
}

/// Tiny square marker for a tag (Todo / Important) or the AgentProcessed badge.
fn tagchip(c: iced::Color) -> El<'static> {
    container(Space::new().width(8).height(8))
        .style(move |_| container::Style {
            background: Some(c.into()),
            ..Default::default()
        })
        .into()
}

fn settings_contacts(s: &Settings) -> El<'_> {
    let mut col = column![
        row![
            section("LDAP directories"),
            Space::new().width(Fill),
            secondary_btn("Add LDAP directory", Some(Msg::Settings(SettingsMsg::AddLdap)))
        ]
        .align_y(Alignment::Center)
    ]
    .spacing(12);
    for (i, l) in s.draft.contacts.ldap.iter().enumerate() {
        let f = |label: &'static str, ph: &'static str, v: &str, field: LdapField| -> El<'_> {
            labeled(label, input(ph, v).on_input(move |x| Msg::Settings(SettingsMsg::Ldap(i, field, x))).into())
        };
        col = col.push(
            container(
                column![
                    row![
                        f("Name", "Company directory", &l.name, LdapField::Name),
                        button(text("Remove").size(12))
                            .style(theme::link)
                            .on_press(Msg::Settings(SettingsMsg::RemoveLdap(i)))
                    ]
                    .spacing(8)
                    .align_y(Alignment::End),
                    f("URL", "ldaps://ldap.example.com:636", &l.url, LdapField::Url),
                    f("Base DN", "ou=people,dc=example,dc=com", &l.base_dn, LdapField::BaseDn),
                    row![
                        f("Bind DN (empty = anonymous)", "cn=reader,dc=example,dc=com", &l.bind_dn, LdapField::BindDn),
                        labeled(
                            "Password (keyring)",
                            input("••••••", &s.ldap_passwords[i])
                                .secure(true)
                                .on_input(move |x| Msg::Settings(SettingsMsg::Ldap(i, LdapField::Password, x)))
                                .into()
                        ),
                    ]
                    .spacing(8),
                    f("Filter", "(mail=*)", &l.filter, LdapField::Filter),
                ]
                .spacing(10),
            )
            .padding(14)
            .style(theme::boxed),
        );
    }
    col = col.push(Space::new().height(8));
    col = col.push(
        row![
            section("Google Contacts"),
            Space::new().width(Fill),
            secondary_btn("Add Google account", Some(Msg::Settings(SettingsMsg::AddGoogle)))
        ]
        .align_y(Alignment::Center),
    );
    col = col.push(
        muted(
            "Create an OAuth client of type \"Desktop app\" in Google Cloud Console, enable the People API, \
             paste its client ID and secret here, then click Connect.",
        )
        .size(12),
    );
    for (i, g) in s.draft.contacts.google.iter().enumerate() {
        let f = |label: &'static str, v: &str, field: GoogleField| -> El<'_> {
            labeled(label, input("", v).on_input(move |x| Msg::Settings(SettingsMsg::Google(i, field, x))).into())
        };
        col = col.push(
            container(
                column![
                    row![
                        f("Name", &g.name, GoogleField::Name),
                        secondary_btn("Connect…", (!s.busy).then_some(Msg::Settings(SettingsMsg::GoogleConnect(i)))),
                        button(text("Remove").size(12))
                            .style(theme::link)
                            .on_press(Msg::Settings(SettingsMsg::RemoveGoogle(i)))
                    ]
                    .spacing(8)
                    .align_y(Alignment::End),
                    f("Client ID", &g.client_id, GoogleField::ClientId),
                    f("Client secret", &g.client_secret, GoogleField::ClientSecret),
                ]
                .spacing(10),
            )
            .padding(14)
            .style(theme::boxed),
        );
    }
    col = col.push(Space::new().height(8));
    col = col.push(section("Proton Mail"));
    col = col.push(
        muted(
            "Bridge only exposes IMAP/SMTP, not contacts. Add them in the address book manually; \
             people you've emailed are suggested automatically.",
        )
        .size(12),
    );
    col.into()
}

fn settings_ai(s: &Settings) -> El<'_> {
    let p = s.draft.ai.provider;
    let model: El<'_> = if s.models.is_empty() {
        input(p.default_model(), &s.draft.ai.model).on_input(|v| Msg::Settings(SettingsMsg::AiModel(v))).into()
    } else {
        pick_list(s.models.clone(), Some(s.draft.ai.model.clone()), |m| Msg::Settings(SettingsMsg::AiModel(m)))
            .width(Fill)
            .text_size(14)
            .style(theme::pick)
            .into()
    };
    let key_label = if p.needs_key() {
        "API key — stored in the OS keyring; leave empty to keep the current one"
    } else {
        "API key (optional)"
    };
    let hint = match p {
        AiProvider::Anthropic => "Claude via the Anthropic API. Get a key at console.anthropic.com.",
        AiProvider::OpenAi => "OpenAI Chat Completions API.",
        AiProvider::Ollama => {
            "Runs fully on your machine — install Ollama and pull a model. Nothing leaves your computer."
        }
        AiProvider::OllamaCloud => "Ollama's hosted models. Create a key at ollama.com/settings/keys.",
        AiProvider::OpencodeZen => "OpenCode Zen gateway. Only models served on /chat/completions work here.",
        AiProvider::OpenRouter => "One key for many model vendors.",
        AiProvider::Custom => "Any OpenAI-compatible server: LM Studio, vLLM, LiteLLM, llama.cpp …",
    };
    column![
        checkbox(s.draft.ai.enabled)
            .label("Enable AI assistant")
            .on_toggle(|b| Msg::Settings(SettingsMsg::AiEnabled(b)))
            .text_size(14)
            .style(theme::check),
        muted(
            "Adds \"Ask AI\" to search, \"AI summary\" to the reader and a drafting line to the composer. \
             Only the text of an action you trigger is sent — never in the background."
        )
        .size(12),
        section("Provider"),
        labeled(
            "Provider",
            pick_list(&AiProvider::ALL[..], Some(p), |p| Msg::Settings(SettingsMsg::AiProvider(p)))
                .width(Fill)
                .text_size(14)
                .style(theme::pick)
                .into()
        ),
        muted(hint).size(12),
        labeled(
            "Endpoint",
            input(p.default_base_url(), &s.draft.ai.base_url)
                .on_input(|v| Msg::Settings(SettingsMsg::AiBaseUrl(v)))
                .into()
        ),
        labeled(key_label, input("sk-…", &s.ai_key).secure(true).on_input(|v| Msg::Settings(SettingsMsg::AiKey(v))).into()),
        row![
            labeled("Model", model),
            container(secondary_btn(
                if s.busy { "Loading…" } else { "Load models" },
                (!s.busy).then_some(Msg::Settings(SettingsMsg::LoadModels))
            ))
            .padding(iced::Padding { top: 18.0, ..Default::default() }),
        ]
        .spacing(8),
    ]
    .spacing(12)
    .into()
}

fn settings_appearance(s: &Settings) -> El<'_> {
    let current = s.draft.appearance.accent.to_lowercase();
    let mut swatches = Row::new().spacing(8);
    for (name, hex) in theme::ACCENTS {
        let c = theme::accent_color(hex);
        swatches = swatches.push(tooltip(
            button(Space::new().width(30).height(30))
                .padding(0)
                .style(theme::swatch(c, current == hex))
                .on_press(Msg::Settings(SettingsMsg::Accent(hex.to_string()))),
            container(text(name).size(12)).padding([4, 8]).style(theme::boxed),
            tooltip::Position::Bottom,
        ));
    }
    let valid = crate::config::parse_hex(&s.draft.appearance.accent).is_some();
    let noop = || Some(Msg::Settings(SettingsMsg::Tab(SettingsTab::Appearance)));
    column![
        section("Primary color"),
        swatches,
        row![
            labeled(
                "Custom (#rrggbb)",
                input("#2f6bed", &s.draft.appearance.accent)
                    .on_input(|v| Msg::Settings(SettingsMsg::Accent(v)))
                    .width(160)
                    .into()
            ),
            container(muted(if valid { "" } else { "Not a valid color" }).size(12))
                .padding(iced::Padding { top: 22.0, ..Default::default() }),
        ]
        .spacing(12),
        section("Preview"),
        row![
            primary_btn("Primary button", noop()),
            secondary_btn("Secondary", noop()),
            button(text("Link").size(13)).style(theme::link).on_press_maybe(noop()),
        ]
        .spacing(10)
        .align_y(Alignment::Center),
        muted("Changes preview live; click Save to keep them.").size(12),
        section("Theme"),
        muted("Light and dark follow your operating system automatically.").size(13),
    ]
    .spacing(14)
    .into()
}

fn section(title: &str) -> El<'_> {
    text(title).font(SEMIBOLD).size(14).into()
}

fn labeled<'a>(label: &'a str, w: El<'a>) -> El<'a> {
    column![muted(label).size(12), w].spacing(5).width(Fill).into()
}

/// Full path for the flat favorites list ("Projects › Alpha"), without the
/// "INBOX" prefix servers like Dovecot put in front of every folder.
fn folder_path(name: &str) -> String {
    let decoded = crate::imap::decode_folder_name(name);
    let parts: Vec<&str> = decoded.split(['/', '.']).collect();
    let parts = match parts.first() {
        Some(p) if p.eq_ignore_ascii_case("INBOX") && parts.len() > 1 => &parts[1..],
        _ => &parts[..],
    };
    parts.join(" › ")
}

/// The floating label following the cursor during a drag (mail or recipient).
fn app_drag_chip<'a>(label: String, what: Option<String>, pos: iced::Point) -> El<'a> {
    let mut body = column![text(label).font(SEMIBOLD).size(12).wrapping(text::Wrapping::None)].spacing(2);
    if let Some(what) = what {
        body = body.push(muted(what).size(12).wrapping(text::Wrapping::None));
    }
    let chip = container(body).padding([6, 10]).max_width(280).clip(true).style(theme::boxed);
    container(chip)
        .padding(iced::Padding { top: pos.y + 14.0, left: pos.x + 14.0, ..Default::default() })
        .into()
}

fn display_folder(name: &str) -> String {
    let leaf = name.rsplit(['/', '.']).next().unwrap_or(name);
    crate::imap::decode_folder_name(leaf)
}

#[cfg(test)]
mod grammar_highlight_tests {
    use super::*;
    use crate::grammar::IssueKind;
    use iced::advanced::text::Highlighter;

    #[test]
    fn issues_overlay_formatting() {
        // Line 1: "Hi **Bobb** teh end" — "Bobb" (bold) and "teh" flagged.
        let issues = std::sync::Arc::new(vec![(1, 5..9, IssueKind::Spelling), (1, 12..15, IssueKind::Grammar)]);
        let mut h = ComposeHighlighter::new(&ComposeHl { revision: 1, issues });
        let first: Vec<_> = h.highlight_line("untouched line").collect();
        assert!(first.is_empty());
        let spans: Vec<_> = h.highlight_line("Hi **Bobb** teh end").collect();
        let bold = crate::markup::Style { bold: true, ..Default::default() };
        assert!(spans.contains(&(5..9, CHl::Issue(IssueKind::Spelling, bold))), "{spans:?}");
        assert!(spans.contains(&(12..15, CHl::Issue(IssueKind::Grammar, Default::default()))), "{spans:?}");
        // Markers keep their own highlight.
        assert!(spans.contains(&(3..5, CHl::Marker(bold))), "{spans:?}");
    }
}
