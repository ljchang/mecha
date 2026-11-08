//! A rule's template reply (MAIL-RULES-DESIGN.md §5.1) and the guards that
//! refuse one (§5.4).
//!
//! **The owner's words, sent as written.** A template may contain only the
//! closed slots below, each filled by the harness from the message being
//! answered and each stating what it strips; a template naming anything else
//! is refused when the rule loads, never rendered with a hole in it.
//!
//! **A refusal is not a demotion.** The guards here refuse the send outright
//! — the row is `skipped: <guard>` — because the message is not one a
//! responder should answer at all: automated, a list, the owner's own, a
//! conversation the owner has joined, or a sender this rule answered this
//! week. Demotions (staging a send for the owner instead of sending it) and
//! mode-level holds are the automatic path's, and arrive with it.
use super::engine::Candidate;
use anyhow::{bail, Result};

/// The closed slot set. A slot added later states what it strips, or it is
/// not a slot.
pub const SLOTS: [&str; 3] = ["sender_first_name", "subject", "today"];

const FIRST_NAME_MAX: usize = 40;
const SUBJECT_MAX: usize = 200;

/// Refuse a template that names a slot outside [`SLOTS`], or has a brace
/// that opens no slot. Checked when the rule loads.
pub fn validate_template(text: &str) -> Result<()> {
    let mut rest = text;
    while let Some(open) = rest.find(['{', '}']) {
        if rest[open..].starts_with('}') {
            bail!(
                "a template's `}}` closes no slot; slots are {}",
                slot_list()
            );
        }
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            bail!("a template's `{{` opens no slot; slots are {}", slot_list());
        };
        let name = &after[..close];
        if !SLOTS.contains(&name) {
            bail!(
                "`{{{name}}}` is not a template slot; slots are {}",
                slot_list()
            );
        }
        rest = &after[close + 1..];
    }
    Ok(())
}

fn slot_list() -> String {
    SLOTS
        .iter()
        .map(|s| format!("{{{s}}}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `{sender_first_name}`: the first word of the `From` display name, keeping
/// only letters, spaces, hyphens and apostrophes, at most 40 characters.
/// Empty when the display name has none.
pub fn sender_first_name(display: &str) -> String {
    let kept: String = display
        .chars()
        .filter(|c| c.is_alphabetic() || *c == ' ' || *c == '-' || *c == '\'')
        .collect();
    kept.split_whitespace()
        .next()
        .unwrap_or_default()
        .chars()
        .take(FIRST_NAME_MAX)
        .collect()
}

/// `{subject}`: the thread's subject as `Re:`, control characters (CR, LF,
/// NUL and the rest) removed, at most 200 characters.
pub fn reply_subject(subject: &str) -> String {
    let clean: String = subject
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let clean = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    let re = if clean.to_ascii_lowercase().starts_with("re:") {
        clean
    } else {
        format!("Re: {clean}")
    };
    re.chars().take(SUBJECT_MAX).collect()
}

/// Fill the closed slots. The template has passed [`validate_template`].
///
/// One pass over the template: a slot's value is never rescanned, so a
/// subject that reads `{today}` stays those words (review of #856).
pub fn render(template: &str, display_name: &str, subject: &str, today: &str) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            out.push_str(&rest[open..]);
            return out;
        };
        match &after[..close] {
            "sender_first_name" => out.push_str(&sender_first_name(display_name)),
            "subject" => out.push_str(&reply_subject(subject)),
            "today" => out.push_str(today),
            other => {
                out.push('{');
                out.push_str(other);
                out.push('}');
            }
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out
}

fn header_values<'a>(c: &'a Candidate, name: &'a str) -> impl Iterator<Item = &'a str> {
    c.headers
        .iter()
        .filter(move |(k, _)| k.eq_ignore_ascii_case(name))
        .flat_map(|(_, v)| v.iter().map(String::as_str))
}

fn automated_local_part(address: &str) -> bool {
    let local = address
        .split('@')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let squashed: String = local
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    squashed.starts_with("noreply")
        || squashed.starts_with("donotreply")
        || local == "mailer-daemon"
        || local == "postmaster"
}

/// Why a reply to this message is refused (§5.4, the RFC 3834 guards, the
/// owner's own address and a conversation the owner has joined), or `None`.
/// Each guard keys on a fact being *present*: one the read could not show
/// does not refuse here, and every send that is not refused is either staged
/// for the owner or, on the automatic path, held to its own completeness
/// checks.
pub fn reply_refusal(c: &Candidate) -> Option<&'static str> {
    if header_values(c, "auto-submitted").any(|v| !v.trim().eq_ignore_ascii_case("no")) {
        return Some("auto-submitted");
    }
    if header_values(c, "precedence").any(|v| {
        matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "bulk" | "list" | "junk"
        )
    }) {
        return Some("precedence: bulk, list or junk");
    }
    if c.list_id.is_some()
        || header_values(c, "list-id").next().is_some()
        || header_values(c, "list-unsubscribe").next().is_some()
    {
        return Some("a mailing list");
    }
    let sender = super::engine::parsed_address(&c.message.sender);
    if sender.as_deref().is_some_and(automated_local_part) {
        return Some("an automated sender");
    }
    if sender.as_deref().is_some_and(|s| {
        c.sending
            .owners
            .iter()
            .any(|own| own.eq_ignore_ascii_case(s))
    }) {
        return Some("one of your own addresses");
    }
    if c.sending.owner_participated == Some(true) {
        return Some("a conversation you have joined");
    }
    None
}

/// The per-sender limit: one reply per sender per rule per seven days
/// (RFC 5230's vacation default).
pub const PER_SENDER_DAYS: i64 = 7;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_template_names_only_closed_slots() {
        assert!(
            validate_template("Hi {sender_first_name}, thanks — {subject} on {today}.").is_ok()
        );
        assert!(validate_template("Plain words.").is_ok());
        for bad in [
            "Dear {name}",
            "a { brace",
            "a } brace",
            "{sender_first_name",
        ] {
            assert!(validate_template(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn slots_strip_what_they_say() {
        assert_eq!(sender_first_name("Dana <x>; DROP"), "Dana");
        assert_eq!(sender_first_name("O'Neil-Park, Jo"), "O'Neil-Park");
        assert_eq!(sender_first_name("12345 !!!"), "");
        assert_eq!(sender_first_name(&"a".repeat(80)).len(), FIRST_NAME_MAX);
        assert_eq!(
            reply_subject("Room\r\nBcc: x@example.com"),
            "Re: Room Bcc: x@example.com"
        );
        assert_eq!(reply_subject("RE: already"), "RE: already");
        assert_eq!(reply_subject(&"s".repeat(400)).chars().count(), SUBJECT_MAX);
        assert_eq!(
            render(
                "Hi {sender_first_name} — {subject}, {today}",
                "Dana Example",
                "Booking",
                "2026-11-07"
            ),
            "Hi Dana — Re: Booking, 2026-11-07"
        );
        // A slot's value is never rescanned for slots.
        assert_eq!(
            render(
                "{subject} / {today}",
                "Dana",
                "{today} {sender_first_name}",
                "2026-11-07"
            ),
            "Re: {today} {sender_first_name} / 2026-11-07"
        );
    }
}
