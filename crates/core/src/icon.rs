//! Dessin de l'icone de la barre : un anneau qui se remplit.
//!
//! Tout est fait a la main dans un tampon RGBA, parce que `tauri::image::Image`
//! en accepte un tel quel. Ca evite d'embarquer une bibliotheque de rendu
//! entiere pour tracer un seul anneau.
//!
//! Ce module vit dans le coeur, et non dans l'app Tauri, pour une raison
//! pratique : le coeur se compile partout, donc le dessin reste testable sur une
//! machine sans serveur graphique. L'app la-bas se contente d'emballer le
//! tampon rendu ici dans une `Image`.

/// Cote de l'icone en pixels. Genereux a dessein : les barres de menus des
/// ecrans haute densite reduisent l'image, et reduire est propre, alors
/// qu'agrandir ne l'est pas.
pub const SIZE: u32 = 64;

/// Rayons de l'anneau, en fraction du demi-cote.
const OUTER: f64 = 0.92;
const INNER: f64 = 0.60;

/// Couleurs, alignees sur celles du CLI et de l'interface Svelte.
pub const VERT: (u8, u8, u8) = (63, 185, 80);
pub const AMBRE: (u8, u8, u8) = (210, 153, 34);
pub const ROUGE: (u8, u8, u8) = (248, 81, 73);
pub const GRIS: (u8, u8, u8) = (110, 118, 129);

/// La couleur qui correspond a un taux d'utilisation.
pub fn teinte(utilization: Option<f64>) -> (u8, u8, u8) {
    match utilization {
        None => GRIS,
        Some(u) if u >= 80.0 => ROUGE,
        Some(u) if u >= 50.0 => AMBRE,
        Some(_) => VERT,
    }
}

/// Fabrique le tampon RGBA de l'icone pour un taux d'utilisation donne.
///
/// `None` dessine un anneau gris vide : on ne sait pas, et ca se voit.
/// Rend le tampon et son cote, prets pour `Image::new_owned`.
pub fn gauge_rgba(utilization: Option<f64>) -> (Vec<u8>, u32) {
    (draw(utilization, teinte(utilization), SIZE), SIZE)
}

/// Couverture d'un pixel par l'anneau, entre 0 et 1.
///
/// Le degrade sur un demi-pixel de part et d'autre des deux bords tient lieu
/// d'anticrenelage : sans lui, l'anneau aurait des marches d'escalier.
fn ring_coverage(r: f64, inner: f64, outer: f64) -> f64 {
    let bord = 0.5;
    let dehors = ((outer - r) / bord).clamp(0.0, 1.0);
    let dedans = ((r - inner) / bord).clamp(0.0, 1.0);
    dehors.min(dedans)
}

fn draw(utilization: Option<f64>, couleur: (u8, u8, u8), size: u32) -> Vec<u8> {
    let mut buf = vec![0u8; (size as usize) * (size as usize) * 4];

    let centre = size as f64 / 2.0;
    let outer = centre * OUTER;
    let inner = centre * INNER;
    let part = utilization.map(|u| (u / 100.0).clamp(0.0, 1.0)).unwrap_or(0.0);

    for y in 0..size {
        for x in 0..size {
            let dx = x as f64 + 0.5 - centre;
            let dy = y as f64 + 0.5 - centre;
            let r = (dx * dx + dy * dy).sqrt();

            let couverture = ring_coverage(r, inner, outer);
            if couverture <= 0.0 {
                continue;
            }

            // Angle mesure depuis le haut, dans le sens horaire. En coordonnees
            // ecran l'axe y descend, donc atan2(dy, dx) tourne deja dans le bon
            // sens ; le quart de tour ne fait que ramener l'origine a midi.
            let angle =
                (dy.atan2(dx) + std::f64::consts::FRAC_PI_2).rem_euclid(std::f64::consts::TAU);
            let position = angle / std::f64::consts::TAU;

            let (rgb, alpha) = if position <= part {
                (couleur, 1.0)
            } else {
                // La portion non consommee reste visible, en retrait.
                (GRIS, 0.28)
            };

            let i = ((y * size + x) * 4) as usize;
            buf[i] = rgb.0;
            buf[i + 1] = rgb.1;
            buf[i + 2] = rgb.2;
            buf[i + 3] = (alpha * couverture * 255.0).round() as u8;
        }
    }

    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Alpha du pixel (x, y) dans un tampon carre.
    fn alpha(buf: &[u8], size: u32, x: u32, y: u32) -> u8 {
        buf[(((y * size + x) * 4) + 3) as usize]
    }

    /// Couleur du pixel (x, y).
    fn rgb(buf: &[u8], size: u32, x: u32, y: u32) -> (u8, u8, u8) {
        let i = ((y * size + x) * 4) as usize;
        (buf[i], buf[i + 1], buf[i + 2])
    }

    #[test]
    fn le_tampon_a_la_bonne_taille() {
        let buf = draw(Some(50.0), VERT, 64);
        assert_eq!(buf.len(), 64 * 64 * 4);
    }

    #[test]
    fn le_centre_est_transparent() {
        // C'est un anneau, pas un disque : le trou doit rester vide.
        let buf = draw(Some(100.0), VERT, 64);
        assert_eq!(alpha(&buf, 64, 32, 32), 0);
    }

    #[test]
    fn les_coins_sont_transparents() {
        let buf = draw(Some(100.0), VERT, 64);
        assert_eq!(alpha(&buf, 64, 0, 0), 0);
        assert_eq!(alpha(&buf, 64, 63, 63), 0);
    }

    #[test]
    fn le_remplissage_part_du_haut_dans_le_sens_horaire() {
        // A 25 %, l'arc va de midi a 3 heures.
        let buf = draw(Some(25.0), ROUGE, 64);
        // Vers 1h30, en plein dans le quart rempli. On evite volontairement le
        // pixel de 3 heures pile : il tombe sur la frontiere meme de l'arc, ou
        // le demi-pixel de decalage suffit a le faire basculer d'un cote ou de
        // l'autre. Un test ne doit pas se poser sur une arete.
        assert_eq!(rgb(&buf, 64, 52, 12), ROUGE);
        // Vers 9 heures : largement au-dela de l'arc, donc en retrait.
        assert_eq!(rgb(&buf, 64, 3, 32), GRIS);
        // Et juste avant midi, donc en toute fin d'anneau : pas encore atteint.
        assert_eq!(rgb(&buf, 64, 12, 12), GRIS);
    }

    #[test]
    fn a_zero_pour_cent_rien_nest_colore() {
        let buf = draw(Some(0.0), VERT, 64);
        assert_eq!(rgb(&buf, 64, 60, 32), GRIS);
        assert_eq!(rgb(&buf, 64, 3, 32), GRIS);
        // L'anneau reste dessine, en retrait : la jauge doit rester lisible.
        assert!(alpha(&buf, 64, 60, 32) > 0);
    }

    #[test]
    fn un_taux_inconnu_donne_un_anneau_gris() {
        assert_eq!(teinte(None), GRIS);
        let buf = draw(None, teinte(None), 64);
        assert_eq!(rgb(&buf, 64, 60, 32), GRIS);
    }

    #[test]
    fn la_teinte_suit_les_memes_seuils_que_le_cli() {
        assert_eq!(teinte(Some(49.9)), VERT);
        assert_eq!(teinte(Some(50.0)), AMBRE);
        assert_eq!(teinte(Some(79.9)), AMBRE);
        assert_eq!(teinte(Some(80.0)), ROUGE);
    }
}
