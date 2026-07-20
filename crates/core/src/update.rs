//! Savoir si une version plus recente existe.
//!
//! On se contente de le DIRE. Rien n'est telecharge, rien n'est installe :
//! l'utilisateur clique sur un lien et decide. Installer tout seul supposerait
//! des binaires signes, ce qu'ils ne sont pas, et macOS refuserait de lancer ce
//! qu'on aurait pose sans son accord.

use crate::Error;
use serde::Deserialize;
use std::time::Duration;

/// Le depot interroge.
pub const REPO: &str = "axolotty/claude-jauge";

/// La version de ce binaire, telle qu'inscrite par cargo a la compilation.
pub const VERSION_COURANTE: &str = env!("CARGO_PKG_VERSION");

/// Delai avant d'abandonner : une verification de mise a jour ne doit jamais
/// faire attendre l'utilisateur.
const TIMEOUT: Duration = Duration::from_secs(10);

/// Ce qu'on retient d'une release.
#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    /// Version sans le `v` : « 0.2.0 ».
    pub version: String,
    /// La page a ouvrir dans le navigateur.
    pub url: String,
}

#[derive(Deserialize)]
struct ReleaseBrute {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

/// Un numero de version comparable.
///
/// Volontairement tolerant : on accepte `v0.2.0`, `0.2`, `0.2.0-beta`. Ce qui
/// n'est pas reconnu vaut `None`, et une version illisible ne declenchera
/// jamais de notification a tort.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(u32, u32, u32);

impl Version {
    pub fn parse(brut: &str) -> Option<Version> {
        let nettoye = brut.trim().trim_start_matches(['v', 'V']);
        // On coupe avant un suffixe eventuel : « 0.2.0-beta.1 » -> « 0.2.0 ».
        let noyau = nettoye
            .split(['-', '+'])
            .next()
            .filter(|s| !s.is_empty())?;

        let mut morceaux = noyau.split('.');
        let majeur = morceaux.next()?.parse().ok()?;
        // Un numero incomplet est complete par des zeros : « 0.2 » vaut 0.2.0.
        let mineur = morceaux.next().unwrap_or("0").parse().ok()?;
        let correctif = morceaux.next().unwrap_or("0").parse().ok()?;
        // Un quatrieme nombre n'a rien a faire la.
        if morceaux.next().is_some() {
            return None;
        }
        Some(Version(majeur, mineur, correctif))
    }
}

/// Y a-t-il mieux que ce qu'on execute ?
///
/// Fonction pure, pour pouvoir l'eprouver sans reseau. Rend la release
/// seulement si elle est strictement plus recente ; a egalite ou en dessous,
/// `None`. Si l'une des deux versions est illisible, on s'abstient : mieux vaut
/// rater une mise a jour que harceler quelqu'un a tort.
pub fn plus_recente(courante: &str, disponible: &Release) -> bool {
    match (Version::parse(courante), Version::parse(&disponible.version)) {
        (Some(ici), Some(la_bas)) => la_bas > ici,
        _ => false,
    }
}

/// Demande a GitHub la derniere version publiee.
///
/// Ignore les brouillons et les preversions : seul ce qui est publie compte.
pub fn derniere_release() -> Result<Release, Error> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");

    let reponse = ureq::get(&url)
        .set("accept", "application/vnd.github+json")
        .set("x-github-api-version", "2022-11-28")
        // GitHub refuse les requetes anonymes sans user-agent.
        .set("user-agent", "claude-jauge")
        .timeout(TIMEOUT)
        .call()
        .map_err(|err| Error::Probe(format!("verification des mises a jour : {err}")))?;

    let brute: ReleaseBrute = reponse
        .into_json()
        .map_err(|err| Error::Probe(format!("reponse de GitHub illisible : {err}")))?;

    if brute.draft || brute.prerelease {
        return Err(Error::Probe(
            "la derniere release est un brouillon ou une preversion".into(),
        ));
    }

    Ok(Release {
        version: brute.tag_name.trim_start_matches(['v', 'V']).to_string(),
        url: brute.html_url,
    })
}

/// La page des releases, a ouvrir quand on ne sait rien de plus.
pub fn page_releases() -> String {
    format!("https://github.com/{REPO}/releases/latest")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(v: &str) -> Release {
        Release { version: v.into(), url: "https://exemple".into() }
    }

    #[test]
    fn lit_les_formes_courantes_de_numero() {
        assert_eq!(Version::parse("0.2.0"), Some(Version(0, 2, 0)));
        assert_eq!(Version::parse("v0.2.0"), Some(Version(0, 2, 0)));
        assert_eq!(Version::parse("  V1.10.3  "), Some(Version(1, 10, 3)));
        // Numero incomplet : complete par des zeros.
        assert_eq!(Version::parse("0.2"), Some(Version(0, 2, 0)));
        assert_eq!(Version::parse("2"), Some(Version(2, 0, 0)));
        // Suffixe de preversion : on garde le noyau.
        assert_eq!(Version::parse("0.2.0-beta.1"), Some(Version(0, 2, 0)));
        assert_eq!(Version::parse("0.2.0+build7"), Some(Version(0, 2, 0)));
    }

    #[test]
    fn refuse_ce_qui_nest_pas_un_numero() {
        assert_eq!(Version::parse(""), None);
        assert_eq!(Version::parse("v"), None);
        assert_eq!(Version::parse("nawak"), None);
        assert_eq!(Version::parse("0.2.x"), None);
        // Quatre nombres : ce n'est pas notre format.
        assert_eq!(Version::parse("1.2.3.4"), None);
    }

    #[test]
    fn compare_par_composant_et_non_par_texte() {
        // Le piege classique : « 0.10.0 » est plus recent que « 0.9.0 », alors
        // qu'il vient avant dans l'ordre alphabetique.
        assert!(Version::parse("0.10.0") > Version::parse("0.9.0"));
        assert!(Version::parse("1.0.0") > Version::parse("0.99.99"));
        assert!(Version::parse("0.1.10") > Version::parse("0.1.9"));
    }

    #[test]
    fn signale_une_version_plus_recente() {
        assert!(plus_recente("0.1.5", &release("0.2.0")));
        assert!(plus_recente("0.1.5", &release("v0.1.6")));
        assert!(plus_recente("0.9.0", &release("0.10.0")));
    }

    #[test]
    fn se_tait_a_egalite_ou_en_dessous() {
        assert!(!plus_recente("0.1.5", &release("0.1.5")));
        assert!(!plus_recente("0.1.5", &release("0.1.4")));
        assert!(!plus_recente("1.0.0", &release("0.9.9")));
    }

    #[test]
    fn se_tait_quand_un_numero_est_illisible() {
        // Mieux vaut rater une mise a jour que harceler a tort.
        assert!(!plus_recente("0.1.5", &release("nawak")));
        assert!(!plus_recente("bizarre", &release("9.9.9")));
        assert!(!plus_recente("0.1.5", &release("")));
    }

    #[test]
    fn la_version_courante_est_celle_du_binaire() {
        // Si cargo ne l'injectait pas, on comparerait a du vide et on
        // annoncerait une mise a jour a chaque demarrage.
        assert!(Version::parse(VERSION_COURANTE).is_some(), "VERSION_COURANTE = {VERSION_COURANTE}");
    }
}
