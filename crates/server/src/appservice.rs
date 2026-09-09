use std::collections::BTreeMap;
use std::time::Duration;

use regex_automata::meta::Regex as MetaRegex;
use subtle::ConstantTimeEq;

use crate::core::appservice::{Namespace, Registration};
use crate::core::identifiers::*;
pub use crate::data::appservice::DbRegistration;
use crate::{AppError, AppResult, data, sending};

/// Compiled regular expressions for a namespace.
///
/// Each pattern is anchored to the start of the identifier, preserving prefix
/// matches like Synapse. The set is compiled straight from the HIR, so the
/// source text is never rewritten and re-parsed.
#[derive(Clone, Debug)]
pub struct NamespaceRegex {
    pub exclusive: Option<MetaRegex>,
    pub non_exclusive: Option<MetaRegex>,
}

impl NamespaceRegex {
    /// Checks if this namespace has rights to a namespace
    pub fn is_match(&self, heystack: &str) -> bool {
        if self.is_exclusive_match(heystack) {
            return true;
        }

        if let Some(non_exclusive) = &self.non_exclusive
            && non_exclusive.is_match(heystack)
        {
            return true;
        }
        false
    }

    /// Checks if this namespace has exlusive rights to a namespace
    pub fn is_exclusive_match(&self, heystack: &str) -> bool {
        if let Some(exclusive) = &self.exclusive
            && exclusive.is_match(heystack)
        {
            return true;
        }
        false
    }
}

fn compile_set(hirs: Vec<regex_syntax::hir::Hir>) -> Result<Option<MetaRegex>, regex::Error> {
    if hirs.is_empty() {
        return Ok(None);
    }
    MetaRegex::builder()
        .build_many_from_hir(&hirs)
        .map(Some)
        .map_err(|e| match e.size_limit() {
            Some(limit) => regex::Error::CompiledTooBig(limit),
            None => regex::Error::Syntax(e.to_string()),
        })
}

impl TryFrom<Vec<Namespace>> for NamespaceRegex {
    fn try_from(value: Vec<Namespace>) -> Result<Self, regex::Error> {
        let mut exclusive = vec![];
        let mut non_exclusive = vec![];

        for namespace in value {
            // Match from the start, like Synapse's regex.match, rather than
            // letting `@ac_.*` claim `prefix@ac_...`. Prefix patterns such as
            // `@irc_` and empty match-all patterns must keep working; an end
            // anchor is the registration author's explicit choice.
            let anchored = anchored_namespace_hir(&namespace.regex)?;
            if namespace.exclusive {
                exclusive.push(anchored);
            } else {
                non_exclusive.push(anchored);
            }
        }

        Ok(NamespaceRegex {
            exclusive: compile_set(exclusive)?,
            non_exclusive: compile_set(non_exclusive)?,
        })
    }
    type Error = regex::Error;
}

/// Parse a registration namespace pattern and anchor it to the start of the
/// identifier in the regex HIR.
///
/// Parsing preserves the original syntax diagnostic without compiling the
/// pattern twice. The anchored HIR is compiled directly (never printed back to
/// text): pasting `^(?:...)` around the source changes what parses, and the HIR
/// printer does not preserve grouping for nested repetitions (`(?:[0-9]{2})?`
/// would come back as `[0-9]{2}?`). Existing end anchors are preserved, while
/// the added start-of-text assertion is unaffected by multiline mode.
fn anchored_namespace_hir(pattern: &str) -> Result<regex_syntax::hir::Hir, regex::Error> {
    let hir = regex_syntax::Parser::new()
        .parse(pattern)
        .map_err(|e| regex::Error::Syntax(e.to_string()))?;
    Ok(regex_syntax::hir::Hir::concat(vec![
        regex_syntax::hir::Hir::look(regex_syntax::hir::Look::Start),
        hir,
    ]))
}

/// Appservice registration combined with its compiled regular expressions.
#[derive(Clone, Debug)]
pub struct RegistrationInfo {
    pub registration: Registration,
    pub users: NamespaceRegex,
    pub aliases: NamespaceRegex,
    pub rooms: NamespaceRegex,
}

impl RegistrationInfo {
    /// Checks if a given user ID matches either the users namespace or the localpart specified in
    /// the appservice registration
    pub fn is_user_match(&self, user_id: &UserId) -> bool {
        self.users.is_match(user_id.as_str())
            || self.registration.sender_localpart == user_id.localpart()
    }

    /// Checks if a given user ID exclusively matches either the users namespace or the localpart
    /// specified in the appservice registration
    pub fn is_exclusive_user_match(&self, user_id: &UserId) -> bool {
        self.users.is_exclusive_match(user_id.as_str())
            || self.registration.sender_localpart == user_id.localpart()
    }
}
impl AsRef<Registration> for RegistrationInfo {
    fn as_ref(&self) -> &Registration {
        &self.registration
    }
}

impl TryFrom<Registration> for RegistrationInfo {
    type Error = regex::Error;

    fn try_from(value: Registration) -> Result<RegistrationInfo, Self::Error> {
        Ok(RegistrationInfo {
            users: value.namespaces.users.clone().try_into()?,
            aliases: value.namespaces.aliases.clone().try_into()?,
            rooms: value.namespaces.rooms.clone().try_into()?,
            registration: value,
        })
    }
}
impl TryFrom<DbRegistration> for RegistrationInfo {
    type Error = AppError;
    fn try_from(value: DbRegistration) -> Result<RegistrationInfo, Self::Error> {
        let value: Registration = value.try_into()?;
        Ok(RegistrationInfo {
            users: value.namespaces.users.clone().try_into()?,
            aliases: value.namespaces.aliases.clone().try_into()?,
            rooms: value.namespaces.rooms.clone().try_into()?,
            registration: value,
        })
    }
}

/// Registers an appservice and returns the ID to the caller
pub async fn register_appservice(registration: Registration) -> AppResult<String> {
    let db_registration: DbRegistration = registration.into();
    data::appservice::insert_registration(&db_registration).await?;
    Ok(db_registration.id)
}

/// Remove an appservice registration
///
/// # Arguments
///
/// * `service_name` - the name you send to register the service previously
pub async fn unregister_appservice(id: &str) -> AppResult<()> {
    data::appservice::delete_registration(id).await?;
    Ok(())
}

/// Set the `disabled` flag on an appservice. Returns true if a row was updated.
pub async fn set_appservice_disabled(id: &str, disabled: bool) -> AppResult<bool> {
    Ok(data::appservice::set_disabled(id, disabled).await?)
}

/// List all registrations in the database, including disabled ones.
pub async fn list_all_registrations() -> AppResult<Vec<(DbRegistration, bool)>> {
    let regs = data::appservice::all_registrations().await?;
    Ok(regs
        .into_iter()
        .map(|r| {
            let disabled = r.disabled;
            (r, disabled)
        })
        .collect())
}

pub async fn get_registration(id: &str) -> AppResult<Option<Registration>> {
    if let Some(registration) = data::appservice::find_registration(id).await? {
        Ok(Some(registration.try_into()?))
    } else {
        Ok(None)
    }
}
pub async fn find_from_token(token: &str) -> AppResult<Option<RegistrationInfo>> {
    // Constant-time comparison so we don't leak `as_token` bytes via
    // response timing. The same pattern is used in `hoops/auth.rs`.
    Ok(all()
        .await?
        .values()
        .find(|info| {
            info.registration
                .as_token
                .as_bytes()
                .ct_eq(token.as_bytes())
                .into()
        })
        .cloned())
}

// Checks if a given user id matches any exclusive appservice regex
pub async fn is_exclusive_user_id(user_id: &UserId) -> AppResult<bool> {
    for info in all().await?.values() {
        if info.is_exclusive_user_match(user_id) {
            return Ok(true);
        }
    }
    Ok(false)
}

// Checks if a given room alias matches any exclusive appservice regex
pub async fn is_exclusive_alias(alias: &RoomAliasId) -> AppResult<bool> {
    for info in all().await?.values() {
        if info.aliases.is_exclusive_match(alias.as_str()) {
            return Ok(true);
        }
    }
    Ok(false)
}

// Checks if a given room id matches any exclusive appservice regex
pub async fn is_exclusive_room_id(room_id: &RoomId) -> AppResult<bool> {
    for info in all().await?.values() {
        if info.rooms.is_exclusive_match(room_id.as_str()) {
            return Ok(true);
        }
    }
    Ok(false)
}

pub async fn all() -> AppResult<BTreeMap<String, RegistrationInfo>> {
    let registrations = data::appservice::enabled_registrations().await?;
    Ok(registrations
        .into_iter()
        .filter_map(|db_registration| {
            let info: RegistrationInfo = match db_registration.try_into() {
                Ok(registration) => registration,
                Err(e) => {
                    warn!("Failed to parse appservice registration: {}", e);
                    return None;
                }
            };
            Some((info.registration.id.clone(), info))
        })
        .collect())
}

/// Sends a request to an appservice
///
/// Only returns None if there is no url specified in the appservice registration file
#[tracing::instrument(skip(request))]
pub(crate) async fn send_request(
    registration: Registration,
    mut request: reqwest::Request,
) -> AppResult<reqwest::Response> {
    let destination = match registration.url {
        Some(url) => url,
        None => {
            return Err(AppError::public("destination is none"));
        }
    };

    let hs_token = registration.hs_token.as_str();

    // let mut http_request = request
    //     .try_into_http_request::<BytesMut>(
    //         &destination,
    //         SendAccessToken::IfRequired(hs_token),
    //         &[MatrixVersion::V1_0],
    //     )
    //     .unwrap()
    //     .map(|body| body.freeze());

    request
        .url_mut()
        .query_pairs_mut()
        .append_pair("access_token", hs_token);

    // let mut reqwest_request = reqwest::Request::try_from(http_request)?;

    *request.timeout_mut() = Some(Duration::from_secs(30));

    let url = request.url().clone();
    let client = sending::default_client();
    let response = match reqwest::Client::execute(&client, request).await {
        Ok(r) => r,
        Err(e) => {
            warn!(
                "Could not send request to appservice {:?} at {}: {}",
                registration.id, destination, e
            );
            return Err(e.into());
        }
    };

    // reqwest::Response -> http::Response conversion
    let status = response.status();
    // std::mem::swap(
    //     response.headers_mut(),
    //     http_response_builder
    //         .headers_mut()
    //         .expect("http::response::Builder is usable"),
    // );

    // let body = response.bytes().await.unwrap_or_else(|e| {
    //     warn!("server error: {}", e);
    //     Vec::new().into()
    // }); // TODO: handle timeout

    if status != 200 {
        let redacted_url = redacted_access_token_url(&url);
        warn!(
            "Appservice returned bad response {} {}\n{}",
            destination, status, redacted_url,
        );
    }

    // let response = T::IncomingResponse::try_from_http_response(
    //     http_response_builder
    //         .body(body)
    //         .expect("reqwest body is valid http body"),
    // );

    Ok(response)
}

fn redacted_access_token_url(url: &url::Url) -> url::Url {
    let mut redacted = url.clone();
    let Some(_) = redacted.query() else {
        return redacted;
    };

    let query_pairs = redacted
        .query_pairs()
        .map(|(key, value)| {
            let value = if key == "access_token" {
                "REDACTED".to_owned()
            } else {
                value.into_owned()
            };
            (key.into_owned(), value)
        })
        .collect::<Vec<_>>();

    redacted.set_query(None);
    redacted.query_pairs_mut().extend_pairs(
        query_pairs
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str())),
    );
    redacted
}

#[cfg(test)]
mod tests {
    use super::redacted_access_token_url;

    #[test]
    fn redacts_access_token_query_parameter() {
        let url = url::Url::parse(
            "https://appservice.example/_matrix/app/v1/transactions/1?foo=bar&access_token=secret",
        )
        .unwrap();

        let redacted = redacted_access_token_url(&url);
        let redacted = redacted.as_str();

        assert!(redacted.contains("foo=bar"));
        assert!(redacted.contains("access_token=REDACTED"));
        assert!(!redacted.contains("secret"));
    }

    use super::{NamespaceRegex, anchored_namespace_hir};
    use crate::core::appservice::Namespace;

    fn users(exclusive: bool, pattern: &str) -> NamespaceRegex {
        NamespaceRegex::try_from(vec![Namespace::new(exclusive, pattern.to_owned())]).unwrap()
    }

    #[test]
    fn namespace_regex_matches_from_the_start_not_a_substring() {
        // `RegexSet::is_match` is a substring search: without anchoring
        // `@ac_.*` also claimed `prefix@ac_alice:example.org`.
        let ns = users(true, "@ac_.*");
        assert!(ns.is_match("@ac_alice:example.org"));
        assert!(ns.is_exclusive_match("@ac_alice:example.org"));
        assert!(!ns.is_match("prefix@ac_alice:example.org"));
        // Never matched, anchored or not: `@xac_` does not contain `@ac_`.
        assert!(!ns.is_match("@xac_alice:example.org"));
    }

    #[test]
    fn namespace_regex_explicit_end_anchor_rejects_a_longer_server_name() {
        let ns = users(false, "!abc:example\\.org$");
        assert!(ns.is_match("!abc:example.org"));
        assert!(!ns.is_match("!abc:example.org.evil"));
        assert!(!ns.is_exclusive_match("!abc:example.org"));
    }

    #[test]
    fn namespace_regex_keeps_working_for_patterns_that_already_carry_anchors() {
        let ns = users(true, "^@ac_[^:]+:example\\.org$");
        assert!(ns.is_match("@ac_alice:example.org"));
        assert!(!ns.is_match("@ac_alice:example.org.evil"));
    }

    #[test]
    fn namespace_regex_non_exclusive_is_anchored_too() {
        let ns = users(false, "@bot_.*");
        assert!(ns.is_match("@bot_x:example.org"));
        assert!(!ns.is_match("junk@bot_x:example.org")); // substring hit before anchoring
        assert!(!ns.is_exclusive_match("@bot_x:example.org"));
    }

    #[test]
    fn namespace_regex_anchors_every_branch_of_an_alternation() {
        let ns = users(true, "@a:x|@b:x");
        assert!(ns.is_match("@a:x"));
        assert!(ns.is_match("@b:x"));
        assert!(ns.is_match("@a:xy"));
        assert!(ns.is_match("@b:xy"));
        assert!(!ns.is_match("y@a:x"));
        assert!(!ns.is_match("y@b:x"));
    }

    #[test]
    fn namespace_regex_preserves_prefix_patterns() {
        for exclusive in [true, false] {
            for (pattern, identifier) in [
                ("@irc_", "@irc_alice:example.org"),
                ("#irc_", "#irc_room:example.org"),
                ("!abc:example\\.org", "!abc:example.org.evil"),
            ] {
                let ns = users(exclusive, pattern);
                assert!(ns.is_match(identifier), "prefix pattern: {pattern}");
                assert_eq!(ns.is_exclusive_match(identifier), exclusive);
                assert!(!ns.is_match(&format!("prefix{identifier}")));
            }
        }
    }

    #[test]
    fn namespace_regex_preserves_empty_match_all_patterns() {
        for exclusive in [true, false] {
            let ns = users(exclusive, "");
            for identifier in [
                "",
                "@alice:example.org",
                "#room:example.org",
                "!room:example.org",
            ] {
                assert!(ns.is_match(identifier));
                assert_eq!(ns.is_exclusive_match(identifier), exclusive);
            }
        }
    }

    #[test]
    fn namespace_regex_preserves_prefixes_in_mixed_namespace_sets() {
        let ns = NamespaceRegex::try_from(vec![
            Namespace::new(true, "@irc_".to_owned()),
            Namespace::new(true, "@other_".to_owned()),
            Namespace::new(false, "@logger_".to_owned()),
        ])
        .unwrap();
        assert!(ns.is_exclusive_match("@irc_alice:example.org"));
        assert!(ns.is_exclusive_match("@other_alice:example.org"));
        assert!(ns.is_match("@logger_alice:example.org"));
        assert!(!ns.is_exclusive_match("@logger_alice:example.org"));
        assert!(!ns.is_match("prefix@irc_alice:example.org"));
        assert!(!ns.is_match("prefix@other_alice:example.org"));
        assert!(!ns.is_match("prefix@logger_alice:example.org"));
    }

    #[test]
    fn namespace_regex_preserves_syntax_error_diagnostics() {
        for pattern in [
            ")",
            "(",
            "[",
            "a{2,1}",
            "(?P<x>a)(?P<x>b)",
            r"\p{Unknown}",
            "(?P<1>a)",
        ] {
            let original = regex::Regex::new(pattern).unwrap_err();
            let err = NamespaceRegex::try_from(vec![Namespace::new(true, pattern.to_owned())])
                .expect_err("invalid namespace syntax must fail");
            assert!(matches!(err, regex::Error::Syntax(_)));
            assert_eq!(err.to_string(), original.to_string(), "pattern: {pattern}");
        }
    }

    #[test]
    fn namespace_regex_compile_set_preserves_size_limit_error() {
        // Exercise the set compiler directly, so an earlier standalone compile
        // cannot mask an incorrect conversion of its error kind.
        let pattern = "(?:a{1000}){1000}";
        let hir = regex_syntax::Parser::new().parse(pattern).unwrap();
        let err = super::compile_set(vec![hir]).expect_err("oversized HIR must fail");
        let original = regex::Regex::new(pattern).unwrap_err();
        match (err, original) {
            (regex::Error::CompiledTooBig(actual), regex::Error::CompiledTooBig(expected)) => {
                assert_eq!(actual, expected);
            }
            (actual, expected) => panic!("expected {expected:?}, got {actual:?}"),
        }
        let err = NamespaceRegex::try_from(vec![Namespace::new(true, pattern.to_owned())])
            .expect_err("oversized namespace must fail");
        assert!(matches!(err, regex::Error::CompiledTooBig(_)));
    }

    #[test]
    fn namespace_regex_rejects_an_invalid_pattern_with_its_original_diagnostic() {
        // Pasting `^(?:...)$` around this text would have turned it into the
        // VALID pattern `^(?:a)|(b)$` — and one that is not anchored at all.
        // Built at runtime so clippy's `invalid_regex` lint does not reject the
        // literal: the point of this test is that it must NOT compile.
        let unbalanced = ["a)", "|(b"].concat();
        let err = NamespaceRegex::try_from(vec![Namespace::new(true, unbalanced.clone())])
            .expect_err("an unbalanced pattern must not compile");
        let original = regex::Regex::new(&unbalanced).unwrap_err();
        assert_eq!(err.to_string(), original.to_string());
    }

    #[test]
    fn namespace_regex_accepts_a_verbose_pattern_with_a_trailing_comment() {
        // In `(?x)` mode `#` starts a comment to end of line; textual wrapping
        // would have put the closing `)$` inside that comment.
        let ns = users(true, "(?x)^@ac_alice:example\\.org$ # exact user");
        assert!(ns.is_match("@ac_alice:example.org"));
        assert!(!ns.is_match("@ac_alice:example.org.evil"));
        assert!(anchored_namespace_hir("(?x)a # c").is_ok());
    }
    #[test]
    fn optional_repeated_suffix_must_remain_optional() {
        // regex-syntax 0.8's HIR printer renders `(?:[0-9]{2})?` as `[0-9]{2}?`,
        // which silently requires the two digits. Compiling straight from the
        // HIR keeps the group optional.
        let ns = users(true, "^@bot(?:[0-9]{2})?:example\\.org$");
        assert!(ns.is_match("@bot:example.org"));
        assert!(ns.is_match("@bot12:example.org"));
        assert!(!ns.is_match("@bot1:example.org"));
        assert!(!ns.is_match("@bot123:example.org"));
        let ns = users(true, "^@bot(?:a+)?:example\\.org$");
        assert!(ns.is_match("@bot:example.org"));
        assert!(ns.is_match("@botaaa:example.org"));
    }

    #[test]
    fn anchoring_does_not_add_parser_nesting() {
        // 249 nested groups compile as a plain regex; anchoring in the HIR must
        // not push the compiled set over the parser's nesting limit.
        let deep = format!("^@{}a{}:x$", "(".repeat(249), ")".repeat(249));
        let ns = users(true, &deep);
        assert!(ns.is_match("@a:x"));
        assert!(!ns.is_match("@a:xy"));
    }

    #[test]
    fn start_anchor_is_a_text_boundary_not_a_line_boundary() {
        let ns = users(true, "(?m)^@ac_.*");
        assert!(ns.is_match("@ac_alice:example.org"));
        assert!(ns.is_match("@ac_alice:example.org\n"));
        assert!(!ns.is_match("x\n@ac_alice:example.org"));
    }
}
