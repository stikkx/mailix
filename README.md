# Mailix

A fast, lightweight, IMAP-first desktop mail client in Rust for Linux, macOS and Windows.

- **Multiple accounts** with per-account signatures (pick one per message, set a default); import HTML
  signatures with images (Settings → Accounts → Import HTML…), sent as inline images
- **IMAP first**: local SQLite cache, so it starts instantly and works offline. New mail is synced
  newest-first, and flag/read/move changes are written back to the server.
- **Conversations**: replies are grouped under the newest message (by References/In-Reply-To, with a
  subject fallback), expanded by default; fold per conversation or switch to a flat list
- **Fast full-text search** (tantivy) over subject, people and body, with prefix-as-you-type matching and
  `from:` / `to:` / `subject:` fields, `"phrases"`, `OR` and `-exclude`
- **Follows the OS** light/dark setting live
- **Address book** with LDAP and Google Contacts sync, plus autocomplete from people you've emailed
- **Proton Mail** via Proton Bridge (preset included)
- **HTML mail** shown formatted in-app with embedded images and colour markings (background highlights
  and text colours, shown as tinted text); remote images (tracking pixels) are blocked until you click
  "Show images". Mail text is selectable and copyable (Ctrl+C); Ctrl+click opens links. "Open in browser"
  shows the original layout with scripts and remote content blocked.
- **Formatting editor**: bold, italic, underline, strikethrough, lists, quotes, links (toolbar or
  Ctrl+B/I/U/K). Sent as HTML with a plain-text alternative; unformatted mail stays plain text.
- **Grammar & spelling** (German and English, optional): LanguageTool marks mistakes in red while you
  write; the bar under the editor offers fixes, Ignore and next/previous. Off by default — use a local server
  (`docker run -d -p 8010:8010 erikvl87/languagetool`, copied to the clipboard from Settings → General)
  to keep your text on your machine
- **Drag & drop**: drag a mail onto a folder to move it; dragging a conversation's top row moves the
  whole conversation; recipients are chips — drag one between the To/Cc/Bcc lines, click to edit it, ×
  to remove. Display names with commas (e.g. "Rueter, Marc <…>") are handled everywhere
- **Folders**: create a folder with the folder-plus button next to an account; Enter creates it, Esc cancels
- **Tags**: click **Todo** or **Important** in the reader toolbar, or press `1` / `2` — stored as
  Thunderbird-compatible IMAP keywords (`$label4`, `$label1`), so other clients see the same tags. Tagged mail
  gets a coloured chip in the list (hover it for the name).
- **AgentProcessed**: mail whose source carries an `X-Agent-Processed` field (`X-AgentProcessed` etc. also match)
  shows a yellow badge in the list and the reader.
- **Answered sign**: when a reply has gone out — sent in Mailix (set automatically) or another client (`\Answered`
  synced) — the list shows a small reply marker.
- **Copy inline images**: right-click an image in the reader to put the original on the clipboard
  (wl-copy/xclip on Linux, osascript on macOS, PowerShell on Windows; PNG/JPEG/GIF).
- **Drafts**: leaving the composer (Esc, clicking a folder or mail) or 30 s of typing saves the message to
  the server's Drafts folder; **Edit** in the Drafts folder continues it; Discard deletes it
- **Attachments**: open/save received files, attach files (picker or drag & drop), forward with attachments
- **Instant new mail** via IMAP IDLE (status bar shows "live"), desktop notifications with the sender,
  subject and the first lines of the message (Settings → show message text, since a lock screen shows it too)
- **Optional AI assistant**: Anthropic (Claude), OpenAI, Ollama (local), Ollama Cloud, OpenCode Zen,
  OpenRouter or any OpenAI-compatible server — natural-language search, drafting/rewriting, summaries.
  Off by default; only the text of an action you trigger is sent.
- **Configurable primary color** (Settings → Appearance), square-cornered UI with Lucide icons,
  sidebar with favorite folders (one button cycles collapsed → favorites → all), online status per account,
  correct non-ASCII folder names (IMAP modified UTF-7)
- Native UI (iced), about 40 MB RAM, no webview

## Install

Download a build from the [latest release](https://github.com/stikkx/mailix/releases/latest) (`SHA256SUMS` lists checksums):

| File | For |
| --- | --- |
| `mailix…-x86_64-linux.tar.gz` | Linux x86_64 (Ubuntu 22.04+, Debian, Arch/Omarchy): unpack and run `./mailix` |
| `mailix…-x86_64-linux.AppImage` | Linux x86_64: portable AppImage — `chmod +x mailix…AppImage && ./mailix…AppImage` |
| `mailix…_amd64.deb` | Ubuntu/Debian: `sudo apt install ./mailix…_amd64.deb` |
| `PKGBUILD` | Arch/Omarchy: `makepkg -si` (binary repack of the tarball, no toolchain needed) |
| `mailix…-aarch64-apple-darwin.dmg` | macOS Apple Silicon |
| `mailix…-x86_64-apple-darwin.dmg` | macOS Intel |
| `mailix…-universal.dmg` | macOS (runs on both architectures) |
| `mailix…-x86_64-windows.zip` | Windows: unpack anywhere and run `mailix.exe` |

### Arch / Omarchy

There is a pacman repository, so Mailix updates with everything else. On Omarchy run
`omarchy update` — a plain `sudo pacman -Syu` is intercepted by Omarchy's pre-transaction hook and
refused. On plain Arch, `sudo pacman -Syu` is the way. One-time setup:

```sh
bash packaging/install-mailix.sh --arch-repo
```

which does the same as:

```sh
printf '[mailix]\nSigLevel = Optional TrustAll\nServer = https://stikkx.github.io/mailix/x86_64\n' \
  | sudo tee /etc/pacman.d/mailix.conf
sudo sed -i '/^\[core\]/i Include = /etc/pacman.d/mailix.conf' /etc/pacman.conf
sudo pacman -Sy mailix
```

The packages are built by the release workflow from the very tarball linked above, so the binary
is identical to every other download. Packages are not GPG-signed; trust comes from HTTPS plus the
checksums in `SHA256SUMS`. Afterwards: `sudo pacman -S mailix` upgrades, `pacman -Qi mailix`
shows the version.

Without the repository, `makepkg -si` with the release `PKGBUILD` builds the same package by hand.

Notes: the Linux tarball needs the usual Wayland/X11 libraries (the `.deb` pulls them in
automatically) plus a keyring provider — `libxkbcommon-x11-0 libxkbcommon0 libx11-6 libxcursor1
libxrandr2 libxi6 libxinerama1 libwayland-client0` and GNOME Keyring or KeePassXC. macOS builds are
ad-hoc signed, not notarized: if Gatekeeper refuses to open Mailix, right-click it and choose
**Open**.

## Build & run

```sh
cargo run --release
# GPU renderer instead of software rendering (smoother on large HiDPI screens):
cargo run --release --features gpu
```

Linux needs the usual Wayland/X11 libs plus a Secret Service provider (GNOME Keyring or KeePassXC)
for the keyring. macOS uses Keychain and Windows uses Credential Manager.

## Setup

On first start the Settings screen opens. Pick a preset, enter your address, adjust servers if needed,
enter your password, and click **Save**. Passwords, tokens and the AI key go into the OS keyring and never
into the config file.

| Provider | Notes |
|---|---|
| Proton Mail | Run Proton Mail Bridge, use the bridge password it shows. STARTTLS on 127.0.0.1:1143 / 1025, self-signed cert accepted. |
| Gmail | App password (needs 2-step verification). |
| Outlook / M365 | App password. OAuth-only tenants aren't supported yet. |
| Anything else | Generic IMAP/SMTP. |

Config file: `~/.config/mailix/config.toml` (Linux), `~/Library/Application Support/de.qwellcode.mailix/`
(macOS), `%APPDATA%\qwellcode\mailix\config\` (Windows).

### Address book

- **LDAP**: URL (`ldaps://…`), base DN, optional bind DN and password, and a filter.
- **Google Contacts**: create an OAuth client of type *Desktop app* in Google Cloud Console, enable the
  People API, paste the client ID/secret, then click **Connect…** to sign in through the browser.
- Click **Sync LDAP / Google** in the address book. Sync is pull-only; you can add local contacts.
- Proton contacts aren't reachable via Bridge. Import them manually or rely on sender autocomplete.

### AI assistant

Settings → AI assistant: enable it, pick a provider, add a key (not needed for local Ollama) and click
**Load models**. Endpoints: OpenAI `https://api.openai.com/v1`, Ollama `http://localhost:11434/v1`,
Ollama Cloud `https://ollama.com/v1`, OpenCode Zen `https://opencode.ai/zen/v1` (only its
chat-completions models), OpenRouter `https://openrouter.ai/api/v1`.

## Keyboard

| Key | Action |
|---|---|
| `j` / `k`, ↑ / ↓ | next / previous message |
| `/`, `Ctrl+F` | search |
| `Ctrl+N` | new message |
| `Ctrl+R`, `Ctrl+Shift+R`, `Ctrl+L` | reply, reply all, forward |
| `e`, `Del`, `s`, `u` | archive, delete, flag, toggle read |
| `Ctrl+Enter` | send |
| `Ctrl+B` / `Ctrl+I` / `Ctrl+U` / `Ctrl+K` | bold / italic / underline / link (composer) |
| `F5` | sync |
| `Esc` | close composer (keeps a draft)/settings, clear search |

## Tests

```sh
cargo test                                   # unit tests
# end-to-end against throwaway servers:
docker run -d --rm -p 3025:3025 -p 3143:3143 -p 3465:3465 -p 3993:3993 \
  -e GREENMAIL_OPTS="-Dgreenmail.setup.test.all -Dgreenmail.hostname=0.0.0.0 -Dgreenmail.users=test:secret@localhost -Dgreenmail.auth.disabled=false" \
  greenmail/standalone:2.1.3
docker run -d --rm -e USER_PASSWORD=pass -p 31143:31143 dovecot/dovecot
cargo test -- --ignored                      # e2e (sync, attachments, STARTTLS, IDLE) + OS keyring
```

## Layout

| File | Purpose |
|---|---|
| `src/imap.rs` | connect (TLS/STARTTLS/plain), sync, flag/move/append |
| `src/smtp.rs` | build and send messages (with attachments) |
| `src/markup.rs` | composer formatting → HTML / plain text |
| `src/mime.rs` | HTML → formatted text, attachments, safe "open in browser" |
| `src/store.rs` | SQLite cache: folders, messages, contacts |
| `src/search.rs` | tantivy full-text index |
| `src/contacts.rs` | LDAP + Google People API (OAuth loopback + PKCE) |
| `src/ai.rs` | AI providers (Anthropic, OpenAI-compatible) |
| `src/grammar.rs` | LanguageTool grammar/spelling check |
| `src/signature.rs` | HTML signature import, inline images |
| `src/app.rs` / `src/view.rs` / `src/theme.rs` | iced state/update, layout, styles |

## Not yet

IDLE for folders other than the inbox, OAuth2 login for Gmail/Outlook IMAP, two-way contact sync,
saving a draft when the window is closed (the 30 s autosave covers most of it).

## License

MIT — see [LICENSE](LICENSE). The bundled fonts keep their own licences:
`assets/fonts/Inter-LICENSE.txt` (OFL-1.1) and `assets/fonts/lucide-LICENSE.txt` (ISC).

