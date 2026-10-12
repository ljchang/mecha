//! mecha's own app registration, and the browser opener every sign-in uses.
//!
//! **One client ships: Google Drive's, for documents.** It asks for
//! `drive.file` alone, a non-sensitive scope, so it carries no user cap and
//! reaches only files mecha made or the person handed it in Google's picker —
//! of little use to anyone who copies it. A first `mecha-docs auth` is then a
//! sign-in page and nothing more (the owner, 2026-10-12).
//!
//! **Mail and calendar bring their own**, by the owner's ruling the same day.
//! A shipped client is public by nature (a desktop app cannot keep a secret,
//! and Google says so), and for Gmail's restricted scope that makes it a
//! phishing kit under mecha's name, a 100-user lifetime cap anyone can spend,
//! and one suspension that signs every user out at once — which is why small
//! open-source mail clients have each user register their own. `mecha-mail
//! auth` walks a person through doing that instead (`mecha-mail`'s wizard).
//!
//! **What protects a sign-in is not this value.** It is the person's consent
//! in their own browser, PKCE, and a redirect only this machine receives.
//!
//! An explicit `--client-id`, a downloaded client file, and a stored login's
//! own client all come before this one.
//!
//! **Rotating it is a release.** Google lets a client hold two secrets at
//! once: add the new one, ship it here, then disable the old. A grant belongs
//! to the client id, so signed-in accounts carry on; a build older than the
//! release stops refreshing until it is updated (or is given `--client-id`
//! and `--client-secret`). A suspended project would need a new client and a
//! release, and every account signing in again.

/// A Google desktop client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GoogleClient {
    pub id: &'static str,
    pub secret: &'static str,
}

/// Google Docs, Sheets and Slides: `drive.file` only.
pub const GOOGLE_DOCS: GoogleClient = GoogleClient {
    id: "949095882298-1d4ige7ae32hihgc6k86seo4fojg13h8.apps.googleusercontent.com",
    secret: "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",
};

/// Open `url` in the person's browser, if this machine has one to open.
/// Returns whether a browser was launched. The URL is always printed by the
/// caller too, so this is a convenience and never the interface: a machine
/// with no browser (a server over SSH) prints the URL and is not told it
/// failed.
pub fn open_browser(url: &str) -> bool {
    let mut cmd = if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else if cfg!(windows) {
        // Not `cmd /C start`: an unquoted URL is split at its first `&`, and
        // an OAuth URL has several (review of #663).
        let mut c = std::process::Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    } else {
        // A headless box has no display, and `xdg-open` there prints an
        // error that reads as this command failing when it has not.
        let display = ["DISPLAY", "WAYLAND_DISPLAY"]
            .iter()
            .any(|v| std::env::var_os(v).is_some_and(|s| !s.is_empty()));
        if !display {
            return false;
        }
        std::process::Command::new("xdg-open")
    };
    cmd.arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A desktop client of a real project — a value pasted from the wrong
    /// field (a project number, a Web client) fails here.
    #[test]
    fn the_docs_client_is_a_desktop_client_id() {
        assert!(GOOGLE_DOCS.id.ends_with(".apps.googleusercontent.com"));
        assert!(GOOGLE_DOCS.secret.starts_with("GOCSPX-"));
    }
}
