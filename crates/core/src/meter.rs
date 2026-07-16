//! Orchestration : decider s'il faut sonder, puis rendre un releve.
//!
//! Tout ce module tourne autour d'un arbitrage qui n'a pas de bonne reponse.
//!
//! Une sonde est une requete comme une autre, donc elle OUVRE une fenetre de
//! 5 heures s'il n'y en a pas deja une en cours. Clawdmeter, le petit ecran
//! ESP32 qui a inspire ce projet, interroge l'API toutes les 60 secondes : ses
//! chiffres sont toujours justes, mais le widget demarre le compteur a la place
//! de l'utilisateur, qui retrouve une fenetre deja entamee en se mettant au
//! travail.
//!
//! L'inverse ne marche pas non plus. On pourrait ne sonder que quand Claude Code
//! a ecrit dans `~/.claude/projects`, mais les quotas sont partages avec
//! claude.ai et l'app de bureau : une conversation dans le navigateur ne laisse
//! aucune trace locale. Se fier aux transcripts, c'est afficher des chiffres
//! faux des que la consommation vient d'ailleurs.
//!
//! D'ou deux politiques, et un reglage pour choisir. Voir [`Policy`].
//!
//! Cela dit, tout ce module n'est qu'un repli. Deux chemins l'evitent
//! entierement, et valent mieux que lui :
//!
//! * la statusline de Claude Code, qui nous sert les memes chiffres
//!   gratuitement (voir [`crate::statusline`]) ;
//! * la lecture du releve d'une autre machine, pour un poste ou Claude Code ne
//!   tourne pas (voir [`crate::config::RemoteSource`]).

use crate::config::{self, Policy};
use crate::quota::{Quota, Source};
use crate::{activity, cache, credentials, paths, probe, Error};
use std::path::PathBuf;

/// Duree d'une fenetre de session, telle qu'appliquee par l'API.
pub const FIVE_HOURS: u64 = 5 * 3600;

/// Delai entre deux sondes periodiques.
pub const POLL_INTERVAL: u64 = 120;

/// Au-dela, on considere que la statusline ne nous alimente plus.
///
/// Elle ne s'execute que pendant que Claude Code affiche sa barre. Tant qu'elle
/// nourrit le cache, sonder serait du gaspillage pur : elle donne les memes
/// chiffres, gratuitement et officiellement. Une fois Claude Code ferme, le
/// relais repasse a la politique en vigueur.
pub const STATUSLINE_FRESH: u64 = 300;

/// Entre deux ecritures de la statusline, en dessous desquelles on ne reecrit
/// pas un cache identique.
///
/// Claude Code rend sa barre plusieurs fois par seconde : sans ce garde-fou, on
/// declencherait autant de `fsync`.
pub const STATUSLINE_WRITE_INTERVAL: u64 = 5;

/// Delai minimal entre deux sondes, toutes politiques confondues.
///
/// Pendant une session active, Claude Code ecrit dans son transcript en
/// permanence. Sans ce plancher, on sonderait a chaque rafraichissement.
pub const MIN_PROBE_INTERVAL: u64 = 60;

/// Delai entre deux lectures d'une machine distante.
///
/// Le cache local sert de tampon : sans ce plancher, chaque battement d'horloge
/// du widget ouvrirait une connexion.
pub const REMOTE_INTERVAL: u64 = 60;

/// D'ou vient le releve affiche.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// On vient d'interroger l'API.
    Probed,
    /// Rejoue depuis le releve d'une autre machine.
    Remote,
    /// Rien n'a bouge : on rejoue le dernier releve, resets appliques.
    Cached,
    /// Aucun releve connu, et la politique en cours interdit de sonder.
    Idle,
}

/// Ce qu'on rend a l'interface.
#[derive(Debug, Clone)]
pub struct Reading {
    /// Le releve, deja recale sur l'heure courante. `None` a froid.
    pub quota: Option<Quota>,
    pub origin: Origin,
    /// La politique qui a decide de sonder, ou non.
    pub policy: Policy,
    /// "pro", "max"... pour l'afficher a cote du nom.
    pub subscription_type: Option<String>,
    /// Avertissement non bloquant : sonde en echec, jeton expire, etc.
    pub note: Option<String>,
}

/// Faut-il envoyer une sonde ?
///
/// Fonction pure, pour pouvoir eprouver la regle sans reseau ni disque.
///
/// `window_closes_at` est la date de reset des 5h du dernier releve : elle dit
/// si une fenetre court encore, donc si sonder est gratuit.
pub fn should_probe(
    policy: Policy,
    force: bool,
    last_probe: Option<u64>,
    last_source: Option<Source>,
    last_activity: Option<u64>,
    window_closes_at: Option<u64>,
    now: u64,
) -> bool {
    // Un geste explicite l'emporte sur tout le reste.
    if force {
        return true;
    }

    // Claude Code nous nourrit gratuitement par sa statusline : sonder donnerait
    // les memes chiffres en depensant une requete. Ceci passe avant les deux
    // politiques, qui ne regissent que le repli.
    if let (Some(Source::Statusline), Some(at)) = (last_source, last_probe) {
        if now.saturating_sub(at) < STATUSLINE_FRESH {
            return false;
        }
    }

    // Plancher anti-rafale, commun aux deux politiques.
    if let Some(probe_at) = last_probe {
        if now.saturating_sub(probe_at) < MIN_PROBE_INTERVAL {
            return false;
        }
    }

    match policy {
        // Toujours juste, quitte a ouvrir une fenetre.
        Policy::Fidele => match last_probe {
            // Au lancement on veut un chiffre tout de suite : c'est bien pour ca
            // que l'utilisateur vient de demarrer le widget.
            None => true,
            Some(probe_at) => now.saturating_sub(probe_at) >= POLL_INTERVAL,
        },

        // Ne sonder que quand c'est gratuit.
        Policy::Prudent => {
            // Claude Code a parle depuis le dernier releve : les compteurs ont
            // forcement bouge, et une fenetre est forcement ouverte.
            if let (Some(probe_at), Some(act)) = (last_probe, last_activity) {
                if act > probe_at {
                    return true;
                }
            }

            match last_probe {
                // A froid : seulement si Claude Code a servi dans les 5 dernieres
                // heures, donc qu'une fenetre est deja ouverte.
                None => matches!(last_activity, Some(act) if now.saturating_sub(act) < FIVE_HOURS),
                // Ensuite : tant que la fenetre connue court, sonder ne coute
                // rien. Une fois fermee, on se tait.
                Some(probe_at) => {
                    let fenetre_ouverte = matches!(window_closes_at, Some(at) if at > now);
                    fenetre_ouverte && now.saturating_sub(probe_at) >= POLL_INTERVAL
                }
            }
        }
    }
}

/// Le point d'entree cote application.
#[derive(Debug, Clone)]
pub struct Meter {
    /// Ou Claude Code ecrit ses transcripts.
    pub projects_dir: PathBuf,
    /// Ou l'on garde le dernier releve.
    pub cache_file: PathBuf,
    /// Ou l'on garde les reglages.
    pub config_file: PathBuf,
    /// Politique imposee pour cet appel, qui court-circuite les reglages.
    pub policy_override: Option<Policy>,
}

impl Meter {
    /// Un compteur cable sur les emplacements standards de la plateforme.
    pub fn new() -> Result<Self, Error> {
        Ok(Meter {
            projects_dir: paths::projects_dir()?,
            cache_file: paths::cache_file()?,
            config_file: paths::config_file()?,
            policy_override: None,
        })
    }

    /// La politique en vigueur.
    ///
    /// Relue a chaque fois, et non gardee en memoire : la bascule du menu ecrit
    /// le fichier, et le tour de boucle suivant doit en tenir compte.
    pub fn policy(&self) -> Policy {
        self.policy_override
            .unwrap_or_else(|| config::load(&self.config_file).policy)
    }

    /// Change la politique et l'ecrit sur le disque.
    ///
    /// On relit le fichier avant d'ecrire : reconstruire un `Config` de zero
    /// effacerait les autres reglages, a commencer par la source des
    /// identifiants, sur un simple clic dans le menu.
    pub fn set_policy(&self, policy: Policy) -> std::io::Result<()> {
        let mut config = config::load(&self.config_file);
        config.policy = policy;
        config::store(&self.config_file, &config)
    }

    /// Enregistre un releve venu de la statusline.
    ///
    /// Appele par `claude-jauge statusline`, donc potentiellement plusieurs fois
    /// par seconde : on evite de reecrire un cache identique et tout frais,
    /// sinon chaque rendu de la barre declencherait un `fsync`.
    pub fn record_statusline(&self, quota: &Quota) -> std::io::Result<()> {
        if let Some(precedent) = cache::load(&self.cache_file) {
            let frais = quota
                .probed_at
                .saturating_sub(precedent.probed_at)
                < STATUSLINE_WRITE_INTERVAL;
            let inchange = precedent.source == Source::Statusline
                && precedent.five_hour.utilization == quota.five_hour.utilization
                && precedent.five_hour.reset_at == quota.five_hour.reset_at
                && precedent.seven_day.utilization == quota.seven_day.utilization;
            if frais && inchange {
                return Ok(());
            }
        }
        cache::store(&self.cache_file, quota)
    }

    /// Va chercher le releve sur une autre machine.
    ///
    /// L'age affiche reste celui du releve d'origine, pas celui de la lecture :
    /// c'est la date qui compte, et faire croire a de la fraicheur parce qu'on
    /// vient de rapatrier un vieux fichier serait un mensonge.
    fn read_remote(
        &self,
        command: &str,
        cached: Option<Quota>,
        force: bool,
        policy: Policy,
        now: u64,
    ) -> Reading {
        // La date du fichier de cache dit quand on a lu pour la derniere fois,
        // ce qui evite d'ajouter un champ au format pour si peu.
        let derniere_lecture = std::fs::metadata(&self.cache_file)
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|instant| instant.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duree| duree.as_secs());

        let tampon_frais = matches!(derniere_lecture, Some(at) if now.saturating_sub(at) < REMOTE_INTERVAL);
        if tampon_frais && !force {
            if let Some(quota) = cached {
                return Reading {
                    quota: Some(quota.settled(now)),
                    origin: Origin::Remote,
                    policy,
                    subscription_type: None,
                    note: None,
                };
            }
        }

        match credentials::run_command(command, std::time::Duration::from_secs(15))
            .map_err(Error::Remote)
            .and_then(|sortie| {
                serde_json::from_str::<Quota>(sortie.trim()).map_err(|err| {
                    Error::Remote(format!(
                        "releve illisible ({err}). La statusline est-elle configuree \
                         la-bas, et le chemin du cache est-il le bon ?"
                    ))
                })
            }) {
            Ok(distant) => {
                let _ = cache::store(&self.cache_file, &distant);
                Reading {
                    quota: Some(distant.settled(now)),
                    origin: Origin::Remote,
                    policy,
                    subscription_type: None,
                    note: None,
                }
            }
            // Machine injoignable : le dernier releve connu vaut mieux qu'un
            // ecran vide, tant qu'on dit clairement ce qui se passe.
            Err(err) => {
                let avait_un_releve = cached.is_some();
                Reading {
                    quota: cached.map(|q| q.settled(now)),
                    origin: if avait_un_releve {
                        Origin::Remote
                    } else {
                        Origin::Idle
                    },
                    policy,
                    subscription_type: None,
                    note: Some(err.to_string()),
                }
            }
        }
    }

    /// Rend l'etat courant des quotas.
    ///
    /// `force` envoie une sonde quoi qu'il arrive : a reserver a une action
    /// explicite de l'utilisateur, puisque cela peut ouvrir une fenetre de 5h.
    pub fn read(&self, force: bool) -> Reading {
        let now = crate::now();
        // Les reglages sont relus a chaque tour : la bascule du menu et une
        // retouche du fichier a la main doivent prendre effet sans redemarrage.
        let config = config::load(&self.config_file);
        let policy = self.policy_override.unwrap_or(config.policy);
        let cached = cache::load(&self.cache_file);
        let last_activity = activity::last_activity(&self.projects_dir);
        let last_probe = cached.as_ref().map(|q| q.probed_at);
        let last_source = cached.as_ref().map(|q| q.source);
        let window_closes_at = cached.as_ref().and_then(|q| q.five_hour.reset_at);

        // Le type d'abonnement se lit en local : son absence n'est pas une
        // raison d'echouer.
        // Lire le releve d'une autre machine court-circuite tout : ni jeton, ni
        // sonde, ni politique. Le cache local ne sert plus que de tampon entre
        // deux lectures.
        if let Some(command) = config.remote.command.clone() {
            return self.read_remote(&command, cached, force, policy, now);
        }

        let creds = credentials::load(&config.credentials);
        let subscription_type = creds
            .as_ref()
            .ok()
            .and_then(|c| c.subscription_type.clone());

        if should_probe(
            policy,
            force,
            last_probe,
            last_source,
            last_activity,
            window_closes_at,
            now,
        ) {
            match creds
                .as_ref()
                .map_err(|err| err.to_string())
                .and_then(|c| probe::probe(c, now).map_err(|err| err.to_string()))
            {
                Ok(fresh) => {
                    // Un cache en echec n'a pas a casser l'affichage.
                    let note = cache::store(&self.cache_file, &fresh)
                        .err()
                        .map(|err| format!("releve non mis en cache : {err}"));
                    return Reading {
                        quota: Some(fresh.settled(now)),
                        origin: Origin::Probed,
                        policy,
                        subscription_type,
                        note,
                    };
                }
                Err(detail) => {
                    // La sonde a echoue : mieux vaut un vieux releve date qu'un
                    // ecran vide.
                    return Reading {
                        quota: cached.map(|q| q.settled(now)),
                        origin: if last_probe.is_some() {
                            Origin::Cached
                        } else {
                            Origin::Idle
                        },
                        policy,
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
                policy,
                subscription_type,
                note: None,
            },
            None => Reading {
                quota: None,
                origin: Origin::Idle,
                policy,
                subscription_type,
                note: Some(
                    "aucun releve : en sonde prudente, Claude Code doit avoir servi \
                     dans les 5 dernieres heures pour qu'une sonde soit gratuite. \
                     Configure `claude-jauge statusline` dans Claude Code pour etre \
                     alimente gratuitement, rafraichis a la main, ou passe en sonde \
                     fidele."
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

    /// Raccourci : sonde prudente, cache issu d'une sonde.
    fn prudent(
        last_probe: Option<u64>,
        last_activity: Option<u64>,
        window_closes_at: Option<u64>,
    ) -> bool {
        should_probe(
            Policy::Prudent,
            false,
            last_probe,
            last_probe.map(|_| Source::Probe),
            last_activity,
            window_closes_at,
            MAINTENANT,
        )
    }

    /// Raccourci : sonde fidele, cache issu d'une sonde.
    fn fidele(last_probe: Option<u64>) -> bool {
        should_probe(
            Policy::Fidele,
            false,
            last_probe,
            last_probe.map(|_| Source::Probe),
            None,
            None,
            MAINTENANT,
        )
    }

    /// Raccourci : cache alimente par la statusline, a l'age donne.
    fn nourri_par_statusline(policy: Policy, age: u64) -> bool {
        should_probe(
            policy,
            false,
            Some(MAINTENANT - age),
            Some(Source::Statusline),
            // Activite et fenetre ouvertes : tout pousserait a sonder, sauf que
            // la statusline rend la sonde inutile.
            Some(MAINTENANT),
            Some(MAINTENANT + 3600),
            MAINTENANT,
        )
    }

    #[test]
    fn force_sonde_toujours_quelle_que_soit_la_politique() {
        for politique in [Policy::Fidele, Policy::Prudent] {
            assert!(should_probe(politique, true, None, None, None, None, MAINTENANT));
            assert!(should_probe(
                politique,
                true,
                Some(MAINTENANT),
                Some(Source::Statusline),
                Some(MAINTENANT),
                None,
                MAINTENANT
            ));
        }
    }

    // --- La statusline prime sur les deux politiques ----------------------

    #[test]
    fn une_statusline_fraiche_dispense_de_sonder() {
        // C'est tout l'interet de la voie officielle : Claude Code donne les
        // memes chiffres gratuitement, donc depenser une requete serait absurde.
        // Vrai meme en sonde fidele, qui sonderait sans cela.
        assert!(!nourri_par_statusline(Policy::Fidele, 0));
        assert!(!nourri_par_statusline(Policy::Fidele, POLL_INTERVAL * 2));
        assert!(!nourri_par_statusline(Policy::Prudent, POLL_INTERVAL * 2));
        assert!(!nourri_par_statusline(Policy::Fidele, STATUSLINE_FRESH - 1));
    }

    #[test]
    fn une_statusline_muette_rend_la_main_a_la_politique() {
        // Claude Code ferme : plus personne ne nourrit le cache, la politique
        // reprend ses droits.
        assert!(nourri_par_statusline(Policy::Fidele, STATUSLINE_FRESH));
        assert!(nourri_par_statusline(Policy::Fidele, STATUSLINE_FRESH + 1));
        // La prudente, elle, ne sonde que parce que la fenetre est ouverte.
        assert!(nourri_par_statusline(Policy::Prudent, STATUSLINE_FRESH + 1));
    }

    // --- Sonde fidele ----------------------------------------------------

    #[test]
    fn fidele_sonde_des_le_lancement_meme_sans_activite() {
        // C'est tout l'interet : l'utilisateur vient de lancer le widget, il veut
        // un chiffre. Et sa consommation peut venir de claude.ai, qui ne laisse
        // aucune trace locale.
        assert!(fidele(None));
    }

    #[test]
    fn fidele_sonde_a_intervalle_regulier() {
        assert!(!fidele(Some(MAINTENANT - POLL_INTERVAL + 1)));
        assert!(fidele(Some(MAINTENANT - POLL_INTERVAL)));
        assert!(fidele(Some(MAINTENANT - 3600)));
    }

    #[test]
    fn fidele_sonde_meme_fenetre_fermee() {
        // La difference de fond avec la politique prudente : on accepte d'ouvrir
        // une fenetre pour ne pas afficher un chiffre faux.
        let fenetre_fermee = Some(MAINTENANT - 1);
        assert!(should_probe(
            Policy::Fidele,
            false,
            Some(MAINTENANT - POLL_INTERVAL),
            Some(Source::Probe),
            None,
            fenetre_fermee,
            MAINTENANT
        ));
    }

    // --- Sonde prudente --------------------------------------------------

    #[test]
    fn prudent_a_froid_sans_activite_ne_sonde_pas() {
        // La regle d'or : ne pas ouvrir de fenetre a la place de l'utilisateur.
        assert!(!prudent(None, None, None));
    }

    #[test]
    fn prudent_a_froid_sonde_si_une_fenetre_est_probablement_ouverte() {
        assert!(prudent(None, Some(MAINTENANT - 600), None));
    }

    #[test]
    fn prudent_a_froid_ne_sonde_pas_si_lactivite_est_trop_vieille() {
        assert!(!prudent(None, Some(MAINTENANT - FIVE_HOURS - 1), None));
        // Pile a la limite, on s'abstient aussi.
        assert!(!prudent(None, Some(MAINTENANT - FIVE_HOURS), None));
    }

    #[test]
    fn prudent_resonde_quand_claude_code_a_parle_depuis_le_dernier_releve() {
        let probe_at = MAINTENANT - 300;
        assert!(prudent(Some(probe_at), Some(probe_at + 10), None));
    }

    #[test]
    fn prudent_sonde_tant_que_la_fenetre_connue_court() {
        // Sonder est gratuit : la fenetre est deja ouverte. C'est ce qui permet
        // de voir la consommation de claude.ai pendant une session en cours.
        let probe_at = MAINTENANT - POLL_INTERVAL;
        let fenetre_ouverte = Some(MAINTENANT + 3600);
        assert!(prudent(Some(probe_at), None, fenetre_ouverte));
    }

    #[test]
    fn prudent_se_tait_des_que_la_fenetre_est_fermee() {
        let probe_at = MAINTENANT - POLL_INTERVAL;
        assert!(!prudent(Some(probe_at), None, Some(MAINTENANT - 1)));
        assert!(!prudent(Some(probe_at), None, Some(MAINTENANT)));
        // Fenetre inconnue : on ne parie pas.
        assert!(!prudent(Some(probe_at), None, None));
    }

    #[test]
    fn prudent_ne_resonde_pas_si_rien_na_bouge() {
        let probe_at = MAINTENANT - 300;
        assert!(!prudent(Some(probe_at), Some(probe_at - 60), None));
        // Activite pile au moment du releve : deja prise en compte.
        assert!(!prudent(Some(probe_at), Some(probe_at), None));
    }

    // --- Commun ----------------------------------------------------------

    #[test]
    fn le_plancher_evite_les_rafales_dans_les_deux_politiques() {
        let tout_frais = Some(MAINTENANT - 5);
        // Fidele : le plancher passe avant l'intervalle.
        assert!(!fidele(tout_frais));
        // Prudent : meme une activite toute fraiche ne force pas la main.
        assert!(!prudent(tout_frais, Some(MAINTENANT), Some(MAINTENANT + 3600)));
    }

    #[test]
    fn une_horloge_qui_recule_ne_fait_pas_paniquer() {
        // Un releve date du futur (NTP qui recale, veille prolongee...) :
        // saturating_sub evite le debordement, et on s'abstient simplement.
        let futur = MAINTENANT + 10_000;
        assert!(!fidele(Some(futur)));
        assert!(!prudent(Some(futur), Some(MAINTENANT), None));
        // Meme du futur, une statusline reste une statusline.
        assert!(!should_probe(
            Policy::Fidele,
            false,
            Some(futur),
            Some(Source::Statusline),
            None,
            None,
            MAINTENANT
        ));
    }
}
