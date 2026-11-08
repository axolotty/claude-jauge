//! L'endpoint usage : ce que l'ecran `/usage` de Claude Code interroge.
//!
//! `GET /api/oauth/usage` avec le jeton OAuth rend l'etat complet du compte, et
//! surtout la seule chose qu'on ne trouve nulle part ailleurs : la fenetre
//! hebdo propre a un modele (« Fable » sur l'abonnement Max), avec son nom. Un
//! extrait de reponse reelle :
//!
//! ```json
//! {
//!   "five_hour": { "utilization": 21.0, "resets_at": "2026-11-08T21:00:00+00:00" },
//!   "limits": [
//!     { "kind": "session", "percent": 21, "resets_at": "..." },
//!     { "kind": "weekly_all", "percent": 8, "resets_at": "..." },
//!     { "kind": "weekly_scoped", "percent": 11, "resets_at": "...",
//!       "scope": { "model": { "display_name": "Fable" } } }
//!   ]
//! }
//! ```
//!
//! Contrairement a la sonde, cet appel ne passe pas par `/v1/messages` : il ne
//! consomme aucun token et n'ouvre JAMAIS de fenetre de 5h. Il n'a donc pas
//! besoin des precautions de [`crate::meter::should_probe`], seulement d'un
//! espacement de politesse ([`crate::meter::USAGE_INTERVAL`]).
//!
//! Deux reserves. L'endpoint n'est pas documente publiquement, il peut changer
//! sans preavis : tout est optionnel ici et l'absence d'un champ rend `None`,
//! jamais une erreur. Et l'echelle differe des en-tetes : `percent` vaut deja
//! 0 a 100, comme la statusline, PAS comme les en-tetes (0.73 pour 73 %).

use crate::quota::{ModelWeek, Window, WindowKind};
use crate::{Credentials, Error};
use serde::Deserialize;
use std::time::Duration;

/// L'endpoint du releve de compte.
pub const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";

/// Au-dela, tant pis : cet appel est un bonus, jamais un pilier, et il peut
/// courir apres le rendu d'une statusline qui ne doit pas trainer.
const TIMEOUT: Duration = Duration::from_secs(5);

/// Interroge l'endpoint et rend la fenetre modele du compte.
///
/// `Ok(None)` veut dire : le compte n'a pas de fenetre propre a un modele
/// (abonnement Pro, par exemple). C'est une reponse, pas un echec.
pub fn fetch(creds: &Credentials) -> Result<Option<ModelWeek>, Error> {
    let response = ureq::get(USAGE_URL)
        .timeout(TIMEOUT)
        .set("authorization", &format!("Bearer {}", creds.token))
        .set("anthropic-beta", crate::probe::OAUTH_BETA)
        .call()
        .map_err(|err| Error::Probe(format!("usage : {err}")))?;

    let body = response
        .into_string()
        .map_err(|err| Error::Probe(format!("usage : reponse illisible ({err})")))?;
    parse(&body)
}

// Tout est optionnel : l'endpoint n'est pas documente, un champ qui disparait
// ne doit jamais faire echouer la lecture des autres.

#[derive(Deserialize)]
struct Document {
    limits: Option<Vec<Limit>>,
}

#[derive(Deserialize)]
struct Limit {
    kind: Option<String>,
    /// Deja un pourcentage de 0 a 100, pas une fraction.
    percent: Option<f64>,
    /// RFC 3339, la ou les en-tetes donnent un epoch.
    resets_at: Option<String>,
    scope: Option<Scope>,
}

#[derive(Deserialize)]
struct Scope {
    model: Option<ScopeModel>,
}

#[derive(Deserialize)]
struct ScopeModel {
    display_name: Option<String>,
}

/// Lit la fenetre modele dans une reponse de l'endpoint.
pub fn parse(document: &str) -> Result<Option<ModelWeek>, Error> {
    let document: Document = serde_json::from_str(document)
        .map_err(|err| Error::Probe(format!("usage : document illisible ({err})")))?;

    // ponytail: premiere fenetre `weekly_scoped` seulement ; si un compte en
    // porte un jour plusieurs, il faudra une jauge par entree.
    let fenetre = document
        .limits
        .unwrap_or_default()
        .into_iter()
        .find(|l| l.kind.as_deref() == Some("weekly_scoped"));

    Ok(fenetre.map(|l| ModelWeek {
        label: l
            .scope
            .and_then(|s| s.model)
            .and_then(|m| m.display_name)
            .unwrap_or_else(|| "modele".to_string()),
        window: Window {
            kind: WindowKind::ModelWeek,
            // Deja en pourcentage : on assainit sans convertir, comme pour la
            // statusline.
            utilization: l.percent.and_then(crate::quota::tidy_percent),
            reset_at: l.resets_at.as_deref().and_then(epoch),
            status: None,
        },
    }))
}

/// Convertit une date RFC 3339 en secondes depuis l'epoch.
///
/// Une date illisible ou anterieure a 1970 rend `None` : le compte a rebours
/// s'affichera en « ? », ce qui vaut mieux qu'une heure inventee.
fn epoch(rfc3339: &str) -> Option<u64> {
    chrono::DateTime::parse_from_rfc3339(rfc3339)
        .ok()
        .and_then(|dt| u64::try_from(dt.timestamp()).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reponse reelle du 8 novembre 2026, reduite a ce qu'on lit.
    const DOCUMENT: &str = r#"{
        "five_hour": { "utilization": 21.0, "resets_at": "2026-11-08T21:00:00.809625+00:00" },
        "seven_day": { "utilization": 8.0, "resets_at": "2026-11-15T07:00:00.809648+00:00" },
        "seven_day_opus": null,
        "extra_usage": { "is_enabled": false, "utilization": 0.0 },
        "limits": [
            { "kind": "session", "group": "session", "percent": 21, "severity": "normal",
              "resets_at": "2026-11-08T21:00:00.442359+00:00", "scope": null, "is_active": true },
            { "kind": "weekly_all", "group": "weekly", "percent": 8, "severity": "normal",
              "resets_at": "2026-11-15T07:00:00.442378+00:00", "scope": null, "is_active": false },
            { "kind": "weekly_scoped", "group": "weekly", "percent": 11, "severity": "normal",
              "resets_at": "2026-11-15T07:00:00.442508+00:00",
              "scope": { "model": { "id": null, "display_name": "Fable" }, "surface": null },
              "is_active": false }
        ]
    }"#;

    #[test]
    fn lit_la_fenetre_modele_dune_reponse_reelle() {
        let mw = parse(DOCUMENT).expect("un document").expect("une fenetre");
        assert_eq!(mw.label, "Fable");
        assert_eq!(mw.window.kind, WindowKind::ModelWeek);
        assert_eq!(mw.window.utilization, Some(11.0));
        // 2026-11-15T07:00:00+00:00
        assert_eq!(mw.window.reset_at, Some(1794726000));
    }

    #[test]
    fn le_pourcentage_nest_pas_reconverti() {
        // Meme piege que la statusline : `percent` vaut deja 11 pour 11 %.
        // Une conversion de fraction l'ecraserait a 100 %.
        let mw = parse(DOCUMENT).unwrap().unwrap();
        assert_eq!(mw.window.utilization, Some(11.0));
    }

    #[test]
    fn un_compte_sans_fenetre_modele_rend_none() {
        // Un abonnement Pro : des limites, mais aucune `weekly_scoped`.
        let pro = r#"{"limits": [
            { "kind": "session", "percent": 50 },
            { "kind": "weekly_all", "percent": 10 }
        ]}"#;
        assert_eq!(parse(pro).unwrap(), None);

        // Et les formes degradees : pas de tableau du tout, ou vide.
        assert_eq!(parse(r#"{}"#).unwrap(), None);
        assert_eq!(parse(r#"{"limits": []}"#).unwrap(), None);
        assert_eq!(parse(r#"{"limits": null}"#).unwrap(), None);
    }

    #[test]
    fn un_nom_de_modele_absent_a_un_libelle_de_secours() {
        let anonyme = r#"{"limits": [{ "kind": "weekly_scoped", "percent": 42 }]}"#;
        let mw = parse(anonyme).unwrap().expect("une fenetre");
        assert_eq!(mw.label, "modele");
        assert_eq!(mw.window.utilization, Some(42.0));
        assert_eq!(mw.window.reset_at, None);
    }

    #[test]
    fn une_date_illisible_rend_un_reset_inconnu_pas_une_erreur() {
        let tordu = r#"{"limits": [{ "kind": "weekly_scoped", "percent": 11,
            "resets_at": "pas une date" }]}"#;
        assert_eq!(parse(tordu).unwrap().unwrap().window.reset_at, None);
    }

    #[test]
    fn un_document_illisible_est_une_erreur_franche() {
        assert!(parse("pas du json").is_err());
    }

    #[test]
    fn une_valeur_aberrante_est_bornee() {
        let fou = r#"{"limits": [{ "kind": "weekly_scoped", "percent": 150 }]}"#;
        assert_eq!(parse(fou).unwrap().unwrap().window.utilization, Some(100.0));
    }
}
