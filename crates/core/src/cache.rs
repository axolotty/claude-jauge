//! Persistance du dernier releve, d'un lancement a l'autre.

use crate::quota::Quota;
use std::fs;
use std::io::Write;
use std::path::Path;

/// Lit le dernier releve connu.
///
/// Toute anomalie rend `None` : un cache abime ne doit jamais empecher le
/// programme de demarrer, il sera simplement reecrit au prochain releve.
pub fn load(path: &Path) -> Option<Quota> {
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Ecrit le releve de facon atomique.
///
/// Le detour par un fichier temporaire suivi d'un rename n'est pas du luxe : le
/// widget et un appel CLI peuvent tourner en meme temps, et une ecriture
/// directe laisserait un JSON tronque derriere elle si les deux se croisent.
pub fn store(path: &Path, quota: &Quota) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let encoded = serde_json::to_vec_pretty(quota)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;

    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(&encoded)?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quota::{Source, Window, WindowKind};
    use std::path::PathBuf;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(nom: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("claude-jauge-cache-{nom}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("dossier temporaire");
            TempDir(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn un_releve() -> Quota {
        Quota {
            five_hour: Window {
                kind: WindowKind::FiveHour,
                utilization: Some(73.0),
                reset_at: Some(1784225400),
                status: Some("allowed".into()),
            },
            seven_day: Window {
                kind: WindowKind::SevenDay,
                utilization: Some(14.0),
                reset_at: Some(1784444400),
                status: Some("allowed".into()),
            },
            overage: Window::unknown(WindowKind::Overage),
            overall_status: Some("allowed".into()),
            fallback: Some("available".into()),
            fallback_percentage: Some(50.0),
            representative_claim: Some("five_hour".into()),
            probed_at: 1784225000,
            source: Source::Probe,
        }
    }

    #[test]
    fn un_releve_ecrit_se_relit_a_lidentique() {
        let dir = TempDir::new("aller-retour");
        let fichier = dir.0.join("state.json");
        store(&fichier, &un_releve()).expect("ecriture");
        assert_eq!(load(&fichier), Some(un_releve()));
    }

    #[test]
    fn cree_les_dossiers_manquants() {
        let dir = TempDir::new("mkdir");
        let fichier = dir.0.join("a").join("b").join("state.json");
        store(&fichier, &un_releve()).expect("ecriture");
        assert!(fichier.exists());
    }

    #[test]
    fn ne_laisse_aucun_fichier_temporaire_derriere_lui() {
        let dir = TempDir::new("propre");
        let fichier = dir.0.join("state.json");
        store(&fichier, &un_releve()).expect("ecriture");
        let restants: Vec<_> = fs::read_dir(&dir.0)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(restants, vec!["state.json".to_string()]);
    }

    #[test]
    fn un_cache_absent_ou_abime_ne_fait_pas_echouer_la_lecture() {
        let dir = TempDir::new("abime");
        assert_eq!(load(&dir.0.join("jamais-ecrit.json")), None);

        let casse = dir.0.join("casse.json");
        fs::write(&casse, "{ceci n'est pas du json").unwrap();
        assert_eq!(load(&casse), None);
    }
}
