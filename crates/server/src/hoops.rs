use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use salvo::http::{ParseError, ResBody};
use salvo::prelude::*;
use salvo::size_limiter;

use crate::AppResult;
use crate::core::MatrixError;
use crate::core::error::RetryAfter;
use crate::exts::DepotExt;

mod auth;
pub use auth::*;
pub mod introspection;

#[handler]
pub async fn ensure_accept(req: &mut Request) {
    if req.accept().is_empty() {
        req.headers_mut().insert(
            "Accept",
            "application/json".parse().expect("should not fail"),
        );
    }
}

#[handler]
pub async fn ensure_content_type(req: &mut Request) {
    if req.content_type().is_none() {
        req.headers_mut().insert(
            "Content-Type",
            "application/json".parse().expect("should not fail"),
        );
    }
}

#[handler]
pub async fn limit_size(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
    ctrl: &mut FlowCtrl,
) {
    let mut max_size = 1024 * 1024 * 16;
    if let Some(ctype) = req.content_type()
        && ctype.type_() == mime::MULTIPART
    {
        max_size = 1024 * 1024 * 1024;
    }
    let limiter = size_limiter::max_size(max_size);
    limiter.handle(req, depot, res, ctrl).await;
}

/// Token-bucket rate limiter: maps a client key to its available tokens.
struct RateLimiter {
    buckets: Mutex<HashMap<String, (f64, Instant)>>,
}

impl RateLimiter {
    fn new() -> Self {
        Self {
            buckets: Mutex::new(HashMap::new()),
        }
    }

    fn check(&self, key: &str, cfg: &crate::config::RateLimitConfig) -> AppResult<()> {
        self.check_with_cost(key, cfg, true)
    }

    fn probe(&self, key: &str, cfg: &crate::config::RateLimitConfig) -> AppResult<()> {
        self.check_with_cost(key, cfg, false)
    }

    fn check_with_cost(
        &self,
        key: &str,
        cfg: &crate::config::RateLimitConfig,
        consume: bool,
    ) -> AppResult<()> {
        if cfg.per_second <= 0.0 || cfg.burst == 0 {
            return Ok(());
        }

        let mut map = self.buckets.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let burst = cfg.burst as f64;
        let entry = map.entry(key.to_owned()).or_insert((burst, now));

        // Refill tokens based on elapsed time
        let elapsed = now.duration_since(entry.1).as_secs_f64();
        entry.1 = now;
        entry.0 = (entry.0 + elapsed * cfg.per_second).min(burst);

        // Try to consume 1 token
        if entry.0 >= 1.0 {
            if consume {
                entry.0 -= 1.0;
            }
            Ok(())
        } else {
            let wait_seconds = (1.0 - entry.0) / cfg.per_second;
            let retry_after = (wait_seconds.is_finite() && wait_seconds < u64::MAX as f64 / 2.0)
                .then(|| RetryAfter::Delay(Duration::from_secs_f64(wait_seconds.max(0.0))));
            Err(MatrixError::limit_exceeded(
                "Too many requests. Please try again later.",
                retry_after,
            )
            .into())
        }
    }
}

fn extract_ip(req: &Request) -> Option<String> {
    let peer = match req.remote_addr() {
        salvo::conn::SocketAddr::IPv4(a) => IpAddr::V4(*a.ip()),
        salvo::conn::SocketAddr::IPv6(a) => IpAddr::V6(*a.ip()),
        _ => return None,
    };
    let forwarded = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok());
    Some(client_ip(peer, forwarded, &crate::config::get().trusted_proxies).to_string())
}

fn client_ip(peer: IpAddr, forwarded: Option<&str>, trusted_proxies: &[String]) -> IpAddr {
    let Some(forwarded) = forwarded else {
        return peer;
    };
    let mut client = peer;
    for hop in forwarded.split(',').rev() {
        if !is_trusted_proxy(client, trusted_proxies) {
            break;
        }
        let Ok(next) = hop.trim().parse() else {
            return peer;
        };
        client = next;
    }
    client
}

fn is_trusted_proxy(ip: IpAddr, trusted_proxies: &[String]) -> bool {
    let Ok(ip) = ipaddress::IPAddress::parse(&ip.to_string()) else {
        return false;
    };
    trusted_proxies
        .iter()
        .any(|cidr| ipaddress::IPAddress::parse(cidr).is_ok_and(|network| network.includes(&ip)))
}

static LOGIN_LIMITER: LazyLock<RateLimiter> = LazyLock::new(RateLimiter::new);
static LOGIN_TOKEN_LIMITER: LazyLock<RateLimiter> = LazyLock::new(RateLimiter::new);
static REGISTRATION_LIMITER: LazyLock<RateLimiter> = LazyLock::new(RateLimiter::new);
static REGISTRATION_AVAILABLE_LIMITER: LazyLock<RateLimiter> = LazyLock::new(RateLimiter::new);
static REGISTRATION_TOKEN_LIMITER: LazyLock<RateLimiter> = LazyLock::new(RateLimiter::new);
static PASSWORD_LIMITER: LazyLock<RateLimiter> = LazyLock::new(RateLimiter::new);
static MESSAGE_LIMITER: LazyLock<RateLimiter> = LazyLock::new(RateLimiter::new);
static USER_DIRECTORY_LIMITER: LazyLock<RateLimiter> = LazyLock::new(RateLimiter::new);

pub fn check_login_rate(req: &Request) -> AppResult<()> {
    if let Some(ip) = extract_ip(req) {
        LOGIN_LIMITER.check(&ip, &crate::config::get().rc_login)?;
    }
    Ok(())
}

pub fn check_login_token_rate(user_id: &str) -> AppResult<()> {
    LOGIN_TOKEN_LIMITER.check(
        user_id,
        &crate::config::RateLimitConfig {
            per_second: 1.0 / 60.0,
            burst: 1,
        },
    )
}

pub fn check_registration_rate(req: &Request) -> AppResult<()> {
    if let Some(ip) = extract_ip(req) {
        REGISTRATION_LIMITER.check(&ip, &crate::config::get().rc_registration)?;
    }
    Ok(())
}

pub fn check_registration_available_rate(req: &Request) -> AppResult<()> {
    if let Some(ip) = extract_ip(req) {
        REGISTRATION_AVAILABLE_LIMITER
            .check(&ip, &crate::config::get().rc_registration_available)?;
    }
    Ok(())
}

pub fn check_registration_token_rate(req: &Request) -> AppResult<()> {
    if let Some(ip) = extract_ip(req) {
        REGISTRATION_TOKEN_LIMITER
            .check(&ip, &crate::config::get().rc_registration_token_validity)?;
    }
    Ok(())
}

pub fn check_password_attempt(user_id: &str) -> AppResult<()> {
    PASSWORD_LIMITER.probe(user_id, &crate::config::get().rc_password)
}

pub fn record_password_failure(user_id: &str) -> AppResult<()> {
    PASSWORD_LIMITER.check(user_id, &crate::config::get().rc_password)
}

/// General rate limiter for authenticated API endpoints.
#[handler]
pub async fn limit_rate(req: &mut Request, depot: &mut Depot) -> AppResult<()> {
    if req.method().is_safe() {
        return Ok(());
    }
    let user_id = depot.authed_info()?.user_id().to_string();
    MESSAGE_LIMITER.check(&user_id, &crate::config::get().rc_message)?;
    Ok(())
}

#[handler]
pub async fn limit_rate_user_directory(depot: &mut Depot) -> AppResult<()> {
    let user_id = depot.authed_info()?.user_id().to_string();
    USER_DIRECTORY_LIMITER.check(&user_id, &crate::config::get().rc_user_directory)
}

#[cfg(test)]
mod rate_limit_tests {
    use std::net::IpAddr;

    use super::{RateLimiter, client_ip};
    use crate::config::RateLimitConfig;

    #[test]
    fn forwarded_chain_stops_at_first_untrusted_hop() {
        let proxies = vec!["172.16.0.0/12".to_owned()];
        let peer: IpAddr = "172.18.0.2".parse().unwrap();
        let actual: IpAddr = "198.51.100.7".parse().unwrap();
        assert_eq!(
            client_ip(peer, Some("203.0.113.9, 198.51.100.7"), &proxies),
            actual
        );
        assert_eq!(client_ip(peer, Some("invalid"), &proxies), peer);
        assert_eq!(client_ip(peer, Some("198.51.100.7"), &[]), peer);
    }

    #[test]
    fn probing_failed_attempt_limit_does_not_charge_successful_requests() {
        let limiter = RateLimiter::new();
        let config = RateLimitConfig {
            per_second: 0.000001,
            burst: 2,
        };
        for _ in 0..3 {
            assert!(limiter.probe("alice", &config).is_ok());
        }
        assert!(limiter.check("alice", &config).is_ok());
        assert!(limiter.check("alice", &config).is_ok());
        assert!(limiter.probe("alice", &config).is_err());
        assert!(limiter.probe("bob", &config).is_ok());
    }
}

// utf8 will cause complement testing fail.
#[handler]
pub async fn remove_json_utf8(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
    ctrl: &mut FlowCtrl,
) {
    ctrl.call_next(req, depot, res).await;
    if let Some(true) = res.headers().get("content-type").map(|h| {
        let h = h.to_str().unwrap_or_default();
        h.contains("application/json") && h.contains(";")
    }) {
        res.add_header("content-type", "application/json", true)
            .expect("should not fail");
    }
}

#[handler]
pub async fn default_accept_json(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
    ctrl: &mut FlowCtrl,
) {
    if !req.headers().contains_key("accept") {
        req.add_header("accept", "application/json", true)
            .expect("should not fail");
    }
    ctrl.call_next(req, depot, res).await;
}

#[handler]
pub async fn catch_status_error(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
    ctrl: &mut FlowCtrl,
) {
    if let ResBody::Error(e) = &res.body {
        if let Some(e) = &e.cause {
            if let Some(e) = e.downcast_ref::<ParseError>() {
                #[cfg(debug_assertions)]
                let matrix = MatrixError::bad_json(e.to_string());
                #[cfg(not(debug_assertions))]
                let matrix = MatrixError::bad_json("bad json");
                matrix.write(req, depot, res).await;
                ctrl.skip_rest();
            }
        } else {
            let matrix = MatrixError::unrecognized(e.brief.clone());
            matrix.write(req, depot, res).await;
            ctrl.skip_rest();
        }
    } else if res.status_code == Some(StatusCode::METHOD_NOT_ALLOWED) {
        let matrix = MatrixError::unrecognized("method not allowed");
        matrix.write(req, depot, res).await;
        ctrl.skip_rest();
    }
}
