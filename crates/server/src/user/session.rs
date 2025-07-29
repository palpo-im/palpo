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

    if cfg!(debug_assertions) && !config.validate_signature {
        warn!("JWT signature validation is disabled!");
        validator.insecure_disable_signature_validation();
    }

    validator.set_required_spec_claims(&required_spec_claims);
    debug!(?validator, "JWT configured");

    Ok(validator)
}
