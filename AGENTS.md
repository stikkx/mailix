# Agent handbook — Mailix

Short guide for AI coding agents working on this repo. Read it before changing code.

## What this is

A native desktop mail client in Rust (Linux/macOS/Windows): IMAP-first with a local SQLite cache,
tantivy full-text search, iced 0.14 UI (no webview), optional AI assistant and LanguageTool grammar
check. Goals: fast, lightweight, clean. User-facing feature list: `README.md`.

## Commands

```sh
cargo build                      # must finish with 0 warnings
cargo test                       # unit tests, no network
cargo run --release              # the app (uses your real config + keyring!)
# end-to-end (ignored by default), needs throwaway servers:
docker run -d --rm -e USER_PASSWORD=pass -p 31143:31143 dovecot/dovecot   # STARTTLS, IDLE, drafts, move
docker run -d --rm -p 3025:3025 -p 3143:3143 -p 3465:3465 -p 3993:3993 \
  -e GREENMAIL_OPTS="-Dgreenmail.setup.test.all -Dgreenmail.hostname=0.0.0.0 -Dgreenmail.users=test:secret@localhost -Dgreenmail.auth.disabled=false" \
  greenmail/standalone:2.1.3
cargo test -- --ignored
```

Some ignored tests take a file or server via env var (`MAILIX_SIG`, `MAILIX_HTML`, `MAILIX_LT`) — see
the doc comment on each test.

## Map

| File | What lives there |
|---|---|
| `src/app.rs` | App state, `Msg`, `update`, subscriptions, compose/draft/drag logic |
| `src/view.rs` | All widgets/layout, icon codepoints (`mod ic`), editor highlighters |
| `src/theme.rs` | Colours and widget styles (all square, radius 0) |
| `src/imap.rs` | IMAP: connect, sync, IDLE, folder roles, move, append, drafts (+ e2e tests) |
| `src/smtp.rs` | Building (lettre) and sending mail; drafts via mail-builder |
| `src/mime.rs` | Parsing, HTML → selectable text runs/blocks, attachments |
| `src/markup.rs` | Composer markup (`**b**`, `*i*`, `__u__`, lists, links) → HTML/plain |
| `src/store.rs` / `src/search.rs` | SQLite cache / tantivy index |
| `src/thread.rs` | Conversation grouping |
| `src/signature.rs`, `src/grammar.rs`, `src/ai.rs`, `src/contacts.rs` | HTML signatures, LanguageTool, AI providers, LDAP/Google |
| `src/config.rs`, `src/secrets.rs` | TOML config (no secrets!), OS keyring |

## Rules

- **Secrets go to the keyring** (`secrets.rs`), never into `config.toml`, logs or test fixtures.
- **Privacy by default.** Nothing leaves the machine unless the user enabled it: AI and grammar are off by
  default, remote images are blocked until clicked, HTML is never rendered with scripts.
- **Never lose user text.** Leaving the composer saves a draft; failures keep a local `.eml` copy.
- **UI style:** square corners (radius 0), colours from the theme palette (follow OS light/dark), Lucide
  icons by codepoint in `view::ic`. Look codepoints up in `assets/fonts/lucide.ttf`; never guess them.
- **Messages to the user are plain English**, short, and say what happened and what to do.
- Keep dependencies lean; prefer what's already in `Cargo.toml`.
- Don't commit unless asked. Never touch the user's real accounts or config in tests — use the
  docker servers above or `Store::in_memory()`.

## iced 0.14 gotchas (learned the hard way)

- Keep the widget tree **structurally stable**. Wrapping/unwrapping the root (e.g. only while dragging)
  resets scroll positions and editor state. Toggle content, not structure.
- `text_editor` highlighters: font changes only apply on fresh content, so the composer rebuilds its
  `Content` after each edit (`refresh_formatting`). cosmic-text picks **one font per word**, so markup
  markers must carry the font of the text they wrap.
- The editor can't draw underlines/strikethrough — we tint instead.
- `Subscription::run_with` / `event::listen_with` need non-capturing fns.
- `mouse_area` doesn't see events a child button captured; global mouse state comes from
  `event::listen_with` (drag & drop works this way).
- Editor `Position.column` is a **byte** offset in the line.

## Mail gotchas

- Folder names are IMAP modified UTF-7 → `imap::decode_folder_name` for display.
- Folder roles: SPECIAL-USE wins; name guesses only at top level / directly under INBOX; one folder per role.
- Message-IDs are stored with `<>`; IMAP `SEARCH HEADER` wants them without.
- LanguageTool offsets are UTF-16 code units; convert to bytes (`grammar::parse`).
- Gmail/Outlook don't show `data:` images: signatures are sent as `cid:` inline parts (`multipart/related`).

## Before you say "done"

1. `cargo build` with 0 warnings, `cargo test` green; e2e when you touched IMAP/SMTP.
2. Add or extend a test for what you changed (pure logic gets unit tests; protocol code gets e2e).
3. If you changed UI and couldn't look at it, say so plainly.
4. Update `README.md` when user-visible behaviour changes.
