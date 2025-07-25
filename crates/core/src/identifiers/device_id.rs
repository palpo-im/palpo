use crate::macros::IdZst;
use diesel::expression::AsExpression;

use super::generate_localpart;

/// A Matrix key ID.
///
/// Device identifiers in Matrix are completely opaque character sequences. This
/// type is provided simply for its semantic value.
///
/// # Example
///
/// ```
/// use palpo_core::{DeviceId, OwnedDeviceId, device_id};
///
/// # #[cfg(feature = "rand")] {
/// let random_id = DeviceId::new();
/// assert_eq!(random_id.as_str().len(), 10);
/// # }
///
/// let static_id = device_id!("01234567");
/// assert_eq!(static_id.as_str(), "01234567");
///
/// let ref_id: &DeviceId = "abcdefghi".into();
/// assert_eq!(ref_id.as_str(), "abcdefghi");
///
/// let owned_id: OwnedDeviceId = "ijklmnop".into();
/// assert_eq!(owned_id.as_str(), "ijklmnop");
/// ```
#[repr(transparent)]
#[derive(PartialEq, Eq, PartialOrd, Ord, Hash, IdZst, AsExpression)]
#[diesel(not_sized, sql_type = diesel::sql_types::Text)]
pub struct DeviceId(str);

impl DeviceId {
    /// Generates a random `DeviceId`, suitable for assignment to a new device.
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> OwnedDeviceId {
        Self::from_borrowed(&generate_localpart(8)).to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::{DeviceId, OwnedDeviceId};

    #[test]
    fn generate_device_id() {
        assert_eq!(DeviceId::new().as_str().len(), 8);
    }

    #[test]
    fn create_device_id_from_str() {
        let ref_id: &DeviceId = "abcdefgh".into();
        assert_eq!(ref_id.as_str(), "abcdefgh");
    }

    #[test]
    fn create_boxed_device_id_from_str() {
        let box_id: OwnedDeviceId = "12345678".into();
        assert_eq!(box_id.as_str(), "12345678");
    }

    #[test]
    fn create_device_id_from_box() {
        let box_str: Box<str> = "ijklmnop".into();
        let device_id: OwnedDeviceId = box_str.into();
        assert_eq!(device_id.as_str(), "ijklmnop");
    }
}
