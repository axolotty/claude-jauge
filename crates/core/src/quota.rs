//! Modele des quotas, et lecture des en-tetes `anthropic-ratelimit-unified-*`.
//!
//! L'API renvoie l'etat des quotas sur CHAQUE reponse, dans ses en-tetes. Un
//! releve reel ressemble a ceci :
//!
//! ```text
//! anthropic-ratelimit-unified-status: allowed
//! anthropic-ratelimit-unified-5h-status: allowed
//! anthropic-ratelimit-unified-5h-reset: 1784225400
//! anthropic-ratelimit-unified-5h-utilization: 0.73
//! anthropic-ratelimit-unified-7d-status: allowed
//! anthropic-ratelimit-unified-7d-reset: 1784444400
//! anthropic-ratelimit-unified-7d-utilization: 0.14
//! anthropic-ratelimit-unified-overage-status: allowed
//! anthropic-ratelimit-unified-overage-reset: 1785542400
//! anthropic-ratelimit-unified-overage-utilization: 0.0
//! anthropic-ratelimit-unified-representative-claim: five_hour
//! anthropic-ratelimit-unified-fallback-percentage: 0.5
//! anthropic-ratelimit-unified-fallback: available
//! ```
//!
//! Ces en-tetes ne sont pas documentes publiquement : ils peuvent changer sans
//! preavis. Tout est donc optionnel ici, et l'absence d'un champ n'est jamais
//! une erreur, seulement un `None` qui s'affichera en "?".

use serde::{Deserialize, Serialize};

/// Prefixe commun a tous les en-tetes de quota.
pub const HEADER_PREFIX: &str = "anthropic-ratelimit-unified";

/// Les trois fenetres de quota exposees par l'API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WindowKind {
    /// Fenetre glissante de 5 heures, ouverte par le premier message.
    FiveHour,
    /// Quota hebdomadaire.
    SevenDay,
    /// Depassement facture, remis a zero chaque mois.
    Overage,
}

impl WindowKind {
    /// Le fragment tel qu'il apparait dans le nom de l'en-tete.
    pub fn slug(self) -> &'static str {
        match self {
            WindowKind::FiveHour => "5h",
            WindowKind::SevenDay => "7d",
            WindowKind::Overage => "overage",
        }
    }

    /// Libelle affichable.
    pub fn label(self) -> &'static str {
        match self {
            WindowKind::FiveHour => "Session 5h",
            WindowKind::SevenDay => "Semaine 7j",
            WindowKind::Overage => "Overage",
        }
    }
}

/// Ce que le statut d'une fenetre veut dire, une fois traduit.
///
/// Trois etats observes en vrai, pas deux : a 95 % d'une fenetre, l'API renvoie
/// `allowed_warning`. Les requetes passent toujours, c'est un simple
/// avertissement. Confondre cet etat avec un refus afficherait une alarme rouge
/// alors que tout va bien.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Health {
    /// Tout passe.
    Allowed,
    /// Ca passe encore, mais la limite approche.
    Warning,
    /// L'API refuse les requetes sur cette fenetre.
    Blocked,
    /// Statut absent ou non reconnu.
    Unknown,
}

/// L'etat d'une fenetre de quota a l'instant du releve.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Window {
    pub kind: WindowKind,
    /// Part consommee, en pourcentage de 0 a 100.
    pub utilization: Option<f64>,
    /// Date de remise a zero, en secondes depuis l'epoch.
    pub reset_at: Option<u64>,
    /// Brut de l'API : "allowed", "allowed_warning"...
    pub status: Option<String>,
}

impl Window {
    /// Fenetre dont on ne sait rien.
    pub fn unknown(kind: WindowKind) -> Self {
        Window { kind, utilization: None, reset_at: None, status: None }
    }

    /// Part encore disponible, en pourcentage.
    pub fn remaining(&self) -> Option<f64> {
        self.utilization.map(|used| (100.0 - used).clamp(0.0, 100.0))
    }

    /// Secondes avant la remise a zero. `None` si la date est inconnue ou passee.
    pub fn seconds_left(&self, now: u64) -> Option<u64> {
        self.reset_at.and_then(|at| at.checked_sub(now)).filter(|&d| d > 0)
    }

    /// Vrai tant que la fenetre court encore.
    pub fn is_open(&self, now: u64) -> bool {
        self.seconds_left(now).is_some()
    }

    /// Traduit le statut brut de l'API.
    ///
    /// Le prefixe `allowed` fait foi : tout ce qui commence ainsi laisse passer
    /// les requetes. C'est volontairement tolerant, pour qu'une variante encore
    /// inconnue (`allowed_quelque_chose`) ne se retrouve pas affichee comme un
    /// refus.
    pub fn health(&self) -> Health {
        match self.status.as_deref() {
            None => Health::Unknown,
            Some("allowed") => Health::Allowed,
            Some(s) if s.starts_with("allowed") => Health::Warning,
            Some(_) => Health::Blocked,
        }
    }

    /// Vrai quand la fenetre ne laisse plus rien passer.
    pub fn is_blocked(&self) -> bool {
        self.health() == Health::Blocked
    }
}

/// Un releve complet, tel que renvoye par une reponse de l'API.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Quota {
    pub five_hour: Window,
    pub seven_day: Window,
    pub overage: Window,
    /// Statut global, "allowed" quand rien ne bloque.
    pub overall_status: Option<String>,
    /// "available" quand un repli vers un modele plus petit est propose.
    pub fallback: Option<String>,
    /// Seuil de declenchement du repli, en pourcentage.
    pub fallback_percentage: Option<f64>,
    /// Quelle fenetre fait actuellement foi ("five_hour", "seven_day"...).
    pub representative_claim: Option<String>,
    /// Date du releve, en secondes depuis l'epoch.
    pub probed_at: u64,
}

impl Quota {
    /// Toutes les fenetres, dans l'ordre d'affichage.
    pub fn windows(&self) -> [&Window; 3] {
        [&self.five_hour, &self.seven_day, &self.overage]
    }

    /// Age du releve, en secondes.
    pub fn age(&self, now: u64) -> u64 {
        now.saturating_sub(self.probed_at)
    }

    /// Lit un releve depuis les en-tetes d'une reponse.
    ///
    /// `get` recoit un nom d'en-tete en minuscules et rend sa valeur. Passer par
    /// une fermeture permet de brancher n'importe quel client HTTP sans que ce
    /// module ait a le connaitre.
    ///
    /// Rend `None` si la reponse ne porte aucun en-tete de quota : c'est le cas
    /// des erreurs reseau ou d'un intermediaire qui les aurait filtres.
    pub fn from_headers<F>(get: F, probed_at: u64) -> Option<Quota>
    where
        F: Fn(&str) -> Option<String>,
    {
        let window = |kind: WindowKind| Window {
            kind,
            utilization: get(&format!("{HEADER_PREFIX}-{}-utilization", kind.slug()))
                .and_then(|raw| parse_utilization(&raw)),
            reset_at: get(&format!("{HEADER_PREFIX}-{}-reset", kind.slug()))
                .and_then(|raw| raw.trim().parse::<u64>().ok()),
            status: get(&format!("{HEADER_PREFIX}-{}-status", kind.slug())),
        };

        let quota = Quota {
            five_hour: window(WindowKind::FiveHour),
            seven_day: window(WindowKind::SevenDay),
            overage: window(WindowKind::Overage),
            overall_status: get(&format!("{HEADER_PREFIX}-status")),
            fallback: get(&format!("{HEADER_PREFIX}-fallback")),
            fallback_percentage: get(&format!("{HEADER_PREFIX}-fallback-percentage"))
                .and_then(|raw| parse_utilization(&raw)),
            representative_claim: get(&format!("{HEADER_PREFIX}-representative-claim")),
            probed_at,
        };

        let vide = quota.windows().iter().all(|w| w.utilization.is_none() && w.reset_at.is_none())
            && quota.overall_status.is_none();
        if vide {
            None
        } else {
            Some(quota)
        }
    }

    /// Rejoue localement les remises a zero deja passees, sans toucher au reseau.
    ///
    /// C'est ce qui permet de ne PAS sonder quand rien n'a bouge : une fenetre
    /// dont la date de reset est derriere nous est forcement retombee a zero, et
    /// sa prochaine date ne sera connue qu'au premier message suivant.
    pub fn settled(&self, now: u64) -> Quota {
        let settle = |w: &Window| match w.reset_at {
            Some(at) if at <= now => Window {
                kind: w.kind,
                utilization: Some(0.0),
                reset_at: None,
                status: Some("allowed".to_string()),
            },
            _ => w.clone(),
        };
        Quota {
            five_hour: settle(&self.five_hour),
            seven_day: settle(&self.seven_day),
            overage: settle(&self.overage),
            ..self.clone()
        }
    }
}

/// Convertit une utilisation d'en-tete en pourcentage.
///
/// L'API l'exprime en fraction : `0.73` vaut 73 %, et `fallback-percentage: 0.5`
/// designe bien un seuil a 50 %. On multiplie donc par 100, puis on borne, pour
/// qu'une valeur inattendue ne fasse pas deborder une jauge.
///
/// L'arrondi au centieme n'est pas cosmetique : `0.14 * 100.0` vaut
/// `14.000000000000002` en virgule flottante, et ce nombre finirait tel quel
/// dans la sortie JSON. Le centieme de pourcent reste largement plus fin que
/// tout ce qu'on affiche.
fn parse_utilization(raw: &str) -> Option<f64> {
    let value: f64 = raw.trim().parse().ok()?;
    if !value.is_finite() {
        return None;
    }
    let percent = (value * 10_000.0).round() / 100.0;
    Some(percent.clamp(0.0, 100.0))
}

/// Ecrit une duree en francais compact : `3j02h`, `1h49`, `12m`, `<1m`.
pub fn human_duration(seconds: u64) -> String {
    let minutes = seconds / 60;
    let hours = minutes / 60;
    let days = hours / 24;
    if days > 0 {
        format!("{}j{:02}h", days, hours % 24)
    } else if hours > 0 {
        format!("{}h{:02}", hours, minutes % 60)
    } else if minutes > 0 {
        format!("{}m", minutes)
    } else {
        "<1m".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Le releve reel capture sur un compte Pro, qui a servi de reference.
    fn releve_reel() -> HashMap<String, String> {
        [
            ("anthropic-ratelimit-unified-status", "allowed"),
            ("anthropic-ratelimit-unified-5h-status", "allowed"),
            ("anthropic-ratelimit-unified-5h-reset", "1784225400"),
            ("anthropic-ratelimit-unified-5h-utilization", "0.73"),
            ("anthropic-ratelimit-unified-7d-status", "allowed"),
            ("anthropic-ratelimit-unified-7d-reset", "1784444400"),
            ("anthropic-ratelimit-unified-7d-utilization", "0.14"),
            ("anthropic-ratelimit-unified-overage-status", "allowed"),
            ("anthropic-ratelimit-unified-overage-reset", "1785542400"),
            ("anthropic-ratelimit-unified-overage-utilization", "0.0"),
            ("anthropic-ratelimit-unified-representative-claim", "five_hour"),
            ("anthropic-ratelimit-unified-fallback-percentage", "0.5"),
            ("anthropic-ratelimit-unified-fallback", "available"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
    }

    fn lecteur(map: HashMap<String, String>) -> impl Fn(&str) -> Option<String> {
        move |name: &str| map.get(name).cloned()
    }

    #[test]
    fn lit_un_releve_reel() {
        let q = Quota::from_headers(lecteur(releve_reel()), 1784225000).expect("un releve");
        assert_eq!(q.five_hour.utilization, Some(73.0));
        assert_eq!(q.five_hour.reset_at, Some(1784225400));
        assert_eq!(q.seven_day.utilization, Some(14.0));
        assert_eq!(q.overage.utilization, Some(0.0));
        assert_eq!(q.fallback.as_deref(), Some("available"));
        assert_eq!(q.fallback_percentage, Some(50.0));
        assert_eq!(q.representative_claim.as_deref(), Some("five_hour"));
    }

    #[test]
    fn calcule_le_restant_et_le_compte_a_rebours() {
        let q = Quota::from_headers(lecteur(releve_reel()), 1784225000).unwrap();
        assert_eq!(q.five_hour.remaining(), Some(27.0));
        // 1784225400 - 1784225000 = 400 s
        assert_eq!(q.five_hour.seconds_left(1784225000), Some(400));
        assert!(q.five_hour.is_open(1784225000));
    }

    #[test]
    fn une_fenetre_echue_nest_plus_ouverte() {
        let q = Quota::from_headers(lecteur(releve_reel()), 1784225000).unwrap();
        assert!(!q.five_hour.is_open(1784225400)); // pile a l'heure du reset
        assert!(!q.five_hour.is_open(1784999999)); // bien apres
        assert_eq!(q.five_hour.seconds_left(1784999999), None);
    }

    #[test]
    fn settled_remet_a_zero_les_fenetres_echues_sans_toucher_aux_autres() {
        let q = Quota::from_headers(lecteur(releve_reel()), 1784225000).unwrap();
        // Juste apres le reset des 5h, mais avant celui des 7j.
        let s = q.settled(1784225401);
        assert_eq!(s.five_hour.utilization, Some(0.0));
        assert_eq!(s.five_hour.reset_at, None);
        // La fenetre hebdo, elle, court toujours : elle ne doit pas bouger.
        assert_eq!(s.seven_day.utilization, Some(14.0));
        assert_eq!(s.seven_day.reset_at, Some(1784444400));
    }

    #[test]
    fn une_reponse_sans_en_tetes_de_quota_ne_donne_rien() {
        let vide: HashMap<String, String> = HashMap::new();
        assert!(Quota::from_headers(lecteur(vide), 0).is_none());
    }

    #[test]
    fn les_en_tetes_partiels_passent_sans_erreur() {
        let mut partiel = HashMap::new();
        partiel.insert(
            "anthropic-ratelimit-unified-5h-utilization".to_string(),
            "0.5".to_string(),
        );
        let q = Quota::from_headers(lecteur(partiel), 0).expect("un releve partiel");
        assert_eq!(q.five_hour.utilization, Some(50.0));
        assert_eq!(q.five_hour.reset_at, None);
        assert_eq!(q.seven_day.utilization, None);
        assert_eq!(q.seven_day.remaining(), None);
    }

    #[test]
    fn une_utilisation_aberrante_est_bornee() {
        assert_eq!(parse_utilization("1.5"), Some(100.0));
        assert_eq!(parse_utilization("-0.2"), Some(0.0));
        assert_eq!(parse_utilization("bruit"), None);
        assert_eq!(parse_utilization("NaN"), None);
        assert_eq!(parse_utilization("inf"), None);
    }

    /// Rejoue un statut donne sur la fenetre de 5h.
    fn avec_statut(statut: &str) -> Window {
        let mut headers = releve_reel();
        headers.insert(
            "anthropic-ratelimit-unified-5h-status".to_string(),
            statut.to_string(),
        );
        Quota::from_headers(lecteur(headers), 0).unwrap().five_hour
    }

    #[test]
    fn allowed_warning_avertit_sans_bloquer() {
        // Statut releve en vrai a 95 % d'une fenetre de 5h : ca passe toujours.
        let w = avec_statut("allowed_warning");
        assert_eq!(w.health(), Health::Warning);
        assert!(!w.is_blocked());
    }

    #[test]
    fn une_variante_allowed_inconnue_ne_passe_pas_pour_un_refus() {
        assert_eq!(avec_statut("allowed_soft_landing").health(), Health::Warning);
    }

    #[test]
    fn un_vrai_refus_bloque() {
        let w = avec_statut("rejected");
        assert_eq!(w.health(), Health::Blocked);
        assert!(w.is_blocked());
    }

    #[test]
    fn allowed_tout_court_ne_declenche_rien() {
        let q = Quota::from_headers(lecteur(releve_reel()), 0).unwrap();
        assert_eq!(q.five_hour.health(), Health::Allowed);
        assert!(!q.five_hour.is_blocked());
    }

    #[test]
    fn un_statut_absent_reste_inconnu() {
        let w = Window::unknown(WindowKind::FiveHour);
        assert_eq!(w.health(), Health::Unknown);
        assert!(!w.is_blocked());
    }

    #[test]
    fn ecrit_les_durees_en_francais() {
        assert_eq!(human_duration(0), "<1m");
        assert_eq!(human_duration(59), "<1m");
        assert_eq!(human_duration(12 * 60), "12m");
        assert_eq!(human_duration(6540), "1h49");
        assert_eq!(human_duration(2 * 86400 + 14 * 3600), "2j14h");
        assert_eq!(human_duration(3 * 86400 + 2 * 3600), "3j02h");
    }
}
