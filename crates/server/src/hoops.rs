use salvo::http::{ParseError, ResBody};
use salvo::prelude::*;
use salvo::size_limiter;

use crate::AppResult;
use crate::core::MatrixError;

mod auth;
pub use auth::*;

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
    headers.insert("Access-Control-Allow-Origin", "*".parse().unwrap());
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
    headers.insert("Access-Control-Allow-Credentials", "true".parse().unwrap());
    headers.insert(
        "Content-Security-Policy",
        "frame-ancestors 'self'".parse().unwrap(),
    );
    ctrl.call_next(req, depot, res).await;
    // headers.insert("Cross-Origin-Embedder-Policy", "require-corp".parse().unwrap());
    // headers.insert("Cross-Origin-Opener-Policy", "same-origin".parse().unwrap());
}

#[handler]
pub async fn limit_rate() -> AppResult<()> {
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
