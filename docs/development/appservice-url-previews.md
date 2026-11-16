# Application-service URL previews

MSC4417 delegates the existing authenticated client `preview_url` request to
explicitly trusted application services. Delegation is disabled by default.

```toml
[url_preview]
domain_explicit_allowlist = ["github.com"]
appservice_providers = [{ id = "github", send_user_id = true }]
```

The registered service must also declare its URL patterns:

```yaml
namespaces:
  uk.half-shot.msc4417.preview_urls:
    - regex: 'https://github\.com/'
      exclusive: true
```

Patterns match from the start of the URL, like other AS namespaces. Providers
are tried in configured order, with at most four unique IDs. Disabled services
are excluded. An invalid preview regex is logged by service ID and ignored for
that provider; other providers and local previews remain available. Unconfigured
services receive no requests. `send_user_id` defaults
to false; enabling it discloses the authenticated requester only to that matching
provider. Trust the service with both the URL (including its query) and, when
enabled, that user's identity.

Requests use `GET /_matrix/app/unstable/uk.half-shot.msc4417/preview_url`, with
`url` and optional `user_id` query parameters, and a Bearer homeserver token.
Tokens are not put in query strings. The AS transport keeps configured proxy,
TLS and connection-pool settings. Redirects are never followed. Operator
configured service addresses may be internal; the previewed URL still passes
the normal scheme, domain and CIDR policy, including DNS checks for hostnames.
The provider remains responsible for protecting its own outbound connections.

A nonempty HTTP 200 OpenGraph object wins. Other statuses, timeout, malformed or
oversized responses try the next matching provider. `null` or `{}` means no
preview. An exclusive match ends selection: an empty success returns `{}`, and
a failed provider returns HTTP 500. Neither case falls back to another provider
or a local lookup. When no nonexclusive provider succeeds, the existing local
preview pipeline and its public cache are used.

Every provider attempt, including its body, has a two-second timeout. Destination
DNS checks also have a two-second timeout. Responses are capped at the smaller
of `max_spider_size` and 256 KB, including streamed bodies. OpenGraph data must be
a flat object; image dimensions and byte sizes are unsigned integers. `og:image`
must be a valid MXC URI. Palpo does not fetch a provider's image URL or bypass
normal media download authentication.

Delegated results never enter the shared URL-only cache. Providers can maintain
their own user-scoped caches. Delegated requests share the existing 16-request
admission bound with local previews, including requests waiting on the same URL;
excess requests return 429. Failed attempts and cancelled requests release their
admission slots. Local fallback acquires its slot after delegation releases it.

No client version flag is invented for this application-service registration
extension. The feature is activated by provider configuration, not by changing
the local preview defaults.
