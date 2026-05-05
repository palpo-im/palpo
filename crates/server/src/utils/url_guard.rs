//! Boundary checks for outbound HTTP destinations supplied by clients.
//!
//! Two layers protect against SSRF:
//!
//! 1. The reqwest clients used for user-influenced outbound traffic (push
//!    gateways, URL preview, federation) install a CIDR-filtering DNS
//!    resolver (see [`crate::sending::resolver::Resolver::new_with_cidr_denylist`]).
//!    That resolver filters DNS results against `config::cidr_range_denylist`
//!    at connect time, which prevents both ordinary hostnames that resolve
//!    to internal addresses and DNS-rebinding attempts.
//!
//! 2. This module supplies fast pre-flight checks: scheme allowlist and
//!    rejection of IP-literal hosts that fall inside the denylist (those
//!    bypass DNS resolution entirely, so the resolver never sees them).
//!
//! Both layers are needed: the resolver alone misses IP-literal hosts,
//! and the pre-flight check alone is racy with the actual connect.

use ipaddress::IPAddress;
use url::Url;

use crate::AppResult;
use crate::core::MatrixError;

/// Reject schemes outside `http`/`https`. This refuses gopher://, file://,
/// ftp://, ssh://, etc., which can otherwise smuggle requests via reqwest's
/// underlying http client behavior or proxy stack.
pub fn ensure_http_scheme(url: &Url) -> AppResult<()> {
    match url.scheme() {
        "http" | "https" => Ok(()),
        other => Err(MatrixError::forbidden(
            format!("URL scheme '{other}' is not allowed for outbound requests"),
            None,
        )
        .into()),
    }
}

/// Reject URLs whose host is an IP literal that falls inside the configured
/// `ip_range_denylist`. IP-literal hosts skip DNS resolution and therefore
/// bypass the CIDR filter installed on the reqwest resolver, so they must be
/// caught here.
pub fn ensure_ip_literal_host_allowed(url: &Url) -> AppResult<()> {
    let conf = crate::config::get();
    if conf.ip_range_denylist.is_empty() {
        return Ok(());
    }
    let Some(host) = url.host_str() else {
        return Err(MatrixError::forbidden("URL has no host", None).into());
    };
    if let Ok(ip) = IPAddress::parse(host)
        && !crate::config::valid_cidr_range(&ip)
    {
        return Err(MatrixError::forbidden(
            "Requesting from this address is forbidden.",
            None,
        )
        .into());
    }
    Ok(())
}

/// Pre-flight boundary check for an outbound URL supplied by a client:
/// scheme allowlist + IP-literal denylist. Hostnames that need DNS
/// resolution are validated at connect time by the safe resolver.
pub fn ensure_safe_outbound_url(url: &Url) -> AppResult<()> {
    ensure_http_scheme(url)?;
    ensure_ip_literal_host_allowed(url)?;
    Ok(())
}
