//! Detection d'activite : quand Claude Code a-t-il parle a l'API pour la
//! derniere fois ?
//!
//! C'est la piece qui rend la sonde intelligente. Les compteurs de quota ne
//! bougent que lorsque Claude Code emet une requete, et il ecrit alors dans le
//! transcript de la conversation. La date de modification la plus recente sous
//! `~/.claude/projects` suffit donc a savoir s'il s'est passe quelque chose
//! depuis notre dernier releve. Tant que rien n'a bouge, sonder ne servirait a
//! rien : le compte a rebours, lui, se calcule tres bien en local.

use std::fs;
use std::path::Path;
use std::time::UNIX_EPOCH;

/// Date de la derniere ecriture dans un transcript, en secondes depuis l'epoch.
///
/// Rend `None` si le dossier n'existe pas ou ne contient aucun transcript.
/// Les erreurs de lecture (droits, fichier disparu en cours de route) sont
/// ignorees : un transcript illisible n'est pas une raison de tout arreter.
pub fn last_activity(projects_dir: &Path) -> Option<u64> {
    let mut latest: Option<u64> = None;
    visit(projects_dir, &mut latest, 0);
    latest
}

/// Profondeur maximale exploree, garde-fou contre les liens symboliques
/// circulaires.
const MAX_DEPTH: usize = 8;

fn visit(dir: &Path, latest: &mut Option<u64>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let kind = match entry.file_type() {
            Ok(kind) => kind,
            Err(_) => continue,
        };

        if kind.is_dir() {
            visit(&path, latest, depth + 1);
            continue;
        }

        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }

        let modified = entry
            .metadata()
            .ok()
            .and_then(|meta| meta.modified().ok())
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs());

        if let Some(secs) = modified {
            if latest.map_or(true, |current| secs > current) {
                *latest = Some(secs);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::path::PathBuf;

    /// Petit dossier temporaire, nettoye a la sortie.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(nom: &str) -> Self {
            let path = std::env::temp_dir().join(format!("claude-jauge-{nom}-{}", std::process::id()));
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

    #[test]
    fn un_dossier_absent_ne_donne_aucune_activite() {
        assert_eq!(last_activity(Path::new("/tmp/absent-pour-de-bon-42")), None);
    }

    #[test]
    fn un_dossier_sans_transcript_ne_donne_aucune_activite() {
        let dir = TempDir::new("vide");
        fs::create_dir_all(dir.0.join("un-projet")).unwrap();
        File::create(dir.0.join("un-projet").join("notes.txt")).unwrap();
        assert_eq!(last_activity(&dir.0), None);
    }

    #[test]
    fn trouve_le_transcript_le_plus_recent_en_descendant_les_sous_dossiers() {
        let dir = TempDir::new("recent");
        let projet = dir.0.join("-un-projet");
        fs::create_dir_all(&projet).unwrap();
        File::create(projet.join("session.jsonl")).unwrap();
        // Un fichier qui n'est pas un transcript ne doit pas compter.
        File::create(projet.join("autre.txt")).unwrap();

        let trouve = last_activity(&dir.0).expect("une activite");
        let maintenant = crate::now();
        // Le fichier vient d'etre cree : sa date doit coller a l'heure courante.
        assert!(
            trouve.abs_diff(maintenant) < 60,
            "date trouvee {trouve} trop loin de {maintenant}"
        );
    }
}
