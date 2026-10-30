//! Boundary checks for outbound HTTP destinations supplied by clients.
//!
//! Two layers protect against SSRF:
//!
//! 1. The reqwest clients used for user-influenced outbound traffic (push gateways, URL preview,
//!    federation) install a CIDR-filtering DNS resolver (see
//!    [`crate::sending::resolver::Resolver::new_with_cidr_denylist`]). That resolver filters DNS
//!    results against `config::cidr_range_denylist` at connect time, which prevents both ordinary
//!    hostnames that resolve to internal addresses and DNS-rebinding attempts.
//!
//! 2. This module supplies fast pre-flight checks: scheme allowlist and rejection of IP-literal
//!    hosts that fall inside the denylist (those bypass DNS resolution entirely, so the resolver
//!    never sees them).
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
    ensure_ip_literal_host_allowed_with(url, crate::config::cidr_range_denylist())
}

fn ensure_ip_literal_host_allowed_with(url: &Url, denied: &[IPAddress]) -> AppResult<()> {
    let ip = match url.host() {
        Some(url::Host::Ipv4(ip)) => Some(std::net::IpAddr::V4(ip)),
        Some(url::Host::Ipv6(ip)) => Some(std::net::IpAddr::V6(ip)),
        Some(url::Host::Domain(_)) => None,
        None => return Err(MatrixError::forbidden("URL has no host", None).into()),
    };
    if let Some(ip) = ip {
        let ip = IPAddress::parse(ip.to_string())
            .map_err(|_| MatrixError::forbidden("Invalid outbound IP address", None))?;
        if denied.iter().any(|network| network.includes(&ip)) {
            return Err(
                MatrixError::forbidden("Requesting from this address is forbidden.", None).into(),
            );
        }
    }
    Ok(())
}

/// Validate every redirect, including IP literals which bypass the DNS resolver.
/// Callers must also validate the initial URL and install the filtering resolver.
pub fn safe_redirect_policy(max_hops: usize) -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(move |attempt| {
        if redirect_destination_allowed(
            attempt.url(),
            attempt.previous().len(),
            max_hops,
            crate::config::cidr_range_denylist(),
        ) {
            attempt.follow()
        } else {
            attempt.stop()
        }
    })
}

fn redirect_destination_allowed(
    url: &Url,
    previous: usize,
    max_hops: usize,
    denied: &[IPAddress],
) -> bool {
    previous <= max_hops
        && ensure_http_scheme(url).is_ok()
        && ensure_ip_literal_host_allowed_with(url, denied).is_ok()
}

/// Pre-flight boundary check for an outbound URL supplied by a client:
/// scheme allowlist + IP-literal denylist. Hostnames that need DNS
/// resolution are validated at connect time by the safe resolver.
pub fn ensure_safe_outbound_url(url: &Url) -> AppResult<()> {
    ensure_http_scheme(url)?;
    ensure_ip_literal_host_allowed(url)?;
    Ok(())
}

#[cfg(test)]
mod outbound_transport_policy_tests {
    use super::*;

    #[test]
    fn redirect_policy_rejects_private_ipv4_ipv6_schemes_and_excess_hops() {
        let denied: Vec<_> = [
            "127.0.0.0/8",
            "10.0.0.0/8",
            "169.254.0.0/16",
            "::1/128",
            "fc00::/7",
        ]
        .into_iter()
        .map(|cidr| IPAddress::parse(cidr).unwrap())
        .collect();
        for target in [
            "http://127.0.0.1/a",
            "http://10.0.0.1/a",
            "http://169.254.169.254/a",
            "http://[::1]/a",
            "https://[fd00::1]/a",
            "file:///a",
        ] {
            assert!(
                !redirect_destination_allowed(&Url::parse(target).unwrap(), 1, 2, &denied),
                "{target}"
            );
        }
        let public = Url::parse("https://public.example/delegation").unwrap();
        assert!(redirect_destination_allowed(&public, 1, 2, &denied));
        assert!(redirect_destination_allowed(&public, 2, 2, &denied));
        assert!(!redirect_destination_allowed(&public, 3, 2, &denied));
        assert!(redirect_destination_allowed(
            &Url::parse("http://127.0.0.1/a").unwrap(),
            1,
            2,
            &[]
        ));
    }
}
