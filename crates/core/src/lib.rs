//! Coeur de `claude-jauge` : savoir ou en est la consommation Claude, sans
//! jamais consommer de quota pour le savoir.
//!
//! Le principe tient en une phrase : l'API renvoie l'etat des quotas dans les
//! en-tetes de chacune de ses reponses, donc une requete minimale suffit a tout
//! apprendre. Le reste du module existe pour n'envoyer cette requete que quand
//! elle sert vraiment (voir [`meter::should_probe`]).
//!
//! Rien ici ne depend d'une interface graphique : le CLI headless et l'app Tauri
//! partagent ce meme code.

pub mod activity;
pub mod cache;
pub mod config;
pub mod credentials;
pub mod icon;
pub mod meter;
pub mod paths;
pub mod probe;
pub mod quota;
pub mod statusline;
pub mod update;

pub use config::{Config, Policy};
pub use credentials::Credentials;
pub use meter::{Meter, Origin, Reading};
pub use quota::{human_duration, Quota, Source, Window, WindowKind};

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

/// L'heure courante, en secondes depuis l'epoch.
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Tout ce qui peut mal tourner.
#[derive(Debug)]
pub enum Error {
    /// Le jeton OAuth de Claude Code est introuvable ou illisible.
    Credentials(String),
    /// L'API a refuse le jeton (401). Porte ce qu'elle en a dit.
    Unauthorized(String),
    /// L'appel a l'API n'a pas abouti.
    Probe(String),
    /// La machine qui detient le releve n'a pas repondu.
    Remote(String),
    /// La reponse est arrivee, mais sans aucun en-tete de quota.
    NoQuotaHeaders,
    /// Un chemin standard n'a pas pu etre determine.
    NoHome,
    /// Un acces fichier a echoue.
    Io(std::io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Credentials(detail) => write!(f, "identifiants Claude Code : {detail}"),
            Error::Unauthorized(detail) => write!(
                f,
                "jeton refuse par l'API ({detail}). S'il vient de Claude Code, lance-le \
                 une fois pour qu'il le renouvelle. S'il vient d'une autre machine, \
                 c'est la-bas qu'il faut le rafraichir."
            ),
            Error::Probe(detail) => write!(f, "la sonde a echoue : {detail}"),
            Error::Remote(detail) => write!(f, "machine distante : {detail}"),
            Error::NoQuotaHeaders => write!(
                f,
                "la reponse de l'API ne portait aucun en-tete de quota (format change ?)"
            ),
            Error::NoHome => write!(f, "impossible de determiner le dossier personnel"),
            Error::Io(err) => write!(f, "acces fichier : {err}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Error::Io(err)
    }
}
