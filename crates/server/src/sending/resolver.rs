use std::error::Error as StdError;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::{Arc, RwLock};
use std::{future, iter};

use futures_util::FutureExt;
use hyper_util::client::legacy::connect::dns::{GaiResolver, Name as HyperName};
use ipaddress::IPAddress;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use tower_service::Service as TowerService;

use crate::TlsNameMap;

pub const MXC_LENGTH: usize = 32;
pub const DEVICE_ID_LENGTH: usize = 10;
pub const TOKEN_LENGTH: usize = 32;
pub const SESSION_ID_LENGTH: usize = 32;
pub const AUTO_GEN_PASSWORD_LENGTH: usize = 15;
pub const RANDOM_USER_ID_LENGTH: usize = 10;

pub struct Resolver {
    inner: GaiResolver,
    overrides: Arc<RwLock<TlsNameMap>>,
    enforce_cidr_denylist: bool,
}

impl Resolver {
    pub fn new(overrides: Arc<RwLock<TlsNameMap>>) -> Self {
        Resolver {
            inner: GaiResolver::new(),
            overrides,
            enforce_cidr_denylist: false,
        }
    }

    /// Build a resolver that filters out any addresses that fall inside the
    /// configured `ip_range_denylist`. This protects outbound HTTP from SSRF
    /// to internal/loopback/link-local addresses, including DNS-rebinding
    /// attempts where a public hostname resolves to a private IP.
    ///
    /// Note: IP-literal hosts skip DNS resolution entirely, so callers must
    /// pre-validate IP-literal URLs (see `crate::utils::url_guard`).
    pub fn new_with_cidr_denylist(overrides: Arc<RwLock<TlsNameMap>>) -> Self {
        Resolver {
            inner: GaiResolver::new(),
            overrides,
            enforce_cidr_denylist: true,
        }
    }
}

fn filter_denied_addrs(addrs: Addrs) -> Addrs {
    if crate::config::get().ip_range_denylist.is_empty() {
        return addrs;
    }
    let allowed: Vec<SocketAddr> = addrs
        .filter(|addr| {
            IPAddress::parse(addr.ip().to_string())
                .map(|ip| crate::config::valid_cidr_range(&ip))
                .unwrap_or(true)
        })
        .collect();
    Box::new(allowed.into_iter())
}

impl Resolve for Resolver {
    fn resolve(&self, name: Name) -> Resolving {
        let enforce = self.enforce_cidr_denylist;
        let resolving: Resolving = self
            .overrides
            .read()
            .unwrap()
            .get(name.as_str())
            .and_then(|(override_name, port)| {
                override_name.first().map(|first_name| {
                    let x: Box<dyn Iterator<Item = SocketAddr> + Send> =
                        Box::new(iter::once(SocketAddr::new(*first_name, *port)));
                    let x: Resolving = Box::pin(future::ready(Ok(x)));
                    x
                })
            })
            .unwrap_or_else(|| {
                let this = &mut self.inner.clone();
                Box::pin(
                    TowerService::<HyperName>::call(
                        this,
                        // Beautiful hack, please remove this in the future.
                        HyperName::from_str(name.as_str())
                            .expect("reqwest Name is just wrapper for hyper-util Name"),
                    )
                    .map(|result| {
                        result
                            .map(|addrs| -> Addrs { Box::new(addrs) })
                            .map_err(|err| -> Box<dyn StdError + Send + Sync> { Box::new(err) })
                    }),
                )
            });

        if !enforce {
            return resolving;
        }

        Box::pin(async move {
            let addrs = resolving.await?;
            Ok(filter_denied_addrs(addrs))
        })
    }
}
