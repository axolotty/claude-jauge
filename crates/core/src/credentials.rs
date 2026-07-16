//! Lecture du jeton OAuth pose par Claude Code.
//!
//! L'emplacement change selon l'OS, mais le contenu est partout le meme document
//! JSON :
//!
//! * macOS   : trousseau, service `Claude Code-credentials`
//! * Linux   : `~/.claude/.credentials.json`
//! * Windows : `%USERPROFILE%\.claude\.credentials.json`, avec des replis
//!   dans `%LOCALAPPDATA%\Claude` et `%APPDATA%\Claude`
//!
//! On se contente de lire. On ne rafraichit JAMAIS le jeton nous-memes : les
//! jetons de rafraichissement tournent a chaque usage, donc en consommer un ici
//! risquerait d'invalider la session de Claude Code lui-meme. Quand le jeton est
//! expire, on le dit et on laisse Claude Code le renouveler de son cote.

use crate::{paths, Error};
use serde::Deserialize;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// Le service sous lequel Claude Code range ses identifiants dans le trousseau.
pub const KEYCHAIN_SERVICE: &str = "Claude Code-credentials";

/// Ce dont la sonde a besoin, extrait du document de Claude Code.
#[derive(Debug, Clone, PartialEq)]
pub struct Credentials {
    pub token: String,
    /// Expiration du jeton, en secondes depuis l'epoch.
    pub expires_at: Option<u64>,
    /// "pro", "max"... tel que declare par Claude Code.
    pub subscription_type: Option<String>,
    /// D'ou vient le jeton, pour pouvoir le dire a l'utilisateur.
    pub source: String,
}

impl Credentials {
    /// Le jeton est-il perime ?
    pub fn is_expired(&self, now: u64) -> bool {
        matches!(self.expires_at, Some(at) if now >= at)
    }

    /// Secondes restantes avant expiration.
    pub fn seconds_before_expiry(&self, now: u64) -> Option<u64> {
        self.expires_at.and_then(|at| at.checked_sub(now))
    }
}

#[derive(Deserialize)]
struct Document {
    #[serde(rename = "claudeAiOauth")]
    oauth: Option<Oauth>,
}

#[derive(Deserialize)]
struct Oauth {
    #[serde(rename = "accessToken")]
    access_token: Option<String>,
    /// Claude Code l'ecrit en millisecondes.
    #[serde(rename = "expiresAt")]
    expires_at: Option<f64>,
    #[serde(rename = "subscriptionType")]
    subscription_type: Option<String>,
}

/// Transforme le document JSON de Claude Code en [`Credentials`].
pub fn parse(document: &str, source: &str) -> Result<Credentials, Error> {
    let parsed: Document = serde_json::from_str(document)
        .map_err(|err| Error::Credentials(format!("JSON invalide ({err})")))?;

    let oauth = parsed
        .oauth
        .ok_or_else(|| Error::Credentials("pas de bloc claudeAiOauth".into()))?;

    let token = oauth
        .access_token
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| Error::Credentials("pas d'accessToken".into()))?;

    Ok(Credentials {
        token,
        expires_at: oauth
            .expires_at
            .filter(|ms| ms.is_finite() && *ms > 0.0)
            .map(|ms| (ms / 1000.0) as u64),
        subscription_type: oauth.subscription_type,
        source: source.to_string(),
    })
}

/// Les emplacements possibles du fichier d'identifiants, par ordre de priorite.
pub fn candidate_files() -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Ok(dir) = paths::claude_dir() {
        files.push(dir.join(".credentials.json"));
    }
    if cfg!(windows) {
        for var in ["LOCALAPPDATA", "APPDATA"] {
            if let Some(base) = std::env::var_os(var) {
                files.push(PathBuf::from(base).join("Claude").join(".credentials.json"));
            }
        }
    }
    files
}

/// Sort le document JSON du trousseau macOS, ou `None` s'il est inaccessible.
fn read_keychain() -> Option<String> {
    let out = Command::new("security")
        .args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-w"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let document = String::from_utf8(out.stdout).ok()?;
    let document = document.trim().to_string();
    if document.is_empty() {
        None
    } else {
        Some(document)
    }
}

/// Trouve et lit le jeton OAuth de Claude Code.
pub fn load() -> Result<Credentials, Error> {
    if cfg!(target_os = "macos") {
        if let Some(document) = read_keychain() {
            return parse(&document, "trousseau macOS");
        }
    }

    let mut tried = Vec::new();
    for path in candidate_files() {
        match fs::read_to_string(&path) {
            Ok(document) => return parse(&document, &path.display().to_string()),
            Err(_) => tried.push(path.display().to_string()),
        }
    }

    Err(Error::Credentials(format!(
        "introuvable (cherche dans : {}). Connecte-toi une fois avec Claude Code.",
        tried.join(", ")
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// La forme reelle du fichier, jeton remplace.
    const DOCUMENT: &str = r#"{
        "claudeAiOauth": {
            "accessToken": "sk-ant-oat01-factice",
            "refreshToken": "sk-ant-ort01-factice",
            "expiresAt": 1784236817000,
            "scopes": ["user:inference", "user:profile"],
            "subscriptionType": "pro",
            "rateLimitTier": "default_claude_ai"
        }
    }"#;

    #[test]
    fn lit_le_document_de_claude_code() {
        let creds = parse(DOCUMENT, "test").expect("des identifiants");
        assert_eq!(creds.token, "sk-ant-oat01-factice");
        assert_eq!(creds.subscription_type.as_deref(), Some("pro"));
        // Les millisecondes de Claude Code deviennent des secondes chez nous.
        assert_eq!(creds.expires_at, Some(1784236817));
    }

    #[test]
    fn juge_lexpiration_par_rapport_a_lheure_donnee() {
        let creds = parse(DOCUMENT, "test").unwrap();
        assert!(!creds.is_expired(1784236816));
        assert!(creds.is_expired(1784236817));
        assert!(creds.is_expired(1784236818));
        assert_eq!(creds.seconds_before_expiry(1784236800), Some(17));
        assert_eq!(creds.seconds_before_expiry(1784236900), None);
    }

    #[test]
    fn un_jeton_sans_expiration_ne_perime_jamais() {
        let creds = parse(
            r#"{"claudeAiOauth": {"accessToken": "x"}}"#,
            "test",
        )
        .unwrap();
        assert_eq!(creds.expires_at, None);
        assert!(!creds.is_expired(u64::MAX));
    }

    #[test]
    fn refuse_un_document_inexploitable() {
        assert!(matches!(parse("pas du json", "t"), Err(Error::Credentials(_))));
        assert!(matches!(parse("{}", "t"), Err(Error::Credentials(_))));
        assert!(matches!(
            parse(r#"{"claudeAiOauth": {}}"#, "t"),
            Err(Error::Credentials(_))
        ));
        // Un jeton vide ne vaut pas mieux qu'un jeton absent.
        assert!(matches!(
            parse(r#"{"claudeAiOauth": {"accessToken": "  "}}"#, "t"),
            Err(Error::Credentials(_))
        ));
    }
}
