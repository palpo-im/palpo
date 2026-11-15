# Trusted SSO client redirects

Local OIDC SSO now rejects client callback destinations unless an operator
explicitly trusts them. Before upgrading a server with `oidc.enable = true`,
configure the Matrix clients that may receive login tokens:

```toml
[oidc]
enable = true
sso_allowed_redirect_origins = [
    "https://chat.example.org",
    "http://127.0.0.1",
    "http://[::1]",
    "element://vector/webapp",
]
```

HTTP(S) entries are complete origins: scheme, hostname, and optional port, with
no callback path, credentials, query, or fragment. Matching is exact, so
`https://chat.example.org.evil.test` and an unlisted port do not match. A
loopback origin without an explicit port permits desktop clients' ephemeral
loopback ports. An explicitly configured port remains fixed, including the default
ports `:80` and `:443`. Add only origins whose applications you trust with account access.

For native applications, register the full hierarchical callback URI. Its
scheme, authority, port, and path must match exactly; trusting one callback does
not trust other hosts or paths using the same scheme. Per-login query parameters
are preserved. Opaque URLs, URL credentials, and native callback fragments are
rejected. The example is illustrative: use the actual callback URI registered
by the client you operate.

The default list is empty and rejects all SSO redirects. Local OIDC's legacy
direct JSON login, which has no client redirect, remains available. The existing
`delegated_auth.sso_allowed_redirect_origins` uses the same matching rules and is
configured independently. The direct `/_matrix/client/oidc/auth` route and the
standard Matrix SSO entry point both pass through local OIDC's destination check.
Callbacks recheck the policy before contacting the IdP or issuing a Matrix token,
including old sessions and client-modified session cookies.

Regression coverage uses an actual loopback HTTP IdP and an empty PostgreSQL
database, while routing Palpo requests through its real Salvo service:

```sh
cargo test --locked -p palpo --lib routing::client::oidc::redirect_tests::http_oidc_redirect_requires_trusted_client -- --ignored --exact
```

Run this test alone with `PALPO_TEST_DATABASE_URL` referring to a dedicated empty
database. It verifies denied requests and modified callback cookies create no
login token and make no IdP token exchange, then completes Web, native, and
ephemeral-loopback SSO flows through `/login`.
