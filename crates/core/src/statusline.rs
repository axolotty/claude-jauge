//! La voie officielle : se faire nourrir par Claude Code.
//!
//! Quand une statusline est configuree, Claude Code lui passe l'etat de la
//! session sur stdin a chaque rendu, et ce document contient les quotas :
//!
//! ```json
//! {
//!   "model": { "display_name": "Opus" },
//!   "context_window": { "used_percentage": 8 },
//!   "rate_limits": {
//!     "five_hour": { "used_percentage": 23.5, "resets_at": 1738425600 },
//!     "seven_day": { "used_percentage": 41.2, "resets_at": 1738857600 }
//!   }
//! }
//! ```
//!
//! C'est strictement mieux que la sonde partout ou c'est possible : documente,
//! gratuit (la statusline ne consomme aucun token), sans jeton a manipuler, sans
//! en-tete non publie, et sans le moindre risque d'ouvrir une fenetre de session.
//!
//! Deux limites : `rate_limits` n'apparait que pour les abonnes Pro et Max, et
//! seulement apres la premiere reponse de l'API dans la session. Et il n'y a ni
//! overage ni fallback, que seule la sonde expose.
//!
//! **Attention a l'echelle.** Ici `used_percentage` vaut deja 0 a 100 (`23.5`
//! signifie 23,5 %), alors que les en-tetes HTTP expriment une fraction (`0.73`
//! pour 73 %). Ne pas multiplier par 100 de ce cote.

use crate::quota::{Quota, Source, Window, WindowKind};
use crate::Error;
use serde::Deserialize;

/// Ce que Claude Code passe sur stdin.
///
/// Tout est optionnel : le document est riche et evolue, on n'y pioche que ce
/// qui nous concerne, et un champ absent ne doit jamais faire echouer un rendu
/// de statusline.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Input {
    pub model: Option<Model>,
    pub context_window: Option<ContextWindow>,
    pub rate_limits: Option<RateLimits>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Model {
    pub id: Option<String>,
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ContextWindow {
    /// Part de la fenetre de contexte consommee, de 0 a 100.
    pub used_percentage: Option<f64>,
    pub context_window_size: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RateLimits {
    pub five_hour: Option<RateWindow>,
    pub seven_day: Option<RateWindow>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RateWindow {
    /// Part consommee, de 0 a 100 (deja en pourcentage, pas en fraction).
    pub used_percentage: Option<f64>,
    /// Epoch en secondes de la remise a zero.
    pub resets_at: Option<u64>,
}

/// Lit le document passe par Claude Code.
pub fn parse(document: &str) -> Result<Input, Error> {
    serde_json::from_str(document)
        .map_err(|err| Error::Probe(format!("statusline : document illisible ({err})")))
}

impl Input {
    /// Convertit en releve exploitable.
    ///
    /// Rend `None` quand Claude Code n'a pas fourni de quotas : compte hors
    /// abonnement, ou tout debut de session avant la premiere reponse de l'API.
    /// Ce n'est pas une erreur, seulement une absence.
    pub fn quota(&self, now: u64) -> Option<Quota> {
        let limits = self.rate_limits.as_ref()?;
        if limits.five_hour.is_none() && limits.seven_day.is_none() {
            return None;
        }

        Some(Quota {
            five_hour: fenetre(WindowKind::FiveHour, limits.five_hour.as_ref()),
            seven_day: fenetre(WindowKind::SevenDay, limits.seven_day.as_ref()),
            // La statusline ne dit rien de l'overage : le pretendre a zero serait
            // inventer une donnee.
            overage: Window::unknown(WindowKind::Overage),
            // Ni du statut, ni du repli. On laisse vide plutot que de deviner.
            overall_status: None,
            fallback: None,
            fallback_percentage: None,
            representative_claim: None,
            probed_at: now,
            source: Source::Statusline,
        })
    }

    /// Le nom du modele en cours, s'il est connu.
    pub fn model_name(&self) -> Option<&str> {
        self.model.as_ref()?.display_name.as_deref()
    }

    /// La part de fenetre de contexte consommee.
    pub fn context_used(&self) -> Option<f64> {
        self.context_window.as_ref()?.used_percentage
    }
}

fn fenetre(kind: WindowKind, brut: Option<&RateWindow>) -> Window {
    match brut {
        None => Window::unknown(kind),
        Some(w) => Window {
            kind,
            // Deja un pourcentage : on borne, sans convertir.
            utilization: w
                .used_percentage
                .filter(|v| v.is_finite())
                .map(|v| v.clamp(0.0, 100.0)),
            reset_at: w.resets_at,
            status: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Le document tel que documente par Claude Code, reduit a ce qu'on lit.
    const DOCUMENT: &str = r#"{
        "cwd": "/un/dossier",
        "session_id": "abc123",
        "model": { "id": "claude-opus-4-8", "display_name": "Opus" },
        "version": "2.1.211",
        "context_window": {
            "total_input_tokens": 15500,
            "context_window_size": 200000,
            "used_percentage": 8,
            "remaining_percentage": 92
        },
        "exceeds_200k_tokens": false,
        "rate_limits": {
            "five_hour": { "used_percentage": 23.5, "resets_at": 1738425600 },
            "seven_day": { "used_percentage": 41.2, "resets_at": 1738857600 }
        }
    }"#;

    #[test]
    fn lit_le_document_de_claude_code() {
        let input = parse(DOCUMENT).expect("un document");
        assert_eq!(input.model_name(), Some("Opus"));
        assert_eq!(input.context_used(), Some(8.0));

        let q = input.quota(1738425000).expect("un releve");
        assert_eq!(q.five_hour.utilization, Some(23.5));
        assert_eq!(q.five_hour.reset_at, Some(1738425600));
        assert_eq!(q.seven_day.utilization, Some(41.2));
        assert_eq!(q.source, Source::Statusline);
    }

    #[test]
    fn le_pourcentage_nest_pas_reconverti() {
        // Piege : les en-tetes HTTP donnent 0.73 pour 73 %, la statusline donne
        // deja 23.5 pour 23,5 %. Multiplier ici saturerait tout a 100 %.
        let q = parse(DOCUMENT).unwrap().quota(0).unwrap();
        assert_eq!(q.five_hour.utilization, Some(23.5));
        assert_eq!(q.five_hour.remaining(), Some(76.5));
    }

    #[test]
    fn loverage_reste_inconnu_plutot_que_zero() {
        let q = parse(DOCUMENT).unwrap().quota(0).unwrap();
        assert_eq!(q.overage.utilization, None);
        assert_eq!(q.overage.remaining(), None);
    }

    #[test]
    fn sans_rate_limits_il_n_y_a_pas_de_releve() {
        // Le cas des comptes hors abonnement, et du tout debut de session.
        let sans = r#"{"model": {"display_name": "Opus"}}"#;
        let input = parse(sans).expect("un document");
        assert_eq!(input.model_name(), Some("Opus"));
        assert!(input.quota(0).is_none());

        let vide = r#"{"rate_limits": {}}"#;
        assert!(parse(vide).unwrap().quota(0).is_none());
    }

    #[test]
    fn une_seule_fenetre_suffit() {
        let partiel = r#"{"rate_limits": {"five_hour": {"used_percentage": 50}}}"#;
        let q = parse(partiel).unwrap().quota(0).expect("un releve");
        assert_eq!(q.five_hour.utilization, Some(50.0));
        assert_eq!(q.seven_day.utilization, None);
    }

    #[test]
    fn un_document_inconnu_ne_fait_pas_echouer_le_rendu() {
        // Claude Code enrichit son document au fil des versions : des champs
        // qu'on ne connait pas ne doivent rien casser.
        let futur = r#"{"rate_limits": {"five_hour": {"used_percentage": 10}}, "truc_futur": {"a": 1}}"#;
        assert!(parse(futur).unwrap().quota(0).is_some());
    }

    #[test]
    fn un_document_illisible_est_une_erreur_franche() {
        assert!(parse("pas du json").is_err());
    }

    #[test]
    fn une_valeur_aberrante_est_bornee() {
        let fou = r#"{"rate_limits": {"five_hour": {"used_percentage": 150}}}"#;
        assert_eq!(parse(fou).unwrap().quota(0).unwrap().five_hour.utilization, Some(100.0));
    }
}
