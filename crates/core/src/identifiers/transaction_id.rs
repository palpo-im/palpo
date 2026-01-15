use diesel::expression::AsExpression;

use crate::macros::IdDst;

/// A Matrix transaction ID.
///
/// Transaction IDs in Matrix are opaque strings. This type is provided simply
/// for its semantic value.
///
/// You can create one from a string (using `.into()`) but the recommended way
/// is to use `TransactionId::new()` to generate a random one. If that function
/// is not available for you, you need to activate this crate's `rand` Cargo
/// feature.
#[repr(transparent)]
#[derive(PartialEq, Eq, PartialOrd, Ord, Hash, IdDst, AsExpression)]
#[diesel(not_sized, sql_type = diesel::sql_types::Text)]
pub struct TransactionId(str);

impl TransactionId {
    /// Creates a random transaction ID.
    ///
    /// This will currently be a UUID without hyphens, but no guarantees are
    /// made about the structure of transaction IDs generated from this
    /// function.
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> OwnedTransactionId {
        let id = uuid::Uuid::new_v4();
        Self::from_borrowed(&id.simple().to_string()).to_owned()
    }
}
