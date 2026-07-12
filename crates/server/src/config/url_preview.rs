use std::net::IpAddr;

use either::Either;
use serde::{Deserialize, Serialize};

use crate::macros::config_example;

#[config_example(filename = "palpo-example.toml", section = "url_preview")]
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct UrlPreviewConfig {
    /// Optional IP address or network interface-name to bind as the source of
    /// URL preview requests. If not set, it will not bind to a specific
    /// address or interface.
    ///
    /// Interface names only supported on Linux, Android, and Fuchsia platforms;
    /// all other platforms can specify the IP address. To list the interfaces
    /// on your system, use the command `ip link show`.
    ///
    /// example: `"eth0"` or `"1.2.3.4"`
    ///
    /// default:
    #[serde(default, with = "either::serde_untagged_optional")]
    pub bound_interface: Option<Either<IpAddr, String>>,

    /// Vector list of domains allowed to send requests to for URL previews.
    ///
    /// This is a *contains* match, not an explicit match. Putting "google.com"
    /// will match "https://google.com" and
    /// "http://mymaliciousdomainexamplegoogle.com" Setting this to "*" will
    /// allow all URL previews. Please note that this opens up significant
    /// attack surface to your server, you are expected to be aware of the risks
    /// by doing so.
    ///
    /// default: []
    #[serde(default)]
    pub domain_contains_allowlist: Vec<String>,

    /// Vector list of explicit domains allowed to send requests to for URL
    /// previews.
    ///
    /// This is an *explicit* match, not a contains match. Putting "google.com"
    /// will match "https://google.com", "http://google.com", but not
    /// "https://mymaliciousdomainexamplegoogle.com". Setting this to "*" will
    /// allow all URL previews. Please note that this opens up significant
    /// attack surface to your server, you are expected to be aware of the risks
    /// by doing so.
    ///
    /// default: []
    #[serde(default)]
    pub domain_explicit_allowlist: Vec<String>,

    /// Vector list of explicit domains not allowed to send requests to for URL
    /// previews.
    ///
    /// This is an *explicit* match, not a contains match. Putting "google.com"
    /// will match "https://google.com", "http://google.com", but not
    /// "https://mymaliciousdomainexamplegoogle.com". The denylist is checked
    /// first before allowlist. Setting this to "*" will not do anything.
    ///
    /// default: []
    #[serde(default)]
    pub domain_explicit_denylist: Vec<String>,

    /// Vector list of URLs allowed to send requests to for URL previews.
    ///
    /// Note that this is a *contains* match, not an explicit match. Putting
    /// "google.com" will match "https://google.com/",
    /// "https://google.com/url?q=https://mymaliciousdomainexample.com", and
    /// "https://mymaliciousdomainexample.com/hi/google.com" Setting this to "*"
    /// will allow all URL previews. Please note that this opens up significant
    /// attack surface to your server, you are expected to be aware of the risks
    /// by doing so.
    ///
    /// default: []
    #[serde(default)]
    pub url_contains_allowlist: Vec<String>,

    /// Maximum amount of bytes allowed in a URL preview body size when
    /// spidering. Defaults to 256KB in bytes.
    ///
    /// default: 256000
    #[serde(default = "default_max_spider_size")]
    pub max_spider_size: usize,

    /// Maximum number of bytes accepted for an image fetched while building a
    /// URL preview. `Content-Length` is checked early and the response stream
    /// is also counted so chunked responses cannot bypass this limit.
    ///
    /// default: 10000000
    #[serde(default = "default_max_image_size")]
    pub max_image_size: usize,

    /// Option to decide whether you would like to run the domain allowlist
    /// checks (contains and explicit) on the root domain or not. Does not apply
    /// to URL contains allowlist. Defaults to false.
    ///
    /// Example usecase: If this is enabled and you have "wikipedia.org" allowed
    /// in the explicit and/or contains domain allowlist, it will allow all
    /// subdomains under "wikipedia.org" such as "en.m.wikipedia.org" as the
    /// root domain is checked and matched. Useful if the domain contains
    /// allowlist is still too broad for you but you still want to allow all the
    /// subdomains under a root domain.
    #[serde(default)]
    pub check_root_domain: bool,
}

impl Default for UrlPreviewConfig {
    fn default() -> Self {
        Self {
            bound_interface: None,
            domain_contains_allowlist: Vec::new(),
            domain_explicit_allowlist: Vec::new(),
            domain_explicit_denylist: Vec::new(),
            url_contains_allowlist: Vec::new(),
            max_spider_size: default_max_spider_size(),
            max_image_size: default_max_image_size(),
            check_root_domain: false,
        }
    }
}

impl UrlPreviewConfig {
    pub fn check(&self) {
        if self.domain_contains_allowlist.contains(&"*".to_owned()) {
            warn!(
                "All URLs are allowed for URL previews via setting \
                 \"url_preview.domain_contains_allowlist\" to \"*\". This opens up significant \
                 attack surface to your server. You are expected to be aware of the risks by doing \
                 this."
            );
        }
        if self.domain_explicit_allowlist.contains(&"*".to_owned()) {
            warn!(
                "All URLs are allowed for URL previews via setting \
                 \"url_preview.domain_explicit_allowlist\" to \"*\". This opens up significant \
                 attack surface to your server. You are expected to be aware of the risks by doing \
                 this."
            );
        }
        if self.url_contains_allowlist.contains(&"*".to_owned()) {
            warn!(
                "All URLs are allowed for URL previews via setting \
                 \"url_preview.url_contains_allowlist\" to \"*\". This opens up significant attack \
                 surface to your server. You are expected to be aware of the risks by doing this."
            );
        }
    }
}

fn default_max_spider_size() -> usize {
    256_000 // 256KB
}

fn default_max_image_size() -> usize {
    10_000_000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_size_limits_are_nonzero() {
        let config = UrlPreviewConfig::default();
        assert_eq!(config.max_spider_size, 256_000);
        assert_eq!(config.max_image_size, 10_000_000);
    }
}
