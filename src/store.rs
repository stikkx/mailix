//! Local mail cache in SQLite. All UI reads come from here, so the app
//! starts instantly and works offline; IMAP sync fills it in the background.

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FolderRole {
    Inbox,
    Sent,
    Drafts,
    Archive,
    Junk,
    Trash,
    All,
    Other,
}

impl FolderRole {
    pub fn as_str(self) -> &'static str {
        match self {
            FolderRole::Inbox => "inbox",
            FolderRole::Sent => "sent",
            FolderRole::Drafts => "drafts",
            FolderRole::Archive => "archive",
            FolderRole::Junk => "junk",
            FolderRole::Trash => "trash",
            FolderRole::All => "all",
            FolderRole::Other => "other",
        }
    }
    pub fn parse(s: &str) -> Self {
        match s {
            "inbox" => FolderRole::Inbox,
            "sent" => FolderRole::Sent,
            "drafts" => FolderRole::Drafts,
            "archive" => FolderRole::Archive,
            "junk" => FolderRole::Junk,
            "trash" => FolderRole::Trash,
            "all" => FolderRole::All,
            _ => FolderRole::Other,
        }
    }
    /// Sidebar order.
    pub fn rank(self) -> u8 {
        match self {
            FolderRole::Inbox => 0,
            FolderRole::Drafts => 1,
            FolderRole::Sent => 2,
            FolderRole::Archive => 3,
            FolderRole::All => 4,
            FolderRole::Junk => 5,
            FolderRole::Trash => 6,
            FolderRole::Other => 7,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Folder {
    pub name: String,
    pub role: FolderRole,
    pub unread: i64,
}

#[derive(Debug, Clone)]
pub struct FolderState {
    pub uidvalidity: u32,
    pub last_uid: u32,
}

#[derive(Debug, Clone)]
pub struct Summary {
    pub id: i64,
    pub account: String,
    pub folder: String,
    pub uid: u32,
    pub subject: String,
    pub from_name: String,
    pub from_addr: String,
    pub date: i64,
    pub seen: bool,
    pub flagged: bool,
    /// The mail has a reply sent for it (IMAP \Answered).
    pub answered: bool,
    pub has_attachments: bool,
    pub preview: String,
    pub message_id: String,
    pub refs: String,
    /// Tag bits (see TAG_* constants).
    pub tags: u8,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub summary: Summary,
    pub message_id: String,
    pub references: String,
    pub to: String,
    pub cc: String,
    pub reply_to: String,
    pub body_text: String,
    pub attachments: Vec<String>,
}

/// What the IMAP sync hands to the store for each new message.
#[derive(Debug, Clone)]
pub struct NewMessage {
    pub uid: u32,
    pub message_id: String,
    pub references: String,
    pub subject: String,
    pub from_name: String,
    pub from_addr: String,
    pub to: String,
    pub cc: String,
    pub reply_to: String,
    pub date: i64,
    pub seen: bool,
    pub flagged: bool,
    pub answered: bool,
    /// Tag bits: server keywords from flags plus locally derived bits.
    pub tags: u8,
    pub attachments: Vec<String>,
    pub preview: String,
    pub body_text: String,
    /// Full RFC 5322 source (HTML, attachments). Empty if not downloaded.
    pub raw: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct Contact {
    pub source: String,
    pub name: String,
    pub email: String,
    pub phone: String,
    pub org: String,
}

const SCHEMA: &str = r#"
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS folders (
    account     TEXT NOT NULL,
    name        TEXT NOT NULL,
    role        TEXT NOT NULL DEFAULT 'other',
    uidvalidity INTEGER NOT NULL DEFAULT 0,
    last_uid    INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (account, name)
);

CREATE TABLE IF NOT EXISTS messages (
    id          INTEGER PRIMARY KEY,
    account     TEXT NOT NULL,
    folder      TEXT NOT NULL,
    uid         INTEGER NOT NULL,
    message_id  TEXT NOT NULL DEFAULT '',
    refs        TEXT NOT NULL DEFAULT '',
    subject     TEXT NOT NULL DEFAULT '',
    from_name   TEXT NOT NULL DEFAULT '',
    from_addr   TEXT NOT NULL DEFAULT '',
    to_addrs    TEXT NOT NULL DEFAULT '',
    cc_addrs    TEXT NOT NULL DEFAULT '',
    reply_to    TEXT NOT NULL DEFAULT '',
    date        INTEGER NOT NULL DEFAULT 0,
    seen        INTEGER NOT NULL DEFAULT 0,
    flagged     INTEGER NOT NULL DEFAULT 0,
    answered    INTEGER NOT NULL DEFAULT 0,
    tags        INTEGER NOT NULL DEFAULT 0,
    attachments TEXT NOT NULL DEFAULT '',
    preview     TEXT NOT NULL DEFAULT '',
    body_text   TEXT NOT NULL DEFAULT '',
    UNIQUE (account, folder, uid)
);
CREATE INDEX IF NOT EXISTS messages_list ON messages (account, folder, date DESC);
CREATE INDEX IF NOT EXISTS messages_date ON messages (date DESC);

CREATE TABLE IF NOT EXISTS raw_messages (
    id   INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    data BLOB NOT NULL
);

CREATE TABLE IF NOT EXISTS contacts (
    id     INTEGER PRIMARY KEY,
    source TEXT NOT NULL,
    name   TEXT NOT NULL DEFAULT '',
    email  TEXT NOT NULL,
    phone  TEXT NOT NULL DEFAULT '',
    org    TEXT NOT NULL DEFAULT '',
    UNIQUE (source, email)
);
CREATE INDEX IF NOT EXISTS contacts_email ON contacts (email);
CREATE INDEX IF NOT EXISTS contacts_name ON contacts (name);
"#;

/// Creates the schema and migrates older databases (adds missing columns).
fn init_db(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA)?;
    let has_tags = conn
        .prepare("PRAGMA table_info(messages)")?
        .query_map([], |r| r.get::<_, String>(1))?
        .any(|c| c.as_deref() == Ok("tags"));
    if !has_tags {
        conn.execute_batch("ALTER TABLE messages ADD COLUMN tags INTEGER NOT NULL DEFAULT 0")?;
    }
    Ok(())
}

/// Tag bits, stored IMAP server-side where possible so Thunderbird and Mailix
/// agree: "Todo" and "Important" map to Thunderbird's standard keywords.
pub const TAG_TODO: u8 = 1;
pub const TAG_IMPORTANT: u8 = 2;
/// Local only: derived from the AgentProcessed header of the raw mail.
pub const TAG_AGENT: u8 = 4;
/// Thunderbird's standard keywords behind the two toggles.
pub const KEYWORD_TODO: &str = "$label4";
pub const KEYWORD_IMPORTANT: &str = "$label1";

const SUMMARY_COLS: &str =
    "id, account, folder, uid, subject, from_name, from_addr, date, seen, flagged, attachments != '', preview, message_id, refs, tags, answered";

fn summary_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Summary> {
    Ok(Summary {
        id: r.get(0)?,
        account: r.get(1)?,
        folder: r.get(2)?,
        uid: r.get(3)?,
        subject: r.get(4)?,
        from_name: r.get(5)?,
        from_addr: r.get(6)?,
        date: r.get(7)?,
        seen: r.get(8)?,
        flagged: r.get(9)?,
        has_attachments: r.get(10)?,
        preview: r.get(11)?,
        message_id: r.get(12)?,
        refs: r.get(13)?,
        tags: r.get(14)?,
        answered: r.get(15)?,
    })
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        init_db(&conn)?;
        Ok(Store { conn: Arc::new(Mutex::new(conn)) })
    }

    #[cfg(test)]
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        init_db(&conn)?;
        Ok(Store { conn: Arc::new(Mutex::new(conn)) })
    }

    fn db(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    // ---- folders ---------------------------------------------------------

    pub fn upsert_folder(&self, account: &str, name: &str, role: FolderRole) -> Result<()> {
        self.db().execute(
            "INSERT INTO folders (account, name, role) VALUES (?1, ?2, ?3)
             ON CONFLICT(account, name) DO UPDATE SET role = excluded.role",
            params![account, name, role.as_str()],
        )?;
        Ok(())
    }

    /// Removes folders that no longer exist on the server.
    pub fn retain_folders(&self, account: &str, names: &[String]) -> Result<()> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let existing: Vec<String> = {
            let mut st = tx.prepare("SELECT name FROM folders WHERE account = ?1")?;
            st.query_map([account], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?
        };
        for name in existing.iter().filter(|n| !names.contains(n)) {
            tx.execute("DELETE FROM messages WHERE account = ?1 AND folder = ?2", params![account, name])?;
            tx.execute("DELETE FROM folders WHERE account = ?1 AND name = ?2", params![account, name])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn folders(&self, account: &str) -> Result<Vec<Folder>> {
        let db = self.db();
        let mut st = db.prepare(
            "SELECT f.name, f.role,
                    (SELECT COUNT(*) FROM messages m WHERE m.account = f.account AND m.folder = f.name AND m.seen = 0)
             FROM folders f WHERE f.account = ?1",
        )?;
        let mut out: Vec<Folder> = st
            .query_map([account], |r| {
                Ok(Folder {
                    name: r.get(0)?,
                    role: FolderRole::parse(&r.get::<_, String>(1)?),
                    unread: r.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        out.sort_by(|a, b| a.role.rank().cmp(&b.role.rank()).then_with(|| a.name.cmp(&b.name)));
        Ok(out)
    }

    pub fn folder_by_role(&self, account: &str, role: FolderRole) -> Result<Option<String>> {
        Ok(self
            .db()
            .query_row(
                "SELECT name FROM folders WHERE account = ?1 AND role = ?2 LIMIT 1",
                params![account, role.as_str()],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn folder_state(&self, account: &str, folder: &str) -> Result<FolderState> {
        Ok(self
            .db()
            .query_row(
                "SELECT uidvalidity, last_uid FROM folders WHERE account = ?1 AND name = ?2",
                params![account, folder],
                |r| Ok(FolderState { uidvalidity: r.get(0)?, last_uid: r.get(1)? }),
            )
            .optional()?
            .unwrap_or(FolderState { uidvalidity: 0, last_uid: 0 }))
    }

    pub fn set_folder_state(&self, account: &str, folder: &str, st: &FolderState) -> Result<()> {
        self.db().execute(
            "UPDATE folders SET uidvalidity = ?3, last_uid = ?4 WHERE account = ?1 AND name = ?2",
            params![account, folder, st.uidvalidity, st.last_uid],
        )?;
        Ok(())
    }

    /// UIDVALIDITY changed: everything we cached for the folder is invalid.
    /// Returns the ids removed so the search index can drop them too.
    pub fn reset_folder(&self, account: &str, folder: &str) -> Result<Vec<i64>> {
        let db = self.db();
        let ids = {
            let mut st = db.prepare("SELECT id FROM messages WHERE account = ?1 AND folder = ?2")?;
            st.query_map(params![account, folder], |r| r.get(0))?.collect::<rusqlite::Result<Vec<i64>>>()?
        };
        db.execute("DELETE FROM messages WHERE account = ?1 AND folder = ?2", params![account, folder])?;
        Ok(ids)
    }

    // ---- messages --------------------------------------------------------

    /// Inserts messages, returning (row id, message) for those newly added.
    pub fn insert_messages(&self, account: &str, folder: &str, msgs: Vec<NewMessage>) -> Result<Vec<(i64, NewMessage)>> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let mut added = Vec::with_capacity(msgs.len());
        {
            let mut st = tx.prepare(
                "INSERT OR IGNORE INTO messages
                 (account, folder, uid, message_id, refs, subject, from_name, from_addr, to_addrs, cc_addrs,
                  reply_to, date, seen, flagged, answered, attachments, preview, body_text, tags)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
            )?;
            let mut raw = tx.prepare("INSERT OR REPLACE INTO raw_messages (id, data) VALUES (?1, ?2)")?;
            for mut m in msgs {
                let n = st.execute(params![
                    account,
                    folder,
                    m.uid,
                    m.message_id,
                    m.references,
                    m.subject,
                    m.from_name,
                    m.from_addr,
                    m.to,
                    m.cc,
                    m.reply_to,
                    m.date,
                    m.seen,
                    m.flagged,
                    m.answered,
                    m.attachments.join("\n"),
                    m.preview,
                    m.body_text,
                    m.tags,
                ])?;
                if n == 1 {
                    let id = tx.last_insert_rowid();
                    if !m.raw.is_empty() {
                        raw.execute(params![id, m.raw])?;
                    }
                    m.raw = Vec::new();
                    added.push((id, m));
                }
            }
        }
        tx.commit()?;
        Ok(added)
    }

    /// Applies server flags for known UIDs and deletes the ones that vanished.
    /// Returns ids of deleted rows. Tags: keyword bits come from the server;
    /// local bits (AgentProcessed) survive.
    pub fn reconcile_flags(
        &self,
        account: &str,
        folder: &str,
        server: &[(u32, bool, bool, u8, bool)],
    ) -> Result<Vec<i64>> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let local: Vec<(i64, u32)> = {
            let mut st = tx.prepare("SELECT id, uid FROM messages WHERE account = ?1 AND folder = ?2")?;
            st.query_map(params![account, folder], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?
        };
        let server_map: std::collections::HashMap<u32, (bool, bool, u8, bool)> =
            server.iter().map(|&(u, s, f, t, a)| (u, (s, f, t, a))).collect();
        let mut removed = Vec::new();
        {
            let mut upd = tx.prepare(
                "UPDATE messages SET seen = ?2, flagged = ?3, tags = (tags & ~3) | ?4, answered = ?5 WHERE id = ?1",
            )?;
            let mut del = tx.prepare("DELETE FROM messages WHERE id = ?1")?;
            for (id, uid) in local {
                match server_map.get(&uid) {
                    Some(&(seen, flagged, tags, answered)) => {
                        upd.execute(params![id, seen, flagged, tags, answered])?;
                    }
                    None => {
                        del.execute([id])?;
                        removed.push(id);
                    }
                }
            }
        }
        tx.commit()?;
        Ok(removed)
    }

    pub fn list(&self, account: &str, folder: &str, limit: usize, offset: usize) -> Result<Vec<Summary>> {
        let db = self.db();
        let mut st = db.prepare(&format!(
            "SELECT {SUMMARY_COLS} FROM messages WHERE account = ?1 AND folder = ?2
             ORDER BY date DESC LIMIT ?3 OFFSET ?4"
        ))?;
        Ok(st
            .query_map(params![account, folder, limit as i64, offset as i64], summary_row)?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// Unified inbox across all accounts.
    pub fn list_role(&self, role: FolderRole, limit: usize, offset: usize) -> Result<Vec<Summary>> {
        let db = self.db();
        let mut st = db.prepare(&format!(
            "SELECT {SUMMARY_COLS} FROM messages m
             WHERE EXISTS (SELECT 1 FROM folders f WHERE f.account = m.account AND f.name = m.folder AND f.role = ?1)
             ORDER BY date DESC LIMIT ?2 OFFSET ?3"
        ))?;
        Ok(st
            .query_map(params![role.as_str(), limit as i64, offset as i64], summary_row)?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn summaries(&self, ids: &[i64]) -> Result<Vec<Summary>> {
        let db = self.db();
        let mut st = db.prepare_cached(&format!("SELECT {SUMMARY_COLS} FROM messages WHERE id = ?1"))?;
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(s) = st.query_row([id], summary_row).optional()? {
                out.push(s);
            }
        }
        Ok(out)
    }

    pub fn message(&self, id: i64) -> Result<Option<Message>> {
        let db = self.db();
        Ok(db
            .query_row(
                &format!(
                    "SELECT {SUMMARY_COLS}, to_addrs, cc_addrs, reply_to, body_text, attachments
                     FROM messages WHERE id = ?1"
                ),
                [id],
                |r| {
                    // SUMMARY_COLS ends at `answered` (index 15); extras follow.
                    let att: String = r.get(20)?;
                    Ok(Message {
                        summary: summary_row(r)?,
                        message_id: r.get(12)?,
                        references: r.get(13)?,
                        to: r.get(16)?,
                        cc: r.get(17)?,
                        reply_to: r.get(18)?,
                        body_text: r.get(19)?,
                        attachments: att.lines().filter(|l| !l.is_empty()).map(String::from).collect(),
                    })
                },
            )
            .optional()?)
    }

    pub fn raw(&self, id: i64) -> Result<Option<Vec<u8>>> {
        Ok(self.db().query_row("SELECT data FROM raw_messages WHERE id = ?1", [id], |r| r.get(0)).optional()?)
    }

    pub fn set_raw(&self, id: i64, data: &[u8]) -> Result<()> {
        self.db().execute("INSERT OR REPLACE INTO raw_messages (id, data) VALUES (?1, ?2)", params![id, data])?;
        Ok(())
    }

    pub fn set_seen(&self, id: i64, seen: bool) -> Result<()> {
        self.db().execute("UPDATE messages SET seen = ?2 WHERE id = ?1", params![id, seen])?;
        Ok(())
    }

    pub fn set_flagged(&self, id: i64, flagged: bool) -> Result<()> {
        self.db().execute("UPDATE messages SET flagged = ?2 WHERE id = ?1", params![id, flagged])?;
        Ok(())
    }

    pub fn set_answered(&self, id: i64, answered: bool) -> Result<()> {
        self.db().execute("UPDATE messages SET answered = ?2 WHERE id = ?1", params![id, answered])?;
        Ok(())
    }

    /// The summary of the mail with this Message-ID, for marking a reply's
    /// source as answered.
    pub fn summary_by_message_id(&self, account: &str, mid: &str) -> Result<Option<Summary>> {
        let db = self.db();
        let sql = format!("SELECT {SUMMARY_COLS} FROM messages WHERE account = ?1 AND message_id = ?2");
        Ok(db.query_row(&sql, params![account, mid], summary_row).optional()?)
    }

    /// Sets or clears one tag bit. The AgentProcessed bit is local only and is
    /// never toggled from here.
    pub fn set_tag(&self, id: i64, bit: u8, on: bool) -> Result<()> {
        self.db().execute(
            "UPDATE messages SET tags = CASE WHEN ?2 THEN tags | ?3 ELSE tags & ~?3 END WHERE id = ?1",
            params![id, on, bit as i64],
        )?;
        Ok(())
    }

    /// Marks cached mail whose raw headers carry an AgentProcessed field
    /// (name without hyphens: X-Agent-Processed, X-AgentProcessed, …). Runs
    /// per sync; existing rows with the tag set are skipped. Returns how many
    /// rows were newly tagged.
    pub fn backfill_agent_tags(&self) -> Result<usize> {
        // Header block = raw source up to the first blank line; scan it for
        // the AgentProcessed field (any hyphenation) case-insensitively.
        const HEAD: &str =
            "lower(CAST(substr(r.data, 1, coalesce(instr(r.data, X'0D0A0D0A'), 30000)) AS TEXT))";
        let sql = format!(
            "UPDATE messages SET tags = tags | ?1 \
             WHERE (tags & ?1) = 0 AND EXISTS (\
               SELECT 1 FROM raw_messages r WHERE r.id = messages.id AND (\
                 instr({HEAD}, 'agentprocessed') > 0 OR \
                 instr({HEAD}, 'agent-processed') > 0))"
        );
        let n = self.db().execute(&sql, params![TAG_AGENT])?;
        Ok(n)
    }

    pub fn delete_message(&self, id: i64) -> Result<()> {
        self.db().execute("DELETE FROM messages WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn delete_account(&self, account: &str) -> Result<Vec<i64>> {
        let db = self.db();
        let ids = {
            let mut st = db.prepare("SELECT id FROM messages WHERE account = ?1")?;
            st.query_map([account], |r| r.get(0))?.collect::<rusqlite::Result<Vec<i64>>>()?
        };
        db.execute("DELETE FROM messages WHERE account = ?1", [account])?;
        db.execute("DELETE FROM folders WHERE account = ?1", [account])?;
        Ok(ids)
    }

    // ---- contacts --------------------------------------------------------

    pub fn replace_contacts(&self, source: &str, contacts: &[Contact]) -> Result<usize> {
        let mut db = self.db();
        let tx = db.transaction()?;
        tx.execute("DELETE FROM contacts WHERE source = ?1", [source])?;
        {
            let mut st = tx.prepare(
                "INSERT OR REPLACE INTO contacts (source, name, email, phone, org) VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for c in contacts {
                st.execute(params![source, c.name, c.email.to_lowercase(), c.phone, c.org])?;
            }
        }
        tx.commit()?;
        Ok(contacts.len())
    }

    pub fn add_contact(&self, c: &Contact) -> Result<()> {
        self.db().execute(
            "INSERT OR REPLACE INTO contacts (source, name, email, phone, org) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![c.source, c.name, c.email.to_lowercase(), c.phone, c.org],
        )?;
        Ok(())
    }

    pub fn delete_contact(&self, source: &str, email: &str) -> Result<()> {
        self.db().execute("DELETE FROM contacts WHERE source = ?1 AND email = ?2", params![source, email])?;
        Ok(())
    }

    /// Prefix / substring lookup over name and email; also suggests people
    /// you have received mail from.
    pub fn find_contacts(&self, q: &str, limit: usize) -> Result<Vec<Contact>> {
        let db = self.db();
        let like = format!("%{}%", q.replace('%', "").replace('_', ""));
        let mut st = db.prepare(
            "SELECT source, name, email, phone, org FROM contacts
             WHERE name LIKE ?1 OR email LIKE ?1
             UNION
             SELECT 'recent', MAX(from_name), from_addr, '', '' FROM messages
             WHERE from_addr != '' AND (from_name LIKE ?1 OR from_addr LIKE ?1)
               AND from_addr NOT IN (SELECT email FROM contacts)
             GROUP BY from_addr
             ORDER BY 2 LIMIT ?2",
        )?;
        Ok(st
            .query_map(params![like, limit as i64], |r| {
                Ok(Contact { source: r.get(0)?, name: r.get(1)?, email: r.get(2)?, phone: r.get(3)?, org: r.get(4)? })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn all_contacts(&self) -> Result<Vec<Contact>> {
        let db = self.db();
        let mut st = db.prepare("SELECT source, name, email, phone, org FROM contacts ORDER BY name, email")?;
        Ok(st
            .query_map([], |r| {
                Ok(Contact { source: r.get(0)?, name: r.get(1)?, email: r.get(2)?, phone: r.get(3)?, org: r.get(4)? })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(uid: u32, subject: &str) -> NewMessage {
        NewMessage {
            uid,
            message_id: format!("<{uid}@x>"),
            references: String::new(),
            subject: subject.into(),
            from_name: "Alice".into(),
            from_addr: "alice@example.com".into(),
            to: "me@example.com".into(),
            cc: String::new(),
            reply_to: String::new(),
            date: uid as i64,
            seen: false,
            flagged: false,
            answered: false,
            tags: 0,
            attachments: vec![],
            preview: String::new(),
            body_text: "hello".into(),
            raw: b"Subject: x\r\n\r\nhello".to_vec(),
        }
    }

    #[test]
    fn insert_list_reconcile() {
        let s = Store::in_memory().unwrap();
        s.upsert_folder("a", "INBOX", FolderRole::Inbox).unwrap();
        let added = s.insert_messages("a", "INBOX", vec![msg(1, "one"), msg(2, "two")]).unwrap();
        assert_eq!(added.len(), 2);
        assert!(s.raw(added[0].0).unwrap().unwrap().starts_with(b"Subject"));
        // duplicates ignored
        assert!(s.insert_messages("a", "INBOX", vec![msg(1, "one")]).unwrap().is_empty());

        let list = s.list("a", "INBOX", 10, 0).unwrap();
        assert_eq!(list[0].subject, "two");
        assert_eq!(s.folders("a").unwrap()[0].unread, 2);

        let removed = s.reconcile_flags("a", "INBOX", &[(2, true, true, TAG_TODO, true)]).unwrap();
        assert_eq!(removed.len(), 1);
        assert!(s.raw(removed[0]).unwrap().is_none(), "raw source deleted with message");
        let list = s.list("a", "INBOX", 10, 0).unwrap();
        assert_eq!(list.len(), 1);
        assert!(list[0].seen && list[0].flagged);
        assert!(list[0].answered);
        assert_eq!(list[0].tags, TAG_TODO);
        assert_eq!(s.list_role(FolderRole::Inbox, 10, 0).unwrap().len(), 1);
    }

    #[test]
    fn tags_and_agent_backfill() {
        let s = Store::in_memory().unwrap();
        s.upsert_folder("a", "INBOX", FolderRole::Inbox).unwrap();
        let mut m1 = msg(1, "one");
        m1.raw = b"From: a@x\r\nX-Agent-Processed: yes\r\n\r\nbody".to_vec();
        // Hyphenation variant in the header name, agent word only in the body:
        let mut m2 = msg(2, "two");
        m2.raw = b"From: a@x\r\nX-AgentProcessed: \r\n\r\nthe agent processed it".to_vec();
        // No agent header at all; a body that only looks like agent prose.
        let mut m3 = msg(3, "three");
        m3.raw = b"From: a@x\r\nSubject: s\r\n\r\nplease agent processed this".to_vec();
        s.insert_messages("a", "INBOX", vec![m1, m2, m3]).unwrap();
        // parse_raw normally derives the bit; here the SQL backfill is the mechanism.
        let n = s.backfill_agent_tags().unwrap();
        let list = s.list("a", "INBOX", 10, 0).unwrap();
        let tags_of = |subject: &str| list.iter().find(|l| l.subject == subject).map(|l| l.tags).unwrap();
        assert_eq!(tags_of("one") & TAG_AGENT, TAG_AGENT, "X-Agent-Processed detected");
        assert_eq!(tags_of("two") & TAG_AGENT, TAG_AGENT, "header variant without hyphen");
        assert_eq!(tags_of("three") & TAG_AGENT, 0, "body text must not trigger the tag");
        assert_eq!(n, 2);
        // Keyword toggles don't touch the local bit; toggling off clears it.
        let id = list.iter().find(|l| l.subject == "one").unwrap().id;
        s.set_tag(id, TAG_AGENT, true).unwrap();
        s.set_tag(id, TAG_TODO, true).unwrap();
        s.set_tag(id, TAG_IMPORTANT, true).unwrap();
        s.set_tag(id, TAG_IMPORTANT, false).unwrap();
        let tags = s.list("a", "INBOX", 10, 0).unwrap();
        let tags = tags.iter().find(|l| l.subject == "one").unwrap().tags;
        assert_eq!(tags, TAG_AGENT | TAG_TODO);
    }

    #[test]
    fn message_reads_tags() {
        let s = Store::in_memory().unwrap();
        s.upsert_folder("a", "INBOX", FolderRole::Inbox).unwrap();
        let added = s.insert_messages("a", "INBOX", vec![msg(1, "one")]).unwrap();
        let m = s.message(added[0].0).unwrap().unwrap();
        assert_eq!(m.summary.subject, "one");
        assert_eq!(m.summary.tags, 0);
        assert_eq!(m.to, "me@example.com");
        s.set_tag(added[0].0, TAG_IMPORTANT, true).unwrap();
        assert_eq!(s.message(added[0].0).unwrap().unwrap().summary.tags, TAG_IMPORTANT);
    }

    #[test]
    fn finds_reply_target_by_message_id() {
        let s = Store::in_memory().unwrap();
        s.upsert_folder("a", "INBOX", FolderRole::Inbox).unwrap();
        let added = s.insert_messages("a", "INBOX", vec![msg(1, "one")]).unwrap();
        let id = added[0].0;
        let mid = added[0].1.message_id.clone();
        let t = s.summary_by_message_id("a", &mid).unwrap().unwrap();
        assert_eq!((t.id, t.account.as_str(), t.folder.as_str()), (id, "a", "INBOX"));
        s.set_answered(id, true).unwrap();
        assert!(s.summary_by_message_id("a", &mid).unwrap().unwrap().answered);
        assert!(s.summary_by_message_id("a", "<other@x>").unwrap().is_none());
    }

    #[test]
    fn migrates_missing_tags_column() {
        use std::path::PathBuf;
        let dir: PathBuf = std::env::temp_dir().join(format!("mailix-mig-{}-0", std::process::id()));
        let _ = std::fs::remove_file(&dir);
        // Old database: full messages table without the tags column.
        let conn = rusqlite::Connection::open(&dir).unwrap();
        conn.execute_batch(&SCHEMA.replace("    tags        INTEGER NOT NULL DEFAULT 0,\n", "")).unwrap();
        drop(conn);
        let s = Store::open(&dir).unwrap();
        s.upsert_folder("a", "INBOX", FolderRole::Inbox).unwrap();
        let added = s.insert_messages("a", "INBOX", vec![msg(1, "one")]).unwrap();
        s.set_tag(added[0].0, TAG_TODO, true).unwrap();
        assert_eq!(s.list("a", "INBOX", 10, 0).unwrap()[0].tags, TAG_TODO);
        let _ = std::fs::remove_file(&dir);
    }

    #[test]
    fn contacts_lookup_includes_senders() {
        let s = Store::in_memory().unwrap();
        s.upsert_folder("a", "INBOX", FolderRole::Inbox).unwrap();
        s.insert_messages("a", "INBOX", vec![msg(1, "x")]).unwrap();
        s.add_contact(&Contact {
            source: "local".into(),
            name: "Bob Builder".into(),
            email: "Bob@Example.com".into(),
            phone: String::new(),
            org: String::new(),
        })
        .unwrap();
        let r = s.find_contacts("bob", 10).unwrap();
        assert_eq!(r[0].email, "bob@example.com");
        let r = s.find_contacts("alice", 10).unwrap();
        assert_eq!(r[0].source, "recent");
    }
}
