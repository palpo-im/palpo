use std::str::FromStr;

use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use serde::Deserialize;

use crate::config::JwtConfig;
use crate::{AppError, AppResult, MatrixError};

#[derive(Debug, Deserialize)]
pub struct JwtClaims {
    pub sub: String,
}

pub fn validate_jwt_token(config: &JwtConfig, token: &str) -> AppResult<JwtClaims> {
    if cfg!(debug_assertions) && !config.validate_signature {
        warn!("JWT signature validation is disabled!");
        let verifier = init_jwt_verifier(config)?;
        let mut validator = init_jwt_validator(config)?;
        #[allow(deprecated)]
        validator.insecure_disable_signature_validation();

        return jsonwebtoken::decode::<JwtClaims>(token, &verifier, &validator)
            .map(|decoded| (decoded.header, decoded.claims))
            .inspect(|(head, claim)| debug!(?head, ?claim, "JWT token decoded (insecure)"))
            .map_err(|e| MatrixError::not_found(format!("invalid JWT token: {e}")).into())
            .map(|(_, claims)| claims);
    }

    let verifier = init_jwt_verifier(config)?;
    let validator = init_jwt_validator(config)?;
    jsonwebtoken::decode::<JwtClaims>(token, &verifier, &validator)
        .map(|decoded| (decoded.header, decoded.claims))
        .inspect(|(head, claim)| debug!(?head, ?claim, "JWT token decoded"))
        .map_err(|e| MatrixError::not_found(format!("invalid JWT token: {e}")).into())
        .map(|(_, claims)| claims)
}

fn init_jwt_verifier(config: &JwtConfig) -> AppResult<DecodingKey> {
    let secret = &config.secret;
    let format = config.format.as_str();

    Ok(match format {
        "HMAC" => DecodingKey::from_secret(secret.as_bytes()),

        "HMACB64" => DecodingKey::from_base64_secret(secret.as_str())
            .map_err(|_e| AppError::public("jwt secret is not valid base64"))?,

        "ECDSA" => DecodingKey::from_ec_pem(secret.as_bytes())
            .map_err(|_e| AppError::public("jwt key is not valid PEM"))?,

        _ => return Err(AppError::public("jwt secret format is not supported")),
    })
}

fn init_jwt_validator(config: &JwtConfig) -> AppResult<Validation> {
    let alg = config.algorithm.as_str();
    let alg = Algorithm::from_str(alg)
        .map_err(|_e| AppError::public("jwt algorithm is not recognized or configured"))?;

    let mut validator = Validation::new(alg);
    let mut required_spec_claims: Vec<_> = ["sub"].into();

    validator.validate_exp = config.validate_exp;
    if config.require_exp {
        required_spec_claims.push("exp");
    }

    validator.validate_nbf = config.validate_nbf;
    if config.require_nbf {
        required_spec_claims.push("nbf");
    }

    if !config.audience.is_empty() {
        required_spec_claims.push("aud");
        validator.set_audience(&config.audience);
    }

    if !config.issuer.is_empty() {
        required_spec_claims.push("iss");
        validator.set_issuer(&config.issuer);
    }

    validator.set_required_spec_claims(&required_spec_claims);
    debug!(?validator, "JWT configured");

    Ok(validator)
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use jsonwebtoken::{EncodingKey, Header, encode};
    use serde::Serialize;

    use super::validate_jwt_token;
    use crate::config::JwtConfig;

    #[derive(Serialize)]
    struct ExpiredClaims {
        sub: String,
        exp: u64,
    }

    #[test]
    fn validate_jwt_token_rejects_expired_token_when_signature_check_disabled() {
        let exp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("current time should be after unix epoch")
            .as_secs()
            .saturating_sub(3600);

        let token = encode(
            &Header::default(),
            &ExpiredClaims {
                sub: "alice".to_owned(),
                exp,
            },
            &EncodingKey::from_secret(b"test-secret"),
        )
        .expect("JWT token should be encoded");

        let mut config = JwtConfig::default();
        config.secret = "test-secret".to_owned();
        config.format = "HMAC".to_owned();
        config.algorithm = "HS256".to_owned();
        config.validate_signature = false;
        config.validate_exp = true;
        config.require_exp = true;

        let result = validate_jwt_token(&config, &token);

        assert!(
            result.is_err(),
            "expired token must be rejected even when signature validation is disabled"
        );
    }
}
