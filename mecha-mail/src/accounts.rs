//! The account registry behind the unified `mecha-mail` server: several
//! mailboxes, each Google or Microsoft, addressed by a short name the model
//! can put in an `account` argument.
//!
//! Layout: `~/.mecha/mail/accounts.toml` (or `$MECHA_MAIL_DIR/accounts.toml`)
//! names the accounts; each account's credentials live beside it at
//! `<dir>/<name>/oauth.json` in the same [`crate::token::StoredCredentials`]
//! format the per-provider binaries use — which is what makes an existing
//! login importable by file copy rather than by re-authenticating.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Google,
    Outlook,
}

impl std::fmt::Display for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Provider::Google => "google",
            Provider::Outlook => "outlook",
        })
    }
}

impl std::str::FromStr for Provider {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "google" | "gmail" => Ok(Provider::Google),
            "outlook" | "microsoft" => Ok(Provider::Outlook),
            other => bail!("unknown provider `{other}` (google or outlook)"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountEntry {
    /// The name the model uses in `account` arguments. Kept to a tame
    /// charset so it never needs quoting anywhere it travels.
    pub name: String,
    pub provider: Provider,
}

/// `accounts.toml`, whole.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AccountsFile {
    /// Where a new mail or event goes when the model names no account.
    /// Absent means "there is no obvious answer": with several accounts a
    /// create without an explicit account is refused with instructions to
    /// ask the user, rather than guessed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    #[serde(default, rename = "account")]
    pub accounts: Vec<AccountEntry>,
}

/// The registry directory: `$MECHA_MAIL_DIR` or `~/.mecha/mail`.
pub fn dir() -> Result<PathBuf> {
    if let Ok(dir) = std::env::var("MECHA_MAIL_DIR") {
        return Ok(PathBuf::from(dir));
    }
    let home = dirs::home_dir().context("cannot determine home directory")?;
    Ok(home.join(".mecha").join("mail"))
}

pub fn file_path() -> Result<PathBuf> {
    Ok(dir()?.join("accounts.toml"))
}

/// One account's credential store.
pub fn credentials_path(name: &str) -> Result<PathBuf> {
    Ok(dir()?.join(name).join("oauth.json"))
}

/// A legal account name: it is a directory component and a tool argument, so
/// it stays lowercase alphanumerics, `-` and `_`, and never empty.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

pub fn load() -> Result<AccountsFile> {
    let path = file_path()?;
    let text = std::fs::read_to_string(&path).with_context(|| {
        format!("reading {} — run `mecha-mail auth <name> --provider ...` first", path.display())
    })?;
    let file: AccountsFile =
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    validate(&file)?;
    Ok(file)
}

pub fn save(file: &AccountsFile) -> Result<()> {
    validate(file)?;
    let path = file_path()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, toml::to_string_pretty(file)?)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

fn validate(file: &AccountsFile) -> Result<()> {
    let mut seen = std::collections::BTreeSet::new();
    for entry in &file.accounts {
        if !valid_name(&entry.name) {
            bail!(
                "account name `{}` is invalid: lowercase letters, digits, `-` and `_` only",
                entry.name
            );
        }
        if !seen.insert(entry.name.as_str()) {
            bail!("account `{}` is listed twice", entry.name);
        }
    }
    if let Some(default) = &file.default {
        if !seen.contains(default.as_str()) {
            bail!("default account `{default}` is not in the account list");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(names: &[&str], default: Option<&str>) -> AccountsFile {
        AccountsFile {
            default: default.map(String::from),
            accounts: names
                .iter()
                .map(|n| AccountEntry { name: n.to_string(), provider: Provider::Google })
                .collect(),
        }
    }

    #[test]
    fn round_trips_through_toml() {
        let original = AccountsFile {
            default: Some("dartmouth".into()),
            accounts: vec![
                AccountEntry { name: "dartmouth".into(), provider: Provider::Outlook },
                AccountEntry { name: "personal".into(), provider: Provider::Google },
            ],
        };
        let text = toml::to_string_pretty(&original).unwrap();
        let parsed: AccountsFile = toml::from_str(&text).unwrap();
        assert_eq!(parsed.default.as_deref(), Some("dartmouth"));
        assert_eq!(parsed.accounts.len(), 2);
        assert_eq!(parsed.accounts[0].provider, Provider::Outlook);
    }

    #[test]
    fn a_default_that_names_no_account_is_rejected() {
        let err = validate(&file(&["personal"], Some("work"))).unwrap_err();
        assert!(err.to_string().contains("work"), "{err}");
    }

    #[test]
    fn duplicate_and_illegal_names_are_rejected() {
        assert!(validate(&file(&["a", "a"], None)).is_err());
        assert!(validate(&file(&["Has Spaces"], None)).is_err());
        assert!(validate(&file(&[""], None)).is_err());
        assert!(validate(&file(&["dartmouth", "personal-2"], None)).is_ok());
    }

    #[test]
    fn provider_parses_common_spellings() {
        assert_eq!("gmail".parse::<Provider>().unwrap(), Provider::Google);
        assert_eq!("microsoft".parse::<Provider>().unwrap(), Provider::Outlook);
        assert!("yahoo".parse::<Provider>().is_err());
    }
}
