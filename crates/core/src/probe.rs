//! La sonde : une requete minimale dont seuls les en-tetes nous interessent.
//!
//! L'API porte l'etat des quotas sur toutes ses reponses. Le corps de la
//! requete n'a donc aucune importance : on demande un seul jeton au plus petit
//! modele disponible, et on jette la reponse. Ce qui compte tient dans les
//! en-tetes.
//!
//! Attention a la portee de ce geste : une sonde est une requete comme une
//! autre, donc elle OUVRE une fenetre de 5 heures s'il n'y en a pas deja une en
//! cours. C'est pour cette raison que [`crate::meter::should_probe`] refuse de
//! sonder quand Claude Code n'a pas servi recemment.

use crate::quota::Quota;
use crate::{Credentials, Error};
use serde_json::json;
use std::time::Duration;

/// Le point d'entree des messages.
pub const API_URL: &str = "https://api.anthropic.com/v1/messages";

/// Le plus petit modele disponible : la sonde doit couter le moins possible.
pub const PROBE_MODEL: &str = "claude-haiku-4-5-20251001";

const ANTHROPIC_VERSION: &str = "2023-06-01";
const OAUTH_BETA: &str = "oauth-2025-04-20";

/// Delai au-dela duquel on abandonne la sonde.
///
/// ureq n'impose aucun delai de lecture par defaut : sans cette borne, un
/// serveur qui accepte la connexion puis se tait fige l'appel indefiniment.
/// Ce n'est pas theorique -- `probe` est appele par la commande Tauri
/// `refresh`, donc par le bouton « Rafraichir » de la popover, qui restait
/// alors bloque pour toujours. La verification des mises a jour, elle, posait
/// deja sa propre limite.
const TIMEOUT: Duration = Duration::from_secs(20);

/// Les jetons OAuth de Claude Code ne sont acceptes qu'avec cette identite.
const SYSTEM_PROMPT: &str = "You are Claude Code, Anthropic's official CLI for Claude.";

/// Interroge l'API et rend l'etat des quotas lu dans les en-tetes.
///
/// On ne refuse plus a priori sur la foi de `expiresAt`. Le jeton peut venir
/// d'une autre machine, dont l'horloge n'est pas la notre, et une date
/// d'expiration n'est qu'une promesse : seule l'API sait vraiment. Un 401 nous
/// le dira, et c'est la reponse qui fait foi.
pub fn probe(creds: &Credentials, now: u64) -> Result<Quota, Error> {
    let body = json!({
        "model": PROBE_MODEL,
        "max_tokens": 1,
        "system": SYSTEM_PROMPT,
        "messages": [{ "role": "user", "content": "." }],
    });

    let request = ureq::post(API_URL)
        .set("authorization", &format!("Bearer {}", creds.token))
        .set("anthropic-version", ANTHROPIC_VERSION)
        .set("anthropic-beta", OAUTH_BETA)
        .set("content-type", "application/json")
        .timeout(TIMEOUT);

    match request.send_json(body) {
        Ok(response) => read(&response, now),
        // Un 401 dit que le jeton est refuse, pas pourquoi : expire, revoque,
        // mal forme, scope insuffisant... Traduire ce statut en une cause
        // precise serait une invention. On rapporte donc ce que l'API dit,
        // elle seule le sait.
        Err(ureq::Error::Status(401, response)) => {
            let detail = response.into_string().unwrap_or_default();
            Err(Error::Unauthorized(resume(&detail)))
        }
        // Un refus de quota, lui, porte justement les en-tetes qui nous
        // interessent le plus : c'est quand le quota est epuise qu'on veut
        // savoir quand il revient.
        Err(ureq::Error::Status(_, response)) => read(&response, now),
        Err(err) => Err(Error::Probe(err.to_string())),
    }
}

/// Longueur maximale d'un message d'erreur affiche, en octets.
const RESUME_MAX: usize = 160;

/// Tire le message d'erreur du corps de la reponse.
///
/// L'API repond `{"type":"error","error":{"type":"...","message":"..."}}`. On
/// n'en veut que le message, et jamais plus d'une ligne : ca finit dans une
/// popover de 300 pixels.
fn resume(corps: &str) -> String {
    let message = serde_json::from_str::<serde_json::Value>(corps)
        .ok()
        .and_then(|v| v.get("error")?.get("message")?.as_str().map(String::from))
        .unwrap_or_else(|| corps.trim().to_string());

    let message = message.replace('\n', " ");
    if message.len() > RESUME_MAX {
        format!("{}...", &message[..bord_de_caractere(&message, RESUME_MAX)])
    } else {
        message
    }
}

/// Recule jusqu'a la frontiere de caractere la plus proche, sans la depasser.
///
/// `len()` compte des OCTETS, pas des caracteres, et trancher une chaine au
/// milieu d'un caractere multi-octets fait paniquer Rust. Un message d'erreur
/// de l'API contenant une lettre accentuee suffisait donc a faire tomber le
/// widget -- 159 caracteres ASCII suivis d'un « e accent aigu » et l'octet 160
/// tombe pile au milieu de ce dernier.
fn bord_de_caractere(texte: &str, mut indice: usize) -> usize {
    while indice > 0 && !texte.is_char_boundary(indice) {
        indice -= 1;
    }
    indice
}

fn read(response: &ureq::Response, now: u64) -> Result<Quota, Error> {
    Quota::from_headers(
        |name| response.header(name).map(|value| value.to_string()),
        now,
    )
    .ok_or(Error::NoQuotaHeaders)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tire_le_message_derreur_de_lapi() {
        let corps = r#"{"type":"error","error":{"type":"authentication_error","message":"OAuth token has expired"}}"#;
        assert_eq!(resume(corps), "OAuth token has expired");
    }

    #[test]
    fn un_corps_inattendu_est_rendu_tel_quel() {
        // L'API peut changer de forme, ou un intermediaire repondre a sa place :
        // mieux vaut montrer le brut que d'avaler l'information.
        assert_eq!(resume("  Gateway timeout  "), "Gateway timeout");
        assert_eq!(resume("{}"), "{}");
    }

    #[test]
    fn un_message_bavard_est_coupe() {
        let long = format!(r#"{{"error":{{"message":"{}"}}}}"#, "x".repeat(500));
        let r = resume(&long);
        assert!(r.len() <= 163, "trop long : {}", r.len());
        assert!(r.ends_with("..."));
    }

    #[test]
    fn un_message_accentue_ne_fait_pas_paniquer() {
        // `len()` compte des octets : 159 caracteres ASCII suivis d'un « e
        // accent aigu » font 161 octets, et l'octet 160 tombe au milieu de ce
        // dernier. Trancher la se traduisait par un panic, donc un widget qui
        // disparait, pour un simple message d'erreur accentue.
        let long = format!(r#"{{"error":{{"message":"{}e\u00e9"}}}}"#, "x".repeat(158));
        let r = resume(&long);
        assert!(r.ends_with("..."));
        assert!(r.len() <= RESUME_MAX + 3);
    }

    #[test]
    fn coupe_proprement_quel_que_soit_lendroit_de_la_coupure() {
        // On balaye les longueurs autour de la limite pour tomber sur chacune
        // des quatre positions possibles a l'interieur d'un caractere de 4
        // octets.
        for n in 150..175 {
            let message = format!("{}\u{1F600}", "x".repeat(n));
            let corps = serde_json::json!({"error": {"message": message}}).to_string();
            let r = resume(&corps); // ne doit jamais paniquer
            assert!(!r.is_empty());
        }
    }

    #[test]
    fn un_message_entierement_accentue_reste_lisible() {
        let message = "e".repeat(10) + &"\u{00e9}".repeat(200);
        let corps = serde_json::json!({"error": {"message": message}}).to_string();
        let r = resume(&corps);
        // La coupure ne doit pas produire de caractere de remplacement.
        assert!(!r.contains('\u{FFFD}'), "caractere abime : {r}");
    }

    #[test]
    fn les_retours_a_la_ligne_sont_aplatis() {
        // Le message finit dans une popover d'une seule ligne. La chaine brute
        // porte bien `\n` en deux caracteres : un vrai saut de ligne au milieu
        // d'une chaine JSON serait invalide, et ne testerait donc pas ce cas.
        assert_eq!(resume(r#"{"error":{"message":"a\nb"}}"#), "a b");
    }
}
