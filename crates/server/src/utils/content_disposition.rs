use std::borrow::Cow;

use crate::core::http_headers::{ContentDisposition, ContentDispositionType};

/// as defined by MSC2702
const ALLOWED_INLINE_CONTENT_TYPES: [&str; 26] = [
    // keep sorted
    "application/json",
    "application/ld+json",
    "audio/aac",
    "audio/flac",
    "audio/mp4",
    "audio/mpeg",
    "audio/ogg",
    "audio/wav",
    "audio/wave",
    "audio/webm",
    "audio/x-flac",
    "audio/x-pn-wav",
    "audio/x-wav",
    "image/apng",
    "image/avif",
    "image/gif",
    "image/jpeg",
    "image/png",
    "image/webp",
    "text/css",
    "text/csv",
    "text/plain",
    "video/mp4",
    "video/ogg",
    "video/quicktime",
    "video/webm",
];

/// Returns a Content-Disposition of `attachment` or `inline`, depending on the
/// Content-Type against MSC2702 list of safe inline Content-Types
/// (`ALLOWED_INLINE_CONTENT_TYPES`)
#[must_use]
pub fn content_disposition_type(content_type: Option<&str>) -> ContentDispositionType {
    let Some(content_type) = content_type else {
        tracing::info!("no content-type was given, assuming attachment for Content-Disposition");
        return ContentDispositionType::Attachment;
    };

    debug_assert!(
        ALLOWED_INLINE_CONTENT_TYPES.is_sorted(),
        "ALLOWED_INLINE_CONTENT_TYPES is not sorted"
    );

    let content_type: Cow<'_, str> = content_type
        .split(';')
        .next()
        .unwrap_or(content_type)
        .to_ascii_lowercase()
        .into();

    if ALLOWED_INLINE_CONTENT_TYPES
        .binary_search(&content_type.as_ref())
        .is_ok()
    {
        ContentDispositionType::Inline
    } else {
        ContentDispositionType::Attachment
    }
}

/// sanitises the file name for the Content-Disposition using
/// `sanitize_filename` crate
#[tracing::instrument(level = "debug")]
pub fn sanitise_filename(filename: &str) -> String {
    sanitize_filename::sanitize_with_options(
        filename,
        sanitize_filename::Options {
            truncate: false,
            ..Default::default()
        },
    )
}

/// creates the final Content-Disposition based on whether the filename exists
/// or not, or if a requested filename was specified (media download with
/// filename)
///
/// if filename exists:
/// `Content-Disposition: attachment/inline; filename=filename.ext`
///
/// else: `Content-Disposition: attachment/inline`
pub fn make_content_disposition(
    disposition_type: Option<ContentDispositionType>,
    content_type: Option<&str>,
    filename: Option<&str>,
) -> ContentDisposition {
    let disposition_type =
        disposition_type.unwrap_or_else(|| content_disposition_type(content_type));
    ContentDisposition::new(disposition_type).with_filename(filename.map(sanitise_filename))
}

#[cfg(test)]
mod tests {
    use super::make_content_disposition;
    use crate::core::http_headers::ContentDisposition;

    #[test]
    fn unicode_filename_is_safe_in_http_header_and_round_trips() {
        let header = make_content_disposition(None, Some("image/png"), Some("🐔")).to_string();
        assert!(header.is_ascii());
        assert_eq!(
            header
                .parse::<ContentDisposition>()
                .unwrap()
                .filename
                .as_deref(),
            Some("🐔")
        );
    }

    #[test]
    fn string_sanitisation() {
        const SAMPLE: &str = "🏳️‍⚧️this\\r\\n įs \r\\n ä \\r\nstrïng 🥴that\n\r \
		                      ../../../../../../../may be\r\n malicious🏳️‍⚧️";
        const SANITISED: &str = "🏳️‍⚧️thisrn įs n ä rstrïng 🥴that ..............may be malicious🏳️‍⚧️";

        let options = sanitize_filename::Options {
            windows: true,
            truncate: true,
            replacement: "",
        };

        // cargo test -- --nocapture
        println!("{SAMPLE}");
        println!(
            "{}",
            sanitize_filename::sanitize_with_options(SAMPLE, options.clone())
        );
        println!("{SAMPLE:?}");
        println!(
            "{:?}",
            sanitize_filename::sanitize_with_options(SAMPLE, options.clone())
        );

        assert_eq!(
            SANITISED,
            sanitize_filename::sanitize_with_options(SAMPLE, options.clone())
        );
    }

    // #[test]
    // fn empty_sanitisation() {
    //     use crate::EMPTY;

    //     let result = sanitize_filename::sanitize_with_options(
    //         EMPTY,
    //         sanitize_filename::Options {
    //             windows: true,
    //             truncate: true,
    //             replacement: "",
    //         },
    //     );

    //     assert_eq!(EMPTY, result);
    // }
}
