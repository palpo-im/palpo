use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::Instant;

use salvo::http::{ParseError, ResBody};
use salvo::prelude::*;
use salvo::size_limiter;

use crate::AppResult;
use crate::core::MatrixError;

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

#[handler]
async fn access_control(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
    ctrl: &mut FlowCtrl,
) {
    let headers = res.headers_mut();
    let origin = req
        .headers()
        .get("origin")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("*");
    let allowed_origins = &crate::config::get().allowed_origins;
    let allow_origin = if allowed_origins.is_empty() || allowed_origins.iter().any(|o| o == origin)
    {
        origin.to_owned()
    } else {
        // If origin is not in the allowed list, don't set credentials
        "*".to_owned()
    };
    headers.insert(
        "Access-Control-Allow-Origin",
        allow_origin
            .parse()
            .unwrap_or_else(|_| "*".parse().unwrap()),
    );
    headers.insert(
        "Access-Control-Allow-Methods",
        "GET,POST,PUT,DELETE,PATCH,OPTIONS".parse().unwrap(),
    );
    headers.insert(
        "Access-Control-Allow-Headers",
        "Accept,Content-Type,Authorization,Range".parse().unwrap(),
    );
    headers.insert(
        "Access-Control-Expose-Headers",
        "Access-Token,Response-Status,Content-Length,Content-Range"
            .parse()
            .unwrap(),
    );
    // Only set Allow-Credentials when origin is not wildcard
    if allow_origin != "*" {
        headers.insert("Access-Control-Allow-Credentials", "true".parse().unwrap());
    }
    headers.insert(
        "Content-Security-Policy",
        "frame-ancestors 'self'".parse().unwrap(),
    );
    headers.insert("X-Content-Type-Options", "nosniff".parse().unwrap());
    ctrl.call_next(req, depot, res).await;
    // headers.insert("Cross-Origin-Embedder-Policy", "require-corp".parse().unwrap());
    // headers.insert("Cross-Origin-Opener-Policy", "same-origin".parse().unwrap());
}

/// Token-bucket rate limiter: maps IP → (available_tokens, last_check_time)
struct RateLimiter {
    buckets: Mutex<HashMap<String, (f64, Instant)>>,
}

impl RateLimiter {
    fn new() -> Self {
        Self {
            buckets: Mutex::new(HashMap::new()),
        }
    }

    fn check(&self, ip: &str, cfg: &crate::config::RateLimitConfig) -> AppResult<()> {
        if cfg.per_second <= 0.0 || cfg.burst == 0 {
            return Ok(());
        }

        let mut map = self.buckets.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let burst = cfg.burst as f64;
        let entry = map.entry(ip.to_owned()).or_insert((burst, now));

        // Refill tokens based on elapsed time
        let elapsed = now.duration_since(entry.1).as_secs_f64();
        entry.1 = now;
        entry.0 = (entry.0 + elapsed * cfg.per_second).min(burst);

        // Try to consume 1 token
        if entry.0 >= 1.0 {
            entry.0 -= 1.0;
            Ok(())
        } else {
            Err(
                MatrixError::limit_exceeded("Too many requests. Please try again later.", None)
                    .into(),
            )
        }
    }
}

fn extract_ip(req: &Request) -> Option<String> {
    match req.remote_addr() {
        salvo::conn::SocketAddr::IPv4(a) => Some(a.ip().to_string()),
        salvo::conn::SocketAddr::IPv6(a) => Some(a.ip().to_string()),
        _ => None,
    }
}

static LOGIN_LIMITER: LazyLock<RateLimiter> = LazyLock::new(RateLimiter::new);
static REGISTRATION_LIMITER: LazyLock<RateLimiter> = LazyLock::new(RateLimiter::new);
static PASSWORD_LIMITER: LazyLock<RateLimiter> = LazyLock::new(RateLimiter::new);
static MESSAGE_LIMITER: LazyLock<RateLimiter> = LazyLock::new(RateLimiter::new);

#[handler]
pub async fn limit_rate_login(req: &mut Request) -> AppResult<()> {
    if let Some(ip) = extract_ip(req) {
        LOGIN_LIMITER.check(&ip, &crate::config::get().rc_login)?;
    }
    Ok(())
}

#[handler]
pub async fn limit_rate_registration(req: &mut Request) -> AppResult<()> {
    if let Some(ip) = extract_ip(req) {
        REGISTRATION_LIMITER.check(&ip, &crate::config::get().rc_registration)?;
    }
    Ok(())
}

#[handler]
pub async fn limit_rate_password(req: &mut Request) -> AppResult<()> {
    if let Some(ip) = extract_ip(req) {
        PASSWORD_LIMITER.check(&ip, &crate::config::get().rc_password)?;
    }
    Ok(())
}

/// General rate limiter for authenticated API endpoints.
#[handler]
pub async fn limit_rate(req: &mut Request) -> AppResult<()> {
    if let Some(ip) = extract_ip(req) {
        MESSAGE_LIMITER.check(&ip, &crate::config::get().rc_message)?;
    }
    Ok(())
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
