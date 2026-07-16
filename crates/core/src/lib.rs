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

pub use config::{Config, Policy};
pub use credentials::Credentials;
pub use meter::{Meter, Origin, Reading};
pub use quota::{human_duration, Quota, Window, WindowKind};

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
    /// Le jeton a expire : c'est a Claude Code de le renouveler, pas a nous.
    TokenExpired,
    /// L'appel a l'API n'a pas abouti.
    Probe(String),
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
            Error::TokenExpired => write!(
                f,
                "le jeton OAuth a expire. Lance Claude Code une fois : il le renouvellera."
            ),
            Error::Probe(detail) => write!(f, "la sonde a echoue : {detail}"),
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
