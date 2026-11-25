//! What a desktop notification shows for newly arrived mail.
//!
//! Thunderbird-style: sender as the title, the subject and the first lines of
//! the message as the body. Everything here is pure so the formatting can be
//! tested without a notification daemon.

/// One unread message that arrived since the last sync.
#[derive(Debug, Clone)]
pub struct NewMail {
    pub from: String,
    pub subject: String,
    /// First non-quoted lines of the plain-text body, joined with `\n`.
    pub preview: String,
}

/// Widest line we emit, in characters. Daemons wrap further if needed.
const WIDTH: usize = 64;
/// How many preview lines a single-message notification carries.
const PREVIEW_LINES: usize = 3;
/// How many messages are listed when several arrive at once.
const LISTED: usize = 4;

/// Title and body for the notification daemon.
pub fn format(account: &str, new: &[NewMail], with_preview: bool) -> (String, String) {
    match new {
        [] => ("Mailix".to_string(), sanitize(account)),
        [one] => {
            let mut body = one.subject.clone();
            if with_preview && !one.preview.is_empty() {
                body.push_str("\n\n");
                body.push_str(&wrap(&one.preview, WIDTH, PREVIEW_LINES));
            }
            (sanitize(&one.from), sanitize(&body))
        }
        many => {
            let mut lines: Vec<String> =
                many.iter().take(LISTED).map(|m| format!("{}: {}", m.from, m.subject)).collect();
            if many.len() > LISTED {
                lines.push(format!("… and {} more", many.len() - LISTED));
            }
            (sanitize(&format!("{} new messages · {account}", many.len())), sanitize(&lines.join("\n")))
        }
    }
}

/// Greedy word wrap. Re-flows the text so every line is filled up to `width`,
/// stops after `max_lines` and marks the cut with an ellipsis.
fn wrap(text: &str, width: usize, max_lines: usize) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut cut = false;
    for word in text.lines().flat_map(str::split_whitespace) {
        let word: String = word.chars().take(width).collect();
        if word.is_empty() {
            continue;
        }
        let fits = match lines.last() {
            Some(line) => line.chars().count() + 1 + word.chars().count() <= width,
            None => false,
        };
        if fits {
            let line = lines.last_mut().expect("checked above");
            line.push(' ');
            line.push_str(&word);
        } else if lines.len() < max_lines {
            lines.push(word);
        } else {
            cut = true;
            break;
        }
    }
    if cut {
        if let Some(last) = lines.last_mut() {
            last.push('…');
        }
    }
    lines.join("\n")
}

/// Notification bodies may be rendered with markup by the daemon (mako, dunst)
/// and the text comes from mail, so angle brackets must not survive as markup.
fn sanitize(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '<' => "&lt;".to_string(),
            '>' => "&gt;".to_string(),
            c if c.is_control() && c != '\n' && c != '\t' => " ".to_string(),
            c => c.to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mail(from: &str, subject: &str, preview: &str) -> NewMail {
        NewMail { from: from.into(), subject: subject.into(), preview: preview.into() }
    }

    #[test]
    fn single_mail_shows_subject_and_first_lines() {
        let (title, body) = format(
            "Work",
            &[mail("Anna", "Quarterly numbers", "Hi Tom,\n\nhere are the numbers\nfor Q4.")],
            true,
        );
        assert_eq!(title, "Anna");
        assert!(body.starts_with("Quarterly numbers\n\n"), "{body}");
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines.len(), 3, "{body}");
        assert_eq!(lines[2], "Hi Tom, here are the numbers for Q4.", "{body}");
    }

    #[test]
    fn preview_can_be_switched_off() {
        let (title, body) = format("Work", &[mail("Anna", "Quarterly numbers", "Hi Tom,")], false);
        assert_eq!(title, "Anna");
        assert_eq!(body, "Quarterly numbers");
    }

    #[test]
    fn several_mails_are_listed() {
        let new: Vec<NewMail> = (0..6).map(|i| mail("Anna", &format!("Mail {i}"), "text")).collect();
        let (title, body) = format("Work", &new, true);
        assert_eq!(title, "6 new messages · Work");
        assert_eq!(body.lines().count(), 5, "{body}");
        assert!(body.lines().last().unwrap().contains("and 2 more"), "{body}");
    }

    #[test]
    fn wrap_respects_width_and_line_budget() {
        let text = "one two three four five six seven eight nine ten";
        let wrapped = wrap(text, 12, 2);
        let lines: Vec<&str> = wrapped.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines.iter().all(|l| l.chars().count() <= 13), "{wrapped:?}");
        assert!(wrapped.ends_with('…'), "{wrapped:?}");
    }

    #[test]
    fn wrap_keeps_short_lines_and_long_words_inside_the_box() {
        let wrapped = wrap("Hi\n\nDonaudampfschifffahrtsgesellschaft\nbye", 10, 3);
        let lines: Vec<&str> = wrapped.lines().collect();
        assert_eq!(lines[0], "Hi");
        assert_eq!(lines[1].chars().count(), 10);
        assert!(!wrapped.ends_with('…'), "nothing was cut: {wrapped:?}");
    }

    #[test]
    fn angle_brackets_never_reach_the_daemon_as_markup() {
        let (_, body) = format("Work", &[mail("A <b>", "a <b>bold</b> claim", "<i>x</i>")], true);
        assert!(!body.contains("<b>"), "{body}");
        assert!(body.contains("&lt;b&gt;bold&lt;/b&gt;"), "{body}");
    }
}
