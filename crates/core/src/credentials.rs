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

use crate::config::CredentialsSource;
use crate::{paths, Error};
use serde::Deserialize;
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Le service sous lequel Claude Code range ses identifiants dans le trousseau.
pub const KEYCHAIN_SERVICE: &str = "Claude Code-credentials";

/// Au-dela, on abandonne la commande.
///
/// Ce n'est pas de la coquetterie : sur macOS, une lecture du trousseau par une
/// application non signee ouvre une fenetre d'autorisation. Sans limite de
/// temps, le processus attendrait indefiniment un clic que personne n'a vu, et
/// le widget resterait fige sans rien dire. Un `ssh` vers une machine eteinte
/// aurait le meme effet.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);

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

/// Lance une commande et rend sa sortie standard, sans jamais attendre
/// indefiniment.
///
/// L'erreur est une simple chaine : cette fonction sert aussi bien a lire un
/// jeton qu'a rapatrier le releve d'une autre machine, donc c'est a l'appelant
/// de dire de quoi il s'agissait.
pub fn run_command(command: &str, timeout: Duration) -> Result<String, String> {
    let mut child = shell(command)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .stdin(Stdio::null())
        .spawn()
        .map_err(|err| format!("commande impossible a lancer ({err})"))?;

    let debut = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if debut.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "la commande n'a pas repondu en {} s. Sur macOS, c'est souvent \
                         une autorisation de trousseau qui attend un clic.",
                        timeout.as_secs()
                    ));
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(err) => return Err(format!("commande interrompue ({err})")),
        }
    };

    let mut sortie = String::new();
    if let Some(mut stdout) = child.stdout.take() {
        let _ = stdout.read_to_string(&mut sortie);
    }

    if !status.success() {
        return Err(format!(
            "la commande a echoue (code {})",
            status.code().map(|c| c.to_string()).unwrap_or_else(|| "?".into())
        ));
    }

    Ok(sortie)
}

/// Prepare l'appel a travers l'interpreteur de commandes de la plateforme.
fn shell(command: &str) -> Command {
    if cfg!(windows) {
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", command]);
        cmd
    } else {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", command]);
        cmd
    }
}

/// Sort le document JSON du trousseau macOS, ou `None` s'il est inaccessible.
fn read_keychain() -> Option<String> {
    let document = run_command(
        &format!("security find-generic-password -s '{KEYCHAIN_SERVICE}' -w"),
        COMMAND_TIMEOUT,
    )
    .ok()?;
    let document = document.trim().to_string();
    if document.is_empty() {
        None
    } else {
        Some(document)
    }
}

/// Trouve et lit le jeton OAuth, en suivant les reglages.
pub fn load(source: &CredentialsSource) -> Result<Credentials, Error> {
    // Une source imposee l'emporte, et son echec est une erreur : si
    // l'utilisateur a pris la peine de la renseigner, retomber en silence sur la
    // detection automatique masquerait sa faute de frappe.
    if let Some(command) = &source.command {
        let document = run_command(command, COMMAND_TIMEOUT).map_err(Error::Credentials)?;
        return parse(&document, "commande configuree");
    }

    if let Some(path) = &source.path {
        let document = fs::read_to_string(path)
            .map_err(|err| Error::Credentials(format!("{path} illisible ({err})")))?;
        return parse(&document, path);
    }

    if let Some(token) = &source.token {
        return Ok(Credentials {
            token: token.clone(),
            // Un jeton colle a la main ne dit rien de son expiration. On ne
            // pretend pas la connaitre : l'API tranchera.
            expires_at: None,
            subscription_type: None,
            source: "jeton configure".to_string(),
        });
    }

    auto()
}

/// La detection automatique : la ou Claude Code pose ses identifiants.
pub fn auto() -> Result<Credentials, Error> {
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
    if cfg!(target_os = "macos") {
        tried.push(format!("trousseau ({KEYCHAIN_SERVICE})"));
    }

    Err(Error::Credentials(format!(
        "aucun jeton trouve (cherche dans : {}). Si Claude Code ne tourne pas sur \
         cette machine, renseigne `credentials.command` dans la config : les quotas \
         sont globaux au compte, le jeton peut donc venir d'ailleurs.",
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

    // --- Sources configurees ---------------------------------------------

    /// Une commande qui recrache le document, comme le ferait `cat` ou un `ssh`.
    fn source_commande() -> CredentialsSource {
        CredentialsSource {
            command: Some(format!("printf '%s' '{}'", DOCUMENT.replace('\n', " "))),
            ..Default::default()
        }
    }

    #[test]
    fn une_commande_configuree_fournit_le_jeton() {
        let creds = load(&source_commande()).expect("des identifiants");
        assert_eq!(creds.token, "sk-ant-oat01-factice");
        assert_eq!(creds.source, "commande configuree");
    }

    #[test]
    fn une_commande_qui_echoue_ne_retombe_pas_en_silence_sur_lauto() {
        // Si l'utilisateur a pris la peine de configurer une source, masquer son
        // erreur derriere la detection automatique lui ferait chercher longtemps.
        let source = CredentialsSource {
            command: Some("exit 3".to_string()),
            ..Default::default()
        };
        assert!(matches!(load(&source), Err(Error::Credentials(_))));
    }

    #[test]
    fn une_commande_qui_traine_est_coupee() {
        let debut = Instant::now();
        let err = run_command("sleep 5", Duration::from_millis(300)).unwrap_err();
        // Coupee bien avant les 5 secondes du sleep.
        assert!(debut.elapsed() < Duration::from_secs(2), "pas coupee assez vite");
        assert!(err.contains("repondu"), "message inattendu : {err}");
    }

    #[test]
    fn un_jeton_colle_a_la_main_est_accepte_tel_quel() {
        let source = CredentialsSource {
            token: Some("sk-ant-oat01-colle".to_string()),
            ..Default::default()
        };
        let creds = load(&source).expect("des identifiants");
        assert_eq!(creds.token, "sk-ant-oat01-colle");
        // On ne pretend pas connaitre son expiration.
        assert_eq!(creds.expires_at, None);
        assert!(!creds.is_expired(u64::MAX));
    }

    #[test]
    fn la_commande_passe_avant_le_jeton() {
        let source = CredentialsSource {
            token: Some("jamais-utilise".to_string()),
            ..source_commande()
        };
        assert_eq!(load(&source).unwrap().token, "sk-ant-oat01-factice");
    }

    #[test]
    fn une_source_vide_laisse_la_detection_automatique_decider() {
        assert!(!CredentialsSource::default().is_explicit());
        assert!(source_commande().is_explicit());
    }

    #[test]
    fn un_chemin_illisible_le_dit() {
        let source = CredentialsSource {
            path: Some("/absent/pour/de/bon.json".to_string()),
            ..Default::default()
        };
        assert!(matches!(load(&source), Err(Error::Credentials(d)) if d.contains("illisible")));
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
