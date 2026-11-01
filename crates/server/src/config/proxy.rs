use reqwest::{Proxy, Url};
use serde::Deserialize;

use crate::AppResult;

/// Outbound proxy configuration is currently unsupported by the restricted
/// federation/discovery/push transports. Startup rejects `Global` and nonempty
/// `ByDomain` rules rather than silently bypassing an operator's egress policy.
/// `None` and empty domain rules are accepted only when the process has no
/// active HTTP_PROXY, HTTPS_PROXY, or ALL_PROXY variables (case-insensitive).
/// See `docs/outbound-network-policy.md` for deployment requirements.
#[derive(Default, Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyConfig {
    #[default]
    None,
    Global {
        #[serde(deserialize_with = "crate::utils::deserialize_from_str")]
        url: Url,
    },
    ByDomain(Vec<PartialProxyConfig>),
}
impl ProxyConfig {
    pub fn to_proxy(&self) -> AppResult<Option<Proxy>> {
        Ok(match self.clone() {
            ProxyConfig::None => None,
            ProxyConfig::Global { url } => Some(Proxy::all(url)?),
            ProxyConfig::ByDomain(proxies) => Some(Proxy::custom(move |url| {
                proxies.iter().find_map(|proxy| proxy.for_url(url)).cloned() // first matching proxy
            })),
        })
    }
}

/// Fail before publishing configuration or opening resources: a proxy may
/// resolve the destination itself and defeat local DNS address validation.
/// Only names/presence are inspected; credentials never enter error messages.
pub(crate) fn validate_restricted_outbound_proxy(proxy: Option<&ProxyConfig>) -> AppResult<()> {
    let environment_proxy = std::env::vars_os().any(|(name, value)| {
        !value.is_empty() && name.to_str().is_some_and(is_forward_proxy_variable)
    });
    validate_restricted_proxy_policy(proxy, environment_proxy)
}

fn is_forward_proxy_variable(name: &str) -> bool {
    ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY"]
        .iter()
        .any(|expected| name.eq_ignore_ascii_case(expected))
}

fn validate_restricted_proxy_policy(
    proxy: Option<&ProxyConfig>,
    environment_proxy: bool,
) -> AppResult<()> {
    let configured = match proxy {
        None | Some(ProxyConfig::None) => false,
        Some(ProxyConfig::ByDomain(rules)) => !rules.is_empty(),
        Some(ProxyConfig::Global { .. }) => true,
    };
    if configured {
        return Err(crate::AppError::internal(
            "Outbound proxy configuration is unsupported by restricted HTTP transports; remove active proxy rules before starting Palpo. Direct outbound connections must be permitted by deployment policy.",
        ));
    }
    if environment_proxy {
        return Err(crate::AppError::internal(
            "Restricted outbound HTTP cannot honor proxy environment variables; unset HTTP_PROXY, HTTPS_PROXY and ALL_PROXY (including lowercase forms) before starting Palpo. NO_PROXY does not enable proxy support.",
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize)]
pub struct PartialProxyConfig {
    #[serde(deserialize_with = "crate::utils::deserialize_from_str")]
    url: Url,
    #[serde(default)]
    include: Vec<WildCardedDomain>,
    #[serde(default)]
    exclude: Vec<WildCardedDomain>,
}
impl PartialProxyConfig {
    pub fn for_url(&self, url: &Url) -> Option<&Url> {
        let domain = url.domain()?;
        let mut included_because = None; // most specific reason it was included
        let mut excluded_because = None; // most specific reason it was excluded
        if self.include.is_empty() {
            // treat empty include list as `*`
            included_because = Some(&WildCardedDomain::WildCard)
        }
        for wc_domain in &self.include {
            if wc_domain.matches(domain) {
                match included_because {
                    Some(prev) if !wc_domain.more_specific_than(prev) => (),
                    _ => included_because = Some(wc_domain),
                }
            }
        }
        for wc_domain in &self.exclude {
            if wc_domain.matches(domain) {
                match excluded_because {
                    Some(prev) if !wc_domain.more_specific_than(prev) => (),
                    _ => excluded_because = Some(wc_domain),
                }
            }
        }
        match (included_because, excluded_because) {
            (Some(a), Some(b)) if a.more_specific_than(b) => Some(&self.url), // included for a
            // more specific
            // reason than
            // excluded
            (Some(_), None) => Some(&self.url),
            _ => None,
        }
    }
}

/// A domain name, that optionally allows a * as its first subdomain.
#[derive(Clone, Debug)]
pub enum WildCardedDomain {
    WildCard,
    WildCarded(String),
    Exact(String),
}
impl WildCardedDomain {
    pub fn matches(&self, domain: &str) -> bool {
        match self {
            WildCardedDomain::WildCard => true,
            WildCardedDomain::WildCarded(d) => domain.ends_with(d),
            WildCardedDomain::Exact(d) => domain == d,
        }
    }
    pub fn more_specific_than(&self, other: &Self) -> bool {
        match (self, other) {
            (WildCardedDomain::WildCard, WildCardedDomain::WildCard) => false,
            (_, WildCardedDomain::WildCard) => true,
            (WildCardedDomain::Exact(a), WildCardedDomain::WildCarded(_)) => other.matches(a),
            (WildCardedDomain::WildCarded(a), WildCardedDomain::WildCarded(b)) => {
                a != b && a.ends_with(b)
            }
            _ => false,
        }
    }
}
impl std::str::FromStr for WildCardedDomain {
    type Err = std::convert::Infallible;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // maybe do some domain validation?
        Ok(if s.starts_with("*.") {
            WildCardedDomain::WildCarded(s[1..].to_owned())
        } else if s == "*" {
            WildCardedDomain::WildCarded("".to_owned())
        } else {
            WildCardedDomain::Exact(s.to_owned())
        })
    }
}
impl<'de> Deserialize<'de> for WildCardedDomain {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::de::Deserializer<'de>,
    {
        crate::utils::deserialize_from_str(deserializer)
    }
}

#[cfg(test)]
mod restricted_proxy_policy_tests {
    use super::*;

    #[test]
    fn direct_policy_accepts_only_absent_disabled_or_empty_proxy_configuration() {
        for config in [
            None,
            Some(ProxyConfig::None),
            Some(ProxyConfig::ByDomain(Vec::new())),
        ] {
            assert!(validate_restricted_proxy_policy(config.as_ref(), false).is_ok());
            assert!(validate_restricted_proxy_policy(config.as_ref(), true).is_err());
        }
        let global = ProxyConfig::Global {
            url: Url::parse("http://user:secret@proxy.example:8080").unwrap(),
        };
        let partial = ProxyConfig::ByDomain(vec![PartialProxyConfig {
            url: Url::parse("http://user:secret@proxy.example:8080").unwrap(),
            include: Vec::new(),
            exclude: Vec::new(),
        }]);
        for config in [global, partial] {
            let error = validate_restricted_proxy_policy(Some(&config), false)
                .unwrap_err()
                .to_string();
            assert!(error.contains("unsupported"));
            assert!(!error.contains("secret"));
            assert!(!error.contains("proxy.example"));
        }
    }

    #[test]
    fn environment_detection_covers_case_variants_without_treating_no_proxy_as_egress() {
        for name in [
            "HTTP_PROXY",
            "https_proxy",
            "All_Proxy",
            "http_proxy",
            "HTTPS_PROXY",
            "ALL_PROXY",
        ] {
            assert!(is_forward_proxy_variable(name));
        }
        for name in ["NO_PROXY", "no_proxy", "PALPO_PROXY", "OTHER"] {
            assert!(!is_forward_proxy_variable(name));
        }
    }
}
