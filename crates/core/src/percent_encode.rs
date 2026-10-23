use percent_encoding::{AsciiSet, CONTROLS};

/// The [path percent-encode set] as defined in the WHATWG URL standard + `/` and `\`
/// since we always encode single segments of the path.
///
/// [path percent-encode set]: https://url.spec.whatwg.org/#path-percent-encode-set
pub(crate) const PATH_PERCENT_ENCODE_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'`')
    .add(b'{')
    .add(b'}')
    .add(b'/')
    .add(b'\\');

#[cfg(test)]
mod tests {
    use percent_encoding::utf8_percent_encode;

    use super::PATH_PERCENT_ENCODE_SET;

    #[test]
    fn backslashes_cannot_be_normalized_into_path_separators() {
        let segment = utf8_percent_encode(r"C:\Path\To", PATH_PERCENT_ENCODE_SET);
        let url = url::Url::parse(&format!("https://example.org/s/{segment}")).unwrap();
        assert_eq!(url.as_str(), "https://example.org/s/C:%5CPath%5CTo");
        assert_eq!(
            url.path_segments().unwrap().collect::<Vec<_>>(),
            ["s", "C:%5CPath%5CTo"]
        );
    }
}
