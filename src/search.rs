//! Full-text search with tantivy. Indexes subject, people and body text of
//! every cached message; queries take a few milliseconds even on large
//! mailboxes.
//!
//! Query syntax (tantivy): words, "exact phrase", `from:alice`,
//! `subject:invoice`, `to:bob`, AND/OR/-exclusion. Every word is matched as a
//! prefix of the last term so results appear while typing.

use anyhow::Result;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::{Field, INDEXED, STORED, STRING, Schema, TEXT, Value};
use tantivy::{Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument, Term, doc};

#[derive(Clone)]
pub struct Search {
    index: Index,
    reader: IndexReader,
    writer: Arc<Mutex<IndexWriter>>,
    f: Fields,
}

#[derive(Clone, Copy)]
struct Fields {
    id: Field,
    account: Field,
    subject: Field,
    from: Field,
    to: Field,
    body: Field,
}

pub struct Doc<'a> {
    pub id: i64,
    pub account: &'a str,
    pub subject: &'a str,
    pub from: &'a str,
    pub to: &'a str,
    pub body: &'a str,
}

fn schema() -> (Schema, Fields) {
    let mut b = Schema::builder();
    let f = Fields {
        id: b.add_i64_field("id", INDEXED | STORED),
        account: b.add_text_field("account", STRING),
        subject: b.add_text_field("subject", TEXT),
        from: b.add_text_field("from", TEXT),
        to: b.add_text_field("to", TEXT),
        body: b.add_text_field("body", TEXT),
    };
    (b.build(), f)
}

impl Search {
    pub fn open(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir)?;
        let (schema, f) = schema();
        let index = match Index::open_in_dir(dir) {
            Ok(i) if i.schema() == schema => i,
            _ => {
                // New or incompatible index: start over (the store can rebuild it).
                std::fs::remove_dir_all(dir).ok();
                std::fs::create_dir_all(dir)?;
                Index::create_in_dir(dir, schema)?
            }
        };
        Self::from_index(index, f)
    }

    #[cfg(test)]
    pub fn in_memory() -> Result<Self> {
        let (schema, f) = schema();
        Self::from_index(Index::create_in_ram(schema), f)
    }

    fn from_index(index: Index, f: Fields) -> Result<Self> {
        let writer = index.writer(32 * 1024 * 1024)?;
        let reader = index.reader_builder().reload_policy(ReloadPolicy::OnCommitWithDelay).try_into()?;
        Ok(Search { index, reader, writer: Arc::new(Mutex::new(writer)), f })
    }

    pub fn add(&self, docs: &[Doc<'_>]) -> Result<()> {
        let w = self.writer.lock().unwrap_or_else(|e| e.into_inner());
        for d in docs {
            // Bodies can be huge (newsletters); cap what we index.
            let body = truncate(d.body, 64 * 1024);
            w.add_document(doc!(
                self.f.id => d.id,
                self.f.account => d.account,
                self.f.subject => d.subject,
                self.f.from => d.from,
                self.f.to => d.to,
                self.f.body => body,
            ))?;
        }
        Ok(())
    }

    pub fn remove(&self, ids: &[i64]) {
        let w = self.writer.lock().unwrap_or_else(|e| e.into_inner());
        for id in ids {
            w.delete_term(Term::from_field_i64(self.f.id, *id));
        }
    }

    pub fn remove_account(&self, account: &str) {
        let w = self.writer.lock().unwrap_or_else(|e| e.into_inner());
        w.delete_term(Term::from_field_text(self.f.account, account));
    }

    pub fn commit(&self) -> Result<()> {
        self.writer.lock().unwrap_or_else(|e| e.into_inner()).commit()?;
        self.reader.reload()?;
        Ok(())
    }

    /// Returns message ids, best match first.
    pub fn query(&self, q: &str, limit: usize) -> Result<Vec<i64>> {
        let q = q.trim();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let mut parser = QueryParser::for_index(&self.index, vec![self.f.subject, self.f.from, self.f.to, self.f.body]);
        parser.set_conjunction_by_default();
        parser.set_field_boost(self.f.subject, 3.0);
        parser.set_field_boost(self.f.from, 2.0);
        // Typing "inv" should already find "invoice".
        for field in [self.f.subject, self.f.from, self.f.to, self.f.body] {
            parser.set_field_fuzzy(field, true, 0, true);
        }
        let (query, _errors) = parser.parse_query_lenient(q);
        let searcher = self.reader.searcher();
        let top = searcher.search(&query, &TopDocs::with_limit(limit).order_by_score())?;
        let mut ids = Vec::with_capacity(top.len());
        for (_score, addr) in top {
            let d: TantivyDocument = searcher.doc(addr)?;
            if let Some(id) = d.get_first(self.f.id).and_then(|v| v.as_i64()) {
                ids.push(id);
            }
        }
        Ok(ids)
    }
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_by_prefix_and_field() {
        let s = Search::in_memory().unwrap();
        s.add(&[
            Doc { id: 1, account: "a", subject: "Invoice October", from: "Billing <bill@shop.de>", to: "me", body: "pay" },
            Doc { id: 2, account: "a", subject: "Lunch?", from: "Alice <alice@x.de>", to: "me", body: "pizza today" },
        ])
        .unwrap();
        s.commit().unwrap();
        assert_eq!(s.query("invo", 10).unwrap(), vec![1]);
        assert_eq!(s.query("from:alice", 10).unwrap(), vec![2]);
        assert_eq!(s.query("pizza", 10).unwrap(), vec![2]);
        s.remove(&[2]);
        s.commit().unwrap();
        assert!(s.query("pizza", 10).unwrap().is_empty());
    }
}
