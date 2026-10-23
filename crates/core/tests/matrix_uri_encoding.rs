use palpo_core::{MatrixToUri, MatrixUri, RoomAliasId};

#[test]
fn matrix_links_encode_backslashes_and_round_trip() {
    let alias = RoomAliasId::parse(r"#room\name:example.org").unwrap();

    let matrix_to = alias.matrix_to_uri();
    let encoded = matrix_to.to_string();
    assert_eq!(encoded, "https://matrix.to/#/%23room%5Cname:example.org");
    assert_eq!(MatrixToUri::parse(&encoded).unwrap(), matrix_to);

    let matrix = alias.matrix_uri(false);
    let encoded = matrix.to_string();
    assert_eq!(encoded, "matrix:r/room%5Cname:example.org");
    assert_eq!(MatrixUri::parse(&encoded).unwrap(), matrix);
}
