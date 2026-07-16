// Sous Windows, empeche une console noire de s'ouvrir derriere le widget.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod icon;

use claude_jauge_core::quota::Health;
use claude_jauge_core::{human_duration, meter::Origin, Meter, Window};
use std::time::Duration;
use tauri::menu::MenuBuilder;
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State};

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
    note: Option<String>,
    probed_at: Option<u64>,
    age: Option<u64>,
    fallback_percent: Option<f64>,
    now: u64,
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

    let windows = reading
        .quota
        .as_ref()
        .map(|q| q.windows().iter().map(|w| view(w, now)).collect())
        .unwrap_or_default();

    Snapshot {
        windows,
        plan: reading.subscription_type.clone(),
        origin: match reading.origin {
            Origin::Probed => "probed",
            Origin::Cached => "cached",
            Origin::Idle => "idle",
        }
        .to_string(),
        note: reading.note.clone(),
        probed_at: reading.quota.as_ref().map(|q| q.probed_at),
        age: reading.quota.as_ref().map(|q| q.age(now)),
        fallback_percent: reading.quota.as_ref().and_then(|q| q.fallback_percentage),
        now,
    }
}

/// Le texte affiche a cote de l'icone. La barre de menus macOS le montre tel
/// quel ; ailleurs il sert d'infobulle.
fn titre(snap: &Snapshot) -> String {
    match snap.windows.iter().find(|w| w.key == "5h") {
        Some(w) => match (w.used, w.seconds_left) {
            (Some(used), Some(left)) => format!("{:.0}% · {}", used, human_duration(left)),
            (Some(used), None) => format!("{used:.0}%"),
            _ => "?".to_string(),
        },
        None => "?".to_string(),
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
        .plugin(tauri_plugin_positioner::init())
        .invoke_handler(tauri::generate_handler![current, refresh, hide])
        .setup(|app| {
            // Un widget de barre de menus n'a rien a faire dans le Dock.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            app.manage(Meter::new()?);

            let menu = MenuBuilder::new(app)
                .text("refresh", "Rafraichir maintenant")
                .separator()
                .text("quit", "Quitter")
                .build()?;

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
            // Sans ca, macOS quitterait l'app a la fermeture de la derniere fenetre.
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                api.prevent_exit();
            }
        });
}
