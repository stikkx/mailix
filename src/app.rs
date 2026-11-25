//! Application state and update logic (Elm architecture via iced).

use crate::config::{AccountConfig, AiProvider, Config, GoogleConfig, LdapConfig, Preset, Signature};
use crate::mime::{self, Attachment, Parsed};
use crate::store::{Contact, Folder, FolderRole, Message as Mail, Store, Summary};
use crate::{ai, contacts, imap, search::Search, secrets, smtp};
use iced::keyboard::{self, Key, key::Named};
use iced::theme::Mode;
use iced::widget::{operation, text_editor};
use iced::{Subscription, Task, Theme};
use futures::SinkExt;
use iced::widget::image;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

pub const PAGE: usize = 200;
pub const SEARCH_ID: &str = "search";

#[derive(Debug, Clone, PartialEq)]
pub enum Selection {
    Unified(FolderRole),
    Folder { account: String, name: String },
}

/// Grammar check state of the composer.
#[derive(Debug, Clone, Default)]
pub struct GrammarState {
    pub issues: Vec<crate::grammar::Issue>,
    /// Issues per line for the editor highlighter.
    pub lines: Arc<Vec<(usize, std::ops::Range<usize>, crate::grammar::IssueKind)>>,
    /// Text the issues refer to.
    pub text: String,
    /// When to check next (a moment after the last edit).
    pub due: Option<std::time::Instant>,
    pub busy: bool,
    pub error: Option<String>,
}

impl GrammarState {
    /// Follows an edit: moves issue ranges and schedules a new check.
    pub fn sync(&mut self, text: &str) {
        if self.text == text {
            return;
        }
        crate::grammar::shift(&mut self.issues, &self.text, text);
        self.text = text.to_string();
        self.lines = Arc::new(crate::grammar::by_line(text, &self.issues));
        self.due = Some(std::time::Instant::now() + Duration::from_millis(900));
    }

    fn set(&mut self, issues: Vec<crate::grammar::Issue>) {
        self.issues = issues;
        self.lines = Arc::new(crate::grammar::by_line(&self.text, &self.issues));
    }
}

#[derive(Debug, Clone)]
pub struct Drag {
    pub ids: Vec<i64>,
    /// First cursor position after the press.
    pub start: Option<iced::Point>,
    pub pos: iced::Point,
    /// Moved far enough to count as a drag (not a click).
    pub active: bool,
}

/// A rich forward: the original HTML body and its embedded images.
#[derive(Debug, Clone)]
pub struct ForwardPart {
    /// Store id of the forwarded message (kept across draft save/reopen).
    pub source: i64,
    /// Original HTML body (references its images as `cid:`).
    pub html: String,
    /// The images referenced by that HTML, as inline parts.
    pub images: Vec<Attachment>,
}

/// Builds the forward payload from a parsed message (None for plain mail).
pub fn forward_payload(parsed: &Parsed, source: i64) -> Option<ForwardPart> {
    let html = parsed.html.clone()?;
    let images = parsed
        .local_images()
        .into_iter()
        .map(|(src, data)| {
            let cid = src.strip_prefix("cid:").unwrap_or(&src).to_string();
            let known = parsed.attachments.iter().find(|a| a.content_id.as_deref() == Some(cid.as_str()));
            Attachment {
                name: known.map(|a| a.name.clone()).unwrap_or_else(|| cid.clone()),
                mime: known.map(|a| a.mime.clone()).unwrap_or_else(|| "application/octet-stream".into()),
                data: Arc::new(data),
                content_id: Some(cid),
                embedded: true,
            }
        })
        .collect();
    Some(ForwardPart { source, html, images })
}

/// A recipient chip being dragged between the To/Cc/Bcc lines.
#[derive(Debug, Clone, Copy)]
pub struct AddrDrag {
    pub field: Field,
    pub idx: usize,
    /// First cursor position after the press.
    pub start: Option<iced::Point>,
    pub pos: iced::Point,
    /// Moved far enough to count as a drag (not a click).
    pub active: bool,
}

pub enum Screen {
    Mail,
    Compose(Box<Compose>),
    Settings(Box<Settings>),
    Contacts(ContactsView),
}

pub struct Compose {
    pub account: String,
    pub signature: Option<String>,
    pub to: String,
    pub cc: String,
    pub bcc: String,
    pub show_cc: bool,
    pub subject: String,
    pub body: text_editor::Content,
    /// Quoted original, appended below the signature when sending.
    pub quote: String,
    pub in_reply_to: Option<String>,
    pub references: Option<String>,
    pub original: Option<String>,
    pub ai_prompt: String,
    pub ai_busy: bool,
    pub suggestions: Vec<Contact>,
    pub attachments: Vec<Attachment>,
    pub sending: bool,
    /// Preview of the selected signature: text and images in order.
    pub sig_preview: Vec<SigPart>,
    /// Rich forward: HTML body + inline images of the forwarded message.
    pub forward: Option<ForwardPart>,
    /// Message-ID of this draft; stays the same across saves.
    pub draft_id: String,
    /// Drafts folder holding a server copy (after a save, or when reopened).
    pub draft_folder: Option<String>,
    /// UID of the server draft this composer was opened from.
    pub draft_uid: Option<u32>,
    /// Fingerprint of the last saved state (skip unchanged saves).
    pub saved_hash: u64,
    /// Recipient chip under the mouse (drag starts there).
    pub addr_hover: Option<(Field, usize)>,
    /// Recipient chip being dragged between the recipient lines.
    pub addr_drag: Option<AddrDrag>,
    /// Recipient line under the cursor while a chip is dragged.
    pub addr_drop: Option<Field>,
    pub grammar: GrammarState,
    /// Bumped on every edit so the formatting highlighter re-runs.
    pub revision: u64,
}

/// Composer formatting toolbar actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fmt {
    Bold,
    Italic,
    Underline,
    Strike,
    Link,
    Bullets,
    Numbers,
    Quote,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    To,
    Cc,
    Bcc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsTab {
    Accounts,
    AddressBook,
    Ai,
    Appearance,
    General,
}

pub struct Settings {
    pub tab: SettingsTab,
    pub draft: Config,
    /// Index into draft.accounts being edited.
    pub account: Option<usize>,
    pub new_preset: Preset,
    pub new_email: String,
    pub imap_password: String,
    pub smtp_password: String,
    pub signatures: Vec<text_editor::Content>,
    pub ldap_passwords: Vec<String>,
    pub ai_key: String,
    pub models: Vec<String>,
    pub busy: bool,
    pub note: Option<Result<String, String>>,
    /// Ignored grammar words as typed (comma separated).
    pub grammar_ignored: String,
}

pub struct ContactsView {
    pub filter: String,
    pub all: Vec<Contact>,
    pub new_name: String,
    pub new_email: String,
}

/// One piece of the reader body: selectable text or an image.
pub enum ReaderBlock {
    Text { content: text_editor::Content, settings: crate::view::HlSettings },
    Image { src: String, link: Option<String> },
}

/// One visible line of the message list.
#[derive(Debug, Clone)]
pub struct ListRow {
    /// Index into `App::list`.
    pub idx: usize,
    /// 0 = conversation head (or single message), 1 = earlier message in it.
    pub depth: u8,
    /// Messages in the conversation (heads only).
    pub count: usize,
    pub key: String,
    pub any_unread: bool,
}

pub struct App {
    pub cfg: Config,
    pub store: Store,
    pub search: Search,
    pub mode: Mode,
    pub theme: Theme,
    pub folders: Vec<(String, Vec<Folder>)>,
    pub selection: Selection,
    pub list: Vec<Summary>,
    pub rows: Vec<ListRow>,
    /// Conversations the user folded (all are expanded by default).
    pub folded: HashSet<String>,
    pub limit: usize,
    pub query: String,
    pub results: bool,
    pub selected: Option<i64>,
    pub open: Option<Mail>,
    pub summary: Option<String>,
    /// Parsed full source of the open message (HTML + attachments).
    pub parsed: Option<Parsed>,
    pub show_html: bool,
    pub loading: bool,
    /// Decoded-on-demand images of the open message, keyed by `src`.
    pub images: HashMap<String, image::Handle>,
    /// Same keys, still holding the original bytes — for copying to the clipboard.
    pub image_bytes: HashMap<String, Vec<u8>>,
    /// Remote images: None = blocked, Some(false) = loading, Some(true) = loaded.
    pub remote_images: Option<bool>,
    /// Reader body (rebuilt when the message, view mode or images change).
    pub reader: Vec<ReaderBlock>,
    reader_blocks: Vec<mime::Block>,
    pub mods: keyboard::Modifiers,
    /// Collapsed sidebar sections (account ids / folder keys).
    pub collapsed: HashSet<String>,
    pub favorites: HashSet<String>,
    /// Folder row under the mouse (shows the favorite star).
    pub hovered: Option<String>,
    /// Message row under the cursor (a drag starts there).
    pub hover_row: Option<i64>,
    /// Mail being dragged onto a folder.
    pub drag: Option<Drag>,
    /// Folder creation in progress: account id and the typed name.
    pub new_folder: Option<(String, String)>,
    /// Last connection result per account: Ok(()) online, Err(reason) offline.
    pub acc_status: HashMap<String, Result<(), String>>,
    pub syncing: HashSet<String>,
    /// Accounts whose IDLE connection is up (instant new mail).
    pub live: HashSet<String>,
    /// Inbox changes reported while a sync was running.
    pending: HashSet<String>,
    pub last_sync: Option<i64>,
    pub ai_busy: bool,
    pub status: Option<Result<String, String>>,
    pub screen: Screen,
}

#[derive(Debug, Clone)]
pub enum Msg {
    ThemeMode(Mode),
    Key(keyboard::Key, keyboard::Modifiers),
    /// A key that is also a shortcut, arriving after the search field was
    /// asked whether it has focus (`bool` = focused).
    KeyChecked(keyboard::Key, keyboard::Modifiers, bool),
    Modifiers(keyboard::Modifiers),
    ReaderAction(usize, text_editor::Action),
    Tick,
    // mail
    SyncAll,
    Synced(String, Result<imap::SyncReport, String>),
    /// IDLE: the server reported a change in this account's inbox.
    InboxChanged(String),
    IdleDown(String),
    Loaded(i64, Result<Parsed, String>),
    ToggleHtml,
    LoadRemoteImages,
    /// Right-click on an inline image: copy it to the clipboard.
    CopyImage(String),
    ImageCopied(Result<(), String>),
    RemoteImages(i64, Vec<(String, Vec<u8>)>),
    ToggleCollapsed(String),
    CycleSidebar,
    ToggleThreads,
    FoldThread(String),
    ToggleFavorite(String),
    HoverFolder(Option<String>),
    HoverRow(i64),
    UnhoverRow(i64),
    MouseDown,
    MouseMoved(iced::Point),
    MouseUp,
    Unhover(String),
    OpenInBrowser,
    Link(String),
    OpenAttachment(usize),
    SaveAttachment(usize),
    Done(Result<String, String>),
    Select(Selection),
    Open(i64),
    MoreRows,
    Query(String),
    SubmitQuery,
    ClearQuery,
    AiSearch,
    AiSearched(Result<String, String>),
    Summarize,
    Summarized(Result<String, String>),
    ToggleSeen,
    ToggleFlag,
    /// Toggles a tag (Todo/Important keyword) on the open mail; the bit is
    /// carried as payload.
    ToggleTag(u8),
    Delete,
    Archive,
    Remote(Result<(), String>),
    /// Re-sync one account (e.g. after a draft changed on the server).
    RefreshAccount(String),
    DismissStatus,
    // compose
    NewMail,
    Reply { all: bool },
    Forward,
    ComposeAccount(String),
    ComposeSignature(String),
    DropQuote,
    /// Leave the composer and delete the draft.
    DiscardDraft,
    AutosaveDraft,
    /// (account, draft id, fingerprint, saved folder or error, user-visible)
    DraftSaved(String, String, u64, Result<String, String>, bool),
    EditDraft,
    ComposeField(Field, String),
    PickSuggestion(Field, String),
    /// Hover a recipient chip (drag may start here).
    AddrChipHover(Option<(Field, usize)>),
    /// Leave a recipient chip (matched, like UnhoverRow).
    AddrChipLeft(Field, usize),
    /// A recipient line is a drop target while a chip is dragged.
    AddrDropHover(Field),
    AddrDropLeft(Field),
    /// Release landed ON this folder while a drag was going on: drop the
    /// dragged mail here (the widget sees the release reliably, unlike the
    /// streamed cursor events that can drop/lag on fast drags).
    DroppedOn(String),
    /// Release landed on this recipient line while a chip was dragged.
    AddrDropped(Field),
    /// Start creating a folder for an account (inline input in the sidebar).
    NewFolderStart(String),
    NewFolderInput(String),
    NewFolderSubmit,
    NewFolderCancel,
    /// A folder was created on the server; refresh the folder list.
    FolderCreated(String, String),
    /// Remove a recipient chip.
    RemoveAddr(Field, usize),
    /// Turn the text being typed into a chip (Enter in a recipient input).
    CommitField(Field),
    ComposeSubject(String),
    ComposeBody(text_editor::Action),
    GrammarTick,
    /// (draft id, checked text, result)
    GrammarChecked(String, String, Result<Vec<crate::grammar::Issue>, String>),
    /// Replace issue `usize` with the suggestion.
    GrammarFix(usize, String),
    GrammarIgnore(usize),
    GrammarGo(usize),
    ToggleCc,
    Format(Fmt),
    Attach,
    Attached(Result<Vec<Attachment>, String>),
    RemoveAttachment(usize),
    FileDropped(PathBuf),
    AiPrompt(String),
    AiWrite,
    AiWritten(Result<String, String>),
    Send,
    /// Reply sent: close the composer; the captured reply target is marked
    /// answered (local + server).
    Sent(Result<(), String>, Option<(String, String, u32, i64)>),
    CloseScreen,
    // contacts
    ShowContacts,
    ContactsFilter(String),
    ContactNewName(String),
    ContactNewEmail(String),
    ContactAdd,
    ContactDelete(String, String),
    SyncContacts,
    ContactsSynced(Result<usize, String>),
    // settings
    ShowSettings,
    Settings(SettingsMsg),
}

#[derive(Debug, Clone)]
pub enum SettingsMsg {
    Tab(SettingsTab),
    EditAccount(usize),
    NewPreset(Preset),
    NewEmail(String),
    AddAccount,
    RemoveAccount,
    Acc(AccField, String),
    AccBool(AccField, bool),
    Security(bool, crate::config::Security),
    ImapPassword(String),
    SmtpPassword(String),
    AddSignature,
    RemoveSignature(usize),
    SignatureName(usize, String),
    SignatureBody(usize, text_editor::Action),
    ImportSignature(usize),
    SignatureImported(usize, Result<(String, String), String>),
    RemoveSignatureHtml(usize),
    DefaultSignature(String),
    Test,
    Tested(Result<String, String>),
    AddLdap,
    RemoveLdap(usize),
    Ldap(usize, LdapField, String),
    AddGoogle,
    RemoveGoogle(usize),
    Google(usize, GoogleField, String),
    GoogleConnect(usize),
    AiEnabled(bool),
    AiProvider(AiProvider),
    AiBaseUrl(String),
    LoadModels,
    Models(Result<Vec<String>, String>),
    AiModel(String),
    Accent(String),
    Notifications(bool),
    NotifyPreview(bool),
    AiKey(String),
    SyncInterval(String),
    GrammarEnabled(bool),
    GrammarServer(String),
    GrammarCopyCmd,
    GrammarLanguage(crate::config::GrammarLang),
    GrammarGb(bool),
    GrammarIgnored(String),
    GrammarTest,
    GrammarTested(Result<String, String>),
    Save,
}

#[derive(Debug, Clone, Copy)]
pub enum AccField {
    Name,
    DisplayName,
    Email,
    ImapHost,
    ImapPort,
    ImapUser,
    ImapInsecure,
    SmtpHost,
    SmtpPort,
    SmtpUser,
    SmtpInsecure,
    SaveSent,
}

#[derive(Debug, Clone, Copy)]
pub enum LdapField {
    Name,
    Url,
    BindDn,
    Password,
    BaseDn,
    Filter,
}

#[derive(Debug, Clone, Copy)]
pub enum GoogleField {
    Name,
    ClientId,
    ClientSecret,
}

fn err<E: std::fmt::Display>(e: E) -> String {
    format!("{e:#}")
}

/// Keyring access can block (D-Bus); keep it off the UI thread.
async fn secret(key: String) -> anyhow::Result<String> {
    tokio::task::spawn_blocking(move || match key.as_str() {
        k if k.ends_with(":imap") => secrets::imap_password(k.trim_start_matches("account:").trim_end_matches(":imap")),
        k if k.ends_with(":smtp") => secrets::smtp_password(k.trim_start_matches("account:").trim_end_matches(":smtp")),
        k => secrets::get(k)?.ok_or_else(|| anyhow::anyhow!("secret {k} not set")),
    })
    .await?
}

impl App {
    pub fn boot() -> (Self, Task<Msg>) {
        let paths = crate::config::paths().expect("no home directory");
        std::fs::create_dir_all(&paths.data_dir).expect("cannot create data dir");
        let (cfg, status) = match Config::load() {
            Ok(c) => (c, None),
            Err(e) => (Config::default(), Some(Err(format!("Config error: {e:#}")))),
        };
        let store = Store::open(&paths.data_dir.join("mail.db")).expect("cannot open database");
        let search = Search::open(&paths.data_dir.join("index")).expect("cannot open search index");
        let first_run = cfg.accounts.is_empty();
        let mut app = App {
            cfg,
            store,
            search,
            mode: Mode::Light,
            theme: crate::theme::build(Mode::Light, &Config::default().appearance.accent),
            folders: Vec::new(),
            selection: Selection::Unified(FolderRole::Inbox),
            list: Vec::new(),
            rows: Vec::new(),
            folded: HashSet::new(),
            limit: PAGE,
            query: String::new(),
            results: false,
            selected: None,
            open: None,
            summary: None,
            parsed: None,
            show_html: true,
            loading: false,
            images: HashMap::new(),
            image_bytes: HashMap::new(),
            remote_images: None,
            reader: Vec::new(),
            reader_blocks: Vec::new(),
            mods: keyboard::Modifiers::default(),
            collapsed: HashSet::new(),
            favorites: HashSet::new(),
            hovered: None,
            hover_row: None,
            drag: None,
            new_folder: None,
            acc_status: HashMap::new(),
            syncing: HashSet::new(),
            live: HashSet::new(),
            pending: HashSet::new(),
            last_sync: None,
            ai_busy: false,
            status,
            screen: Screen::Mail,
        };
        app.theme = crate::theme::build(Mode::Light, &app.cfg.appearance.accent);
        app.collapsed = app.cfg.collapsed.iter().cloned().collect();
        app.favorites = app.cfg.favorites.iter().cloned().collect();
        app.reload_folders();
        app.reload_list();
        let mut tasks = vec![iced::system::theme().map(Msg::ThemeMode)];
        if first_run {
            app.screen = Screen::Settings(Box::new(app.settings_state()));
        } else {
            tasks.push(Task::done(Msg::SyncAll));
        }
        (app, Task::batch(tasks))
    }

    pub fn title(&self) -> String {
        let unread: i64 = self
            .folders
            .iter()
            .flat_map(|(_, f)| f)
            .filter(|f| f.role == FolderRole::Inbox)
            .map(|f| f.unread)
            .sum();
        if unread > 0 { format!("Mailix ({unread})") } else { "Mailix".into() }
    }

    pub fn theme(&self) -> Option<Theme> {
        Some(self.theme.clone())
    }

    pub fn subscription(&self) -> Subscription<Msg> {
        let mut subs = vec![
            iced::system::theme_changes().map(Msg::ThemeMode),
            keyboard::listen().filter_map(|e| match e {
                keyboard::Event::KeyPressed { key, modifiers, .. } => Some(Msg::Key(key, modifiers)),
                keyboard::Event::ModifiersChanged(m) => Some(Msg::Modifiers(m)),
                _ => None,
            }),
        ];
        if self.cfg.sync_interval_minutes > 0 {
            subs.push(iced::time::every(Duration::from_secs(self.cfg.sync_interval_minutes * 60)).map(|_| Msg::Tick));
        }
        if matches!(self.screen, Screen::Compose(_)) {
            subs.push(iced::time::every(Duration::from_secs(30)).map(|_| Msg::AutosaveDraft));
            if self.cfg.grammar.enabled {
                subs.push(iced::time::every(Duration::from_millis(400)).map(|_| Msg::GrammarTick));
            }
            subs.push(iced::event::listen_with(|e, _, _| match e {
                iced::Event::Window(iced::window::Event::FileDropped(p)) => Some(Msg::FileDropped(p)),
                _ => None,
            }));
        }
        // Drag & drop of mail onto folders. Cursor moves only while a drag
        // may be going on.
        subs.push(iced::event::listen_with(|e, _, _| match e {
            iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left)) => Some(Msg::MouseDown),
            iced::Event::Mouse(iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left)) => Some(Msg::MouseUp),
            _ => None,
        }));
        let addr_drag = matches!(&self.screen, Screen::Compose(c) if c.addr_drag.is_some());
        if self.drag.is_some() || addr_drag {
            subs.push(iced::event::listen_with(|e, _, _| match e {
                iced::Event::Mouse(iced::mouse::Event::CursorMoved { position }) => Some(Msg::MouseMoved(position)),
                _ => None,
            }));
        }
        // One IDLE connection per account for instant new mail.
        for acc in &self.cfg.accounts {
            subs.push(Subscription::run_with(acc.clone(), |acc| watch(acc.clone())));
        }
        Subscription::batch(subs)
    }

    // ---- data helpers ----------------------------------------------------

    /// What a drag starting on this row moves: the whole conversation when
    /// it is the conversation's newest message, else just this mail.
    fn drag_ids(&self, id: i64) -> Vec<i64> {
        let row = self.rows.iter().find(|r| self.list[r.idx].id == id);
        match row {
            Some(r) if r.depth == 0 && r.count > 1 && !r.key.is_empty() => crate::thread::group(&self.list)
                .into_iter()
                .find(|g| g.key == r.key)
                .map(|g| g.members.iter().map(|&i| self.list[i].id).collect())
                .unwrap_or_else(|| vec![id]),
            _ => vec![id],
        }
    }

    /// Moves messages (dropped on a folder) into `folder` of `account`.
    fn move_to(&mut self, ids: &[i64], account: String, folder: String) -> Task<Msg> {
        let Some(acc) = self.account(&account).cloned() else { return Task::none() };
        let mut by_folder: HashMap<String, Vec<u32>> = HashMap::new();
        let (mut moved, mut other_account) = (Vec::new(), 0);
        for id in ids {
            let Some(s) = self.list.iter().find(|s| s.id == *id) else { continue };
            if s.account != account {
                other_account += 1;
            } else if s.folder != folder {
                by_folder.entry(s.folder.clone()).or_default().push(s.uid);
                moved.push(s.id);
            }
        }
        if moved.is_empty() {
            if other_account > 0 {
                self.status = Some(Err("Mail can only be moved to folders of its own account.".into()));
            }
            return Task::none();
        }
        for id in &moved {
            let _ = self.store.delete_message(*id);
        }
        self.search.remove(&moved);
        let _ = self.search.commit();
        if self.selected.is_some_and(|s| moved.contains(&s)) {
            self.selected = None;
            self.open = None;
        }
        self.refresh();
        let name = crate::imap::decode_folder_name(folder.rsplit(['/', '.']).next().unwrap_or(&folder));
        let n = moved.len();
        let mut note = if n == 1 { format!("Moved to {name}.") } else { format!("Moved {n} messages to {name}.") };
        if other_account > 0 {
            note.push_str(&format!(" {other_account} from another account stayed where they were."));
        }
        self.status = Some(Ok(note));
        let tasks: Vec<_> = by_folder
            .into_iter()
            .map(|(src, uids)| {
                let dest = folder.clone();
                self.remote(acc.clone(), move |a, pw| async move {
                    imap::move_messages(&a, &pw, &src, &uids, Some(&dest)).await
                })
            })
            .collect();
        // Fetch the moved mail into the target folder.
        let id = acc.id.clone();
        Task::batch(tasks).chain(Task::done(Msg::RefreshAccount(id)))
    }

    /// Whether the message lives in its account's Drafts folder.
    pub fn is_draft(&self, s: &Summary) -> bool {
        self.folders.iter().any(|(acc, fs)| {
            *acc == s.account && fs.iter().any(|f| f.name == s.folder && f.role == FolderRole::Drafts)
        })
    }

    pub fn account(&self, id: &str) -> Option<&AccountConfig> {
        self.cfg.account(id)
    }

    fn reload_folders(&mut self) {
        self.folders = self
            .cfg
            .accounts
            .iter()
            .map(|a| (a.id.clone(), self.store.folders(&a.id).unwrap_or_default()))
            .collect();
    }

    fn reload_list(&mut self) {
        let res = if self.results {
            self.search.query(&self.query, self.limit).and_then(|ids| self.store.summaries(&ids))
        } else {
            match &self.selection {
                Selection::Unified(role) => self.store.list_role(*role, self.limit, 0),
                Selection::Folder { account, name } => self.store.list(account, name, self.limit, 0),
            }
        };
        match res {
            Ok(l) => self.list = l,
            Err(e) => self.status = Some(Err(err(e))),
        }
        self.rebuild_rows();
        if let Some(id) = self.selected {
            self.open = self.store.message(id).ok().flatten();
        }
    }

    /// Lays the list out as conversations (newest message first, earlier
    /// ones indented below it) or flat, depending on the setting.
    fn rebuild_rows(&mut self) {
        // Search results are ranked by relevance; keep them flat.
        if !self.cfg.threads || self.results {
            self.rows = (0..self.list.len())
                .map(|i| ListRow { idx: i, depth: 0, count: 1, key: String::new(), any_unread: !self.list[i].seen })
                .collect();
            return;
        }
        let mut rows = Vec::with_capacity(self.list.len());
        for g in crate::thread::group(&self.list) {
            let any_unread = g.members.iter().any(|&i| !self.list[i].seen);
            rows.push(ListRow { idx: g.members[0], depth: 0, count: g.members.len(), key: g.key.clone(), any_unread });
            if g.members.len() > 1 && !self.folded.contains(&g.key) {
                for &i in &g.members[1..] {
                    rows.push(ListRow { idx: i, depth: 1, count: 1, key: g.key.clone(), any_unread: !self.list[i].seen });
                }
            }
        }
        self.rows = rows;
    }

    /// Message ids in the order they're shown.
    fn visible_ids(&self) -> Vec<i64> {
        self.rows.iter().map(|r| self.list[r.idx].id).collect()
    }

    fn refresh(&mut self) {
        self.reload_folders();
        self.reload_list();
    }

    fn current(&self) -> Option<(AccountConfig, Summary)> {
        let m = self.open.as_ref()?;
        Some((self.account(&m.summary.account)?.clone(), m.summary.clone()))
    }

    fn sync(&mut self, acc: AccountConfig) -> Task<Msg> {
        self.sync_with(acc, false)
    }

    fn sync_with(&mut self, acc: AccountConfig, inbox_only: bool) -> Task<Msg> {
        if !self.syncing.insert(acc.id.clone()) {
            if inbox_only {
                self.pending.insert(acc.id.clone());
            }
            return Task::none();
        }
        let (store, search) = (self.store.clone(), self.search.clone());
        let id = acc.id.clone();
        Task::perform(
            async move {
                let pw = secret(secrets::imap_key(&acc.id)).await?;
                if inbox_only {
                    imap::sync_inbox(&acc, &pw, &store, &search).await
                } else {
                    imap::sync_account(&acc, &pw, &store, &search).await
                }
            },
            move |r| Msg::Synced(id.clone(), r.map_err(err)),
        )
    }

    /// The original bytes of a reader image, for the clipboard: downloaded
    /// remote images and cid-embedded parts (even if not yet decoded).
    fn image_data(&self, src: &str) -> Option<Vec<u8>> {
        if let Some(b) = self.image_bytes.get(src) {
            return Some(b.clone());
        }
        let cid = src.strip_prefix("cid:")?;
        self.parsed
            .as_ref()?
            .attachments
            .iter()
            .find(|a| a.content_id.as_deref() == Some(cid))
            .map(|a| (*a.data).clone())
    }

    /// Turns the open message into selectable text blocks and images.
    fn rebuild_reader(&mut self) {        let blocks = match (&self.parsed, &self.open) {
            (Some(p), _) if self.show_html && !p.rich.is_empty() => mime::blocks(&p.rich, |src| self.images.contains_key(src)),
            (_, Some(m)) => vec![mime::Block::Text {
                text: m.body_text.trim_end().to_string(),
                lines: Vec::new(),
                links: Vec::new(),
            }],
            _ => Vec::new(),
        };
        self.reader = blocks
            .iter()
            .map(|b| match b {
                mime::Block::Text { text, lines, links } => ReaderBlock::Text {
                    content: text_editor::Content::with_text(text),
                    settings: crate::view::HlSettings(Arc::new(lines.clone()), !links.is_empty()),
                },
                mime::Block::Image { src, link } => ReaderBlock::Image { src: src.clone(), link: link.clone() },
            })
            .collect();
        self.reader_blocks = blocks;
    }

    fn save_collapsed(&mut self) {
        let mut v: Vec<String> = self.collapsed.iter().cloned().collect();
        v.sort();
        self.cfg.collapsed = v;
        let mut f: Vec<String> = self.favorites.iter().cloned().collect();
        f.sort();
        self.cfg.favorites = f;
        if let Err(e) = self.cfg.save() {
            self.status = Some(Err(format!("Could not save settings: {e:#}")));
        }
    }

    fn move_list_selection(&mut self, delta: isize) -> Task<Msg> {
        let ids = self.visible_ids();
        if ids.is_empty() {
            return Task::none();
        }
        let idx = self.selected.and_then(|id| ids.iter().position(|&x| x == id));
        let next = match idx {
            None => 0,
            Some(i) => (i as isize + delta).clamp(0, ids.len() as isize - 1) as usize,
        };
        self.update(Msg::Open(ids[next]))
    }

    fn remote<F, Fut>(&self, acc: AccountConfig, f: F) -> Task<Msg>
    where
        F: FnOnce(AccountConfig, String) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = anyhow::Result<()>> + Send + 'static,
    {
        Task::perform(
            async move {
                let pw = secret(secrets::imap_key(&acc.id)).await?;
                f(acc, pw).await
            },
            |r| Msg::Remote(r.map_err(err)),
        )
    }

    fn compose_from(&self, account: Option<&str>) -> Compose {
        let acc = account.and_then(|a| self.account(a)).or(self.cfg.accounts.first());
        Compose {
            account: acc.map(|a| a.id.clone()).unwrap_or_default(),
            signature: acc.and_then(|a| a.default_signature.clone()),
            to: String::new(),
            cc: String::new(),
            bcc: String::new(),
            show_cc: false,
            subject: String::new(),
            body: text_editor::Content::new(),
            quote: String::new(),
            in_reply_to: None,
            references: None,
            original: None,
            ai_prompt: String::new(),
            ai_busy: false,
            suggestions: Vec::new(),
            attachments: Vec::new(),
            sending: false,
            sig_preview: sig_preview(acc, acc.and_then(|a| a.default_signature.as_deref())),
            forward: None,
            draft_id: format!("<{:016x}.{:08x}@mailix>", rand::random::<u64>(), chrono::Utc::now().timestamp() as u32),
            draft_folder: None,
            draft_uid: None,
            saved_hash: 0,
            addr_hover: None,
            addr_drag: None,
            addr_drop: None,
            grammar: GrammarState::default(),
            revision: 0,
        }
    }

    fn settings_state(&self) -> Settings {
        let draft = self.cfg.clone();
        let account = if draft.accounts.is_empty() { None } else { Some(0) };
        let mut s = Settings {
            tab: SettingsTab::Accounts,
            ldap_passwords: vec![String::new(); draft.contacts.ldap.len()],
            draft,
            account: None,
            new_preset: Preset::ProtonBridge,
            new_email: String::new(),
            imap_password: String::new(),
            smtp_password: String::new(),
            signatures: Vec::new(),
            ai_key: String::new(),
            models: Vec::new(),
            busy: false,
            note: None,
            grammar_ignored: String::new(),
        };
        s.grammar_ignored = s.draft.grammar.ignored.join(", ");
        if let Some(i) = account {
            s.edit_account(i);
        }
        s
    }

    // ---- update ----------------------------------------------------------

    pub fn update(&mut self, msg: Msg) -> Task<Msg> {
        // Leaving the composer keeps the message as a draft.
        let leaving = matches!(&self.screen, Screen::Compose(c) if !c.sending)
            && matches!(
                msg,
                Msg::Select(_)
                    | Msg::Reply { .. }
                    | Msg::Forward
                    | Msg::NewMail
                    | Msg::EditDraft
                    | Msg::ShowContacts
                    | Msg::ShowSettings
                    | Msg::CloseScreen
            );
        let save = if leaving { self.save_draft(true) } else { Task::none() };
        let task = self.update_inner(msg);
        Task::batch([save, task])
    }

    /// Saves the open composer as a draft (no-op when empty or unchanged).
    /// `leaving`: the composer is being closed, so report and refresh.
    fn save_draft(&mut self, leaving: bool) -> Task<Msg> {
        let Screen::Compose(c) = &mut self.screen else { return Task::none() };
        let Some(acc) = self.cfg.account(&c.account).cloned() else { return Task::none() };
        let text = c.body.text().trim_end().to_string();
        let worth = !text.trim().is_empty()
            || (c.original.is_none()
                && (!c.to.trim().is_empty() || !c.cc.trim().is_empty() || !c.subject.trim().is_empty() || !c.attachments.is_empty()));
        let hash = draft_hash(c);
        if !worth || hash == c.saved_hash {
            return Task::none();
        }
        let mut body = text;
        if !c.quote.is_empty() {
            body.push_str("\n\n");
            body.push_str(&c.quote);
        }
        let html = crate::markup::has_markup(&body).then(|| crate::markup::document(&crate::markup::to_html(&body)));
        let out = smtp::Outgoing {
            to: c.to.clone(),
            cc: c.cc.clone(),
            bcc: c.bcc.clone(),
            subject: c.subject.clone(),
            body,
            in_reply_to: c.in_reply_to.clone(),
            references: c.references.clone(),
            attachments: c.attachments.clone(),
            html,
            inline: Vec::new(),
        };
        let sig = c.signature.clone().unwrap_or_else(|| NO_SIGNATURE.to_string());
        let raw = match smtp::build_draft(&acc, &out, &c.draft_id, Some(&sig), c.forward.as_ref().map(|f| f.source)) {
            Ok(r) => r,
            Err(e) => {
                self.status = Some(Err(format!("Could not save the draft: {e:#}")));
                return Task::none();
            }
        };
        c.saved_hash = hash;
        let folder = c
            .draft_folder
            .clone()
            .or_else(|| self.store.folder_by_role(&acc.id, FolderRole::Drafts).ok().flatten());
        let (draft_id, opened_from) = (c.draft_id.clone(), c.draft_uid);
        let backup_dir = crate::config::paths().map(|p| p.data_dir.join("drafts")).ok();
        let (acc_id, id2) = (acc.id.clone(), draft_id.clone());
        Task::perform(
            async move {
                let r = async {
                    let pw = secret(secrets::imap_key(&acc.id)).await?;
                    imap::save_draft(&acc, &pw, folder, &raw, &draft_id, opened_from).await
                }
                .await;
                r.map_err(|e| {
                    // Never lose the text: keep a local copy.
                    let kept = backup_dir.and_then(|d| {
                        std::fs::create_dir_all(&d).ok()?;
                        let name = draft_id.trim_matches(['<', '>']).replace(['@', '/'], "_");
                        let path = d.join(format!("{name}.eml"));
                        std::fs::write(&path, &raw).ok()?;
                        Some(path)
                    });
                    match kept {
                        Some(p) => format!("{e:#} — a copy was saved to {}", p.display()),
                        None => format!("{e:#}"),
                    }
                })
            },
            move |r| Msg::DraftSaved(acc_id.clone(), id2.clone(), hash, r, leaving),
        )
    }

    fn update_inner(&mut self, msg: Msg) -> Task<Msg> {
        match msg {
            Msg::ThemeMode(mode) => {
                self.mode = mode;
                self.theme = crate::theme::build(mode, &self.cfg.appearance.accent);
            }
            Msg::Key(key, mods) => {
                // Typing into the search field must not trigger "e" (archive),
                // "s" (flag) and friends, so shortcuts of that shape ask the
                // widget tree whether the search input has focus first.
                if needs_focus_check(&key, mods) {
                    let key = key.clone();
                    return operation::is_focused(SEARCH_ID)
                        .map(move |focused| Msg::KeyChecked(key.clone(), mods, focused));
                }
                return self.on_key(key, mods);
            }
            Msg::KeyChecked(key, mods, focused) => {
                if !focused {
                    return self.on_key(key, mods);
                }
            }
            Msg::Modifiers(m) => self.mods = m,
            Msg::ReaderAction(i, action) => {
                // Read-only: allow selecting, copying and moving, never editing.
                if action.is_edit() {
                    return Task::none();
                }
                let ctrl_click = matches!(action, text_editor::Action::Click(_)) && self.mods.command();
                if let Some(ReaderBlock::Text { content, .. }) = self.reader.get_mut(i) {
                    content.perform(action);
                    if ctrl_click {
                        let pos = content.cursor().position;
                        if let Some(url) = self.reader_blocks.get(i).and_then(|b| b.link_at(pos.line, pos.column)) {
                            return self.update(Msg::Link(url.to_string()));
                        }
                    }
                }
            }
            Msg::Tick | Msg::SyncAll => {
                let accs = self.cfg.accounts.clone();
                return Task::batch(accs.into_iter().map(|a| self.sync(a)));
            }
            Msg::Synced(id, r) => {
                self.syncing.remove(&id);
                let mut tasks = Vec::new();
                match r {
                    Ok(report) => {
                        self.acc_status.insert(id.clone(), Ok(()));
                        self.last_sync = Some(chrono::Local::now().timestamp());
                        if self.cfg.notifications && !report.new_inbox.is_empty() {
                            let acc = self.account(&id).map(|a| a.name.clone()).unwrap_or_default();
                            tasks.push(notify(acc, report.new_inbox, self.cfg.notify_preview));
                        }
                    }
                    Err(e) => {
                        self.acc_status.insert(id.clone(), Err(e.clone()));
                        let name = self.account(&id).map(|a| a.name.clone()).unwrap_or(id.clone());
                        self.status = Some(Err(format!("{name}: {e}")));
                    }
                }
                if self.pending.remove(&id) {
                    if let Some(acc) = self.account(&id).cloned() {
                        tasks.push(self.sync_with(acc, true));
                    }
                }
                self.refresh();
                return Task::batch(tasks);
            }
            Msg::InboxChanged(id) => {
                self.live.insert(id.clone());
                self.acc_status.insert(id.clone(), Ok(()));
                if let Some(acc) = self.account(&id).cloned() {
                    return self.sync_with(acc, true);
                }
            }
            Msg::IdleDown(id) => {
                self.live.remove(&id);
            }
            Msg::Loaded(id, r) => {
                if self.selected == Some(id) {
                    self.loading = false;
                    match r {
                        Ok(p) => {
                            self.show_html = p.html.is_some();
                            for (src, bytes) in p.local_images() {
                                self.image_bytes.insert(src.clone(), bytes.clone());
                                self.images.insert(src, image::Handle::from_bytes(bytes));
                            }
                            self.parsed = Some(p);
                            self.rebuild_reader();
                        }
                        Err(e) => self.status = Some(Err(format!("Could not load message: {e}"))),
                    }
                }
            }
            Msg::ToggleHtml => {
                self.show_html = !self.show_html;
                self.rebuild_reader();
            }
            Msg::LoadRemoteImages => {
                let (Some(p), Some(id)) = (&self.parsed, self.selected) else { return Task::none() };
                let urls = p.remote_images();
                self.remote_images = Some(false);
                return Task::perform(fetch_images(urls), move |imgs| Msg::RemoteImages(id, imgs));
            }
            Msg::CopyImage(src) => {
                let Some(bytes) = self.image_data(&src) else {
                    self.status = Some(Err("The image is not loaded yet.".into()));
                    return Task::none();
                };
                return Task::perform(
                    async move { crate::clipboard::copy_image(&bytes).await },
                    |r| Msg::ImageCopied(r.map_err(err)),
                );
            }
            Msg::ImageCopied(r) => {
                self.status = Some(match r {
                    Ok(()) => Ok("Image copied to the clipboard.".into()),
                    Err(e) => Err(format!("Copying the image failed: {e}")),
                });
            }
            Msg::RemoteImages(id, imgs) => {
                if self.selected == Some(id) {
                    self.remote_images = Some(true);
                    for (src, bytes) in imgs {
                        self.image_bytes.insert(src.clone(), bytes.clone());
                        self.images.insert(src, image::Handle::from_bytes(bytes));
                    }
                    self.rebuild_reader();
                }
            }
            Msg::ToggleCollapsed(key) => {
                if !self.collapsed.remove(&key) {
                    self.collapsed.insert(key);
                }
                self.save_collapsed();
            }
            Msg::ToggleThreads => {
                self.cfg.threads = !self.cfg.threads;
                self.rebuild_rows();
                self.save_collapsed();
            }
            Msg::FoldThread(key) => {
                if !self.folded.remove(&key) {
                    self.folded.insert(key);
                }
                self.rebuild_rows();
            }
            Msg::CycleSidebar => {
                let mode = self.cfg.sidebar_mode.next();
                self.cfg.sidebar_mode = mode;
                // Each mode starts from a clean slate; accounts can still be
                // collapsed individually afterwards.
                let ids: Vec<String> = self.cfg.accounts.iter().map(|a| a.id.clone()).collect();
                self.collapsed.retain(|k| !ids.contains(k));
                if mode == crate::config::SidebarMode::Collapsed {
                    self.collapsed.extend(ids);
                }
                self.save_collapsed();
            }
            Msg::ToggleFavorite(key) => {
                if !self.favorites.remove(&key) {
                    self.favorites.insert(key);
                }
                self.save_collapsed();
            }
            Msg::HoverFolder(key) => {
                tracing::debug!("[DnD] HoverFolder {:?}", key);
                self.hovered = key;
            }
            Msg::HoverRow(id) => self.hover_row = Some(id),
            Msg::UnhoverRow(id) => {
                if self.hover_row == Some(id) {
                    self.hover_row = None;
                }
            }
            Msg::MouseDown => {
                let hover = self.hover_row;
                tracing::debug!("[DnD] MouseDown hover_row={:?}", hover);
                if let Screen::Compose(c) = &mut self.screen {
                    if c.addr_drag.is_none() {
                        if let Some((field, idx)) = c.addr_hover {
                            c.addr_drag = Some(AddrDrag {
                                field,
                                idx,
                                start: None,
                                pos: iced::Point::ORIGIN,
                                active: false,
                            });
                        }
                    }
                }
                self.drag = match (self.hover_row, &self.screen) {
                    (Some(id), Screen::Mail) => Some(Drag { ids: self.drag_ids(id), start: None, pos: iced::Point::ORIGIN, active: false }),
                    _ => None,
                };
            }
            Msg::MouseMoved(p) => {
                if let Some(d) = &mut self.drag {
                    let before = (d.active, d.start.is_some());
                    match d.start {
                        None => d.start = Some(p),
                        Some(s) if !d.active && s.distance(p) > 6.0 => d.active = true,
                        _ => {}
                    }
                    d.pos = p;
                    if before != (d.active, d.start.is_some()) {
                        tracing::debug!("[DnD] drag transitions to {:?} at {:?}", (d.start.is_some(), d.active), p);
                    }
                }
                if let Screen::Compose(c) = &mut self.screen {
                    if let Some(d) = &mut c.addr_drag {
                        match d.start {
                            None => d.start = Some(p),
                            Some(s) if !d.active && s.distance(p) > 6.0 => d.active = true,
                            _ => {}
                        }
                        d.pos = p;
                    }
                }
            }
            Msg::MouseUp => {
                tracing::debug!("[DnD] MouseUp drag_active={:?} hovered={:?}", self.drag.as_ref().map(|d| d.active), self.hovered);
                if let Screen::Compose(c) = &mut self.screen {
                    if let Some(d) = c.addr_drag.take() {
                        if !d.active {
                            // A click pulls the chip back into the input for editing.
                            addr_edit(c, d.field, d.idx);
                            return operation::focus(addr_input_id(d.field));
                        }
                        if let Some(target) = c.addr_drop.take() {
                            if target != d.field {
                                addr_move(c, d.field, d.idx, target);
                                return operation::focus(addr_input_id(target));
                            }
                        }
                        return Task::none();
                    }
                }
                let Some(d) = self.drag.take() else { return Task::none() };
                if !d.active {
                    return Task::none();
                }
                let Some((account, folder)) = self.hovered.as_deref().and_then(|k| k.split_once('\u{1f}')) else {
                    return Task::none();
                };
                return self.move_to(&d.ids, account.to_string(), folder.to_string());
            }
            Msg::Unhover(key) => {
                // Leave events can arrive after the next row's enter.
                if self.hovered.as_deref() == Some(key.as_str()) {
                    self.hovered = None;
                }
            }
            Msg::OpenInBrowser => {
                if let (Some(p), Some(id)) = (&self.parsed, self.selected) {
                    let dir = cache_dir().join("html");
                    match mime::html_file(&dir, id, p, false) {
                        Ok(path) => {
                            if let Err(e) = open::that_detached(&path) {
                                self.status = Some(Err(err(e)));
                            }
                        }
                        Err(e) => self.status = Some(Err(err(e))),
                    }
                }
            }
            Msg::Link(url) => {
                if let Some(addr) = url.strip_prefix("mailto:") {
                    let mut c = self.compose_from(self.open.as_ref().map(|m| m.summary.account.as_str()));
                    c.to = addr.split('?').next().unwrap_or(addr).to_string();
                    self.screen = Screen::Compose(Box::new(c));
                } else if url.starts_with("https://") || url.starts_with("http://") {
                    if let Err(e) = open::that_detached(&url) {
                        self.status = Some(Err(err(e)));
                    }
                }
            }
            Msg::OpenAttachment(i) => {
                let Some(a) = self.parsed.as_ref().and_then(|p| p.attachments.get(i)).cloned() else {
                    return Task::none();
                };
                let dir = cache_dir().join("attachments").join(self.selected.unwrap_or(0).to_string());
                return Task::perform(
                    async move {
                        tokio::fs::create_dir_all(&dir).await?;
                        let path = dir.join(&a.name);
                        tokio::fs::write(&path, a.data.as_slice()).await?;
                        open::that_detached(&path)?;
                        Ok::<_, anyhow::Error>(String::new())
                    },
                    |r| Msg::Done(r.map_err(err)),
                );
            }
            Msg::SaveAttachment(i) => {
                let Some(a) = self.parsed.as_ref().and_then(|p| p.attachments.get(i)).cloned() else {
                    return Task::none();
                };
                return Task::perform(
                    async move {
                        let mut dialog = rfd::AsyncFileDialog::new().set_file_name(&a.name);
                        if let Some(dl) = directories::UserDirs::new().and_then(|d| d.download_dir().map(PathBuf::from)) {
                            dialog = dialog.set_directory(dl);
                        }
                        let Some(target) = dialog.save_file().await else { return Ok(String::new()) };
                        tokio::fs::write(target.path(), a.data.as_slice()).await?;
                        Ok::<_, anyhow::Error>(format!("Saved {}", target.path().display()))
                    },
                    |r| Msg::Done(r.map_err(err)),
                );
            }
            Msg::Done(r) => match r {
                Ok(m) if m.is_empty() => {}
                other => self.status = Some(other),
            },
            Msg::Select(sel) => {
                self.selection = sel;
                self.results = false;
                self.query.clear();
                self.limit = PAGE;
                self.screen = Screen::Mail;
                self.reload_list();
            }
            Msg::Open(id) => {
                self.selected = Some(id);
                self.summary = None;
                self.parsed = None;
                self.images.clear();
                self.image_bytes.clear();
                self.remote_images = None;
                self.open = self.store.message(id).ok().flatten();
                self.rebuild_reader();
                let Some((acc, s)) = self.current() else { return Task::none() };
                self.loading = true;
                let load = {
                    let store = self.store.clone();
                    let (acc, s) = (acc.clone(), s.clone());
                    Task::perform(
                        async move {
                            let raw = match store.raw(id)? {
                                Some(r) => r,
                                None => {
                                    let pw = secret(secrets::imap_key(&acc.id)).await?;
                                    let r = imap::fetch_raw(&acc, &pw, &s.folder, s.uid).await?;
                                    store.set_raw(id, &r)?;
                                    r
                                }
                            };
                            Ok::<_, anyhow::Error>(tokio::task::spawn_blocking(move || mime::parse(&raw)).await?)
                        },
                        move |r| Msg::Loaded(id, r.map_err(err)),
                    )
                };
                if !s.seen {
                    let _ = self.store.set_seen(id, true);
                    self.refresh();
                    let mark = self.remote(acc, move |a, pw| async move {
                        imap::set_flag(&a, &pw, &s.folder, s.uid, "\\Seen", true).await
                    });
                    return Task::batch([load, mark, unfocus()]);
                }
                return Task::batch([load, unfocus()]);
            }
            Msg::MoreRows => {
                self.limit += PAGE;
                self.reload_list();
            }
            Msg::Query(q) => {
                self.query = q;
                self.results = !self.query.trim().is_empty();
                self.limit = PAGE;
                self.reload_list();
            }
            Msg::SubmitQuery => {
                if let Some(first) = self.list.first().map(|s| s.id) {
                    return Task::batch([self.update(Msg::Open(first)), unfocus()]);
                }
                return unfocus();
            }
            Msg::ClearQuery => {
                self.query.clear();
                self.results = false;
                self.reload_list();
                return unfocus();
            }
            Msg::AiSearch => {
                if self.query.trim().is_empty() || self.ai_busy {
                    return Task::none();
                }
                self.ai_busy = true;
                let (cfg, q) = (self.cfg.ai.clone(), self.query.clone());
                return Task::perform(async move { ai::search_query(&cfg, &q).await }, |r| {
                    Msg::AiSearched(r.map_err(err))
                });
            }
            Msg::AiSearched(r) => {
                self.ai_busy = false;
                match r {
                    Ok(q) => {
                        self.status = Some(Ok(format!("AI search: {q}")));
                        return self.update(Msg::Query(q));
                    }
                    Err(e) => self.status = Some(Err(e)),
                }
            }
            Msg::Summarize => {
                if let Some(m) = &self.open {
                    self.ai_busy = true;
                    let cfg = self.cfg.ai.clone();
                    let text = format!("Subject: {}\nFrom: {}\n\n{}", m.summary.subject, m.summary.from_addr, m.body_text);
                    return Task::perform(async move { ai::summarize(&cfg, &text).await }, |r| {
                        Msg::Summarized(r.map_err(err))
                    });
                }
            }
            Msg::Summarized(r) => {
                self.ai_busy = false;
                match r {
                    Ok(s) => self.summary = Some(s),
                    Err(e) => self.status = Some(Err(e)),
                }
            }
            Msg::ToggleTag(bit) => {
                let Some((acc, s)) = self.current() else { return Task::none() };
                let keyword = if bit == crate::store::TAG_TODO {
                    crate::store::KEYWORD_TODO
                } else {
                    crate::store::KEYWORD_IMPORTANT
                };
                let on = s.tags & bit == 0;
                let _ = self.store.set_tag(s.id, bit, on);
                self.refresh();
                return self.remote(acc, move |a, pw| async move {
                    imap::set_flag(&a, &pw, &s.folder, s.uid, keyword, on).await
                });
            }
            Msg::ToggleSeen | Msg::ToggleFlag => {
                let Some((acc, s)) = self.current() else { return Task::none() };
                let (flag, on) = if matches!(msg, Msg::ToggleSeen) {
                    let _ = self.store.set_seen(s.id, !s.seen);
                    ("\\Seen", !s.seen)
                } else {
                    let _ = self.store.set_flagged(s.id, !s.flagged);
                    ("\\Flagged", !s.flagged)
                };
                self.refresh();
                return self.remote(acc, move |a, pw| async move {
                    imap::set_flag(&a, &pw, &s.folder, s.uid, flag, on).await
                });
            }
            Msg::Delete | Msg::Archive => {
                let Some((acc, s)) = self.current() else { return Task::none() };
                let role = if matches!(msg, Msg::Delete) { FolderRole::Trash } else { FolderRole::Archive };
                let dest = self.store.folder_by_role(&acc.id, role).ok().flatten();
                if matches!(msg, Msg::Archive) && dest.is_none() {
                    self.status = Some(Err("This account has no Archive folder.".into()));
                    return Task::none();
                }
                // Already in trash → delete for good.
                let dest = dest.filter(|d| *d != s.folder);
                let ids = self.visible_ids();
                let next = ids.iter().position(|&x| x == s.id).and_then(|i| {
                    ids.get(i + 1).or_else(|| i.checked_sub(1).and_then(|j| ids.get(j))).copied()
                });
                let _ = self.store.delete_message(s.id);
                self.search.remove(&[s.id]);
                let _ = self.search.commit();
                self.selected = None;
                self.open = None;
                self.refresh();
                let t = self.remote(acc.clone(), move |a, pw| async move {
                    imap::move_message(&a, &pw, &s.folder, s.uid, dest.as_deref()).await
                });
                let open_next = next.map(|id| self.update(Msg::Open(id))).unwrap_or_else(Task::none);
                return Task::batch([t, open_next]);
            }
            Msg::Remote(r) => {
                if let Err(e) = r {
                    self.status = Some(Err(format!("Server update failed: {e}")));
                }
            }
            Msg::RefreshAccount(id) => {
                if let Some(acc) = self.account(&id).cloned() {
                    return self.sync(acc);
                }
            }
            Msg::NewFolderStart(id) => {
                self.new_folder = Some((id, String::new()));
                return operation::focus(iced::widget::Id::new("new-folder-input"));
            }
            Msg::NewFolderInput(v) => {
                if let Some((_, n)) = &mut self.new_folder {
                    *n = v;
                }
            }
            Msg::NewFolderCancel => self.new_folder = None,
            Msg::NewFolderSubmit => {
                let (id, name) = match self.new_folder.take() {
                    Some(v) => v,
                    None => return Task::none(),
                };
                let name = name.trim().to_string();
                if name.is_empty() {
                    self.new_folder = Some((id, name));
                    self.status = Some(Err("Enter a folder name first.".into()));
                    return Task::none();
                }
                let Some(acc) = self.account(&id).cloned() else { return Task::none() };
                let id2 = id.clone();
                return Task::perform(
                    async move {
                        let pw = secret(secrets::imap_key(&acc.id)).await?;
                        // Return the trimmed name so the note shows it decoded.
                        crate::imap::create_folder(&acc, &pw, &name).await?;
                        Ok::<String, anyhow::Error>(name)
                    },
                    move |r| match r {
                        Ok(name) => Msg::FolderCreated(id2.clone(), name),
                        Err(e) => Msg::Remote(Err(err(e))),
                    },
                );
            }
            Msg::FolderCreated(id, name) => {
                self.new_folder = None;
                self.status = Some(Ok(format!("Folder \"{}\" created.", crate::imap::decode_folder_name(&name))));
                if let Some(acc) = self.account(&id).cloned() {
                    return self.sync(acc);
                }
            }
            Msg::DismissStatus => self.status = None,

            // ---- compose ----
            Msg::NewMail => {
                if self.cfg.accounts.is_empty() {
                    return self.update(Msg::ShowSettings);
                }
                let acc = match &self.selection {
                    Selection::Folder { account, .. } => Some(account.clone()),
                    _ => None,
                };
                self.screen = Screen::Compose(Box::new(self.compose_from(acc.as_deref())));
                return operation::focus(COMPOSE_TO_ID);
            }
            Msg::Reply { all } => {
                let Some(m) = self.open.clone() else { return Task::none() };
                let mut c = self.compose_from(Some(&m.summary.account));
                let me = self.account(&m.summary.account).map(|a| a.email.to_lowercase()).unwrap_or_default();
                c.to = if m.reply_to.is_empty() { sender(&m.summary) } else { m.reply_to.clone() };
                if all {
                    let others: Vec<&str> = m
                        .to
                        .split(", ")
                        .chain(m.cc.split(", "))
                        .filter(|a| !a.is_empty() && !a.to_lowercase().contains(&me))
                        .collect();
                    c.cc = others.join(", ");
                    c.show_cc = !c.cc.is_empty();
                }
                c.subject = prefixed("Re: ", &m.summary.subject);
                c.in_reply_to = Some(m.message_id.clone()).filter(|s| !s.is_empty());
                c.references = Some(format!("{} {}", m.references, m.message_id).trim().to_string());
                c.quote = quote(&m);
                c.original = Some(m.body_text.clone());
                self.screen = Screen::Compose(Box::new(c));
                return operation::focus(COMPOSE_BODY_ID);
            }
            Msg::Forward => {
                let Some(m) = self.open.clone() else { return Task::none() };
                let mut c = self.compose_from(Some(&m.summary.account));
                c.subject = prefixed("Fwd: ", &m.summary.subject);
                c.quote = format!(
                    "---------- Forwarded message ----------\nFrom: {}\nDate: {}\nSubject: {}\nTo: {}\n\n{}",
                    sender(&m.summary),
                    full_date(m.summary.date),
                    m.summary.subject,
                    m.to,
                    m.body_text
                );
                c.original = Some(m.body_text.clone());
                c.forward = self.parsed.as_ref().and_then(|p| forward_payload(p, m.summary.id));
                c.attachments = self.parsed.as_ref().map(|p| p.files().map(|(_, a)| a.clone()).collect()).unwrap_or_default();
                self.screen = Screen::Compose(Box::new(c));
                return operation::focus(COMPOSE_TO_ID);
            }
            Msg::ComposeAccount(id) => {
                let acc = self.account(&id);
                let sig = acc.and_then(|a| a.default_signature.clone());
                let imgs = sig_preview(acc, sig.as_deref());
                if let Screen::Compose(c) = &mut self.screen {
                    c.account = id;
                    c.signature = sig;
                    c.sig_preview = imgs;
                }
            }
            Msg::DiscardDraft => {
                let Screen::Compose(c) = std::mem::replace(&mut self.screen, Screen::Mail) else { return Task::none() };
                self.status = Some(Ok("Draft discarded.".into()));
                let (Some(folder), Some(acc)) = (c.draft_folder.clone(), self.cfg.account(&c.account).cloned()) else {
                    return Task::none();
                };
                return Task::perform(
                    async move {
                        let pw = secret(secrets::imap_key(&acc.id)).await?;
                        imap::delete_draft(&acc, &pw, &folder, &c.draft_id, c.draft_uid).await?;
                        Ok::<_, anyhow::Error>(acc.id)
                    },
                    |r| match r {
                        Ok(id) => Msg::RefreshAccount(id),
                        Err(e) => Msg::Remote(Err(err(e))),
                    },
                );
            }
            Msg::AutosaveDraft => return self.save_draft(false),
            Msg::DraftSaved(acc_id, draft_id, hash, r, leaving) => {
                let open = match &mut self.screen {
                    Screen::Compose(c) if c.draft_id == draft_id => Some(c),
                    _ => None,
                };
                match r {
                    Ok(folder) => {
                        if let Some(c) = open {
                            c.draft_folder = Some(folder.clone());
                            // The copy it was opened from is gone now.
                            c.draft_uid = None;
                        }
                        if leaving {
                            self.status = Some(Ok(format!("Draft saved to {}.", crate::imap::decode_folder_name(&folder))));
                            if let Some(acc) = self.account(&acc_id).cloned() {
                                return self.sync(acc);
                            }
                        }
                    }
                    Err(e) => {
                        if let Some(c) = open {
                            if c.saved_hash == hash {
                                c.saved_hash = 0;
                            }
                        }
                        self.status = Some(Err(format!("Draft not saved on the server: {e}")));
                    }
                }
            }
            Msg::EditDraft => {
                let Some(m) = self.open.clone() else { return Task::none() };
                let s = &m.summary;
                let mut c = self.compose_from(Some(&s.account));
                c.to = m.to.clone();
                c.cc = m.cc.clone();
                c.show_cc = !c.cc.is_empty();
                c.subject = s.subject.clone();
                c.body = text_editor::Content::with_text(m.body_text.trim_end());
                c.attachments = self.parsed.as_ref().map(|p| p.files().map(|(_, a)| a.clone()).collect()).unwrap_or_default();
                let refs: Vec<&str> = m.references.split_whitespace().collect();
                c.references = (!refs.is_empty()).then(|| refs.join(" "));
                c.in_reply_to = refs.last().map(|r| r.to_string());
                if !m.message_id.is_empty() {
                    c.draft_id = m.message_id.clone();
                }
                c.draft_folder = Some(s.folder.clone());
                c.draft_uid = Some(s.uid);
                // Signature chosen when the draft was saved by Mailix, and
                // a rich forward's source message, both carried as headers.
                let (sig, fwd) = self
                    .store
                    .raw(s.id)
                    .ok()
                    .flatten()
                    .and_then(|raw| {
                        let msg = mail_parser::MessageParser::default().parse(&raw)?;
                        let h = msg.headers();
                        let sig = h
                            .value("X-Mailix-Signature")
                            .or_else(|| h.value("X-Mailixx-Signature"))
                            .and_then(|v| v.as_text().map(|t| t.to_string()))
                            .map(|v| v.trim().to_string());
                        let fwd = h
                            .value("X-Mailix-Forward")
                            .and_then(|v| v.as_text().map(|t| t.to_string()))
                            .and_then(|v| v.trim().parse::<i64>().ok());
                        Some((sig, fwd))
                    })
                    .unwrap_or_default();
                if let Some(name) = sig {
                    c.signature = (name != NO_SIGNATURE).then_some(name);
                    c.sig_preview = sig_preview(self.cfg.account(&s.account), c.signature.as_deref());
                }
                // Rich forward: reload the original message's HTML from the
                // local cache (plain forward if it is gone).
                if let Some(source) = fwd {
                    if let Ok(Some(raw)) = self.store.raw(source) {
                        c.forward = forward_payload(&mime::parse(&raw), source);
                    }
                }
                c.saved_hash = draft_hash(&c);
                self.screen = Screen::Compose(Box::new(c));
                return operation::focus(COMPOSE_BODY_ID);
            }
            Msg::DropQuote => {
                if let Screen::Compose(c) = &mut self.screen {
                    c.quote.clear();
                }
            }
            Msg::ComposeSignature(name) => {
                let name = if name == NO_SIGNATURE { None } else { Some(name) };
                if let Screen::Compose(c) = &mut self.screen {
                    let imgs = sig_preview(self.cfg.account(&c.account), name.as_deref());
                    c.signature = name;
                    c.sig_preview = imgs;
                }
            }
            Msg::ComposeField(field, v) => {
                let v = v.trim().to_string();
                let suggestions = if v.len() >= 2 { self.store.find_contacts(&v, 6).unwrap_or_default() } else { Vec::new() };
                if let Screen::Compose(c) = &mut self.screen {
                    // Only the tail is being edited; chips stay as they are.
                    let (chips, _) = addr_parts_owned(field_mut(c, field));
                    *field_mut(c, field) = addr_text(&chips, &v);
                    c.suggestions = suggestions;
                }
            }
            Msg::PickSuggestion(field, addr) => {
                if let Screen::Compose(c) = &mut self.screen {
                    let f = field_mut(c, field);
                    let (mut chips, tail) = addr_parts_owned(f);
                    if !tail.trim().is_empty() {
                        chips.push(tail);
                    }
                    chips.push(addr);
                    *f = addr_text(&chips, "");
                    c.suggestions.clear();
                }
            }
            Msg::AddrChipHover(h) => {
                if let Screen::Compose(c) = &mut self.screen {
                    c.addr_hover = h;
                }
            }
            Msg::AddrChipLeft(field, idx) => {
                if let Screen::Compose(c) = &mut self.screen {
                    if c.addr_hover == Some((field, idx)) {
                        c.addr_hover = None;
                    }
                }
            }
            Msg::AddrDropHover(field) => {
                if let Screen::Compose(c) = &mut self.screen {
                    c.addr_drop = Some(field);
                }
            }
            Msg::DroppedOn(key) => {
                // The release happened over a folder: move the dragged mail,
                // even if the streamed cursor events never activated the drag
                // (fast drags lose `CursorMoved` messages). A click releases
                // over the row itself, so this only fires on real drops.
                let Some(d) = self.drag.take() else { return Task::none() };
                let Some((account, folder)) = key.split_once('\u{1f}') else { return Task::none() };
                return self.move_to(&d.ids, account.to_string(), folder.to_string());
            }
            Msg::AddrDropped(target) => {
                if let Screen::Compose(c) = &mut self.screen {
                    if c.addr_drag.as_ref().is_some_and(|d| target != d.field) {
                        let d = c.addr_drag.take().expect("drag present");
                        addr_move(c, d.field, d.idx, target);
                        return operation::focus(addr_input_id(target));
                    }
                    // Same line: leave the drag for the global MouseUp
                    // (a click then pulls the chip in for editing).
                }
            }
            Msg::AddrDropLeft(field) => {
                if let Screen::Compose(c) = &mut self.screen {
                    if c.addr_drop == Some(field) {
                        c.addr_drop = None;
                    }
                }
            }
            Msg::RemoveAddr(field, idx) => {
                if let Screen::Compose(c) = &mut self.screen {
                    let (mut chips, tail) = addr_parts_owned(field_mut(c, field));
                    if idx < chips.len() {
                        chips.remove(idx);
                        *field_mut(c, field) = addr_text(&chips, &tail);
                    }
                }
            }
            Msg::CommitField(field) => {
                if let Screen::Compose(c) = &mut self.screen {
                    let f = field_mut(c, field);
                    let (mut chips, tail) = addr_parts_owned(f);
                    if !tail.trim().is_empty() {
                        chips.push(tail);
                    }
                    *f = addr_text(&chips, "");
                    c.suggestions.clear();
                }
            }
            Msg::ComposeSubject(s) => {
                if let Screen::Compose(c) = &mut self.screen {
                    c.subject = s;
                }
            }
            Msg::ComposeBody(a) => {
                if let Screen::Compose(c) = &mut self.screen {
                    let edit = a.is_edit();
                    c.body.perform(a);
                    if edit {
                        refresh_formatting(c);
                    }
                }
            }
            Msg::GrammarTick => {
                let cfg = self.cfg.grammar.clone();
                let Screen::Compose(c) = &mut self.screen else { return Task::none() };
                let text = c.body.text();
                c.grammar.sync(&text);
                let due = c.grammar.due.is_some_and(|d| d <= std::time::Instant::now());
                if !cfg.enabled || !due || c.grammar.busy {
                    return Task::none();
                }
                c.grammar.due = None;
                if text.trim().is_empty() {
                    c.grammar.set(Vec::new());
                    return Task::none();
                }
                c.grammar.busy = true;
                let id = c.draft_id.clone();
                return Task::perform(
                    async move {
                        let r = crate::grammar::check(&cfg, &text).await.map_err(|e| format!("{e:#}"));
                        (text, r)
                    },
                    move |(text, r)| Msg::GrammarChecked(id.clone(), text, r),
                );
            }
            Msg::GrammarChecked(id, checked, r) => {
                let Screen::Compose(c) = &mut self.screen else { return Task::none() };
                if c.draft_id != id {
                    return Task::none();
                }
                c.grammar.busy = false;
                match r {
                    Ok(mut issues) => {
                        c.grammar.error = None;
                        // The text may have changed while checking.
                        crate::grammar::shift(&mut issues, &checked, &c.grammar.text);
                        c.grammar.set(issues);
                        refresh_formatting(c);
                    }
                    Err(e) => c.grammar.error = Some(e),
                }
            }
            Msg::GrammarFix(i, replacement) => {
                if let Screen::Compose(c) = &mut self.screen {
                    let text = c.body.text();
                    let Some(issue) = c.grammar.issues.get(i).cloned() else { return Task::none() };
                    if c.grammar.text != text || issue.range.end > text.len() {
                        return Task::none();
                    }
                    let mut new = text.clone();
                    new.replace_range(issue.range.clone(), &replacement);
                    let (line, column) = crate::grammar::position(&new, issue.range.start + replacement.len());
                    c.body = text_editor::Content::with_text(&new);
                    c.body.move_to(text_editor::Cursor {
                        position: text_editor::Position { line, column },
                        selection: None,
                    });
                    refresh_formatting(c);
                }
                return operation::focus(COMPOSE_BODY_ID);
            }
            Msg::GrammarIgnore(i) => {
                let Screen::Compose(c) = &mut self.screen else { return Task::none() };
                let Some(issue) = c.grammar.issues.get(i).cloned() else { return Task::none() };
                let Some(word) = c.grammar.text.get(issue.range.clone()).map(str::to_string) else { return Task::none() };
                let text = c.grammar.text.clone();
                let rest: Vec<_> =
                    c.grammar.issues.iter().filter(|j| text.get(j.range.clone()) != Some(word.as_str())).cloned().collect();
                c.grammar.set(rest);
                refresh_formatting(c);
                if !self.cfg.grammar.ignored.contains(&word) {
                    self.cfg.grammar.ignored.push(word);
                    if let Err(e) = self.cfg.save() {
                        self.status = Some(Err(err(e)));
                    }
                }
                return operation::focus(COMPOSE_BODY_ID);
            }
            Msg::GrammarGo(i) => {
                if let Screen::Compose(c) = &mut self.screen {
                    if let Some(issue) = c.grammar.issues.get(i) {
                        let (line, column) = crate::grammar::position(&c.grammar.text, issue.range.start);
                        c.body.move_to(text_editor::Cursor { position: text_editor::Position { line, column }, selection: None });
                    }
                }
                return operation::focus(COMPOSE_BODY_ID);
            }
            Msg::Format(f) => {
                if let Screen::Compose(c) = &mut self.screen {
                    apply_format(&mut c.body, f);
                    refresh_formatting(c);
                }
                return operation::focus(COMPOSE_BODY_ID);
            }
            Msg::ToggleCc => {
                if let Screen::Compose(c) = &mut self.screen {
                    c.show_cc = !c.show_cc;
                }
            }
            Msg::Attach => {
                return Task::perform(
                    async move {
                        let files = rfd::AsyncFileDialog::new().set_title("Attach files").pick_files().await.unwrap_or_default();
                        let mut out = Vec::new();
                        for f in files {
                            out.push(read_attachment(f.path().to_path_buf()).await?);
                        }
                        Ok::<_, anyhow::Error>(out)
                    },
                    |r| Msg::Attached(r.map_err(err)),
                );
            }
            Msg::FileDropped(path) => {
                return Task::perform(async move { read_attachment(path).await.map(|a| vec![a]) }, |r| {
                    Msg::Attached(r.map_err(err))
                });
            }
            Msg::Attached(r) => match r {
                Ok(files) => {
                    if let Screen::Compose(c) = &mut self.screen {
                        c.attachments.extend(files);
                        let total: usize = c.attachments.iter().map(|a| a.data.len()).sum();
                        if total > 25 * 1024 * 1024 {
                            self.status = Some(Err(format!(
                                "Attachments total {} — many servers reject mail over 25 MB.",
                                mime::human_size(total)
                            )));
                        }
                    }
                }
                Err(e) => self.status = Some(Err(e)),
            },
            Msg::RemoveAttachment(i) => {
                if let Screen::Compose(c) = &mut self.screen {
                    if i < c.attachments.len() {
                        c.attachments.remove(i);
                    }
                }
            }
            Msg::AiPrompt(p) => {
                if let Screen::Compose(c) = &mut self.screen {
                    c.ai_prompt = p;
                }
            }
            Msg::AiWrite => {
                let cfg = self.cfg.ai.clone();
                if let Screen::Compose(c) = &mut self.screen {
                    if c.ai_busy || c.ai_prompt.trim().is_empty() {
                        return Task::none();
                    }
                    c.ai_busy = true;
                    let (p, d, o) = (c.ai_prompt.clone(), c.body.text(), c.original.clone());
                    return Task::perform(async move { ai::write(&cfg, &p, &d, o.as_deref()).await }, |r| {
                        Msg::AiWritten(r.map_err(err))
                    });
                }
            }
            Msg::AiWritten(r) => {
                if let Screen::Compose(c) = &mut self.screen {
                    c.ai_busy = false;
                    match r {
                        Ok(t) => {
                            c.body = text_editor::Content::with_text(&t);
                            c.ai_prompt.clear();
                        }
                        Err(e) => self.status = Some(Err(e)),
                    }
                }
            }
            Msg::Send => {
                let Screen::Compose(c) = &mut self.screen else { return Task::none() };
                let Some(acc) = self.cfg.account(&c.account).cloned() else { return Task::none() };
                let text = c.body.text().trim_end().to_string();
                let sig = acc.signature(c.signature.as_deref()).cloned();
                let sig_html = sig.as_ref().and_then(|s| s.html.clone());
                let (body, html, inline) = assemble_body(
                    &text,
                    sig.as_ref().map(|s| s.body.as_str()),
                    sig_html.as_deref(),
                    &c.quote,
                    c.forward.as_ref(),
                );
                let out = smtp::Outgoing {
                    to: c.to.clone(),
                    cc: c.cc.clone(),
                    bcc: c.bcc.clone(),
                    subject: c.subject.clone(),
                    body,
                    in_reply_to: c.in_reply_to.clone(),
                    attachments: c.attachments.clone(),
                    html: html.map(|h| crate::markup::document(&h)),
                    inline,
                    references: c.references.clone(),
                };
                let msg = match smtp::build(&acc, &out) {
                    Ok(m) => m,
                    Err(e) => {
                        self.status = Some(Err(err(e)));
                        return Task::none();
                    }
                };
                c.sending = true;
                let draft = c.draft_folder.clone().map(|f| (f, c.draft_id.clone(), c.draft_uid));
                let sent_folder =
                    if acc.save_sent_copy { self.store.folder_by_role(&acc.id, FolderRole::Sent).ok().flatten() } else { None };
                // A reply marks its original answered — locally and on the server.
                let reply_target = c.in_reply_to.as_ref().and_then(|mid| {
                    self.store
                        .summary_by_message_id(&acc.id, mid)
                        .ok()
                        .flatten()
                        .map(|s| (s.account.clone(), s.folder.clone(), s.uid, s.id))
                });
                return Task::perform(
                    async move {
                        let pw = secret(secrets::smtp_key(&acc.id)).await?;
                        smtp::send(&acc, &pw, &msg).await?;
                        if let Some(folder) = sent_folder {
                            let ipw = secret(secrets::imap_key(&acc.id)).await?;
                            if let Err(e) = imap::append(&acc, &ipw, &folder, &msg.formatted()).await {
                                tracing::warn!("saving to Sent failed: {e:#}");
                            }
                        }
                        if let Some((folder, id, uid)) = draft {
                            let ipw = secret(secrets::imap_key(&acc.id)).await?;
                            if let Err(e) = imap::delete_draft(&acc, &ipw, &folder, &id, uid).await {
                                tracing::warn!("removing the draft failed: {e:#}");
                            }
                        }
                        Ok::<_, anyhow::Error>(())
                    },
                    |r| Msg::Sent(r.map_err(err), reply_target),
                );
            }
            Msg::Sent(r, replied) => match r {
                Ok(()) => {
                    self.screen = Screen::Mail;
                    self.status = Some(Ok("Message sent.".into()));
                    if let Some((account, folder, uid, id)) = replied {
                        let _ = self.store.set_answered(id, true);
                        self.refresh();
                        if let Some(acc) = self.cfg.account(&account).cloned() {
                            return self.remote(acc, move |a, pw| async move {
                                imap::set_flag(&a, &pw, &folder, uid, "\\Answered", true).await
                            });
                        }
                    }
                }
                Err(e) => {
                    if let Screen::Compose(c) = &mut self.screen {
                        c.sending = false;
                    }
                    self.status = Some(Err(e));
                }
            },
            Msg::CloseScreen => {
                self.screen = Screen::Mail;
                self.theme = crate::theme::build(self.mode, &self.cfg.appearance.accent);
                self.refresh();
            }

            // ---- contacts ----
            Msg::ShowContacts => {
                self.screen = Screen::Contacts(ContactsView {
                    filter: String::new(),
                    all: self.store.all_contacts().unwrap_or_default(),
                    new_name: String::new(),
                    new_email: String::new(),
                });
            }
            Msg::ContactsFilter(f) => {
                if let Screen::Contacts(v) = &mut self.screen {
                    v.filter = f;
                }
            }
            Msg::ContactNewName(s) => {
                if let Screen::Contacts(v) = &mut self.screen {
                    v.new_name = s;
                }
            }
            Msg::ContactNewEmail(s) => {
                if let Screen::Contacts(v) = &mut self.screen {
                    v.new_email = s;
                }
            }
            Msg::ContactAdd => {
                if let Screen::Contacts(v) = &mut self.screen {
                    if v.new_email.contains('@') {
                        let c = Contact {
                            source: "local".into(),
                            name: std::mem::take(&mut v.new_name),
                            email: std::mem::take(&mut v.new_email),
                            phone: String::new(),
                            org: String::new(),
                        };
                        if let Err(e) = self.store.add_contact(&c) {
                            self.status = Some(Err(err(e)));
                        }
                        v.all = self.store.all_contacts().unwrap_or_default();
                    }
                }
            }
            Msg::ContactDelete(source, email) => {
                let _ = self.store.delete_contact(&source, &email);
                if let Screen::Contacts(v) = &mut self.screen {
                    v.all = self.store.all_contacts().unwrap_or_default();
                }
            }
            Msg::SyncContacts => {
                let (ldap, google, store) =
                    (self.cfg.contacts.ldap.clone(), self.cfg.contacts.google.clone(), self.store.clone());
                if ldap.is_empty() && google.is_empty() {
                    self.status = Some(Err("No address books configured (Settings → Address book).".into()));
                    return Task::none();
                }
                self.status = Some(Ok("Syncing address books…".into()));
                return Task::perform(
                    async move {
                        let mut n = 0;
                        let mut errors = Vec::new();
                        for l in &ldap {
                            match contacts::sync_ldap(l).await {
                                Ok(c) => n += store.replace_contacts(&format!("ldap:{}", l.name), &c)?,
                                Err(e) => errors.push(format!("{}: {e:#}", l.name)),
                            }
                        }
                        for g in &google {
                            match contacts::sync_google(g).await {
                                Ok(c) => n += store.replace_contacts(&format!("google:{}", g.name), &c)?,
                                Err(e) => errors.push(format!("{}: {e:#}", g.name)),
                            }
                        }
                        if errors.is_empty() { Ok(n) } else { Err(anyhow::anyhow!(errors.join("; "))) }
                    },
                    |r| Msg::ContactsSynced(r.map_err(err)),
                );
            }
            Msg::ContactsSynced(r) => {
                self.status = Some(r.map(|n| format!("{n} contacts synced.")));
                if let Screen::Contacts(v) = &mut self.screen {
                    v.all = self.store.all_contacts().unwrap_or_default();
                }
            }

            // ---- settings ----
            Msg::ShowSettings => self.screen = Screen::Settings(Box::new(self.settings_state())),
            Msg::Settings(m) => return self.update_settings(m),
        }
        Task::none()
    }

    fn on_key(&mut self, key: Key, mods: keyboard::Modifiers) -> Task<Msg> {
        let cmd = mods.command();
        match (&self.screen, key.as_ref()) {
            (_, Key::Named(Named::Escape)) if self.new_folder.is_some() => self.update(Msg::NewFolderCancel),
            (_, Key::Named(Named::Escape)) if !matches!(self.screen, Screen::Mail) => self.update(Msg::CloseScreen),
            (_, Key::Named(Named::Escape)) if self.results => self.update(Msg::ClearQuery),
            (Screen::Compose(_), Key::Named(Named::Enter)) if cmd => self.update(Msg::Send),
            (Screen::Mail, Key::Character("n")) if cmd => self.update(Msg::NewMail),
            (Screen::Mail, Key::Character("f")) if cmd => operation::focus(SEARCH_ID),
            (Screen::Mail, Key::Character("/")) => operation::focus(SEARCH_ID),
            (Screen::Mail, Key::Character("r")) if cmd && mods.shift() => self.update(Msg::Reply { all: true }),
            (Screen::Mail, Key::Character("r")) if cmd => self.update(Msg::Reply { all: false }),
            (Screen::Mail, Key::Character("l")) if cmd => self.update(Msg::Forward),
            (Screen::Mail, Key::Named(Named::F5)) => self.update(Msg::SyncAll),
            (Screen::Mail, Key::Named(Named::ArrowDown) | Key::Character("j")) => self.move_list_selection(1),
            (Screen::Mail, Key::Named(Named::ArrowUp) | Key::Character("k")) => self.move_list_selection(-1),
            (Screen::Mail, Key::Named(Named::Delete)) => self.update(Msg::Delete),
            (Screen::Mail, Key::Character("e")) => self.update(Msg::Archive),
            (Screen::Mail, Key::Character("s")) => self.update(Msg::ToggleFlag),
            (Screen::Mail, Key::Character("u")) => self.update(Msg::ToggleSeen),
            (Screen::Mail, Key::Character("1")) => self.update(Msg::ToggleTag(crate::store::TAG_TODO)),
            (Screen::Mail, Key::Character("2")) => self.update(Msg::ToggleTag(crate::store::TAG_IMPORTANT)),
            _ => Task::none(),
        }
    }

    fn update_settings(&mut self, m: SettingsMsg) -> Task<Msg> {
        let Screen::Settings(s) = &mut self.screen else { return Task::none() };
        s.note = None;
        match m {
            SettingsMsg::Tab(t) => s.tab = t,
            SettingsMsg::EditAccount(i) => s.edit_account(i),
            SettingsMsg::NewPreset(p) => s.new_preset = p,
            SettingsMsg::NewEmail(e) => s.new_email = e,
            SettingsMsg::AddAccount => {
                if !s.new_email.contains('@') {
                    s.note = Some(Err("Enter an email address first.".into()));
                } else {
                    let acc = AccountConfig::from_preset(s.new_preset, s.new_email.trim());
                    s.draft.accounts.push(acc);
                    s.new_email.clear();
                    let i = s.draft.accounts.len() - 1;
                    s.edit_account(i);
                }
            }
            SettingsMsg::RemoveAccount => {
                if let Some(i) = s.account.take() {
                    s.draft.accounts.remove(i);
                    if !s.draft.accounts.is_empty() {
                        s.edit_account(0);
                    }
                }
            }
            SettingsMsg::Acc(f, v) => {
                if let Some(a) = s.account.and_then(|i| s.draft.accounts.get_mut(i)) {
                    match f {
                        AccField::Name => a.name = v,
                        AccField::DisplayName => a.display_name = v,
                        AccField::Email => a.email = v,
                        AccField::ImapHost => a.imap.host = v,
                        AccField::ImapPort => a.imap.port = v.parse().unwrap_or(a.imap.port),
                        AccField::ImapUser => a.imap.username = v,
                        AccField::SmtpHost => a.smtp.host = v,
                        AccField::SmtpPort => a.smtp.port = v.parse().unwrap_or(a.smtp.port),
                        AccField::SmtpUser => a.smtp.username = v,
                        _ => {}
                    }
                }
            }
            SettingsMsg::AccBool(f, v) => {
                if let Some(a) = s.account.and_then(|i| s.draft.accounts.get_mut(i)) {
                    match f {
                        AccField::ImapInsecure => a.imap.accept_invalid_certs = v,
                        AccField::SmtpInsecure => a.smtp.accept_invalid_certs = v,
                        AccField::SaveSent => a.save_sent_copy = v,
                        _ => {}
                    }
                }
            }
            SettingsMsg::Security(imap, sec) => {
                if let Some(a) = s.account.and_then(|i| s.draft.accounts.get_mut(i)) {
                    if imap { a.imap.security = sec } else { a.smtp.security = sec }
                }
            }
            SettingsMsg::ImapPassword(p) => s.imap_password = p,
            SettingsMsg::SmtpPassword(p) => s.smtp_password = p,
            SettingsMsg::AddSignature => {
                if let Some(a) = s.account.and_then(|i| s.draft.accounts.get_mut(i)) {
                    let name = format!("Signature {}", a.signatures.len() + 1);
                    if a.default_signature.is_none() {
                        a.default_signature = Some(name.clone());
                    }
                    let body = if a.display_name.is_empty() { a.email.clone() } else { a.display_name.clone() };
                    s.signatures.push(text_editor::Content::with_text(&body));
                    a.signatures.push(Signature { name, body, html: None });
                }
            }
            SettingsMsg::RemoveSignature(i) => {
                if let Some(a) = s.account.and_then(|i| s.draft.accounts.get_mut(i)) {
                    let removed = a.signatures.remove(i);
                    s.signatures.remove(i);
                    if a.default_signature.as_deref() == Some(&removed.name) {
                        a.default_signature = a.signatures.first().map(|s| s.name.clone());
                    }
                }
            }
            SettingsMsg::SignatureName(i, n) => {
                if let Some(a) = s.account.and_then(|i| s.draft.accounts.get_mut(i)) {
                    if a.default_signature.as_deref() == Some(&a.signatures[i].name) {
                        a.default_signature = Some(n.clone());
                    }
                    a.signatures[i].name = n;
                }
            }
            SettingsMsg::SignatureBody(i, action) => {
                if let Some(a) = s.account.and_then(|i| s.draft.accounts.get_mut(i)) {
                    s.signatures[i].perform(action);
                    a.signatures[i].body = s.signatures[i].text();
                }
            }
            SettingsMsg::ImportSignature(i) => {
                return Task::perform(
                    async move {
                        let Some(f) = rfd::AsyncFileDialog::new()
                            .set_title("Import HTML signature")
                            .add_filter("HTML", &["html", "htm"])
                            .pick_file()
                            .await
                        else {
                            return Err(String::new());
                        };
                        let path = f.path().to_path_buf();
                        tokio::task::spawn_blocking(move || crate::signature::import(&path))
                            .await
                            .map_err(|e| e.to_string())?
                            .map_err(err)
                    },
                    move |r| Msg::Settings(SettingsMsg::SignatureImported(i, r)),
                );
            }
            SettingsMsg::SignatureImported(i, r) => match r {
                Ok((html, plain)) => {
                    if let Some(a) = s.account.and_then(|i| s.draft.accounts.get_mut(i)) {
                        if let (Some(sig), Some(content)) = (a.signatures.get_mut(i), s.signatures.get_mut(i)) {
                            let n = crate::signature::images(&html).len();
                            sig.html = Some(html);
                            sig.body = plain;
                            *content = text_editor::Content::with_text(&sig.body);
                            s.note = Some(Ok(format!(
                                "HTML signature imported ({n} image{}). Click Save to keep it.",
                                if n == 1 { "" } else { "s" }
                            )));
                        }
                    }
                }
                Err(e) if e.is_empty() => {}
                Err(e) => s.note = Some(Err(e)),
            },
            SettingsMsg::RemoveSignatureHtml(i) => {
                if let Some(sig) = s.account.and_then(|a| s.draft.accounts.get_mut(a)).and_then(|a| a.signatures.get_mut(i)) {
                    sig.html = None;
                }
            }
            SettingsMsg::DefaultSignature(n) => {
                if let Some(a) = s.account.and_then(|i| s.draft.accounts.get_mut(i)) {
                    a.default_signature = if n == NO_SIGNATURE { None } else { Some(n) };
                }
            }
            SettingsMsg::Test => {
                let Some(acc) = s.account.and_then(|i| s.draft.accounts.get(i)).cloned() else { return Task::none() };
                s.busy = true;
                let typed = s.imap_password.clone();
                return Task::perform(
                    async move {
                        let pw = if typed.is_empty() { secret(secrets::imap_key(&acc.id)).await? } else { typed };
                        imap::test(&acc, &pw).await?;
                        Ok::<_, anyhow::Error>(format!("Connected to {} successfully.", acc.imap.host))
                    },
                    |r| Msg::Settings(SettingsMsg::Tested(r.map_err(err))),
                );
            }
            SettingsMsg::Tested(r) => {
                s.busy = false;
                s.note = Some(r);
            }
            SettingsMsg::AddLdap => {
                s.draft.contacts.ldap.push(LdapConfig {
                    name: format!("Directory {}", s.draft.contacts.ldap.len() + 1),
                    url: "ldaps://".into(),
                    filter: crate::config::ldap_filter(),
                    ..Default::default()
                });
                s.ldap_passwords.push(String::new());
            }
            SettingsMsg::RemoveLdap(i) => {
                s.draft.contacts.ldap.remove(i);
                s.ldap_passwords.remove(i);
            }
            SettingsMsg::Ldap(i, f, v) => {
                let l = &mut s.draft.contacts.ldap[i];
                match f {
                    LdapField::Name => l.name = v,
                    LdapField::Url => l.url = v,
                    LdapField::BindDn => l.bind_dn = v,
                    LdapField::Password => s.ldap_passwords[i] = v,
                    LdapField::BaseDn => l.base_dn = v,
                    LdapField::Filter => l.filter = v,
                }
            }
            SettingsMsg::AddGoogle => s.draft.contacts.google.push(GoogleConfig {
                name: format!("Google {}", s.draft.contacts.google.len() + 1),
                ..Default::default()
            }),
            SettingsMsg::RemoveGoogle(i) => {
                s.draft.contacts.google.remove(i);
            }
            SettingsMsg::Google(i, f, v) => {
                let g = &mut s.draft.contacts.google[i];
                match f {
                    GoogleField::Name => g.name = v,
                    GoogleField::ClientId => g.client_id = v,
                    GoogleField::ClientSecret => g.client_secret = v,
                }
            }
            SettingsMsg::GoogleConnect(i) => {
                let g = s.draft.contacts.google[i].clone();
                if g.client_id.trim().is_empty() {
                    s.note = Some(Err("Enter the OAuth client ID first.".into()));
                    return Task::none();
                }
                s.busy = true;
                s.note = Some(Ok("Complete the sign-in in your browser…".into()));
                return Task::perform(
                    async move {
                        contacts::google_authorize(&g).await?;
                        Ok::<_, anyhow::Error>(format!("{} connected. Save, then sync contacts.", g.name))
                    },
                    |r| Msg::Settings(SettingsMsg::Tested(r.map_err(err))),
                );
            }
            SettingsMsg::AiEnabled(b) => s.draft.ai.enabled = b,
            SettingsMsg::AiModel(m) => s.draft.ai.model = m,
            SettingsMsg::AiKey(k) => s.ai_key = k,
            SettingsMsg::AiProvider(p) => {
                s.draft.ai.provider = p;
                s.draft.ai.base_url = p.default_base_url().into();
                s.draft.ai.model = p.default_model().into();
                s.models.clear();
                s.ai_key.clear();
            }
            SettingsMsg::AiBaseUrl(u) => s.draft.ai.base_url = u,
            SettingsMsg::LoadModels => {
                // Store a freshly typed key first so the request can use it.
                if !s.ai_key.is_empty() {
                    if let Err(e) = secrets::set(&s.draft.ai.provider.key_name(), &s.ai_key) {
                        s.note = Some(Err(err(e)));
                        return Task::none();
                    }
                    s.ai_key.clear();
                }
                s.busy = true;
                let cfg = s.draft.ai.clone();
                return Task::perform(async move { ai::list_models(&cfg).await }, |r| {
                    Msg::Settings(SettingsMsg::Models(r.map_err(err)))
                });
            }
            SettingsMsg::Models(r) => {
                s.busy = false;
                match r {
                    Ok(m) if m.is_empty() => s.note = Some(Err("The provider returned no models.".into())),
                    Ok(m) => {
                        s.note = Some(Ok(format!("{} models available.", m.len())));
                        s.models = m;
                    }
                    Err(e) => s.note = Some(Err(e)),
                }
            }
            SettingsMsg::Accent(hex) => {
                s.draft.appearance.accent = hex;
                if crate::config::parse_hex(&s.draft.appearance.accent).is_some() {
                    // Live preview.
                    self.theme = crate::theme::build(self.mode, &s.draft.appearance.accent);
                }
            }
            SettingsMsg::Notifications(b) => s.draft.notifications = b,
            SettingsMsg::NotifyPreview(b) => s.draft.notify_preview = b,
            SettingsMsg::GrammarEnabled(b) => s.draft.grammar.enabled = b,
            SettingsMsg::GrammarServer(v) => s.draft.grammar.server = v,
            SettingsMsg::GrammarLanguage(l) => s.draft.grammar.language = l,
            SettingsMsg::GrammarGb(b) => s.draft.grammar.english_gb = b,
            SettingsMsg::GrammarIgnored(v) => {
                s.draft.grammar.ignored =
                    v.split(',').map(str::trim).filter(|w| !w.is_empty()).map(String::from).collect();
                s.grammar_ignored = v;
            }
            SettingsMsg::GrammarCopyCmd => {
                s.note = Some(Ok(
                    "Copied the command that runs a local LanguageTool server. Start it, then press Local and Test.".into(),
                ));
                return iced::clipboard::write(crate::config::LANGUAGETOOL_DOCKER.to_string());
            }
            SettingsMsg::GrammarTest => {
                s.busy = true;
                let cfg = crate::config::GrammarConfig { enabled: true, ..s.draft.grammar.clone() };
                return Task::perform(
                    async move {
                        let de = crate::grammar::check(
                            &crate::config::GrammarConfig { language: crate::config::GrammarLang::German, ..cfg.clone() },
                            "Das ist ein Tesst.",
                        )
                        .await?;
                        let en = crate::grammar::check(
                            &crate::config::GrammarConfig { language: crate::config::GrammarLang::English, ..cfg },
                            "This are a tesst.",
                        )
                        .await?;
                        anyhow::ensure!(!de.is_empty() && !en.is_empty(), "the server answered but found nothing");
                        Ok::<_, anyhow::Error>(format!(
                            "LanguageTool works: {} issue(s) in a German and {} in an English test sentence.",
                            de.len(),
                            en.len()
                        ))
                    },
                    |r| Msg::Settings(SettingsMsg::GrammarTested(r.map_err(|e| format!("{e:#}")))),
                );
            }
            SettingsMsg::GrammarTested(r) => {
                s.busy = false;
                s.note = Some(r);
            }
            SettingsMsg::SyncInterval(v) => {
                if let Ok(n) = v.parse() {
                    s.draft.sync_interval_minutes = n;
                } else if v.is_empty() {
                    s.draft.sync_interval_minutes = 0;
                }
            }
            SettingsMsg::Save => {
                s.draft.collapsed = self.cfg.collapsed.clone();
                s.draft.favorites = self.cfg.favorites.clone();
                s.draft.sidebar_mode = self.cfg.sidebar_mode;
                s.draft.threads = self.cfg.threads;
                let result = (|| -> anyhow::Result<()> {
                    if let Some(a) = s.account.and_then(|i| s.draft.accounts.get(i)) {
                        if !s.imap_password.is_empty() {
                            secrets::set(&secrets::imap_key(&a.id), &s.imap_password)?;
                        }
                        if !s.smtp_password.is_empty() {
                            secrets::set(&secrets::smtp_key(&a.id), &s.smtp_password)?;
                        }
                    }
                    for (l, pw) in s.draft.contacts.ldap.iter().zip(&s.ldap_passwords) {
                        if !pw.is_empty() {
                            secrets::set(&secrets::ldap_key(&l.name), pw)?;
                        }
                    }
                    if !s.ai_key.is_empty() {
                        secrets::set(&s.draft.ai.provider.key_name(), &s.ai_key)?;
                    }
                    s.draft.save()
                })();
                match result {
                    Err(e) => s.note = Some(Err(err(e))),
                    Ok(()) => {
                        s.imap_password.clear();
                        s.smtp_password.clear();
                        s.ai_key.clear();
                        s.ldap_passwords.iter_mut().for_each(String::clear);
                        s.note = Some(Ok("Saved.".into()));
                        // Clean up removed accounts.
                        let removed: Vec<String> = self
                            .cfg
                            .accounts
                            .iter()
                            .filter(|a| s.draft.account(&a.id).is_none())
                            .map(|a| a.id.clone())
                            .collect();
                        let added: Vec<AccountConfig> = s
                            .draft
                            .accounts
                            .iter()
                            .filter(|a| self.cfg.account(&a.id).is_none())
                            .cloned()
                            .collect();
                        self.cfg = s.draft.clone();
                        self.theme = crate::theme::build(self.mode, &self.cfg.appearance.accent);
                        for id in removed {
                            let _ = self.store.delete_account(&id);
                            self.search.remove_account(&id);
                            let _ = secrets::delete(&secrets::imap_key(&id));
                            let _ = secrets::delete(&secrets::smtp_key(&id));
                        }
                        let _ = self.search.commit();
                        self.refresh();
                        return Task::batch(added.into_iter().map(|a| self.sync(a)));
                    }
                }
            }
        }
        Task::none()
    }
}

impl Settings {
    fn edit_account(&mut self, i: usize) {
        self.account = Some(i);
        self.imap_password.clear();
        self.smtp_password.clear();
        self.signatures = self.draft.accounts[i]
            .signatures
            .iter()
            .map(|s| text_editor::Content::with_text(&s.body))
            .collect();
    }
}

pub const NO_SIGNATURE: &str = "No signature";

fn draft_hash(c: &Compose) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (&c.account, &c.to, &c.cc, &c.bcc, &c.subject, &c.quote, &c.signature).hash(&mut h);
    (c.forward.as_ref().map(|f| (f.source, f.html.len()))).hash(&mut h);
    c.body.text().trim_end().hash(&mut h);
    for a in &c.attachments {
        (&a.name, a.data.len()).hash(&mut h);
    }
    h.finish()
}

#[derive(Debug, Clone)]
pub enum SigPart {
    Text(String),
    Image(iced::widget::image::Handle),
}

fn sig_preview(acc: Option<&crate::config::AccountConfig>, name: Option<&str>) -> Vec<SigPart> {
    let Some(sig) = acc.and_then(|a| a.signature(name)) else { return Vec::new() };
    let Some(html) = sig.html.as_deref() else { return vec![SigPart::Text(sig.body.trim_end().to_string())] };
    let runs = mime::html_runs(html);
    mime::blocks(&runs, |src| src.starts_with("data:"))
        .into_iter()
        .filter_map(|b| match b {
            mime::Block::Text { text, .. } => {
                let t = text.trim_matches('\n').to_string();
                (!t.trim().is_empty()).then_some(SigPart::Text(t))
            }
            mime::Block::Image { src, .. } => crate::signature::decode_data_uri(&src)
                .map(|(_, data)| SigPart::Image(iced::widget::image::Handle::from_bytes(data))),
        })
        .collect()
}
pub const COMPOSE_BODY_ID: &str = "compose-body";
pub const COMPOSE_TO_ID: &str = "compose-to";

/// iced only applies font changes (bold/italic) from the highlighter on a
/// fresh layout, so rebuild the editor content after each edit, keeping the
/// cursor and selection. Cheap for email-sized text.
fn refresh_formatting(c: &mut Compose) {
    c.revision += 1;
    let cursor = c.body.cursor();
    let text = c.body.text();
    c.grammar.sync(&text);
    c.body = text_editor::Content::with_text(&text);
    c.body.move_to(cursor);
}

/// Index of the issue to show: the one at the cursor, else the next one
/// after it, else the first.
pub fn current_issue(c: &Compose) -> Option<usize> {
    let g = &c.grammar;
    if g.issues.is_empty() {
        return None;
    }
    let p = c.body.cursor().position;
    let at = crate::grammar::offset(&g.text, p.line, p.column);
    g.issues
        .iter()
        .position(|i| i.range.start <= at && at <= i.range.end)
        .or_else(|| g.issues.iter().position(|i| i.range.start >= at))
        .or(Some(0))
}

/// Applies a toolbar format to the composer: wraps the selection in
/// markers (or inserts an empty pair with the cursor inside), or prefixes
/// the current line for lists and quotes.
fn apply_format(body: &mut text_editor::Content, f: Fmt) {
    use text_editor::{Action, Edit, Motion};
    let paste = |body: &mut text_editor::Content, s: String| body.perform(Action::Edit(Edit::Paste(Arc::new(s))));
    let wrap = |m: &str| -> (String, String) { (m.to_string(), m.to_string()) };
    let (open, close) = match f {
        Fmt::Bold => wrap("**"),
        Fmt::Italic => wrap("*"),
        Fmt::Underline => wrap("__"),
        Fmt::Strike => wrap("~~"),
        Fmt::Link => ("[".into(), "](https://)".into()),
        Fmt::Bullets | Fmt::Numbers | Fmt::Quote => {
            let prefix = match f {
                Fmt::Bullets => "- ",
                Fmt::Numbers => "1. ",
                _ => "> ",
            };
            body.perform(Action::Move(Motion::Home));
            paste(body, prefix.into());
            body.perform(Action::Move(Motion::End));
            return;
        }
    };
    match body.selection().filter(|s| !s.is_empty()) {
        Some(sel) => {
            let back = if f == Fmt::Link { close.len() - 2 } else { 0 };
            paste(body, format!("{open}{sel}{close}"));
            // Links: put the cursor where the URL goes.
            for _ in 0..back {
                body.perform(Action::Move(Motion::Left));
            }
        }
        None => {
            let placeholder = if f == Fmt::Link { "link text" } else { "" };
            paste(body, format!("{open}{placeholder}{close}"));
            for _ in 0..close.chars().count() {
                body.perform(Action::Move(Motion::Left));
            }
        }
    }
}

fn cache_dir() -> PathBuf {
    directories::ProjectDirs::from("de", "qwellcode", "mailix")
        .map(|d| d.cache_dir().to_path_buf())
        .unwrap_or_else(std::env::temp_dir)
}

/// Downloads remote images of one message (only on explicit request).
async fn fetch_images(urls: Vec<String>) -> Vec<(String, Vec<u8>)> {
    use futures::StreamExt;
    const MAX_IMAGES: usize = 60;
    const MAX_BYTES: usize = 8 * 1024 * 1024;
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent("Mozilla/5.0 (Mailix)")
        .build()
    else {
        return Vec::new();
    };
    futures::stream::iter(urls.into_iter().take(MAX_IMAGES))
        .map(|url| {
            let client = client.clone();
            async move {
                let resp = client.get(&url).send().await.ok()?.error_for_status().ok()?;
                if resp.content_length().is_some_and(|n| n as usize > MAX_BYTES) {
                    return None;
                }
                let bytes = resp.bytes().await.ok()?;
                (bytes.len() <= MAX_BYTES).then(|| (url, bytes.to_vec()))
            }
        })
        .buffer_unordered(6)
        .filter_map(|x| async move { x })
        .collect()
        .await
}

async fn read_attachment(path: PathBuf) -> anyhow::Result<Attachment> {
    let data = tokio::fs::read(&path).await.map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
    let name = mime::sanitize_name(&path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    Ok(Attachment {
        mime: mime_guess::from_path(&path).first_or_octet_stream().essence_str().to_string(),
        name,
        data: Arc::new(data),
        content_id: None,
        embedded: false,
    })
}

/// Drops focus from whatever has it. Used to hand the keyboard back to the
/// mail shortcuts after searching, since a focused search field would keep
/// eating them.
fn unfocus() -> Task<Msg> {
    iced::advanced::widget::operate(iced::advanced::widget::operation::focusable::unfocus())
}

/// Whether a key is both a mail shortcut and something you may type into a
/// text field. Command/Control combinations never insert text, so they skip
/// the focus check; Escape, F5 and the arrows keep their meaning everywhere.
fn needs_focus_check(key: &Key, mods: keyboard::Modifiers) -> bool {
    if mods.command() {
        return false;
    }
    matches!(key, Key::Character(c) if !c.is_empty()) || matches!(key, Key::Named(Named::Delete))
}

/// Desktop notification for newly arrived inbox mail.
fn notify(account: String, new: Vec<crate::notify::NewMail>, preview: bool) -> Task<Msg> {
    let (summary, body) = crate::notify::format(&account, &new, preview);
    Task::future(async move {
        let _ = tokio::task::spawn_blocking(move || {
            notify_rust::Notification::new()
                .appname("Mailix")
                .icon("mail-unread")
                .summary(&summary)
                .body(&body)
                .show()
                .map(|_| ())
        })
        .await;
    })
    .discard()
}

/// IMAP IDLE watcher for one account. Emits `InboxChanged` on connect
/// (to catch up) and on every server push; reconnects with backoff.
fn watch(acc: AccountConfig) -> impl futures::Stream<Item = Msg> {
    iced::stream::channel(16, async move |mut tx: futures::channel::mpsc::Sender<Msg>| {
        let mut backoff = 5u64;
        loop {
            let started = std::time::Instant::now();
            let result = async {
                let pw = secret(secrets::imap_key(&acc.id)).await?;
                let mut tx2 = tx.clone();
                let id = acc.id.clone();
                imap::watch_inbox(&acc, &pw, &mut move || {
                    let _ = tx2.try_send(Msg::InboxChanged(id.clone()));
                })
                .await
            }
            .await;
            let _ = tx.send(Msg::IdleDown(acc.id.clone())).await;
            if let Err(e) = result {
                if format!("{e}").contains("does not support IDLE") {
                    // Fall back to the periodic sync only.
                    std::future::pending::<()>().await;
                }
                tracing::info!("IDLE {}: {e:#}", acc.name);
            }
            if started.elapsed() > Duration::from_secs(120) {
                backoff = 5;
            }
            tokio::time::sleep(Duration::from_secs(backoff)).await;
            backoff = (backoff * 2).min(300);
        }
    })
}

fn field_mut(c: &mut Compose, f: Field) -> &mut String {
    match f {
        Field::To => &mut c.to,
        Field::Cc => &mut c.cc,
        Field::Bcc => &mut c.bcc,
    }
}

fn addr_input_id(f: Field) -> iced::widget::Id {
    match f {
        Field::To => iced::widget::Id::new(COMPOSE_TO_ID),
        Field::Cc => iced::widget::Id::new("compose-cc"),
        Field::Bcc => iced::widget::Id::new("compose-bcc"),
    }
}

// ---- composer recipient chips ----------------------------------------------

/// The chips to render plus the tail the input shows (the tail is "" once the
/// text ends with a separator, i.e. after a commit).
pub fn addr_parts(text: &str) -> (Vec<&str>, &str) {
    let mut all = crate::smtp::split_recipients(text);
    if text.trim_end().ends_with([',', ';']) {
        (all, "")
    } else {
        let tail = all.pop().unwrap_or("");
        (all, tail)
    }
}

fn addr_parts_owned(text: &str) -> (Vec<String>, String) {
    let (chips, tail) = addr_parts(text);
    (chips.iter().map(|s| s.to_string()).collect(), tail.to_string())
}

/// Rebuilds a recipient field text from chips and an in-progress tail.
pub fn addr_text(chips: &[String], tail: &str) -> String {
    let mut parts: Vec<String> = chips.to_vec();
    let tail = tail.trim();
    if !tail.is_empty() {
        parts.push(tail.to_string());
    }
    if parts.is_empty() {
        String::new()
    } else if tail.is_empty() {
        format!("{}, ", parts.join(", "))
    } else {
        parts.join(", ")
    }
}

/// Moves chip `idx` from `src` to the end of `dst` (pure, for tests).
fn addr_move_parts(src: &str, dst: &str, idx: usize) -> Option<(String, String)> {
    let (chips, tail) = addr_parts(src);
    let moved = chips.get(idx)?.to_string();
    let rest: Vec<String> =
        chips.iter().enumerate().filter(|(i, _)| *i != idx).map(|(_, s)| s.to_string()).collect();
    let (dst_chips, dst_tail) = addr_parts(dst);
    let mut dchips: Vec<String> = dst_chips.iter().map(|s| s.to_string()).collect();
    if !dst_tail.trim().is_empty() {
        dchips.push(dst_tail.to_string());
    }
    dchips.push(moved);
    Some((addr_text(&rest, &tail), addr_text(&dchips, "")))
}

/// A click pulls a chip back into the tail for editing (pure). An in-progress
/// tail is committed first.
fn addr_edit_parts(src: &str, idx: usize) -> Option<String> {
    let (chips, tail) = addr_parts(src);
    if idx >= chips.len() {
        return None;
    }
    let mut chips: Vec<String> = chips.iter().map(|s| s.to_string()).collect();
    let moved = chips.remove(idx);
    let tail = tail.to_string();
    if !tail.trim().is_empty() {
        chips.push(tail);
    }
    Some(addr_text(&chips, &moved))
}

/// Drag a chip onto another recipient line.
fn addr_move(c: &mut Compose, from: Field, idx: usize, to: Field) {
    if from == to {
        return;
    }
    let (src_text, dst_text) = (field_mut(c, from).clone(), field_mut(c, to).clone());
    let Some((new_src, new_dst)) = addr_move_parts(&src_text, &dst_text, idx) else { return };
    *field_mut(c, from) = new_src;
    *field_mut(c, to) = new_dst;
    c.addr_hover = None;
    c.addr_drop = None;
}

/// A click pulls a chip back into the input for editing (the tail).
fn addr_edit(c: &mut Compose, field: Field, idx: usize) {
    let Some(text) = addr_edit_parts(field_mut(c, field), idx) else { return };
    *field_mut(c, field) = text;
    c.addr_hover = None;
}

/// Assembles the outgoing body from the composer parts: plain fallback,
/// optional HTML (with the signature and a forwarded mail embedded) and the
/// inline parts for embedded images. Pure, so it is unit-tested.
pub fn assemble_body(
    text: &str,
    sig_body: Option<&str>,
    sig_html: Option<&str>,
    quote: &str,
    forward: Option<&ForwardPart>,
) -> (String, Option<String>, Vec<Attachment>) {
    let formatted = crate::markup::has_markup(text);
    let mut body = if formatted { crate::markup::to_plain(text) } else { text.to_string() };
    // An HTML signature or a forwarded mail make the whole message HTML.
    let mut html = (formatted || sig_html.is_some() || forward.is_some()).then(|| crate::markup::to_html(text));
    let mut inline: Vec<Attachment> = Vec::new();
    if let Some(sb) = sig_body {
        body.push_str("\n\n-- \n");
        body.push_str(sb.trim_end());
    }
    if let Some(h) = &mut html {
        if let Some(sh) = sig_html {
            let (sh, parts) = crate::signature::inline_images(sh);
            inline = parts;
            h.push_str(&format!("<div class=\"mailix-signature\">\n{sh}\n</div>\n"));
        } else if let Some(sb) = sig_body {
            h.push_str(&format!(
                "<p style=\"color:#666\">-- <br>\n{}</p>\n",
                crate::markup::plain_html(sb.trim_end())
            ));
        }
    }
    if !quote.is_empty() {
        body.push_str("\n\n");
        body.push_str(quote);
        if let Some(h) = &mut html {
            h.push_str(&format!(
                "<blockquote style=\"margin:0 0 0 .8em;padding-left:.8em;border-left:3px solid #ccc;color:#555\">{}</blockquote>\n",
                crate::markup::plain_html(quote)
            ));
        }
    }
    if let Some(f) = forward {
        // The forwarded mail's own HTML, with its inline images merged in
        // (cid clashes with signature images are deduped).
        if let Some(h) = &mut html {
            h.push_str(&format!("<div class=\"mailix-forward\">\n{}\n</div>\n", f.html));
            let mut have: HashSet<String> = inline.iter().filter_map(|a| a.content_id.clone()).collect();
            for a in &f.images {
                if a.content_id.as_deref().is_some_and(|id| !have.insert(id.to_string())) {
                    continue;
                }
                inline.push(a.clone());
            }
        }
    }
    (body, html, inline)
}

fn sender(s: &Summary) -> String {
    if s.from_name.is_empty() { s.from_addr.clone() } else { crate::smtp::mailbox_text(&s.from_name, &s.from_addr) }
}

fn prefixed(prefix: &str, subject: &str) -> String {
    let lower = subject.to_lowercase();
    let p = prefix.trim().to_lowercase();
    if lower.starts_with(&p) || (p == "re:" && lower.starts_with("aw:")) || (p == "fwd:" && lower.starts_with("wg:")) {
        subject.to_string()
    } else {
        format!("{prefix}{subject}")
    }
}

fn quote(m: &Mail) -> String {
    let who = if m.summary.from_name.is_empty() { &m.summary.from_addr } else { &m.summary.from_name };
    let quoted: String = m.body_text.lines().map(|l| format!("> {l}\n")).collect();
    format!("On {}, {} wrote:\n{}", full_date(m.summary.date), who, quoted)
}

pub fn full_date(ts: i64) -> String {
    use chrono::{Local, TimeZone};
    Local.timestamp_opt(ts, 0).single().map(|d| d.format("%a, %d %b %Y %H:%M").to_string()).unwrap_or_default()
}

pub fn short_date(ts: i64) -> String {
    use chrono::{Datelike, Local, TimeZone};
    let Some(d) = Local.timestamp_opt(ts, 0).single() else { return String::new() };
    let now = Local::now();
    if d.date_naive() == now.date_naive() {
        d.format("%H:%M").to_string()
    } else if d.year() == now.year() {
        d.format("%d %b").to_string()
    } else {
        d.format("%d.%m.%y").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_typing_shape_keys_wait_for_the_search_focus() {
        let none = keyboard::Modifiers::default();
        let cmd = keyboard::Modifiers::COMMAND;
        // typed characters and Delete may be meant for the search field
        assert!(needs_focus_check(&Key::Character("e".into()), none));
        assert!(needs_focus_check(&Key::Character("1".into()), none));
        assert!(needs_focus_check(&Key::Named(Named::Delete), none));
        // shortcuts with a command modifier never insert text
        assert!(!needs_focus_check(&Key::Character("r".into()), cmd));
        // and these keep working while typing
        assert!(!needs_focus_check(&Key::Named(Named::Escape), none));
        assert!(!needs_focus_check(&Key::Named(Named::F5), none));
        assert!(!needs_focus_check(&Key::Named(Named::ArrowDown), none));
    }

    #[test]
    fn forward_includes_html_and_images() {
        // The DHL-style case: an HTML body with cid images and a plain note.
        let parsed = crate::mime::parse(
            b"From: a@x\r\nSubject: s\r\nContent-Type: multipart/related; boundary=\"R\"\r\n\r\n\
--R\r\nContent-Type: text/html\r\n\r\n<p>QR:</p><img src=\"cid:qr\">\r\n\
--R\r\nContent-Type: image/png\r\nContent-ID: <qr>\r\n\
Content-Disposition: inline; filename=\"qr.png\"\r\n\
Content-Transfer-Encoding: base64\r\n\r\naGVsbG8=\r\n\
--R--\r\n",
        );
        let fwd = forward_payload(&parsed, 77).expect("html forward");
        assert_eq!(fwd.source, 77);
        assert!(fwd.html.contains("cid:qr"));
        assert_eq!(fwd.images.len(), 1);
        assert_eq!(fwd.images[0].content_id.as_deref(), Some("qr"));

        // A user note + signature + the forward: images merged, html forced.
        let (body, html, inline) = assemble_body(
            "see attached",
            Some("Me"),
            None,
            "\n---------- Forwarded message ----------\nFrom: a@x\n\nQR:",
            Some(&fwd),
        );
        let html = html.expect("forward makes it HTML");
        assert!(html.contains("class=\"mailix-forward\""));
        assert!(html.contains("QR:"), "{html}");
        assert!(inline.iter().any(|a| a.content_id.as_deref() == Some("qr") && a.data.as_ref() == b"hello"));
        // Plain fallback still holds the readable text.
        assert!(body.contains("Forwarded message"));
    }

    #[test]
    fn send_without_keeps_plain() {
        let (body, html, inline) = assemble_body("plain text", None, None, "", None);
        assert_eq!(body, "plain text");
        assert_eq!(html, None);
        assert!(inline.is_empty());
    }

    #[test]
    fn subject_prefixes() {
        assert_eq!(prefixed("Re: ", "Hello"), "Re: Hello");
        assert_eq!(prefixed("Re: ", "RE: Hello"), "RE: Hello");
        assert_eq!(prefixed("Re: ", "AW: Hallo"), "AW: Hallo");
        assert_eq!(prefixed("Fwd: ", "Hello"), "Fwd: Hello");
    }

    #[test]
    fn addr_chips_roundtrip() {
        assert_eq!(addr_text(&[], "a@b"), "a@b");
        assert_eq!(addr_text(&["a@b".to_string()], ""), "a@b, ");
        assert_eq!(addr_text(&["\"a, b\" <a@b>".to_string()], "c@d"), "\"a, b\" <a@b>, c@d");

        let (chips, tail) = addr_parts("\"a, b\" <a@b>, c");
        assert_eq!(chips, vec!["\"a, b\" <a@b>"]);
        assert_eq!(tail, "c");
        // After a commit (trailing separator) the input is empty.
        let (chips, tail) = addr_parts("a@b, c@d, ");
        assert_eq!(chips, vec!["a@b", "c@d"]);
        assert_eq!(tail, "");
        // Typing continues into the tail without disturbing the chips.
        assert_eq!(addr_text(&["a@b".to_string()], "c"), "a@b, c");
    }

    #[test]
    fn addr_move_between_lines() {
        let (src, dst) = addr_move_parts("a@b, c@d, ", "x@y", 0).unwrap();
        assert_eq!(src, "c@d, ");
        assert_eq!(dst, "x@y, a@b, ");
        // Moving the only chip empties its line; the target had no tail.
        let (src, dst) = addr_move_parts("a@b, ", "", 0).unwrap();
        assert_eq!(src, "");
        assert_eq!(dst, "a@b, ");
        assert!(addr_move_parts("a@b", "x@y", 5).is_none());
    }

    #[test]
    fn addr_edit_pulls_chip() {
        // Pull the last chip: it becomes the tail, still part of the text.
        assert_eq!(addr_edit_parts("a@b, c@d, ", 1).unwrap(), "a@b, c@d");
        // Pull the first chip while a tail is being typed: the tail commits first.
        assert_eq!(addr_edit_parts("a@b, x@y", 0).unwrap(), "x@y, a@b");
        // A field still being typed has no chips to pull.
        assert!(addr_edit_parts("a@b", 0).is_none());
    }
}
