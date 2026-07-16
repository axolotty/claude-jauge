//! Orchestration : decider s'il faut sonder, puis rendre un releve.
//!
//! Clawdmeter, le petit ecran ESP32 qui a inspire ce projet, interroge l'API
//! toutes les 60 secondes. C'est simple, mais ca a un defaut : une sonde envoyee
//! alors qu'on ne travaillait pas OUVRE une fenetre de 5 heures. Le widget
//! demarre alors le compteur a la place de l'utilisateur, qui retrouve une
//! fenetre deja entamee quand il se met enfin au travail.
//!
//! Ici on s'appuie sur deux constats :
//!
//! 1. les compteurs ne bougent que si Claude Code parle a l'API, ce qui laisse
//!    une trace datee dans `~/.claude/projects` ;
//! 2. le compte a rebours avant reset et la retombee a zero d'une fenetre echue
//!    se calculent parfaitement en local, sans reseau.
//!
//! On ne sonde donc que quand un releve apprendrait vraiment quelque chose.

use crate::quota::Quota;
use crate::{activity, cache, credentials, paths, probe, Error};
use std::path::PathBuf;

/// Duree d'une fenetre de session, telle qu'appliquee par l'API.
pub const FIVE_HOURS: u64 = 5 * 3600;

/// Delai minimal entre deux sondes.
///
/// Pendant une session active, Claude Code ecrit dans son transcript en
/// permanence. Sans ce plancher, on sonderait a chaque rafraichissement de
/// l'icone.
pub const MIN_PROBE_INTERVAL: u64 = 60;

/// D'ou vient le releve affiche.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// On vient d'interroger l'API.
    Probed,
    /// Rien n'a bouge : on rejoue le dernier releve, resets appliques.
    Cached,
    /// Aucun releve connu, et sonder ouvrirait une fenetre pour rien.
    Idle,
}

/// Ce qu'on rend a l'interface.
#[derive(Debug, Clone)]
pub struct Reading {
    /// Le releve, deja recale sur l'heure courante. `None` a froid.
    pub quota: Option<Quota>,
    pub origin: Origin,
    /// "pro", "max"... pour l'afficher a cote du nom.
    pub subscription_type: Option<String>,
    /// Avertissement non bloquant : sonde en echec, jeton expire, etc.
    pub note: Option<String>,
}

/// Faut-il envoyer une sonde ?
///
/// La regle d'or : ne jamais ouvrir une fenetre de 5 heures a la place de
/// l'utilisateur. Une fonction pure, pour pouvoir la tester sans reseau ni
/// disque.
pub fn should_probe(
    force: bool,
    last_probe: Option<u64>,
    last_activity: Option<u64>,
    now: u64,
) -> bool {
    if force {
        return true;
    }

    match last_probe {
        Some(probe_at) => {
            // Plancher anti-rafale pendant les sessions actives.
            if now.saturating_sub(probe_at) < MIN_PROBE_INTERVAL {
                return false;
            }
            // Un releve ne vieillit que si Claude Code a parle depuis.
            matches!(last_activity, Some(act) if act > probe_at)
        }
        // Premier lancement : on ne se risque a sonder que si Claude Code a
        // servi dans les cinq dernieres heures. Une fenetre est alors deja
        // ouverte, donc la sonde n'en ouvre pas.
        None => matches!(last_activity, Some(act) if now.saturating_sub(act) < FIVE_HOURS),
    }
}

/// Le point d'entree cote application.
#[derive(Debug, Clone)]
pub struct Meter {
    /// Ou Claude Code ecrit ses transcripts.
    pub projects_dir: PathBuf,
    /// Ou l'on garde le dernier releve.
    pub cache_file: PathBuf,
}

impl Meter {
    /// Un compteur cable sur les emplacements standards de la plateforme.
    pub fn new() -> Result<Self, Error> {
        Ok(Meter {
            projects_dir: paths::projects_dir()?,
            cache_file: paths::cache_file()?,
        })
    }

    /// Rend l'etat courant des quotas.
    ///
    /// `force` envoie une sonde quoi qu'il arrive : a reserver a une action
    /// explicite de l'utilisateur, puisque ca peut ouvrir une fenetre de 5h.
    pub fn read(&self, force: bool) -> Reading {
        let now = crate::now();
        let cached = cache::load(&self.cache_file);
        let last_activity = activity::last_activity(&self.projects_dir);
        let last_probe = cached.as_ref().map(|q| q.probed_at);

        // Le type d'abonnement se lit en local, sans reseau : son absence n'est
        // pas une raison d'echouer.
        let creds = credentials::load();
        let subscription_type = creds
            .as_ref()
            .ok()
            .and_then(|c| c.subscription_type.clone());

        if should_probe(force, last_probe, last_activity, now) {
            match creds.as_ref().map_err(|e| e.to_string()).and_then(|c| {
                probe::probe(c, now).map_err(|e| e.to_string())
            }) {
                Ok(fresh) => {
                    // Un cache en echec n'a pas a casser l'affichage.
                    let note = cache::store(&self.cache_file, &fresh)
                        .err()
                        .map(|e| format!("releve non mis en cache : {e}"));
                    return Reading {
                        quota: Some(fresh.settled(now)),
                        origin: Origin::Probed,
                        subscription_type,
                        note,
                    };
                }
                Err(detail) => {
                    // La sonde a echoue : mieux vaut un vieux releve date qu'un
                    // ecran vide.
                    return Reading {
                        quota: cached.map(|q| q.settled(now)),
                        origin: if last_probe.is_some() { Origin::Cached } else { Origin::Idle },
                        subscription_type,
                        note: Some(detail),
                    };
                }
            }
        }

        match cached {
            Some(quota) => Reading {
                quota: Some(quota.settled(now)),
                origin: Origin::Cached,
                subscription_type,
                note: None,
            },
            None => Reading {
                quota: None,
                origin: Origin::Idle,
                subscription_type,
                note: Some(
                    "aucun releve : Claude Code n'a pas servi depuis plus de 5h, et sonder \
                     maintenant ouvrirait une fenetre de session pour rien."
                        .to_string(),
                ),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAINTENANT: u64 = 1_784_225_000;

    #[test]
    fn force_sonde_toujours() {
        // Meme a froid, meme sans la moindre activite.
        assert!(should_probe(true, None, None, MAINTENANT));
        // Meme juste apres un releve tout frais.
        assert!(should_probe(true, Some(MAINTENANT), Some(MAINTENANT), MAINTENANT));
    }

    #[test]
    fn a_froid_sans_activite_on_ne_sonde_pas() {
        // C'est la regle d'or : ne pas ouvrir de fenetre a la place de l'utilisateur.
        assert!(!should_probe(false, None, None, MAINTENANT));
    }

    #[test]
    fn a_froid_on_sonde_si_une_fenetre_est_probablement_ouverte() {
        // Claude Code a servi il y a 10 minutes : une fenetre court deja.
        let recent = MAINTENANT - 600;
        assert!(should_probe(false, None, Some(recent), MAINTENANT));
    }

    #[test]
    fn a_froid_on_ne_sonde_pas_si_la_derniere_activite_est_trop_vieille() {
        // Plus de 5h sans rien : la fenetre est retombee, sonder en rouvrirait une.
        let vieux = MAINTENANT - FIVE_HOURS - 1;
        assert!(!should_probe(false, None, Some(vieux), MAINTENANT));

        // Pile a la limite, on s'abstient aussi.
        let limite = MAINTENANT - FIVE_HOURS;
        assert!(!should_probe(false, None, Some(limite), MAINTENANT));
    }

    #[test]
    fn on_resonde_quand_claude_a_parle_depuis_le_dernier_releve() {
        let probe_at = MAINTENANT - 300;
        let activite = probe_at + 10;
        assert!(should_probe(false, Some(probe_at), Some(activite), MAINTENANT));
    }

    #[test]
    fn on_ne_resonde_pas_si_rien_na_bouge() {
        let probe_at = MAINTENANT - 300;
        // Derniere activite anterieure au releve : les compteurs n'ont pas change.
        assert!(!should_probe(false, Some(probe_at), Some(probe_at - 60), MAINTENANT));
        // Activite pile au moment du releve : deja prise en compte.
        assert!(!should_probe(false, Some(probe_at), Some(probe_at), MAINTENANT));
        // Aucune activite connue.
        assert!(!should_probe(false, Some(probe_at), None, MAINTENANT));
    }

    #[test]
    fn le_plancher_evite_les_rafales_pendant_une_session_active() {
        // Claude Code ecrit en continu, mais on vient de sonder il y a 5 s.
        let probe_at = MAINTENANT - 5;
        let activite = MAINTENANT;
        assert!(!should_probe(false, Some(probe_at), Some(activite), MAINTENANT));

        // Passe le plancher, la meme situation redevient sondable.
        let vieux_probe = MAINTENANT - MIN_PROBE_INTERVAL;
        assert!(should_probe(false, Some(vieux_probe), Some(MAINTENANT), MAINTENANT));
    }

    #[test]
    fn une_horloge_qui_recule_ne_fait_pas_paniquer() {
        // Un releve date du futur (NTP qui recale, veille prolongee...) :
        // saturating_sub evite le debordement, et on s'abstient simplement.
        let futur = MAINTENANT + 10_000;
        assert!(!should_probe(false, Some(futur), Some(MAINTENANT), MAINTENANT));
    }
}
