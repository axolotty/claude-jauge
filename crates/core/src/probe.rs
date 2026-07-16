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

/// Le point d'entree des messages.
pub const API_URL: &str = "https://api.anthropic.com/v1/messages";

/// Le plus petit modele disponible : la sonde doit couter le moins possible.
pub const PROBE_MODEL: &str = "claude-haiku-4-5-20251001";

const ANTHROPIC_VERSION: &str = "2023-06-01";
const OAUTH_BETA: &str = "oauth-2025-04-20";

/// Les jetons OAuth de Claude Code ne sont acceptes qu'avec cette identite.
const SYSTEM_PROMPT: &str = "You are Claude Code, Anthropic's official CLI for Claude.";

/// Interroge l'API et rend l'etat des quotas lu dans les en-tetes.
pub fn probe(creds: &Credentials, now: u64) -> Result<Quota, Error> {
    if creds.is_expired(now) {
        return Err(Error::TokenExpired);
    }

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
        .set("content-type", "application/json");

    match request.send_json(body) {
        Ok(response) => read(&response, now),
        // Un refus porte justement les en-tetes qui nous interessent le plus :
        // c'est quand le quota est epuise qu'on veut savoir quand il revient.
        Err(ureq::Error::Status(_, response)) => read(&response, now),
        Err(err) => Err(Error::Probe(err.to_string())),
    }
}

fn read(response: &ureq::Response, now: u64) -> Result<Quota, Error> {
    Quota::from_headers(
        |name| response.header(name).map(|value| value.to_string()),
        now,
    )
    .ok_or(Error::NoQuotaHeaders)
}
