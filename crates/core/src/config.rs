//! Reglages persistants.
//!
//! Il n'y en a qu'un pour l'instant, mais c'est le plus important du projet :
//! choisir entre voir des chiffres toujours justes, ou ne jamais ouvrir de
//! fenetre de session a la place de l'utilisateur. Les deux ne sont pas
//! conciliables, donc c'est a lui de trancher.

use crate::paths;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::Path;

/// Quand envoyer une sonde.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Policy {
    /// Sonde a intervalle regulier, quoi qu'il arrive.
    ///
    /// Les chiffres restent justes meme quand la consommation vient d'ailleurs
    /// que de Claude Code (claude.ai dans le navigateur, l'app de bureau...),
    /// ce que rien ne permet de detecter en local.
    ///
    /// En contrepartie, une sonde envoyee au repos ouvre une fenetre de 5h :
    /// les fenetres finissent par s'enchainer sans rapport avec les heures de
    /// travail reelles.
    #[default]
    Fidele,

    /// Ne sonde que lorsque c'est gratuit, ou explicitement demande.
    ///
    /// Gratuit veut dire : une fenetre de 5h court deja, ou Claude Code a ecrit
    /// depuis le dernier releve. Dans ces deux cas la fenetre est ouverte de
    /// toute facon, donc la sonde ne coute rien. Sinon, silence : le widget
    /// n'ouvrira jamais de fenetre de lui-meme, pas meme au lancement. Le bouton
    /// « Rafraichir » reste la porte de sortie, et lui l'assume.
    ///
    /// En contrepartie, apres un reset, l'affichage reste a 0 % jusqu'au
    /// prochain geste ou a la prochaine activite de Claude Code.
    Prudent,
}

impl Policy {
    /// Libelle affichable.
    pub fn label(self) -> &'static str {
        match self {
            Policy::Fidele => "Sonde fidele",
            Policy::Prudent => "Sonde prudente",
        }
    }

    /// Explication courte, pour une infobulle.
    pub fn hint(self) -> &'static str {
        match self {
            Policy::Fidele => {
                "Chiffres toujours justes, y compris pour claude.ai. Peut ouvrir \
                 une fenetre de session quand tu ne travailles pas."
            }
            Policy::Prudent => {
                "N'ouvre jamais de fenetre de session. Peut afficher 0 % apres un \
                 reset, jusqu'au prochain rafraichissement."
            }
        }
    }

    /// L'autre politique, pour une bascule.
    pub fn toggled(self) -> Policy {
        match self {
            Policy::Fidele => Policy::Prudent,
            Policy::Prudent => Policy::Fidele,
        }
    }

    /// Lit une politique ecrite a la main.
    pub fn parse(raw: &str) -> Option<Policy> {
        match raw.trim().to_lowercase().as_str() {
            "fidele" | "fidèle" => Some(Policy::Fidele),
            "prudent" | "prudente" => Some(Policy::Prudent),
            _ => None,
        }
    }
}

/// Les reglages.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub policy: Policy,
}

/// Lit les reglages.
///
/// Un fichier absent ou abime rend les valeurs par defaut : un reglage casse ne
/// doit pas empecher le widget de demarrer.
pub fn load(path: &Path) -> Config {
    fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

/// Lit les reglages a l'emplacement standard.
pub fn load_default() -> Config {
    match paths::config_file() {
        Ok(path) => load(&path),
        Err(_) => Config::default(),
    }
}

/// Ecrit les reglages, de facon atomique.
pub fn store(path: &Path, config: &Config) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let encoded = serde_json::to_vec_pretty(config)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;

    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(&encoded)?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new(nom: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("claude-jauge-cfg-{nom}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("dossier temporaire");
            TempDir(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn la_politique_par_defaut_est_fidele() {
        assert_eq!(Config::default().policy, Policy::Fidele);
    }

    #[test]
    fn un_reglage_ecrit_se_relit() {
        let dir = TempDir::new("aller-retour");
        let fichier = dir.0.join("config.json");
        let config = Config { policy: Policy::Prudent };
        store(&fichier, &config).expect("ecriture");
        assert_eq!(load(&fichier), config);
    }

    #[test]
    fn un_fichier_absent_ou_abime_rend_les_defauts() {
        let dir = TempDir::new("abime");
        assert_eq!(load(&dir.0.join("jamais-ecrit.json")), Config::default());

        let casse = dir.0.join("casse.json");
        fs::write(&casse, "{pas du json").unwrap();
        assert_eq!(load(&casse), Config::default());
    }

    #[test]
    fn un_fichier_partiel_garde_les_defauts_pour_le_reste() {
        let dir = TempDir::new("partiel");
        let fichier = dir.0.join("config.json");
        fs::write(&fichier, "{}").unwrap();
        assert_eq!(load(&fichier).policy, Policy::Fidele);
    }

    #[test]
    fn la_bascule_fait_bien_un_aller_retour() {
        assert_eq!(Policy::Fidele.toggled(), Policy::Prudent);
        assert_eq!(Policy::Prudent.toggled(), Policy::Fidele);
        assert_eq!(Policy::Fidele.toggled().toggled(), Policy::Fidele);
    }

    #[test]
    fn lit_une_politique_ecrite_a_la_main() {
        assert_eq!(Policy::parse("fidele"), Some(Policy::Fidele));
        assert_eq!(Policy::parse("  FIDELE "), Some(Policy::Fidele));
        assert_eq!(Policy::parse("prudent"), Some(Policy::Prudent));
        assert_eq!(Policy::parse("nawak"), None);
    }
}
