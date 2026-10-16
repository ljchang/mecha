//! Registering your own sign-in app, walked through: what `mecha-mail auth`
//! does when nothing names a client and nothing stored has one.
//!
//! **Why each person registers their own** (the owner, 2026-10-12): Gmail's
//! scope is restricted, so a client mecha shipped would be public, a phishing
//! kit under mecha's name, a 100-user lifetime cap anyone could spend, and one
//! suspension away from signing every user out at once. Small open-source mail
//! clients make the same choice; what they add is a wizard, so the cost is a
//! few minutes once rather than a page of documentation (`builtin`'s doc).
//!
//! The steps are data and the checks are functions, so what a person is told
//! and what is accepted from them are tested without a browser or a terminal;
//! the interactive driver is `mecha-mail`'s.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// One thing to do in a web console: what to do there, and where.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    pub what: &'static str,
    pub url: &'static str,
}

/// A Google Cloud project with a Desktop client for Gmail and Calendar.
/// Ends with the client's JSON downloaded, which [`find_client_json`] picks up.
pub const GOOGLE: &[Step] = &[
    Step {
        what: "Create a project (any name, e.g. `mecha-mail`) and select it.",
        url: "https://console.cloud.google.com/projectcreate",
    },
    Step {
        what: "Enable the Gmail API and the Google Calendar API for it.",
        url: "https://console.cloud.google.com/flows/enableapi?apiid=gmail.googleapis.com,calendar-json.googleapis.com",
    },
    Step {
        what: "Set up the sign-in screen: Get started, an app name (e.g. `mecha`), your email, \
               audience External, then Create.",
        url: "https://console.cloud.google.com/auth/overview",
    },
    Step {
        what: "Under Audience, choose Publish app. Left in Testing, Google signs you out every \
               seven days. Google will show its \"unverified app\" screen to you alone — it is \
               your app.",
        url: "https://console.cloud.google.com/auth/audience",
    },
    Step {
        what: "Create a client: application type Desktop app, any name, Create, then Download \
               JSON.",
        url: "https://console.cloud.google.com/auth/clients/create",
    },
];

/// An Entra app registration for Outlook. Ends with two ids to paste.
pub const MICROSOFT: &[Step] = &[
    Step {
        what: "New registration: any name (e.g. `mecha`). Supported account types: personal \
               Microsoft accounts and any organisation, unless this is for one organisation \
               only. Leave the redirect URI empty. Register.",
        url: "https://entra.microsoft.com/#view/Microsoft_AAD_RegisteredApps/ApplicationsListBlade",
    },
    Step {
        what: "In the app's Authentication page, set \"Allow public client flows\" to Yes, and \
               Save.",
        url: "https://entra.microsoft.com/#view/Microsoft_AAD_RegisteredApps/ApplicationsListBlade",
    },
    Step {
        what: "In API permissions: Add a permission, Microsoft Graph, Delegated, then \
               Mail.ReadWrite, Mail.Send, Calendars.ReadWrite and offline_access. A work or \
               school directory may need an administrator to grant them.",
        url: "https://entra.microsoft.com/#view/Microsoft_AAD_RegisteredApps/ApplicationsListBlade",
    },
    Step {
        what: "On the app's Overview, keep the Application (client) ID and the Directory \
               (tenant) ID to hand.",
        url: "https://entra.microsoft.com/#view/Microsoft_AAD_RegisteredApps/ApplicationsListBlade",
    },
];

/// Where an app already registered — for another machine, say — gives its
/// client file again: the Desktop client's page downloads it.
pub const GOOGLE_CLIENTS: &str = "https://console.cloud.google.com/auth/clients";

/// A path as pasted at a terminal: the quotes a copy or a drag-and-drop adds
/// are dropped, outside quotes a backslash escapes the character after it
/// (a drag escapes every shell metacharacter, so `\ ` and `\(1\)` both),
/// and a leading `~/` is the home folder.
pub fn pasted_path(text: &str, home: Option<&Path>) -> PathBuf {
    let t = text.trim();
    let quoted = ['"', '\'']
        .iter()
        .find_map(|q| t.strip_prefix(*q).and_then(|s| s.strip_suffix(*q)));
    let t = match quoted {
        Some(inner) => inner.to_string(),
        None => {
            let mut out = String::with_capacity(t.len());
            let mut chars = t.chars();
            while let Some(c) = chars.next() {
                match c {
                    '\\' => out.extend(chars.next()),
                    c => out.push(c),
                }
            }
            out
        }
    };
    match (t.strip_prefix("~/"), home) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(t),
    }
}

/// Where a browser saves the downloaded client file.
pub fn downloads_dir() -> Option<PathBuf> {
    dirs::download_dir().or_else(|| dirs::home_dir().map(|h| h.join("Downloads")))
}

/// The newest `client_secret_*.json` in `dir` written at or after `since` —
/// the file the last step downloaded, never an older one left from another
/// project.
pub fn find_client_json(dir: &Path, since: SystemTime) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            name.starts_with("client_secret") && name.ends_with(".json")
        })
        .filter_map(|e| {
            let modified = e.metadata().ok()?.modified().ok()?;
            (modified >= since).then(|| (modified, e.path()))
        })
        .max_by_key(|(m, _)| *m)
        .map(|(_, p)| p)
}

/// A Google client file's id and secret. Only a Desktop client will do: a
/// Web client's redirect cannot be this machine's loopback.
pub fn read_google_client_json(text: &str) -> Result<(String, String), String> {
    let json: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("not a client file: {e}"))?;
    if json.get("web").is_some() {
        return Err(
            "that is a Web application client — create one of type Desktop app instead".into(),
        );
    }
    let cfg = json.get("installed").unwrap_or(&json);
    let id = cfg["client_id"]
        .as_str()
        .filter(|id| id.ends_with(".apps.googleusercontent.com"))
        .ok_or("no Google client_id in the file")?;
    let secret = cfg["client_secret"].as_str().unwrap_or_default();
    Ok((id.to_string(), secret.to_string()))
}

/// An Entra id as pasted: a GUID, or for the directory one of the three
/// names that stand for many.
pub fn entra_id(text: &str, directory: bool) -> Option<String> {
    let t = text.trim().to_ascii_lowercase();
    if directory && matches!(t.as_str(), "common" | "organizations" | "consumers") {
        return Some(t);
    }
    let groups: Vec<&str> = t.split('-').collect();
    let shape = [8, 4, 4, 4, 12];
    (groups.len() == 5
        && groups
            .iter()
            .zip(shape)
            .all(|(g, n)| g.len() == n && g.chars().all(|c| c.is_ascii_hexdigit())))
    .then_some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every step names a page to open, and Google's last step is the
    /// download the driver waits for.
    #[test]
    fn every_step_has_a_page() {
        for s in GOOGLE.iter().chain(MICROSOFT) {
            assert!(s.url.starts_with("https://"), "{s:?}");
        }
        assert!(GOOGLE.last().unwrap().what.contains("Download JSON"));
        assert!(GOOGLE
            .iter()
            .any(|s| s.url.contains("gmail.googleapis.com") && s.url.contains("calendar")));
        // The four scopes `microsoft::auth::SCOPES` asks for are the ones the
        // registration is told to grant.
        let perms = MICROSOFT
            .iter()
            .map(|s| s.what)
            .collect::<Vec<_>>()
            .join(" ");
        for scope in crate::microsoft::auth::SCOPES {
            let name = scope.rsplit('/').next().unwrap();
            assert!(perms.contains(name), "{name} is not in the steps");
        }
    }

    /// The file the last step downloaded is found; one from before the wizard
    /// started, or any other file, is not.
    #[test]
    fn the_downloaded_client_file_is_found() {
        let dir = std::env::temp_dir().join(format!("mecha-wizard-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let old = dir.join("client_secret_old.json");
        std::fs::write(&old, "{}").unwrap();
        let since = SystemTime::now() + std::time::Duration::from_secs(1);
        assert_eq!(find_client_json(&dir, since), None);
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let new = dir.join("client_secret_123-abc.apps.googleusercontent.com.json");
        std::fs::write(&new, "{}").unwrap();
        std::fs::write(dir.join("notes.json"), "{}").unwrap();
        let found = find_client_json(&dir, since);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(found, Some(new));
    }

    /// A path dragged into a terminal or copied with quotes reads as the
    /// file it names.
    #[test]
    fn a_pasted_path_is_the_file_it_names() {
        let home = Path::new("/Users/sam");
        let want = PathBuf::from("/Users/sam/Downloads/client secret.json");
        for pasted in [
            "/Users/sam/Downloads/client\\ secret.json ",
            "'/Users/sam/Downloads/client secret.json'",
            "\"/Users/sam/Downloads/client secret.json\"",
            "~/Downloads/client secret.json",
        ] {
            assert_eq!(pasted_path(pasted, Some(home)), want, "{pasted}");
        }
        // A second download of one file, dragged in: its parens come escaped.
        let want = PathBuf::from("/Users/sam/Downloads/client_secret_1 (1).json");
        for pasted in [
            "/Users/sam/Downloads/client_secret_1\\ \\(1\\).json",
            "'/Users/sam/Downloads/client_secret_1 (1).json'",
        ] {
            assert_eq!(pasted_path(pasted, Some(home)), want, "{pasted}");
        }
        // Without a home folder, `~` is left as typed.
        assert_eq!(pasted_path("~/x.json", None), PathBuf::from("~/x.json"));
    }

    #[test]
    fn only_a_desktop_client_file_is_read() {
        let desktop = r#"{"installed":{"client_id":"1-a.apps.googleusercontent.com","client_secret":"GOCSPX-x"}}"#;
        assert_eq!(
            read_google_client_json(desktop).unwrap(),
            ("1-a.apps.googleusercontent.com".into(), "GOCSPX-x".into())
        );
        let web = r#"{"web":{"client_id":"1-a.apps.googleusercontent.com"}}"#;
        assert!(read_google_client_json(web)
            .unwrap_err()
            .contains("Desktop"));
        assert!(read_google_client_json("not json").is_err());
        assert!(read_google_client_json(r#"{"installed":{"client_id":"123"}}"#).is_err());
    }

    #[test]
    fn entra_ids_are_guids_or_a_shared_directory() {
        let guid = "12345678-0000-4000-8000-0123456789ab";
        assert_eq!(entra_id(guid, false).as_deref(), Some(guid));
        assert_eq!(entra_id(&guid.to_uppercase(), true).as_deref(), Some(guid));
        assert_eq!(entra_id("common", true).as_deref(), Some("common"));
        assert_eq!(entra_id("common", false), None);
        assert_eq!(entra_id("12345678", false), None);
        assert_eq!(entra_id("not-a-guid-at-all-x", true), None);
    }
}
