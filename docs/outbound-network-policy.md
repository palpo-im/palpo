# Outbound network policy

Palpo's federation, server discovery, and push transports validate DNS destinations and every redirect against `ip_range_denylist`. They currently require direct HTTP(S) connections. They do not support forward proxies.

Startup rejects an active `proxy.global` configuration or any nonempty `proxy.by_domain` rule list, even if its current domain patterns would match no requests. Omitting `proxy`, setting `proxy = "none"`, or using an empty domain-rule list is accepted when no forward-proxy environment variable is active.

Unset `HTTP_PROXY`, `HTTPS_PROXY`, and `ALL_PROXY`, including their lowercase or mixed-case equivalents, before starting Palpo. Empty values are inactive. `NO_PROXY` alone is allowed; it does not make an active forward proxy supported. Proxy rejection messages do not print proxy URLs or credentials. The guarded transport builder repeats these checks defensively.

These checks run before CLI configuration publication and before embedded-server global state or resources are initialized. Palpo fails startup instead of silently routing around a configured proxy. Operators whose policy requires proxy-only egress must keep Palpo stopped until a transport with equivalent destination validation through that proxy is available; removing the setting is appropriate only when direct egress is authorized.

The CIDR denylist remains configurable, including an explicitly empty list for controlled private federation deployments. Network firewalls remain useful defense in depth. This behavior does not claim support for proxy-resolved destinations or proxy authentication.
