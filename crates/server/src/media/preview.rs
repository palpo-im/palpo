use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, LazyLock, OnceLock};
use std::time::Duration;

use ipaddress::IPAddress;
use serde::Serialize;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use url::Url;

use crate::core::identifiers::*;
use crate::core::{MatrixError, UnixMillis};
use crate::data::media::{DbUrlPreview, NewDbMetadata, NewDbUrlPreview};
use crate::{AppResult, config, data, utils};

static URL_PREVIEW_MUTEX: LazyLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> =
    LazyLock::new(Default::default);
async fn get_url_preview_mutex(url: &str) -> Arc<Mutex<()>> {
    let mut locks = URL_PREVIEW_MUTEX.lock().await;
    locks
        .entry(url.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::ClientBuilder::new()
            .timeout(Duration::from_secs(20))
            .user_agent("Palpo")
            .build()
            .expect("Failed to create reqwest client")
    })
}

#[derive(Serialize, Default, Clone, Debug)]
pub struct UrlPreviewData {
    #[serde(skip_serializing_if = "Option::is_none", rename(serialize = "og:url"))]
    pub og_url: Option<String>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        rename(serialize = "og:title")
    )]
    pub og_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename(serialize = "og:type"))]
    pub og_type: Option<String>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        rename(serialize = "og:description")
    )]
    pub og_description: Option<String>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        rename(serialize = "og:image")
    )]
    pub og_image: Option<String>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        rename(serialize = "matrix:image:size")
    )]
    pub image_size: Option<u64>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        rename(serialize = "og:image:width")
    )]
    pub og_image_width: Option<u32>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        rename(serialize = "og:image:height")
    )]
    pub og_image_height: Option<u32>,
}
impl UrlPreviewData {
    pub fn into_new_db_url_preview(self, url: impl Into<String>) -> NewDbUrlPreview {
        let Self {
            og_title,
            og_type,
            og_url,
            og_description,
            og_image,
            image_size,
            og_image_width,
            og_image_height,
            ..
        } = self;
        NewDbUrlPreview {
            url: url.into(),
            og_title,
            og_type,
            og_url,
            og_description,
            og_image,
            image_size: image_size.map(|s| s as i64),
            og_image_width: og_image_width.map(|w| w as i32),
            og_image_height: og_image_height.map(|h| h as i32),
            created_at: UnixMillis::now(),
        }
    }
}
impl From<DbUrlPreview> for UrlPreviewData {
    fn from(preview: DbUrlPreview) -> Self {
        let DbUrlPreview {
            og_title,
            og_type,
            og_url,
            og_description,
            og_image,
            image_size,
            og_image_width,
            og_image_height,
            ..
        } = preview;
        Self {
            og_title,
            og_type,
            og_url,
            og_description,
            og_image,
            image_size: image_size.map(|s| s as u64),
            og_image_width: og_image_width.map(|w| w as u32),
            og_image_height: og_image_height.map(|h| h as u32),
        }
    }
}

pub fn url_preview_allowed(url: &Url) -> bool {
    if ["http", "https"]
        .iter()
        .all(|&scheme| scheme != url.scheme().to_lowercase())
    {
        debug!("Ignoring non-HTTP/HTTPS URL to preview: {}", url);
        return false;
    }

    let host = match url.host_str() {
        None => {
            debug!(
                "Ignoring URL preview for a URL that does not have a host (?): {}",
                url
            );
            return false;
        }
        Some(h) => h.to_owned(),
    };

    let conf = crate::config::get();
    let allowlist_domain_contains = &conf.url_preview.domain_contains_allowlist;
    let allowlist_domain_explicit = &conf.url_preview.domain_explicit_allowlist;
    let denylist_domain_explicit = &conf.url_preview.domain_explicit_denylist;
    let allowlist_url_contains = &conf.url_preview.url_contains_allowlist;

    if allowlist_domain_contains.contains(&"*".to_owned())
        || allowlist_domain_explicit.contains(&"*".to_owned())
        || allowlist_url_contains.contains(&"*".to_owned())
    {
        debug!(
            "Config key contains * which is allowing all URL previews. Allowing URL {}",
            url
        );
        return true;
    }

    if !host.is_empty() {
        if denylist_domain_explicit.contains(&host) {
            debug!(
                "Host {} is not allowed by url_preview_domain_explicit_denylist (check 1/4)",
                &host
            );
            return false;
        }

        if allowlist_domain_explicit.contains(&host) {
            debug!(
                "Host {} is allowed by url_preview_domain_explicit_allowlist (check 2/4)",
                &host
            );
            return true;
        }

        if allowlist_domain_contains
            .iter()
            .any(|domain_s| domain_s.contains(&host.clone()))
        {
            debug!(
                "Host {} is allowed by url_preview_domain_contains_allowlist (check 3/4)",
                &host
            );
            return true;
        }

        if allowlist_url_contains
            .iter()
            .any(|url_s| url.to_string().contains(url_s))
        {
            debug!(
                "URL {} is allowed by url_preview_url_contains_allowlist (check 4/4)",
                &host
            );
            return true;
        }

        // check root domain if available and if user has root domain checks
        if conf.url_preview.check_root_domain {
            debug!("Checking root domain");
            match host.split_once('.') {
                None => return false,
                Some((_, root_domain)) => {
                    if denylist_domain_explicit.contains(&root_domain.to_owned()) {
                        debug!(
                            "Root domain {} is not allowed by \
    						 url_preview_domain_explicit_denylist (check 1/3)",
                            &root_domain
                        );
                        return true;
                    }

                    if allowlist_domain_explicit.contains(&root_domain.to_owned()) {
                        debug!(
                            "Root domain {} is allowed by url_preview_domain_explicit_allowlist \
    						 (check 2/3)",
                            &root_domain
                        );
                        return true;
                    }

                    if allowlist_domain_contains
                        .iter()
                        .any(|domain_s| domain_s.contains(&root_domain.to_owned()))
                    {
                        debug!(
                            "Root domain {} is allowed by url_preview_domain_contains_allowlist \
    						 (check 3/3)",
                            &root_domain
                        );
                        return true;
                    }
                }
            }
        }
    }

    false
}

pub async fn get_url_preview(url: &Url) -> AppResult<UrlPreviewData> {
    if let Ok(preview) = data::media::get_url_preview(url.as_str()) {
        return Ok(preview.into());
    }

    // ensure that only one request is made per URL
    let mutex = get_url_preview_mutex(url.as_str()).await;
    let _request_lock = mutex.lock().await;

    match data::media::get_url_preview(url.as_str()) {
        Ok(preview) => Ok(preview.into()),
        Err(_) => request_url_preview(url).await,
    }
}

async fn request_url_preview(url: &Url) -> AppResult<UrlPreviewData> {
    let client = client();
    if let Ok(ip) = IPAddress::parse(url.host_str().expect("URL previously validated"))
        && !config::valid_cidr_range(&ip)
    {
        return Err(
            MatrixError::forbidden("Requesting from this address is forbidden.", None).into(),
        );
    }

    let response = client.get(url.clone()).send().await?;
    debug!(
        ?url,
        "URL preview response headers: {:?}",
        response.headers()
    );

    if let Some(remote_addr) = response.remote_addr() {
        debug!(
            ?url,
            "URL preview response remote address: {:?}", remote_addr
        );
        if let Ok(ip) = IPAddress::parse(remote_addr.ip().to_string())
            && !config::valid_cidr_range(&ip)
        {
            return Err(
                MatrixError::forbidden("Requesting from this address is forbidden.", None).into(),
            );
        }
    }

    let Some(content_type) = response.headers().get(reqwest::header::CONTENT_TYPE) else {
        return Err(MatrixError::unknown("Unknown or invalid Content-Type header").into());
    };

    let content_type = content_type.to_str().map_err(|e| {
        MatrixError::unknown(format!("Unknown or invalid Content-Type header: {e}"))
    })?;

    let data = match content_type {
        html if html.starts_with("text/html") => download_html(url).await?,
        img if img.starts_with("image/") => download_image(url).await?,
        _ => return Err(MatrixError::unknown("Unsupported Content-Type").into()),
    };
    crate::data::media::set_url_preview(&data.clone().into_new_db_url_preview(url.as_str()))?;

    Ok(data)
}
async fn download_image(url: &Url) -> AppResult<UrlPreviewData> {
    use image::ImageReader;

    let conf = crate::config::get();
    let image = client().get(url.to_owned()).send().await?;
    let content_type = image.headers().get(reqwest::header::CONTENT_TYPE);
    let content_type = content_type
        .and_then(|ct| ct.to_str().ok())
        .map(|c| c.to_owned());
    let image = image.bytes().await?;
    let mxc = Mxc {
        server_name: &conf.server_name,
        media_id: &utils::random_string(crate::MXC_LENGTH),
    };

    let dest_path = crate::media::get_media_path(&conf.server_name, mxc.media_id);
    let dest_path = Path::new(&dest_path);
    if !dest_path.exists() {
        let parent_dir = utils::fs::get_parent_dir(dest_path);
        std::fs::create_dir_all(&parent_dir)?;

        let mut file = tokio::fs::File::create(dest_path).await?;
        file.write_all(&image).await?;
        let metadata = NewDbMetadata {
            media_id: mxc.media_id.to_string(),
            origin_server: conf.server_name.clone(),
            disposition_type: Some("inline".into()),
            content_type,
            file_name: None,
            file_extension: None,
            file_size: image.len() as i64,
            file_hash: None,
            created_by: None,
            created_at: UnixMillis::now(),
        };

        crate::data::media::insert_metadata(&metadata)?;
    }

    let cursor = std::io::Cursor::new(&image);
    let (width, height) = match ImageReader::new(cursor).with_guessed_format() {
        Err(_) => (None, None),
        Ok(reader) => match reader.into_dimensions() {
            Err(_) => (None, None),
            Ok((width, height)) => (Some(width), Some(height)),
        },
    };

    Ok(UrlPreviewData {
        og_image: Some(mxc.to_string()),
        image_size: Some(image.len() as u64),
        og_image_width: width,
        og_image_height: height,
        ..Default::default()
    })
}

async fn download_html(url: &Url) -> AppResult<UrlPreviewData> {
    use webpage::HTML;

    let conf = crate::config::get();
    let client = client();
    let mut response = client.get(url.to_owned()).send().await?;

    let mut bytes: Vec<u8> = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        bytes.extend_from_slice(&chunk);
        if bytes.len() > conf.url_preview.max_spider_size {
            debug!(
                "Response body from URL {} exceeds url_preview.max_spider_size ({}), not \
				 processing the rest of the response body and assuming our necessary data is in \
				 this range.",
                url, conf.url_preview.max_spider_size
            );
            break;
        }
    }
    let body = String::from_utf8_lossy(&bytes);
    let Ok(html) = HTML::from_string(body.to_string(), Some(url.to_string())) else {
        return Err(MatrixError::unknown("Failed to parse HTML").into());
    };

    let mut data = match html.opengraph.images.first() {
        None => UrlPreviewData::default(),
        Some(obj) => download_image(&url.join(&obj.url)?).await?,
    };

    data.og_type = Some(html.opengraph.og_type);
    let props = html.opengraph.properties;
    // use OpenGraph title/description, but fall back to HTML if not available
    data.og_url = props.get("url").cloned().or(Some(url.to_string()));
    data.og_title = props.get("title").cloned().or(html.title);
    data.og_description = props.get("description").cloned().or(html.description);

    Ok(data)
}
