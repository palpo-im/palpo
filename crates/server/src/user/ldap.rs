use std::collections::HashMap;

use ldap3::{LdapConnAsync, Scope, SearchEntry};

use crate::core::UserId;
use crate::{AppError, AppResult, MatrixError, config};

/// Escape special characters in LDAP filter values according to RFC 4515.
///
/// Characters that need escaping:
/// - * (asterisk) -> \2a
/// - ( (left paren) -> \28
/// - ) (right paren) -> \29
/// - \ (backslash) -> \5c
/// - NUL (null) -> \00
fn escape_ldap_filter_value(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() * 2);
    for c in value.chars() {
        match c {
            '*' => escaped.push_str("\\2a"),
            '(' => escaped.push_str("\\28"),
            ')' => escaped.push_str("\\29"),
            '\\' => escaped.push_str("\\5c"),
            '\0' => escaped.push_str("\\00"),
            _ => escaped.push(c),
        }
    }
    escaped
}

pub async fn search_ldap(user_id: &UserId) -> AppResult<Vec<(String, bool)>> {
    let localpart = user_id.localpart().to_owned();
    let lowercased_localpart = localpart.to_lowercase();
    // Escape special LDAP filter characters to prevent LDAP injection attacks
    let escaped_localpart = escape_ldap_filter_value(&lowercased_localpart);

    let conf =
        config::enabled_ldap().ok_or_else(|| AppError::public("LDAP is not enabled in the configuration"))?;
    let uri = conf
        .uri
        .as_ref()
        .ok_or_else(|| AppError::public("LDAP URI is not configured."))?;

    debug!(?uri, "LDAP creating connection...");
    let (conn, mut ldap) = LdapConnAsync::new(uri.as_str())
        .await
        .map_err(|e| AppError::public("LDAP connection setup error: {e}"))?;

    let driver = tokio::spawn(async move {
        match conn.drive().await {
            Err(e) => error!("LDAP connection error: {e}"),
            Ok(()) => debug!("LDAP connection completed"),
        }
    });

    match (&conf.bind_dn, &conf.bind_password_file) {
        (Some(bind_dn), Some(bind_password_file)) => {
            let bind_pw = String::from_utf8(std::fs::read(bind_password_file)?)?;
            ldap.simple_bind(bind_dn, bind_pw.trim())
                .await
                .and_then(ldap3::LdapResult::success)
                .map_err(|e| AppError::public(format!("LDAP bind error: {e}")))?;
        }
        (..) => {}
    }

    let attr = [&conf.uid_attribute, &conf.name_attribute];

    let user_filter = &conf.filter.replace("{username}", &escaped_localpart);

    let (entries, _result) = ldap
        .search(&conf.base_dn, Scope::Subtree, user_filter, &attr)
        .await
        .and_then(ldap3::SearchResult::success)
        .inspect(|(entries, result)| trace!(?entries, ?result, "LDAP Search"))
        .map_err(|e| AppError::public(format!("LDAP search error: {e}")))?;

    let mut dns: HashMap<String, bool> = entries
        .into_iter()
        .filter_map(|entry| {
            let search_entry = SearchEntry::construct(entry);
            debug!(?search_entry, "LDAP search entry");
            search_entry
                .attrs
                .get(&conf.uid_attribute)
                .into_iter()
                .chain(search_entry.attrs.get(&conf.name_attribute))
                .any(|ids| ids.contains(&localpart) || ids.contains(&lowercased_localpart))
                .then_some((search_entry.dn, false))
        })
        .collect();

    if !conf.admin_filter.is_empty() {
        let admin_base_dn = if conf.admin_base_dn.is_empty() {
            &conf.base_dn
        } else {
            &conf.admin_base_dn
        };

        let admin_filter = &conf.admin_filter.replace("{username}", &escaped_localpart);

        let (admin_entries, _result) = ldap
            .search(admin_base_dn, Scope::Subtree, admin_filter, &attr)
            .await
            .and_then(ldap3::SearchResult::success)
            .inspect(|(entries, result)| trace!(?entries, ?result, "LDAP Admin Search"))
            .map_err(|e| AppError::public(format!("Ldap admin search error: {e}")))?;

        dns.extend(admin_entries.into_iter().filter_map(|entry| {
            let search_entry = SearchEntry::construct(entry);
            debug!(?search_entry, "LDAP search entry");
            search_entry
                .attrs
                .get(&conf.uid_attribute)
                .into_iter()
                .chain(search_entry.attrs.get(&conf.name_attribute))
                .any(|ids| ids.contains(&localpart) || ids.contains(&lowercased_localpart))
                .then_some((search_entry.dn, true))
        }));
    }

    ldap.unbind()
        .await
        .map_err(|e| AppError::public(format!("LDAP unbind error: {e}")))?;

    driver.await.ok();

    Ok(dns.drain().collect())
}

pub async fn auth_ldap(user_dn: &str, password: &str) -> AppResult<()> {
    let conf =
        config::enabled_ldap().ok_or_else(|| AppError::public("LDAP is not enabled in the configuration"))?;
    let uri = conf
        .uri
        .as_ref()
        .ok_or_else(|| AppError::public(format!("LDAP URI is not configured")))?;

    debug!(?uri, "LDAP creating connection...");
    let (conn, mut ldap) = LdapConnAsync::new(uri.as_str())
        .await
        .map_err(|e| AppError::public(format!("LDAP connection setup error: {e}")))?;

    let driver = tokio::spawn(async move {
        match conn.drive().await {
            Err(e) => error!("LDAP connection error: {e}"),
            Ok(()) => debug!("LDAP connection completed."),
        }
    });

    ldap.simple_bind(user_dn, password)
        .await
        .and_then(ldap3::LdapResult::success)
        .map_err(|e| MatrixError::forbidden(format!("LDAP authentication error: {e}"), None))?;

    ldap.unbind()
        .await
        .map_err(|e| AppError::public(format!("LDAP unbind error: {e}")))?;

    driver.await.ok();

    Ok(())
}
