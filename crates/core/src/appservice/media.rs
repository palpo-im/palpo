//! Application-service URL previews (MSC4417).
use serde::{Deserialize, Serialize};
use url::Url;

use crate::OwnedUserId;
use crate::sending::{SendRequest, SendResult};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PreviewUrlReqArgs {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<OwnedUserId>,
}

/// Build the current unstable request. The caller supplies homeserver authentication.
pub fn preview_url_request(origin: &str, args: PreviewUrlReqArgs) -> SendResult<SendRequest> {
    let mut url = Url::parse(&format!(
        "{}/_matrix/app/unstable/uk.half-shot.msc4417/preview_url",
        origin.trim_end_matches('/')
    ))?;
    url.query_pairs_mut().append_pair("url", &args.url);
    if let Some(user) = args.user_id {
        url.query_pairs_mut().append_pair("user_id", user.as_str());
    }
    Ok(crate::sending::get(url))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_encodes_url_and_optional_identity_without_credentials() {
        let target = "https://example.org/a?x=1&secret=private#part";
        for user in [None, Some("@alice:example.org".try_into().unwrap())] {
            let request = preview_url_request(
                "https://bridge.example/",
                PreviewUrlReqArgs {
                    url: target.into(),
                    user_id: user.clone(),
                },
            )
            .unwrap();
            assert_eq!(request.method(), reqwest::Method::GET);
            assert_eq!(
                request.url().path(),
                "/_matrix/app/unstable/uk.half-shot.msc4417/preview_url"
            );
            let query: std::collections::BTreeMap<_, _> = request.url().query_pairs().collect();
            assert_eq!(query["url"], target);
            assert_eq!(
                query.get("user_id").map(|id| id.as_ref()),
                user.as_deref().map(|id| id.as_str())
            );
            assert!(!query.contains_key("access_token"));
        }
    }
}
