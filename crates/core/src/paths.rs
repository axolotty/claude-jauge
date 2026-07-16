//! Les quelques chemins dont on a besoin, resolus a la main.
//!
//! Une caisse comme `directories` ferait ca tres bien, mais elle amenerait ses
//! dependances pour trois dossiers. Le coeur reste donc sans dependance autre
//! que serde et le client HTTP.

use crate::Error;
use std::env;
use std::path::PathBuf;

/// Nom du dossier applicatif, cote cache.
pub const APP: &str = "claude-jauge";

/// Le dossier personnel.
pub fn home() -> Result<PathBuf, Error> {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    env::var_os(var).map(PathBuf::from).ok_or(Error::NoHome)
}

/// `~/.claude`, ou Claude Code range tout.
pub fn claude_dir() -> Result<PathBuf, Error> {
    Ok(home()?.join(".claude"))
}

/// `~/.claude/projects`, ou vivent les transcripts de conversation.
///
/// Leur date de modification est notre detecteur d'activite : Claude Code y
/// ecrit des qu'il parle a l'API.
pub fn projects_dir() -> Result<PathBuf, Error> {
    Ok(claude_dir()?.join("projects"))
}

/// Le fichier ou l'on garde le dernier releve entre deux lancements.
pub fn cache_file() -> Result<PathBuf, Error> {
    Ok(cache_dir()?.join("state.json"))
}

/// Le fichier de reglages.
pub fn config_file() -> Result<PathBuf, Error> {
    Ok(config_dir()?.join("config.json"))
}

/// Le dossier de configuration, selon les conventions de chaque plateforme.
///
/// Distinct du cache : on peut jeter le cache sans perdre les reglages.
pub fn config_dir() -> Result<PathBuf, Error> {
    if cfg!(target_os = "macos") {
        return Ok(home()?
            .join("Library")
            .join("Application Support")
            .join(APP));
    }
    if cfg!(windows) {
        if let Some(roaming) = env::var_os("APPDATA") {
            return Ok(PathBuf::from(roaming).join(APP));
        }
        return Ok(home()?.join("AppData").join("Roaming").join(APP));
    }
    if let Some(xdg) = env::var_os("XDG_CONFIG_HOME") {
        let xdg = PathBuf::from(xdg);
        if xdg.is_absolute() {
            return Ok(xdg.join(APP));
        }
    }
    Ok(home()?.join(".config").join(APP))
}

/// Le dossier de cache, selon les conventions de chaque plateforme.
pub fn cache_dir() -> Result<PathBuf, Error> {
    if cfg!(target_os = "macos") {
        return Ok(home()?.join("Library").join("Caches").join(APP));
    }
    if cfg!(windows) {
        if let Some(local) = env::var_os("LOCALAPPDATA") {
            return Ok(PathBuf::from(local).join(APP));
        }
        return Ok(home()?.join("AppData").join("Local").join(APP));
    }
    if let Some(xdg) = env::var_os("XDG_CACHE_HOME") {
        let xdg = PathBuf::from(xdg);
        if xdg.is_absolute() {
            return Ok(xdg.join(APP));
        }
    }
    Ok(home()?.join(".cache").join(APP))
}
