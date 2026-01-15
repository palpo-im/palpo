use std::collections::HashMap;
use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use salvo::http::headers::{CacheControl, Header};

use crate::core::identifiers::*;
use crate::{LazyRwLock, sending};

type WellKnownMap = HashMap<OwnedServerName, DestinationResponse>;
pub static ACTUAL_DESTINATION_CACHE: LazyRwLock<WellKnownMap> = LazyLock::new(Default::default); // actual_destination, host
pub trait GetUrlOrigin {
    fn origin(&self) -> impl Future<Output = String>;
}

impl GetUrlOrigin for OwnedServerName {
    async fn origin(&self) -> String {
        AsRef::<ServerName>::as_ref(self).origin().await
    }
}
impl GetUrlOrigin for ServerName {
    async fn origin(&self) -> String {
        let cached_result = crate::ACTUAL_DESTINATION_CACHE
            .read()
            .unwrap()
            .get(self)
            .cloned();

        let actual_destination = if let Some(DestinationResponse {
            actual_destination,
            dest_type,
        }) = cached_result
        {
            match dest_type {
                DestType::IsIpOrHasPort => actual_destination,
                DestType::LookupFailed {
                    well_known_retry,
                    well_known_backoff_mins,
                } => {
                    if well_known_retry < Instant::now() {
                        find_actual_destination(self, None, false, Some(well_known_backoff_mins))
                            .await
                    } else {
                        actual_destination
                    }
                }

                DestType::WellKnown { expires } => {
                    if expires < Instant::now() {
                        find_actual_destination(self, None, false, None).await
                    } else {
                        actual_destination
                    }
                }
                DestType::WellKnownSrv {
                    srv_expires,
                    well_known_expires,
                    well_known_host,
                } => {
                    if well_known_expires < Instant::now() {
                        find_actual_destination(self, None, false, None).await
                    } else if srv_expires < Instant::now() {
                        find_actual_destination(self, Some(well_known_host), true, None).await
                    } else {
                        actual_destination
                    }
                }
                DestType::Srv {
                    well_known_retry,
                    well_known_backoff_mins,
                    srv_expires,
                } => {
                    if well_known_retry < Instant::now() {
                        find_actual_destination(self, None, false, Some(well_known_backoff_mins))
                            .await
                    } else if srv_expires < Instant::now() {
                        find_actual_destination(self, None, true, Some(well_known_backoff_mins))
                            .await
                    } else {
                        actual_destination
                    }
                }
            }
        } else {
            find_actual_destination(self, None, false, None).await
        };
        actual_destination.clone().into_https_string()
    }
}

/// Wraps either an literal IP address plus port, or a hostname plus complement
/// (colon-plus-port if it was specified).
///
/// Note: A `FedDest::Named` might contain an IP address in string form if there
/// was no port specified to construct a SocketAddr with.
///
/// # Examples:
/// ```rust
/// # use palpo::api::server_server::FedDest;
/// # fn main() -> Result<(), std::net::AddrParseError> {
/// FedDest::Literal("198.51.100.3:8448".parse()?);
/// FedDest::Literal("[2001:db8::4:5]:443".parse()?);
/// FedDest::Named("matrix.example.org".to_owned(), "".to_owned());
/// FedDest::Named("matrix.example.org".to_owned(), ":8448".to_owned());
/// FedDest::Named("198.51.100.5".to_owned(), "".to_owned());
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FedDest {
    Literal(SocketAddr),
    Named(String, String),
}

impl FedDest {
    fn into_https_string(self) -> String {
        match self {
            Self::Literal(addr) => format!("https://{addr}"),
            Self::Named(host, port) => format!("https://{host}{port}"),
        }
    }

    fn into_uri_string(self) -> String {
        match self {
            Self::Literal(addr) => addr.to_string(),
            Self::Named(host, ref port) => host + port,
        }
    }

    fn hostname(&self) -> String {
        match &self {
            Self::Literal(addr) => addr.ip().to_string(),
            Self::Named(host, _) => host.clone(),
        }
    }

    fn port(&self) -> Option<u16> {
        match &self {
            Self::Literal(addr) => Some(addr.port()),
            Self::Named(_, port) => port[1..].parse().ok(),
        }
    }
}

fn get_ip_with_port(destination_str: &str) -> Option<FedDest> {
    if let Ok(destination) = destination_str.parse::<SocketAddr>() {
        Some(FedDest::Literal(destination))
    } else if let Ok(ip_addr) = destination_str.parse::<IpAddr>() {
        Some(FedDest::Literal(SocketAddr::new(ip_addr, 8448)))
    } else {
        None
    }
}

fn add_port_to_hostname(destination_str: &str) -> FedDest {
    let (host, port) = match destination_str.find(':') {
        None => (destination_str, ":8448"),
        Some(pos) => destination_str.split_at(pos),
    };
    FedDest::Named(host.to_owned(), port.to_owned())
}

#[derive(Clone)]
pub struct DestinationResponse {
    pub actual_destination: FedDest,
    pub dest_type: DestType,
}

#[derive(Debug, Clone)]
pub enum DestType {
    WellKnownSrv {
        srv_expires: Instant,
        well_known_expires: Instant,
        well_known_host: String,
    },
    WellKnown {
        expires: Instant,
    },
    Srv {
        srv_expires: Instant,
        well_known_retry: Instant,
        well_known_backoff_mins: u16,
    },
    IsIpOrHasPort,
    LookupFailed {
        well_known_retry: Instant,
        well_known_backoff_mins: u16,
    },
}

/// Implemented according to the specification at <https://spec.matrix.org/v1.11/server-server-api/#resolving-server-names>
/// Numbers in comments below refer to bullet points in linked section of specification
async fn find_actual_destination(
    destination: &'_ ServerName,
    // The host used to potentially lookup SRV records against, only used when only_request_srv is
    // true
    well_known_dest: Option<String>,
    // Should be used when only the SRV lookup has expired
    only_request_srv: bool,
    // The backoff time for the last well known failure, if any
    well_known_backoff_mins: Option<u16>,
) -> FedDest {
    debug!("Finding actual destination for {destination}");
    let destination_str = destination.to_string();
    let next_backoff_mins = well_known_backoff_mins
        // Errors are recommended to be cached for up to an hour
        .map(|mins| (mins * 2).min(60))
        .unwrap_or(1);

    let (actual_destination, dest_type) = if only_request_srv {
        let destination_str = well_known_dest.unwrap_or(destination_str);
        let (dest, expires) = get_srv_destination(destination_str).await;
        let well_known_retry =
            Instant::now() + Duration::from_secs((60 * next_backoff_mins).into());
        (
            dest,
            if let Some(expires) = expires {
                DestType::Srv {
                    well_known_backoff_mins: next_backoff_mins,
                    srv_expires: expires,

                    well_known_retry,
                }
            } else {
                DestType::LookupFailed {
                    well_known_retry,
                    well_known_backoff_mins: next_backoff_mins,
                }
            },
        )
    } else {
        match get_ip_with_port(&destination_str) {
            Some(host_port) => {
                debug!("1: IP literal with provided or default port");
                (host_port, DestType::IsIpOrHasPort)
            }
            None => {
                if let Some(pos) = destination_str.find(':') {
                    debug!("2: Hostname with included port");
                    let (host, port) = destination_str.split_at(pos);
                    (
                        FedDest::Named(host.to_owned(), port.to_owned()),
                        DestType::IsIpOrHasPort,
                    )
                } else {
                    debug!("Requesting well known for {destination_str}");
                    match request_well_known(destination_str.as_str()).await {
                        Some((delegated_hostname, timestamp)) => {
                            debug!("3: A .well-known file is available");
                            match get_ip_with_port(&delegated_hostname) {
                                // 3.1: IP literal in .well-known file
                                Some(host_and_port) => {
                                    (host_and_port, DestType::WellKnown { expires: timestamp })
                                }
                                None => {
                                    if let Some(pos) = delegated_hostname.find(':') {
                                        debug!("3.2: Hostname with port in .well-known file");
                                        let (host, port) = delegated_hostname.split_at(pos);
                                        (
                                            FedDest::Named(host.to_owned(), port.to_owned()),
                                            DestType::WellKnown { expires: timestamp },
                                        )
                                    } else {
                                        debug!("Delegated hostname has no port in this branch");
                                        let (dest, srv_expires) =
                                            get_srv_destination(delegated_hostname.clone()).await;
                                        (
                                            dest,
                                            if let Some(srv_expires) = srv_expires {
                                                DestType::WellKnownSrv {
                                                    srv_expires,
                                                    well_known_expires: timestamp,
                                                    well_known_host: delegated_hostname,
                                                }
                                            } else {
                                                DestType::WellKnown { expires: timestamp }
                                            },
                                        )
                                    }
                                }
                            }
                        }
                        None => {
                            debug!("4: No .well-known or an error occured");
                            let (dest, expires) = get_srv_destination(destination_str).await;
                            let well_known_retry = Instant::now()
                                + Duration::from_secs((60 * next_backoff_mins).into());
                            (
                                dest,
                                if let Some(expires) = expires {
                                    DestType::Srv {
                                        srv_expires: expires,
                                        well_known_retry,
                                        well_known_backoff_mins: next_backoff_mins,
                                    }
                                } else {
                                    DestType::LookupFailed {
                                        well_known_retry,
                                        well_known_backoff_mins: next_backoff_mins,
                                    }
                                },
                            )
                        }
                    }
                }
            }
        }
    };

    debug!("Actual destination: {actual_destination:?}");

    let response = DestinationResponse {
        actual_destination,
        dest_type,
    };

    if let Ok(mut cache) = crate::ACTUAL_DESTINATION_CACHE.write() {
        cache.insert(destination.to_owned(), response.clone());
    }

    response.actual_destination
}

/// Looks up the SRV records for federation usage
///
/// If no timestamp is returned, that means no SRV record was found
async fn get_srv_destination(delegated_hostname: String) -> (FedDest, Option<Instant>) {
    if let Some((hostname_override, timestamp)) = query_srv_record(&delegated_hostname).await {
        debug!("SRV lookup successful");
        let force_port = hostname_override.port();

        if let Ok(override_ip) = crate::dns_resolver()
            .lookup_ip(hostname_override.hostname())
            .await
        {
            crate::TLS_NAME_OVERRIDE.write().unwrap().insert(
                delegated_hostname.clone(),
                (override_ip.iter().collect(), force_port.unwrap_or(8448)),
            );
        } else {
            // Removing in case there was previously a SRV record
            crate::TLS_NAME_OVERRIDE
                .write()
                .unwrap()
                .remove(&delegated_hostname);
            warn!("Using SRV record, but could not resolve to IP");
        }

        if let Some(port) = force_port {
            (
                FedDest::Named(delegated_hostname, format!(":{port}")),
                Some(timestamp),
            )
        } else {
            (add_port_to_hostname(&delegated_hostname), Some(timestamp))
        }
    } else {
        // Removing in case there was previously a SRV record
        crate::TLS_NAME_OVERRIDE
            .write()
            .unwrap()
            .remove(&delegated_hostname);
        debug!("No SRV records found");
        (add_port_to_hostname(&delegated_hostname), None)
    }
}

async fn query_given_srv_record(record: &str) -> Option<(FedDest, Instant)> {
    crate::dns_resolver()
        .srv_lookup(record)
        .await
        .map(|srv| {
            srv.iter().next().map(|result| {
                (
                    FedDest::Named(
                        result.target().to_string().trim_end_matches('.').to_owned(),
                        format!(":{}", result.port()),
                    ),
                    srv.as_lookup().valid_until(),
                )
            })
        })
        .unwrap_or(None)
}

async fn query_srv_record(hostname: &'_ str) -> Option<(FedDest, Instant)> {
    let hostname = hostname.trim_end_matches('.');

    if let Some(host_port) = query_given_srv_record(&format!("_matrix-fed._tcp.{hostname}.")).await
    {
        Some(host_port)
    } else {
        query_given_srv_record(&format!("_matrix._tcp.{hostname}.")).await
    }
}

async fn request_well_known(destination: &str) -> Option<(String, Instant)> {
    let response = sending::default_client()
        .get(format!("https://{destination}/.well-known/matrix/server"))
        .send()
        .await;
    debug!("Got well known response");
    let response = match response {
        Err(e) => {
            debug!("Well known error: {e:?}");
            return None;
        }
        Ok(r) => r,
    };

    let mut headers = response.headers().values();

    let cache_for = CacheControl::decode(&mut headers)
        .ok()
        .and_then(|cc| {
            // Servers should respect the cache control headers present on the response, or use a sensible default when headers are not present.
            if cc.no_store() || cc.no_cache() {
                Some(Duration::ZERO)
            } else {
                cc.max_age()
                    // Servers should additionally impose a maximum cache time for responses: 48 hours is recommended.
                    .map(|age| age.min(Duration::from_secs(60 * 60 * 48)))
            }
        })
        // The recommended sensible default is 24 hours.
        .unwrap_or_else(|| Duration::from_secs(60 * 60 * 24));

    let text = response.text().await;
    debug!("Got well known response text");

    let host = || {
        let body: serde_json::Value = serde_json::from_str(&text.ok()?).ok()?;
        body.get("m.server")?.as_str().map(ToOwned::to_owned)
    };

    host().map(|host| (host, Instant::now() + cache_for))
}
