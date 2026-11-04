//! Conversation grouping for the message list.
//!
//! Messages are grouped by the root of their `References` chain (falling
//! back to `In-Reply-To`, stored in the same field). Replies whose client
//! dropped those headers join a conversation with the same subject.

use crate::store::Summary;
use std::collections::HashMap;

/// One conversation: indices into the (date-descending) list, newest first.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub key: String,
    pub members: Vec<usize>,
}

fn ids(s: &str) -> impl Iterator<Item = &str> {
    s.split_whitespace().filter(|t| t.starts_with('<') && t.ends_with('>'))
}

/// "Re: AW: Fwd: Hello" → "hello"
pub fn base_subject(s: &str) -> String {
    let mut t = s.trim();
    loop {
        let lower = t.to_lowercase();
        let Some(p) = ["re:", "aw:", "fwd:", "fw:", "wg:", "antw:", "sv:", "vs:"].iter().find(|p| lower.starts_with(**p))
        else {
            break;
        };
        t = t[p.len()..].trim_start();
    }
    t.to_lowercase()
}

fn is_reply(s: &str) -> bool {
    base_subject(s) != s.trim().to_lowercase()
}

pub fn group(list: &[Summary]) -> Vec<Group> {
    // Union-find over message ids so A→B→C chains with partial References
    // still end up together.
    let mut parent: HashMap<String, String> = HashMap::new();
    fn find(p: &mut HashMap<String, String>, x: &str) -> String {
        let mut cur = x.to_string();
        while let Some(next) = p.get(&cur).cloned() {
            if next == cur {
                break;
            }
            cur = next;
        }
        cur
    }
    fn union(p: &mut HashMap<String, String>, a: &str, b: &str) {
        let (ra, rb) = (find(p, a), find(p, b));
        if ra != rb {
            p.insert(ra, rb);
        }
    }
    let own = |i: usize, s: &Summary| {
        if s.message_id.is_empty() { format!("<local-{i}>") } else { s.message_id.clone() }
    };
    for (i, s) in list.iter().enumerate() {
        let me = own(i, s);
        parent.entry(me.clone()).or_insert_with(|| me.clone());
        for r in ids(&s.refs) {
            parent.entry(r.to_string()).or_insert_with(|| r.to_string());
            union(&mut parent, &me, r);
        }
    }
    // Subject fallback for replies without threading headers.
    let mut by_subject: HashMap<String, String> = HashMap::new();
    for (i, s) in list.iter().enumerate() {
        if !s.refs.is_empty() || is_reply(&s.subject) {
            continue;
        }
        by_subject.entry(base_subject(&s.subject)).or_insert_with(|| own(i, s));
    }
    for (i, s) in list.iter().enumerate() {
        if s.refs.is_empty() && is_reply(&s.subject) {
            if let Some(orig) = by_subject.get(&base_subject(&s.subject)).cloned() {
                union(&mut parent, &own(i, s), &orig);
            }
        }
    }

    let mut groups: Vec<Group> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for (i, s) in list.iter().enumerate() {
        let root = find(&mut parent, &own(i, s));
        match index.get(&root) {
            Some(&g) => groups[g].members.push(i),
            None => {
                index.insert(root.clone(), groups.len());
                groups.push(Group { key: root, members: vec![i] });
            }
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(id: i64, subject: &str, mid: &str, refs: &str) -> Summary {
        Summary {
            id,
            account: "a".into(),
            folder: "INBOX".into(),
            uid: id as u32,
            subject: subject.into(),
            from_name: String::new(),
            from_addr: String::new(),
            date: 100 - id,
            seen: true,
            flagged: false,
            has_attachments: false,
            preview: String::new(),
            message_id: mid.into(),
            refs: refs.into(),
        }
    }

    #[test]
    fn groups_by_references_and_subject() {
        // Newest first, as the list is sorted.
        let list = vec![
            s(1, "Re: Lunch", "<c@x>", "<a@x> <b@x>"),
            s(2, "Invoice", "<i@x>", ""),
            s(3, "Re: Lunch", "<b@x>", "<a@x>"),
            s(4, "AW: Plan", "<p2@x>", ""), // reply without headers
            s(5, "Lunch", "<a@x>", ""),
            s(6, "Plan", "<p1@x>", ""),
            s(7, "Re: Lunch", "<d@x>", "<b@x>"), // partial chain
        ];
        let g = group(&list);
        let members: Vec<Vec<usize>> = g.iter().map(|g| g.members.clone()).collect();
        assert_eq!(members, vec![vec![0, 2, 4, 6], vec![1], vec![3, 5]]);
        assert_eq!(base_subject("Re: AW: fwd: Hello"), "hello");
    }
}
