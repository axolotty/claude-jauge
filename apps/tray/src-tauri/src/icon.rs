//! Emballage de l'icone dessinee par le coeur dans une `Image` Tauri.
//!
//! Le dessin lui-meme vit dans `claude_jauge_core::icon`, pour rester testable
//! sur une machine sans serveur graphique. Ici on ne fait que le presenter a
//! Tauri.

use tauri::image::Image;

/// L'icone de la barre, pour un taux d'utilisation donne.
pub fn gauge(utilization: Option<f64>) -> Image<'static> {
    let (rgba, size) = claude_jauge_core::icon::gauge_rgba(utilization);
    Image::new_owned(rgba, size, size)
}
