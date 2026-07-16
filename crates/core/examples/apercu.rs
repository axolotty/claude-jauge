//! Dessine l'icone a plusieurs taux et ecrit les tampons RGBA bruts.
//!
//! Sert a regarder le rendu sans lancer l'app Tauri, donc sans serveur
//! graphique. Le RGBA brut se convertit ensuite en PNG avec n'importe quel
//! outil, par exemple :
//!
//! ```sh
//! cargo run --example apercu -- /tmp/apercu
//! python3 -c "from PIL import Image; Image.frombytes('RGBA', (64, 64), open('/tmp/apercu-73.rgba','rb').read()).save('/tmp/apercu-73.png')"
//! ```

use claude_jauge_core::icon;

fn main() {
    let prefixe = std::env::args().nth(1).unwrap_or_else(|| "/tmp/apercu".into());

    for taux in [Some(0.0), Some(25.0), Some(50.0), Some(73.0), Some(96.0), Some(100.0), None] {
        let (rgba, size) = icon::gauge_rgba(taux);
        let nom = match taux {
            Some(t) => format!("{prefixe}-{t:.0}.rgba"),
            None => format!("{prefixe}-inconnu.rgba"),
        };
        std::fs::write(&nom, &rgba).expect("ecriture du tampon");
        println!("{nom} ({size}x{size})");
    }
}
