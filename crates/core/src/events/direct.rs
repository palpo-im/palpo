//! Types for the [`m.direct`] event.
//!
//! [`m.direct`]: https://spec.matrix.org/latest/client-server-api/#mdirect

use std::collections::{BTreeMap, btree_map};
use std::ops::{Deref, DerefMut};

use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize};

use crate::macros::EventContent;
use crate::{OwnedRoomId, OwnedUserId};

/// The content of an `m.direct` event.
///
/// A mapping of `UserId`s to a list of `RoomId`s which are considered *direct*
/// for that particular user.
///
/// Informs the client about the rooms that are considered direct by a user.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize, EventContent)]
#[allow(clippy::exhaustive_structs)]
#[palpo_event(type = "m.direct", kind = GlobalAccountData)]
pub struct DirectEventContent(pub BTreeMap<OwnedUserId, Vec<OwnedRoomId>>);

impl Deref for DirectEventContent {
    type Target = BTreeMap<OwnedUserId, Vec<OwnedRoomId>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for DirectEventContent {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl IntoIterator for DirectEventContent {
    type Item = (OwnedUserId, Vec<OwnedRoomId>);
    type IntoIter = btree_map::IntoIter<OwnedUserId, Vec<OwnedRoomId>>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl FromIterator<(OwnedUserId, Vec<OwnedRoomId>)> for DirectEventContent {
    fn from_iter<T>(iter: T) -> Self
    where
        T: IntoIterator<Item = (OwnedUserId, Vec<OwnedRoomId>)>,
    {
        Self(BTreeMap::from_iter(iter))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::{from_value as from_json_value, json, to_value as to_json_value};

    use super::{DirectEvent, DirectEventContent};
    use crate::{owned_room_id, owned_user_id};

    #[test]
    fn serialization() {
        let mut content = DirectEventContent(BTreeMap::new());
        let alice = owned_user_id!("@alice:palpo.io");
        let rooms = vec![owned_room_id!("!1:palpo.io")];

        content.insert(alice.clone(), rooms.clone());

        let json_data = json!({
            alice: rooms,
        });

        assert_eq!(to_json_value(&content).unwrap(), json_data);
    }

    #[test]
    fn deserialization() {
        let alice = owned_user_id!("@alice:palpo.io");
        let rooms = vec![owned_room_id!("!1:palpo.io"), owned_room_id!("!2:palpo.io")];

        let json_data = json!({
            "content": {
                alice.to_string(): rooms,
            },
            "type": "m.direct"
        });

        let event: DirectEvent = from_json_value(json_data).unwrap();
        let direct_rooms = event.content.get(&alice).unwrap();

        assert!(direct_rooms.contains(&rooms[0]));
        assert!(direct_rooms.contains(&rooms[1]));
    }
}
