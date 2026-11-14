use std::collections::{BTreeSet, HashMap};
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
#[cfg(feature = "unstable-msc4363")]
mod step_up;

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

/// Do not evict depleted buckets: that would give their clients a new burst.
/// At capacity, reject unknown clients until an existing bucket fully refills.
const MAX_RATE_LIMIT_BUCKETS: usize = 65_536;

struct RateBucket {
    tokens: f64,
    updated_at: Instant,
    expires_at: Option<Instant>,
}

#[derive(Default)]
struct RateBuckets {
    buckets: HashMap<String, RateBucket>,
    // One deadline per bucket, not one record per request. Forgetting a fully
    // refilled bucket is equivalent to retaining it with its entire burst.
    expiry: BTreeSet<(Instant, String)>,
}

/// A bounded token-bucket limiter with lazy, exact full-refill reclamation.
struct RateLimiter {
    buckets: Mutex<RateBuckets>,
    capacity: usize,
}

impl RateLimiter {
    fn new() -> Self {
        Self::with_capacity(MAX_RATE_LIMIT_BUCKETS)
    }

    fn with_capacity(capacity: usize) -> Self {
        Self {
            buckets: Mutex::new(RateBuckets::default()),
            capacity,
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
        self.check_at(key, cfg, consume, Instant::now())
    }

    fn check_at(
        &self,
        key: &str,
        cfg: &crate::config::RateLimitConfig,
        consume: bool,
        now: Instant,
    ) -> AppResult<()> {
        if cfg.per_second <= 0.0 || cfg.burst == 0 {
            return Ok(());
        }
        if !cfg.per_second.is_finite() {
            return Err(Self::limited(None));
        }

        let mut state = self.buckets.lock().unwrap_or_else(|e| e.into_inner());
        while state.expiry.first().is_some_and(|(at, _)| *at <= now) {
            let (_, expired_key) = state.expiry.pop_first().expect("expiry was checked");
            state.buckets.remove(&expired_key);
        }

        let burst = cfg.burst as f64;
        let Some(mut bucket) = state.buckets.remove(key) else {
            if state.buckets.len() >= self.capacity {
                return Err(Self::limited(Some(Duration::from_secs(1))));
            }
            // A successful probe costs nothing, so it needs no stored bucket.
            if !consume {
                return Ok(());
            }
            let tokens = burst - 1.0;
            Self::store_bucket(&mut state, key, tokens, now, cfg);
            return Ok(());
        };
        if let Some(at) = bucket.expires_at {
            state.expiry.remove(&(at, key.to_owned()));
        }
        // A thread may sample its clock before waiting for the state mutex.
        // Never move the refill clock backwards.
        let now = now.max(bucket.updated_at);
        let elapsed = now.duration_since(bucket.updated_at).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * cfg.per_second).min(burst);
        let result = if bucket.tokens >= 1.0 {
            if consume {
                bucket.tokens -= 1.0;
            }
            Ok(())
        } else {
            let delay = Duration::try_from_secs_f64((1.0 - bucket.tokens) / cfg.per_second).ok();
            Err(Self::limited(delay))
        };
        Self::store_bucket(&mut state, key, bucket.tokens, now, cfg);
        result
    }

    fn store_bucket(
        state: &mut RateBuckets,
        key: &str,
        tokens: f64,
        now: Instant,
        cfg: &crate::config::RateLimitConfig,
    ) {
        let deficit = cfg.burst as f64 - tokens;
        if deficit <= 0.0 {
            return;
        }
        // Round conservatively, so sub-nanosecond truncation cannot reset a
        // bucket before all its tokens would actually have regenerated.
        let expires_at = Duration::try_from_secs_f64(deficit / cfg.per_second)
            .ok()
            .and_then(|delay| delay.checked_add(Duration::from_nanos(1)))
            .and_then(|delay| now.checked_add(delay));
        if let Some(at) = expires_at {
            state.expiry.insert((at, key.to_owned()));
        }
        state.buckets.insert(
            key.to_owned(),
            RateBucket {
                tokens,
                updated_at: now,
                expires_at,
            },
        );
    }

    fn limited(delay: Option<Duration>) -> crate::AppError {
        MatrixError::limit_exceeded(
            "Too many requests. Please try again later.",
            delay.map(RetryAfter::Delay),
        )
        .into()
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
    let Ok(ip) = ipaddress::IPAddress::parse(ip.to_string()) else {
        return false;
    };
    trusted_proxies
        .iter()
        .any(|cidr| ipaddress::IPAddress::parse(cidr).is_ok_and(|network| network.includes(&ip)))
}

static LOGIN_LIMITER: LazyLock<RateLimiter> = LazyLock::new(RateLimiter::new);
static REFRESH_LIMITER: LazyLock<RateLimiter> = LazyLock::new(RateLimiter::new);
static INVALID_REFRESH_LIMITER: LazyLock<RateLimiter> = LazyLock::new(RateLimiter::new);
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

// Allow early refreshes and retries throughout even short configured lifetimes.
fn refresh_rate_config(access_ttl_ms: u64, refresh_ttl_ms: u64) -> crate::config::RateLimitConfig {
    crate::config::RateLimitConfig {
        per_second: 4_000.0 / access_ttl_ms.min(refresh_ttl_ms).max(1) as f64,
        burst: 5,
    }
}

pub fn check_refresh_rate(user_id: &str, device_id: &str) -> AppResult<()> {
    let config = crate::config::get();
    let key = serde_json::to_string(&(user_id, device_id)).expect("string tuple serializes");
    REFRESH_LIMITER.check(
        &key,
        &refresh_rate_config(config.access_token_ttl, config.refresh_token_ttl),
    )
}

pub fn check_invalid_refresh_rate(req: &Request) -> AppResult<()> {
    if let Some(ip) = extract_ip(req) {
        INVALID_REFRESH_LIMITER.check(&ip, &crate::config::get().rc_login)?;
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

/// Apply the normal per-user message rate to an event emitted later by the
/// delayed-event scheduler, which has no originating HTTP request.
pub fn limit_delayed_message(user_id: &crate::core::UserId) -> AppResult<()> {
    MESSAGE_LIMITER.check(user_id.as_str(), &crate::config::get().rc_message)
}

#[cfg(test)]
mod rate_limit_tests {
    use std::net::IpAddr;

    use super::{RateLimiter, client_ip};
    use crate::config::RateLimitConfig;

    #[test]
    fn refresh_rate_sustains_sessions_and_bounds_retry_bursts() {
        use std::time::{Duration, Instant};
        for (access, refresh) in [(60_000, 120_000), (10, 20), (120_000, 30_000)] {
            let config = super::refresh_rate_config(access, refresh);
            let limiter = RateLimiter::new();
            let start = Instant::now();
            // Early refresh every half lifetime remains sustainable long after
            // the old shared login bucket's initial burst would be exhausted.
            for i in 0..1_000 {
                let at = start + Duration::from_millis(i * access.min(refresh) / 2);
                assert!(limiter.check_at("device-a", &config, true, at).is_ok());
                assert!(limiter.check_at("device-b", &config, true, at).is_ok());
            }
            let limiter = RateLimiter::new();
            for _ in 0..config.burst {
                assert!(limiter.check_at("device-a", &config, true, start).is_ok());
            }
            assert!(limiter.check_at("device-a", &config, true, start).is_err());
            assert!(limiter.check_at("device-b", &config, true, start).is_ok());
            assert!(
                limiter
                    .check_at(
                        "device-a",
                        &config,
                        true,
                        start + Duration::from_millis(access.min(refresh))
                    )
                    .is_ok()
            );
        }
    }

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

    #[test]
    fn review_rate_limit_reclaims_only_fully_refilled_buckets_and_bounds_keys() {
        use std::time::{Duration, Instant};

        let limiter = RateLimiter::with_capacity(2);
        let config = RateLimitConfig {
            per_second: 1.0,
            burst: 2,
        };
        let start = Instant::now();
        assert!(limiter.check_at("alice", &config, true, start).is_ok());
        assert!(limiter.check_at("alice", &config, true, start).is_ok());
        assert!(limiter.check_at("bob", &config, true, start).is_ok());
        for index in 0..10_000 {
            assert!(
                limiter
                    .check_at(&format!("new-{index}"), &config, true, start)
                    .is_err()
            );
        }
        {
            let state = limiter.buckets.lock().unwrap();
            assert_eq!(state.buckets.len(), 2);
            assert_eq!(state.expiry.len(), 2);
        }
        // Capacity pressure must not evict an exhausted client or reset debt.
        assert!(limiter.check_at("alice", &config, true, start).is_err());
        assert!(
            limiter
                .check_at("new-probe", &config, false, start)
                .is_err()
        );
        let half = start + Duration::from_millis(500);
        assert!(limiter.check_at("charlie", &config, true, half).is_err());
        assert!(limiter.check_at("alice", &config, true, half).is_err());
        assert!(limiter.check_at("bob", &config, false, half).is_ok());
        // Bob can be safely forgotten once his entire burst refills, while
        // Alice remains charged. Only Bob's slot becomes reusable.
        let later = start + Duration::from_millis(1100);
        assert!(limiter.check_at("charlie", &config, true, later).is_ok());
        assert!(limiter.check_at("alice", &config, true, later).is_ok());
        assert!(limiter.check_at("alice", &config, true, later).is_err());
        let full = later + Duration::from_secs(3);
        assert!(limiter.check_at("probe-only", &config, false, full).is_ok());
        let state = limiter.buckets.lock().unwrap();
        assert!(state.buckets.is_empty());
        assert!(state.expiry.is_empty());
    }

    #[test]
    fn review_rate_limit_expiry_index_stays_bounded_and_clock_never_regresses() {
        use std::time::{Duration, Instant};

        let limiter = RateLimiter::with_capacity(1);
        let config = RateLimitConfig {
            per_second: 1.0,
            burst: 2,
        };
        let start = Instant::now();
        assert!(limiter.check_at("alice", &config, true, start).is_ok());
        let later = start + Duration::from_millis(500);
        assert!(limiter.check_at("alice", &config, true, later).is_ok());
        for _ in 0..10_000 {
            assert!(limiter.check_at("alice", &config, true, start).is_err());
        }
        assert!(limiter.check_at("alice", &config, true, later).is_err());
        let state = limiter.buckets.lock().unwrap();
        assert_eq!(state.buckets.len(), 1);
        assert_eq!(state.expiry.len(), 1);
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

#[cfg(test)]
mod tests {
    use super::RateLimiter;
    use crate::config::RateLimitConfig;

    #[test]
    fn rate_limit_buckets_are_isolated_by_key() {
        let limiter = RateLimiter::new();
        let config = RateLimitConfig {
            per_second: 1.0,
            burst: 2,
        };

        assert!(limiter.check("alice", &config).is_ok());
        assert!(limiter.check("alice", &config).is_ok());
        assert!(limiter.check("alice", &config).is_err());
        assert!(limiter.check("bob", &config).is_ok());
    }
}

static EMAIL_SEND_LIMITER: LazyLock<RateLimiter> = LazyLock::new(RateLimiter::new);
static EMAIL_VERIFY_LIMITER: LazyLock<RateLimiter> = LazyLock::new(RateLimiter::new);

pub fn check_email_send_rate(req: &Request) -> AppResult<()> {
    if let Some(ip) = extract_ip(req) {
        EMAIL_SEND_LIMITER.check(
            &ip,
            &crate::config::RateLimitConfig {
                per_second: 1.0 / 60.0,
                burst: 6,
            },
        )?;
    }
    Ok(())
}
pub fn check_email_verify_rate(req: &Request) -> AppResult<()> {
    if let Some(ip) = extract_ip(req) {
        EMAIL_VERIFY_LIMITER.check(
            &ip,
            &crate::config::RateLimitConfig {
                per_second: 0.5,
                burst: 15,
            },
        )?;
    }
    Ok(())
}
