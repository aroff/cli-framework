//! Authorization-code ID-token validation, independent of API token audiences.
use super::state::BrowserLayerState;
use crate::jwks::KeyResult;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use jsonwebtoken::{Algorithm, Validation};
use serde_json::Value;
use sha2::{Digest, Sha256, Sha384, Sha512};

pub(crate) async fn verify(
    token: &str,
    nonce: &str,
    access_token: &str,
    state: &BrowserLayerState,
) -> Result<String, &'static str> {
    if token.is_empty() || token.len() > 16 * 1024 {
        return Err("invalid ID token");
    }
    let header = jsonwebtoken::decode_header(token).map_err(|_| "invalid ID token")?;
    if !state.algorithms.contains(&header.alg) {
        return Err("unsupported ID-token algorithm");
    }
    let KeyResult::Keys(keys) = state.get_decoding_keys(&header.kid).await else {
        return Err("ID-token keys unavailable");
    };
    let mut validation = Validation::new(header.alg);
    validation.set_issuer(&[&state.cfg.issuer_url]);
    validation.set_audience(&[&state.cfg.client_id]);
    validation.set_required_spec_claims(&["iss", "sub", "aud", "exp", "iat", "nonce"]);
    validation.validate_nbf = true;
    validation.leeway = state.cfg.clock_skew.as_secs();
    for key in keys {
        if let Ok(data) = jsonwebtoken::decode::<Value>(token, &key, &validation) {
            return verify_claims(&data.claims, nonce, access_token, header.alg, state);
        }
    }
    Err("ID-token signature or claims rejected")
}

fn verify_claims(
    claims: &Value,
    nonce: &str,
    access_token: &str,
    algorithm: Algorithm,
    state: &BrowserLayerState,
) -> Result<String, &'static str> {
    let subject = claims["sub"]
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or("invalid ID-token subject")?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "clock unavailable")?
        .as_secs();
    let issued_at = claims["iat"]
        .as_u64()
        .ok_or("invalid ID-token issue time")?;
    if issued_at > now.saturating_add(state.cfg.clock_skew.as_secs())
        || claims["exp"].as_i64().is_none()
        || claims["iss"].as_str() != Some(state.cfg.issuer_url.as_str())
        || claims["nonce"].as_str() != Some(nonce)
    {
        return Err("ID-token identity or nonce rejected");
    }
    let audience_count = match &claims["aud"] {
        Value::String(value) if value == &state.cfg.client_id => 1,
        Value::Array(values)
            if !values.is_empty()
                && values.iter().all(Value::is_string)
                && values
                    .iter()
                    .any(|value| value.as_str() == Some(&state.cfg.client_id)) =>
        {
            values.len()
        }
        _ => return Err("ID-token audience rejected"),
    };
    let authorized_party = claims.get("azp");
    if (audience_count > 1 || authorized_party.is_some())
        && authorized_party.and_then(Value::as_str) != Some(&state.cfg.client_id)
    {
        return Err("ID-token authorized party rejected");
    }
    if let Some(hash) = claims.get("at_hash") {
        let digest = match algorithm {
            Algorithm::RS256 | Algorithm::PS256 | Algorithm::ES256 => {
                Sha256::digest(access_token.as_bytes()).to_vec()
            }
            Algorithm::RS384 | Algorithm::PS384 | Algorithm::ES384 => {
                Sha384::digest(access_token.as_bytes()).to_vec()
            }
            Algorithm::RS512 | Algorithm::PS512 => Sha512::digest(access_token.as_bytes()).to_vec(),
            _ => return Err("unsupported access-token hash"),
        };
        if hash.as_str() != Some(URL_SAFE_NO_PAD.encode(&digest[..digest.len() / 2]).as_str()) {
            return Err("ID-token access-token hash rejected");
        }
    }
    Ok(subject.to_string())
}
