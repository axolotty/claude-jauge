//! `claude-jauge` en ligne de commande.
//!
//! Meme coeur que le widget, mais sans le moindre serveur graphique : c'est la
//! forme qui convient a un serveur headless, a un script ou a une barre tmux.

use chrono::{Datelike, Local, TimeZone};
use claude_jauge_core::quota::Health;
use claude_jauge_core::statusline::Input;
use claude_jauge_core::{
    human_duration, meter::Origin, Meter, Policy, Quota, Reading, Source, Window, WindowKind,
};
use std::io::{self, IsTerminal, Read, Write};
use std::process::ExitCode;
use std::thread::sleep;
use std::time::Duration;

const AIDE: &str = "\
claude-jauge : ou en est la consommation Claude.

USAGE
    claude-jauge [OPTIONS]
    claude-jauge statusline      Se fait alimenter par Claude Code (voir plus bas).

STATUSLINE
    La voie a privilegier : documentee, gratuite, sans jeton ni appel API.
    Ajoute ceci a ~/.claude/settings.json :

        { \"statusLine\": { \"type\": \"command\", \"command\": \"claude-jauge statusline\" } }

    Claude Code lui passe alors les quotas a chaque rendu de sa barre. La
    commande les affiche, et en garde une copie que le widget relit. Tant
    qu'elle alimente le cache, aucune sonde n'est envoyee.

OPTIONS
    -j, --json           Sortie JSON, pour scripter ou alimenter une barre de statut.
    -w, --watch [SECS]   Rafraichit en continu (5 s par defaut). Ctrl+C pour sortir.
    -f, --force          Envoie une sonde meme si rien n'a bouge.
                         Attention : a froid, cela ouvre une fenetre de session de 5h.
    -p, --policy NOM     `fidele` ou `prudent`, pour cet appel seulement.
                         Par defaut : la valeur du fichier de reglages.
        --no-color       N'emet aucune couleur.
    -h, --help           Affiche cette aide.
    -V, --version        Affiche la version.

FONCTIONNEMENT
    Les chiffres valent pour TOUT Claude : claude.ai, l'app de bureau et Claude
    Code partagent le meme pool.

    Quand la statusline ci-dessus est configuree, elle suffit : tant qu'elle
    alimente le cache, rien d'autre ne se declenche. Ce qui suit ne concerne que
    le repli, pour les machines ou Claude Code ne tourne pas.

    Faute de statusline, les quotas sont lus dans les en-tetes que l'API renvoie
    sur chacune de ses reponses. Une sonde est alors une requete, donc elle ouvre
    une fenetre de session de 5h s'il n'y en a pas deja une. D'ou deux
    politiques :

      fidele    Sonde toutes les 2 min. Chiffres toujours justes, y compris
                quand la consommation vient de claude.ai. Peut ouvrir une
                fenetre quand tu ne travailles pas. C'est le defaut.
      prudent   Ne sonde que si c'est gratuit : au lancement, sur activite de
                Claude Code, et tant qu'une fenetre deja ouverte court. N'ouvre
                jamais rien tout seul, mais peut afficher 0 % apres un reset.

    Entre deux sondes, le compte a rebours est recalcule en local, sans reseau.
";

struct Options {
    json: bool,
    force: bool,
    watch: Option<u64>,
    color: bool,
    /// Impose une politique pour cet appel, sans toucher aux reglages.
    policy: Option<Policy>,
}

enum Action {
    Run(Options),
    /// Lit le document de Claude Code sur stdin et rend une ligne de statut.
    Statusline,
    Aide,
    Version,
}

fn parse(args: &[String]) -> Result<Action, String> {
    // Seule sous-commande, et elle doit venir en premier : Claude Code appelle
    // la commande telle quelle, sans rien y ajouter.
    if args.first().map(String::as_str) == Some("statusline") {
        if args.len() > 1 {
            return Err(format!("statusline ne prend pas d'option : {}", args[1]));
        }
        return Ok(Action::Statusline);
    }

    let mut opts = Options {
        json: false,
        force: false,
        watch: None,
        color: io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none(),
        policy: None,
    };

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => return Ok(Action::Aide),
            "-V" | "--version" => return Ok(Action::Version),
            "-j" | "--json" => opts.json = true,
            "-f" | "--force" => opts.force = true,
            "--no-color" => opts.color = false,
            "-p" | "--policy" => {
                let nom = args
                    .get(i + 1)
                    .ok_or("--policy attend `fidele` ou `prudent`")?;
                opts.policy = Some(
                    Policy::parse(nom)
                        .ok_or_else(|| format!("politique inconnue : {nom} (attendu `fidele` ou `prudent`)"))?,
                );
                i += 1;
            }
            "-w" | "--watch" => {
                // La valeur est facultative : `--watch` seul vaut 5 secondes.
                let secs = args
                    .get(i + 1)
                    .and_then(|next| next.parse::<u64>().ok())
                    .map(|secs| {
                        i += 1;
                        secs
                    })
                    .unwrap_or(5);
                if secs == 0 {
                    return Err("--watch demande un intervalle d'au moins 1 seconde".into());
                }
                opts.watch = Some(secs);
            }
            autre => return Err(format!("option inconnue : {autre}")),
        }
        i += 1;
    }

    Ok(Action::Run(opts))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let action = match parse(&args) {
        Ok(action) => action,
        Err(err) => {
            eprintln!("claude-jauge : {err}");
            eprintln!("Essaie `claude-jauge --help`.");
            return ExitCode::FAILURE;
        }
    };

    let opts = match action {
        Action::Aide => {
            print!("{AIDE}");
            return ExitCode::SUCCESS;
        }
        Action::Version => {
            println!("claude-jauge {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Action::Statusline => return statusline(),
        Action::Run(opts) => opts,
    };

    let mut meter = match Meter::new() {
        Ok(meter) => meter,
        Err(err) => {
            eprintln!("claude-jauge : {err}");
            return ExitCode::FAILURE;
        }
    };
    meter.policy_override = opts.policy;

    match opts.watch {
        None => {
            let reading = meter.read(opts.force);
            afficher(&reading, &opts);
        }
        Some(secs) => {
            // Seul le premier tour honore --force : reforcer en boucle
            // reviendrait a sonder toutes les N secondes, exactement ce qu'on
            // cherche a eviter.
            let mut force = opts.force;
            loop {
                let reading = meter.read(force);
                force = false;
                if opts.color {
                    // Efface l'ecran et remonte en haut a gauche.
                    print!("\x1b[2J\x1b[H");
                }
                afficher(&reading, &opts);
                let _ = io::stdout().flush();
                sleep(Duration::from_secs(secs));
            }
        }
    }

    ExitCode::SUCCESS
}

// --- Mode statusline -----------------------------------------------------

/// Lit le document de Claude Code sur stdin, en garde une copie pour le widget,
/// et rend une ligne de statut.
///
/// Cette fonction s'execute dans la barre de quelqu'un qui travaille : elle ne
/// doit jamais crier. Toute anomalie rend une sortie vide et un succes, plutot
/// que de barbouiller son interface d'un message d'erreur a chaque rendu.
fn statusline() -> ExitCode {
    let mut document = String::new();
    if io::stdin().read_to_string(&mut document).is_err() {
        return ExitCode::SUCCESS;
    }

    let input = match claude_jauge_core::statusline::parse(&document) {
        Ok(input) => input,
        Err(_) => return ExitCode::SUCCESS,
    };

    let now = claude_jauge_core::now();
    let quota = input.quota(now);

    // C'est ici que la statusline nourrit le widget. Un cache en echec ne doit
    // pas priver l'utilisateur de sa barre pour autant.
    if let (Some(releve), Ok(meter)) = (quota.as_ref(), Meter::new()) {
        let _ = meter.record_statusline(releve);
    }

    println!("{}", ligne_statut(&input, quota.as_ref(), now));
    ExitCode::SUCCESS
}

fn ligne_statut(input: &Input, quota: Option<&Quota>, now: u64) -> String {
    // Claude Code capture cette sortie : ce n'est jamais un terminal, donc
    // `is_terminal()` dirait non a tort. Les couleurs sont rendues par Claude
    // Code lui-meme, on les emet donc toujours, sauf NO_COLOR.
    let couleur = std::env::var_os("NO_COLOR").is_none();
    let c = |code: &'static str| if couleur { code } else { "" };

    let mut bouts: Vec<String> = Vec::new();

    if let Some(nom) = input.model_name() {
        bouts.push(format!("{}{}{}", c(GRAS), nom, c(RESET)));
    }

    if let Some(ctx) = input.context_used() {
        bouts.push(format!("{}ctx {:.0}%{}", c(GRIS), ctx, c(RESET)));
    }

    if let Some(releve) = quota {
        for fenetre in [&releve.five_hour, &releve.seven_day] {
            let Some(taux) = fenetre.utilization else {
                continue;
            };
            let etiquette = match fenetre.kind {
                WindowKind::FiveHour => "5h",
                WindowKind::SevenDay => "7j",
                WindowKind::Overage => "over",
            };
            let reste = match fenetre.seconds_left(now) {
                Some(secondes) => format!(" {}{}{}", c(GRIS), human_duration(secondes), c(RESET)),
                None => String::new(),
            };
            bouts.push(format!(
                "{etiquette} {}{}{} {}{taux:.0}%{}{reste}",
                c(teinte(fenetre.utilization)),
                barre(fenetre.utilization, 5),
                c(RESET),
                c(teinte(fenetre.utilization)),
                c(RESET),
            ));
        }
    }

    bouts.join(&format!("{} · {}", c(GRIS), c(RESET)))
}

fn afficher(reading: &Reading, opts: &Options) {
    if opts.json {
        println!("{}", en_json(reading));
    } else {
        print!("{}", en_texte(reading, opts.color));
    }
}

// --- Sortie JSON ---------------------------------------------------------

fn en_json(reading: &Reading) -> String {
    let now = claude_jauge_core::now();

    let fenetre = |w: &Window| {
        serde_json::json!({
            "label": w.kind.label(),
            "used_percent": w.utilization,
            "remaining_percent": w.remaining(),
            "reset_at": w.reset_at,
            "seconds_left": w.seconds_left(now),
            // `status` est le brut de l'API, `health` notre lecture des trois
            // etats. Les deux sont exposes : un script peut vouloir l'original.
            "status": w.status,
            "health": match w.health() {
                Health::Allowed => "allowed",
                Health::Warning => "warning",
                Health::Blocked => "blocked",
                Health::Unknown => "unknown",
            },
            "blocked": w.is_blocked(),
        })
    };

    let quota = reading.quota.as_ref().map(|q| {
        serde_json::json!({
            "five_hour": fenetre(&q.five_hour),
            "seven_day": fenetre(&q.seven_day),
            "overage": fenetre(&q.overage),
            "overall_status": q.overall_status,
            "fallback": q.fallback,
            "fallback_percent": q.fallback_percentage,
            "representative_claim": q.representative_claim,
            "probed_at": q.probed_at,
            "age_seconds": q.age(now),
            "source": match q.source {
                Source::Statusline => "statusline",
                Source::Probe => "probe",
            },
        })
    });

    let origine = match reading.origin {
        Origin::Probed => "probed",
        Origin::Cached => "cached",
        Origin::Idle => "idle",
    };

    serde_json::json!({
        "origin": origine,
        "policy": match reading.policy {
            Policy::Fidele => "fidele",
            Policy::Prudent => "prudent",
        },
        "subscription_type": reading.subscription_type,
        "note": reading.note,
        "quota": quota,
        "now": now,
    })
    .to_string()
}

// --- Sortie texte --------------------------------------------------------

const VERT: &str = "\x1b[32m";
const AMBRE: &str = "\x1b[33m";
const ROUGE: &str = "\x1b[31m";
const GRIS: &str = "\x1b[90m";
const GRAS: &str = "\x1b[1m";
const RESET: &str = "\x1b[0m";

/// Le seuil a partir duquel une jauge passe a l'ambre, puis au rouge.
const SEUIL_AMBRE: f64 = 50.0;
const SEUIL_ROUGE: f64 = 80.0;

fn teinte(utilization: Option<f64>) -> &'static str {
    match utilization {
        None => GRIS,
        Some(u) if u >= SEUIL_ROUGE => ROUGE,
        Some(u) if u >= SEUIL_AMBRE => AMBRE,
        Some(_) => VERT,
    }
}

fn barre(utilization: Option<f64>, largeur: usize) -> String {
    match utilization {
        None => "?".repeat(largeur),
        Some(u) => {
            let pleins = ((u / 100.0) * largeur as f64).round() as usize;
            let pleins = pleins.min(largeur);
            format!("{}{}", "▓".repeat(pleins), "░".repeat(largeur - pleins))
        }
    }
}

fn heure_locale(epoch: u64) -> String {
    match Local.timestamp_opt(epoch as i64, 0).single() {
        // Aujourd'hui : l'heure suffit. Sinon on rappelle le jour.
        Some(dt) if dt.date_naive() == Local::now().date_naive() => dt.format("%H:%M").to_string(),
        Some(dt) => {
            // chrono ne traduit pas les noms de jours sans sa feature instable
            // `unstable-locales`. Sept mots ne valent pas cette dependance.
            const JOURS: [&str; 7] = ["lun", "mar", "mer", "jeu", "ven", "sam", "dim"];
            let jour = JOURS[dt.weekday().num_days_from_monday() as usize];
            format!("{jour} {}", dt.format("%d/%m %H:%M"))
        }
        None => "?".to_string(),
    }
}

fn en_texte(reading: &Reading, couleur: bool) -> String {
    let c = |code: &'static str| if couleur { code } else { "" };
    let now = claude_jauge_core::now();
    let mut out = String::new();

    let plan = reading
        .subscription_type
        .as_deref()
        .map(|p| format!(" · {p}"))
        .unwrap_or_default();
    out.push_str(&format!("\n  {}Claude{}{}\n\n", c(GRAS), plan, c(RESET)));

    let quota = match &reading.quota {
        Some(quota) => quota,
        None => {
            out.push_str(&format!("  {}aucun releve disponible{}\n", c(GRIS), c(RESET)));
            if let Some(note) = &reading.note {
                out.push_str(&format!("  {}{}{}\n", c(GRIS), note, c(RESET)));
            }
            out.push_str(&format!(
                "\n  {}Lance `claude-jauge --force` pour sonder quand meme.{}\n\n",
                c(GRIS),
                c(RESET)
            ));
            return out;
        }
    };

    for w in quota.windows() {
        // Une fenetre dont on ne sait rien n'apprend rien. La statusline ne donne
        // pas l'overage : mieux vaut taire la ligne qu'aligner des points
        // d'interrogation.
        if w.utilization.is_none() && w.reset_at.is_none() {
            continue;
        }
        out.push_str(&ligne(w, now, couleur));
    }

    out.push('\n');
    out.push_str(&pied(reading, quota, now, couleur));
    out.push('\n');
    out
}

fn ligne(w: &Window, now: u64, couleur: bool) -> String {
    let c = |code: &'static str| if couleur { code } else { "" };

    let pourcent = match w.utilization {
        Some(u) => format!("{u:>3.0}%"),
        None => "  ?".to_string(),
    };

    let reste = match w.remaining() {
        Some(r) => format!("reste {r:>3.0}%"),
        None => String::new(),
    };

    let reset = match (w.reset_at, w.seconds_left(now)) {
        (Some(at), Some(left)) => format!(
            "reset {} ({})",
            heure_locale(at),
            human_duration(left)
        ),
        // Une fenetre sans date de reset connue n'a simplement pas encore
        // ete ouverte : c'est le cas juste apres une remise a zero.
        _ => format!("{}fenetre non ouverte{}", c(GRIS), c(RESET)),
    };

    let alerte = match w.health() {
        Health::Blocked => format!("  {}BLOQUE{}", c(ROUGE), c(RESET)),
        Health::Warning => format!("  {}limite proche{}", c(AMBRE), c(RESET)),
        Health::Allowed | Health::Unknown => String::new(),
    };

    format!(
        "  {:<11} {}{}{}  {}  {:<11}  {}{}\n",
        w.kind.label(),
        c(teinte(w.utilization)),
        barre(w.utilization, 10),
        c(RESET),
        pourcent,
        reste,
        reset,
        alerte
    )
}

fn pied(reading: &Reading, quota: &Quota, now: u64, couleur: bool) -> String {
    let c = |code: &'static str| if couleur { code } else { "" };
    let mut bouts = Vec::new();

    let age = quota.age(now);
    bouts.push(match reading.origin {
        Origin::Probed => "releve a l'instant".to_string(),
        Origin::Cached if age < 60 => "releve a l'instant".to_string(),
        Origin::Cached => format!("releve il y a {}", human_duration(age)),
        Origin::Idle => "aucun releve".to_string(),
    });

    if let Some(seuil) = quota.fallback_percentage {
        if quota.fallback.as_deref() == Some("available") {
            bouts.push(format!("repli dispo a {seuil:.0}%"));
        }
    }

    // Quand Claude Code nous alimente, la politique de sonde ne joue aucun role :
    // l'annoncer laisserait croire qu'une requete est partie.
    bouts.push(match quota.source {
        Source::Statusline => "via statusline".to_string(),
        Source::Probe => reading.policy.label().to_lowercase(),
    });

    let mut out = format!("  {}{}{}\n", c(GRIS), bouts.join(" · "), c(RESET));
    if let Some(note) = &reading.note {
        out.push_str(&format!("  {}{}{}\n", c(AMBRE), note, c(RESET)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_barre_se_remplit_proportionnellement() {
        assert_eq!(barre(Some(0.0), 10), "░░░░░░░░░░");
        assert_eq!(barre(Some(50.0), 10), "▓▓▓▓▓░░░░░");
        assert_eq!(barre(Some(100.0), 10), "▓▓▓▓▓▓▓▓▓▓");
        // 73 % de 10 cases arrondit a 7.
        assert_eq!(barre(Some(73.0), 10), "▓▓▓▓▓▓▓░░░");
        // Une valeur inconnue se voit tout de suite.
        assert_eq!(barre(None, 4), "????");
    }

    #[test]
    fn la_barre_ne_deborde_jamais() {
        assert_eq!(barre(Some(150.0), 5).chars().count(), 5);
        assert_eq!(barre(Some(-10.0), 5).chars().count(), 5);
    }

    #[test]
    fn la_teinte_suit_les_seuils() {
        assert_eq!(teinte(Some(0.0)), VERT);
        assert_eq!(teinte(Some(49.9)), VERT);
        assert_eq!(teinte(Some(50.0)), AMBRE);
        assert_eq!(teinte(Some(79.9)), AMBRE);
        assert_eq!(teinte(Some(80.0)), ROUGE);
        assert_eq!(teinte(None), GRIS);
    }

    #[test]
    fn watch_accepte_une_valeur_facultative() {
        let sans = vec!["--watch".to_string()];
        assert!(matches!(parse(&sans), Ok(Action::Run(o)) if o.watch == Some(5)));

        let avec = vec!["--watch".to_string(), "10".to_string()];
        assert!(matches!(parse(&avec), Ok(Action::Run(o)) if o.watch == Some(10)));

        let zero = vec!["--watch".to_string(), "0".to_string()];
        assert!(parse(&zero).is_err());
    }

    #[test]
    fn refuse_une_option_inconnue() {
        let args = vec!["--nawak".to_string()];
        assert!(parse(&args).is_err());
    }

    #[test]
    fn policy_accepte_les_deux_politiques() {
        let fidele = vec!["--policy".to_string(), "fidele".to_string()];
        assert!(matches!(parse(&fidele), Ok(Action::Run(o)) if o.policy == Some(Policy::Fidele)));

        let prudent = vec!["-p".to_string(), "prudent".to_string()];
        assert!(matches!(parse(&prudent), Ok(Action::Run(o)) if o.policy == Some(Policy::Prudent)));
    }

    #[test]
    fn policy_refuse_une_valeur_inconnue_ou_absente() {
        assert!(parse(&["--policy".to_string(), "nawak".to_string()]).is_err());
        assert!(parse(&["--policy".to_string()]).is_err());
    }

    #[test]
    fn sans_policy_on_laisse_les_reglages_decider() {
        assert!(matches!(parse(&[]), Ok(Action::Run(o)) if o.policy.is_none()));
    }

    #[test]
    fn les_drapeaux_se_combinent() {
        let args = vec!["--json".to_string(), "--force".to_string(), "--no-color".to_string()];
        match parse(&args) {
            Ok(Action::Run(o)) => {
                assert!(o.json && o.force && !o.color);
            }
            _ => panic!("attendu des options"),
        }
    }
}
