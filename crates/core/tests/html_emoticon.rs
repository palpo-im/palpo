#![cfg(feature = "html")]

use palpo_core::html::{
    Html, HtmlSanitizerMode, RemoveReplyFallback, SanitizerConfig, sanitize_html,
};

#[test]
fn matrix_modes_preserve_inline_emoticons() {
    let input =
        "<img data-mx-emoticon height=\"32\" src=\"mxc://notareal.hs/abcdef\" alt=\":wave:\">";
    let expected =
        "<img alt=\":wave:\" data-mx-emoticon=\"\" height=\"32\" src=\"mxc://notareal.hs/abcdef\">";
    for config in [SanitizerConfig::strict(), SanitizerConfig::compat()] {
        let html = Html::parse(input);
        html.sanitize_with(&config);
        assert_eq!(html.to_string(), expected);
    }
    for mode in [HtmlSanitizerMode::Strict, HtmlSanitizerMode::Compat] {
        assert_eq!(
            sanitize_html(input, mode, RemoveReplyFallback::Yes),
            expected
        );
    }
}

#[test]
fn emoticons_keep_existing_scheme_and_attribute_restrictions() {
    for mode in [HtmlSanitizerMode::Strict, HtmlSanitizerMode::Compat] {
        for scheme in [
            "https://example.org/image",
            "javascript:alert(1)",
            "data:image/png;base64,AA",
        ] {
            assert_eq!(
                sanitize_html(
                    &format!("<img data-mx-emoticon src=\"{scheme}\">"),
                    mode,
                    RemoveReplyFallback::No
                ),
                ""
            );
        }
        assert_eq!(
            sanitize_html(
                "<img src=\"mxc://example.org/image\" data-mx-emoticon onerror=\"alert(1)\" unknown=\"x\">",
                mode,
                RemoveReplyFallback::No
            ),
            "<img data-mx-emoticon=\"\" src=\"mxc://example.org/image\">"
        );
        assert_eq!(
            sanitize_html(
                "<span data-mx-emoticon>:wave:</span>",
                mode,
                RemoveReplyFallback::No
            ),
            "<span>:wave:</span>"
        );
    }
}
