// Sous Windows, empeche une console noire de s'ouvrir derriere le widget.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod icon;

use claude_jauge_core::quota::Health;
use claude_jauge_core::{human_duration, meter::Origin, Meter, Policy, Window};
use std::time::Duration;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, Wry};

const TRAY_ID: &str = "claude-jauge";

/// Cadence de rafraichissement de l'affichage.
///
/// Ce n'est PAS une cadence de sonde : la plupart de ces tours ne font que
/// recalculer un compte a rebours en local. C'est `should_probe` dans le coeur
/// qui decide si le reseau est sollicite, et il s'en abstient tant que Claude
/// Code n'a rien fait.
const TICK: Duration = Duration::from_secs(20);

// --- Ce qu'on envoie a l'interface ---------------------------------------

#[derive(serde::Serialize, Clone)]
struct WindowView {
    key: String,
    label: String,
    used: Option<f64>,
    remaining: Option<f64>,
    reset_at: Option<u64>,
    seconds_left: Option<u64>,
    health: String,
}

#[derive(serde::Serialize, Clone)]
struct Snapshot {
    windows: Vec<WindowView>,
    plan: Option<String>,
    origin: String,
    /// "statusline" ou "probe" : d'ou viennent les chiffres affiches.
    source: Option<String>,
    policy: String,
    policy_hint: String,
    note: Option<String>,
    probed_at: Option<u64>,
    age: Option<u64>,
    fallback_percent: Option<f64>,
    now: u64,
}

/// Les deux entrees cochables du menu, gardees pour pouvoir bouger les coches
/// quand la politique change.
struct MenuPolitique {
    fidele: CheckMenuItem<Wry>,
    prudent: CheckMenuItem<Wry>,
}

impl MenuPolitique {
    /// Aligne les coches sur la politique en vigueur.
    fn refleter(&self, policy: Policy) {
        let _ = self.fidele.set_checked(policy == Policy::Fidele);
        let _ = self.prudent.set_checked(policy == Policy::Prudent);
    }
}

fn view(w: &Window, now: u64) -> WindowView {
    WindowView {
        key: w.kind.slug().to_string(),
        label: w.kind.label().to_string(),
        used: w.utilization,
        remaining: w.remaining(),
        reset_at: w.reset_at,
        seconds_left: w.seconds_left(now),
        health: match w.health() {
            Health::Allowed => "allowed",
            Health::Warning => "warning",
            Health::Blocked => "blocked",
            Health::Unknown => "unknown",
        }
        .to_string(),
    }
}

fn snapshot(meter: &Meter, force: bool) -> Snapshot {
    let reading = meter.read(force);
    let now = claude_jauge_core::now();

    // Une fenetre dont on ne sait rien n'a rien a montrer : la statusline ne
    // donne pas l'overage, et une jauge vide a cote de deux jauges pleines
    // ressemble a un bug.
    let windows = reading
        .quota
        .as_ref()
        .map(|q| {
            q.windows()
                .iter()
                .filter(|w| w.utilization.is_some() || w.reset_at.is_some())
                .map(|w| view(w, now))
                .collect()
        })
        .unwrap_or_default();

    Snapshot {
        windows,
        plan: reading.subscription_type.clone(),
        origin: match reading.origin {
            Origin::Probed => "probed",
            Origin::Remote => "remote",
            Origin::Cached => "cached",
            Origin::Idle => "idle",
        }
        .to_string(),
        source: reading.quota.as_ref().map(|q| {
            match q.source {
                claude_jauge_core::Source::Statusline => "statusline",
                claude_jauge_core::Source::Probe => "probe",
            }
            .to_string()
        }),
        policy: match reading.policy {
            Policy::Fidele => "fidele",
            Policy::Prudent => "prudent",
        }
        .to_string(),
        policy_hint: reading.policy.hint().to_string(),
        note: reading.note.clone(),
        probed_at: reading.quota.as_ref().map(|q| q.probed_at),
        age: reading.quota.as_ref().map(|q| q.age(now)),
        fallback_percent: reading.quota.as_ref().and_then(|q| q.fallback_percentage),
        now,
    }
}

/// Le texte affiche a cote de l'icone. La barre de menus macOS le montre tel
/// quel ; ailleurs il sert d'infobulle.
///
/// Un widget muet est un widget casse : quand il n'y a rien a montrer, le titre
/// doit dire pourquoi, pas afficher un « ? » enigmatique. Le detail complet
/// attend dans la popover.
fn titre(snap: &Snapshot) -> String {
    match snap.windows.iter().find(|w| w.key == "5h") {
        Some(w) => match (w.used, w.seconds_left) {
            (Some(used), Some(left)) => format!("{:.0}% · {}", used, human_duration(left)),
            (Some(used), None) => format!("{used:.0}%"),
            _ => "-".to_string(),
        },
        // Pas de releve du tout : soit la sonde a echoue (jeton, reseau), soit
        // la politique prudente s'abstient. Les deux cas se disent.
        None if snap.note.is_some() => "!".to_string(),
        None => "...".to_string(),
    }
}

/// Le taux de la fenetre de 5h, celui que l'icone represente.
fn taux_5h(snap: &Snapshot) -> Option<f64> {
    snap.windows.iter().find(|w| w.key == "5h").and_then(|w| w.used)
}

/// Repercute un releve sur l'icone, le titre et l'interface.
fn pousser(app: &AppHandle, snap: &Snapshot) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_icon(Some(icon::gauge(taux_5h(snap))));
        let _ = tray.set_title(Some(titre(snap)));
        let _ = tray.set_tooltip(Some(&infobulle(snap)));
    }
    // La fenetre peut etre fermee : l'evenement se perd alors sans dommage,
    // l'interface redemande un instantane a son ouverture.
    let _ = app.emit("snapshot", snap);
}

fn infobulle(snap: &Snapshot) -> String {
    snap.windows
        .iter()
        .map(|w| match w.used {
            Some(used) => format!("{} : {:.0} %", w.label, used),
            None => format!("{} : ?", w.label),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Applique une nouvelle politique : disque, coches du menu, releve immediat.
fn changer_politique(app: &AppHandle, policy: Policy) {
    let meter = app.state::<Meter>();
    if let Err(err) = meter.set_policy(policy) {
        eprintln!("claude-jauge : reglage non enregistre : {err}");
    }
    app.state::<MenuPolitique>().refleter(policy);
    // Passer en sonde fidele doit donner un chiffre tout de suite, sans attendre
    // le prochain tour de boucle.
    let snap = snapshot(&meter, false);
    pousser(app, &snap);
}

/// Montre ou cache la popover.
fn basculer(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    if window.is_visible().unwrap_or(false) {
        let _ = window.hide();
    } else {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

// --- Commandes appelees depuis Svelte ------------------------------------

/// L'etat courant, sans forcer de sonde.
#[tauri::command]
fn current(meter: State<'_, Meter>) -> Snapshot {
    snapshot(&meter, false)
}

/// Sonde explicitement, a la demande de l'utilisateur.
///
/// C'est le seul chemin qui peut ouvrir une fenetre de session : il faut une
/// action deliberee, jamais une minuterie.
#[tauri::command]
fn refresh(app: AppHandle, meter: State<'_, Meter>) -> Snapshot {
    let snap = snapshot(&meter, true);
    pousser(&app, &snap);
    snap
}

#[tauri::command]
fn hide(app: AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
}

fn main() {
    tauri::Builder::default()
        // Doit venir en PREMIER : le plugin coupe court avant que le reste ne
        // s'initialise, sinon une seconde instance a le temps de poser sa propre
        // icone dans la barre. C'est ce qui produisait les icones fantomes a
        // chaque `tauri dev` relance.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _dossier| {
            // On ne rend pas la main en silence : l'utilisateur a relance
            // l'application, il attend quelque chose. On lui montre celle qui
            // tourne deja.
            if let Some(fenetre) = app.get_webview_window("main") {
                let _ = fenetre.show();
                let _ = fenetre.set_focus();
            }
        }))
        .plugin(tauri_plugin_positioner::init())
        .invoke_handler(tauri::generate_handler![current, refresh, hide])
        .setup(|app| {
            // Un widget de barre de menus n'a rien a faire dans le Dock.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let meter = Meter::new()?;
            let policy = meter.policy();
            app.manage(meter);

            let rafraichir =
                MenuItem::with_id(app, "refresh", "Rafraichir maintenant", true, None::<&str>)?;
            let fidele = CheckMenuItem::with_id(
                app,
                "policy_fidele",
                Policy::Fidele.label(),
                true,
                policy == Policy::Fidele,
                None::<&str>,
            )?;
            let prudent = CheckMenuItem::with_id(
                app,
                "policy_prudent",
                Policy::Prudent.label(),
                true,
                policy == Policy::Prudent,
                None::<&str>,
            )?;
            let quitter = MenuItem::with_id(app, "quit", "Quitter", true, None::<&str>)?;
            let sep_haut = PredefinedMenuItem::separator(app)?;
            let sep_bas = PredefinedMenuItem::separator(app)?;

            let menu = Menu::with_items(
                app,
                &[&rafraichir, &sep_haut, &fidele, &prudent, &sep_bas, &quitter],
            )?;
            app.manage(MenuPolitique { fidele, prudent });

            TrayIconBuilder::with_id(TRAY_ID)
                .icon(icon::gauge(None))
                .menu(&menu)
                // Le clic gauche ouvre la popover ; le menu reste sur le clic droit.
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "refresh" => {
                        let snap = snapshot(&app.state::<Meter>(), true);
                        pousser(app, &snap);
                    }
                    "policy_fidele" => changer_politique(app, Policy::Fidele),
                    "policy_prudent" => changer_politique(app, Policy::Prudent),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    tauri_plugin_positioner::on_tray_event(tray.app_handle(), &event);
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        basculer(tray.app_handle());
                    }
                })
                .build(app)?;

            // Premier releve tout de suite, puis a la cadence d'affichage.
            let handle = app.handle().clone();
            std::thread::spawn(move || loop {
                let snap = snapshot(&handle.state::<Meter>(), false);
                pousser(&handle, &snap);
                std::thread::sleep(TICK);
            });

            Ok(())
        })
        .on_window_event(|window, event| {
            // Fermer la popover ne doit pas quitter l'application : le widget
            // vit dans la barre, pas dans une fenetre.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("construction de l'application")
        .run(|_app, event| {
            if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
                // `code` distingue les deux raisons de partir, et tout est la :
                //
                //   None       la derniere fenetre s'est fermee. Un widget de
                //              barre de menus n'a aucune raison de mourir pour
                //              ca : on retient l'application.
                //   Some(_)    quelqu'un a appele `app.exit(n)`, donc « Quitter ».
                //              C'est une volonte explicite, on la respecte.
                //
                // Retenir les deux rendait l'application inquittable : le menu
                // appelait bien exit(0), et on l'annulait aussitot.
                if code.is_none() {
                    api.prevent_exit();
                }
            }
        });
}
