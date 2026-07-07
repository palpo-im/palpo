use salvo::prelude::*;

use crate::core::identifiers::*;
use crate::{AppResult, AuthedInfo, config};

mod url;
pub use url::*;

pub trait DepotExt {
    fn authed_info(&self) -> AppResult<&AuthedInfo>;
    fn set_origin(&mut self, origin: OwnedServerName);
    fn origin(&mut self) -> AppResult<&OwnedServerName>;
    fn take_authed_info(&mut self) -> AppResult<AuthedInfo>;
}
impl DepotExt for Depot {
    fn authed_info(&self) -> AppResult<&AuthedInfo> {
        self.get_typed::<AuthedInfo>()
            .map_err(|_| StatusError::unauthorized().into())
    }
    fn set_origin(&mut self, origin: OwnedServerName) {
        self.insert("origin", origin);
    }
    fn origin(&mut self) -> AppResult<&OwnedServerName> {
        self.get::<OwnedServerName>("origin")
            .map_err(|_| StatusError::unauthorized().into())
    }
    fn take_authed_info(&mut self) -> AppResult<AuthedInfo> {
        self.remove_typed::<AuthedInfo>()
            .map_err(|_| StatusError::unauthorized().into())
    }
}

pub trait IsRemoteOrLocal {
    fn is_remote(&self) -> bool;
    fn is_local(&self) -> bool;
}
impl IsRemoteOrLocal for UserId {
    fn is_remote(&self) -> bool {
        self.server_name() != config::get().server_name
    }
    fn is_local(&self) -> bool {
        self.server_name() == config::get().server_name
    }
}
impl IsRemoteOrLocal for OwnedUserId {
    fn is_remote(&self) -> bool {
        self.server_name() != config::get().server_name
    }
    fn is_local(&self) -> bool {
        self.server_name() == config::get().server_name
    }
}

impl IsRemoteOrLocal for RoomId {
    fn is_remote(&self) -> bool {
        self.server_name()
            .map(|s| s != config::get().server_name)
            .unwrap_or(false)
    }
    fn is_local(&self) -> bool {
        self.server_name()
            .map(|s| s == config::get().server_name)
            .unwrap_or(false)
    }
}
impl IsRemoteOrLocal for OwnedRoomId {
    fn is_remote(&self) -> bool {
        self.server_name()
            .map(|s| s != config::get().server_name)
            .unwrap_or(false)
    }
    fn is_local(&self) -> bool {
        self.server_name()
            .map(|s| s == config::get().server_name)
            .unwrap_or(false)
    }
}

impl IsRemoteOrLocal for RoomAliasId {
    fn is_remote(&self) -> bool {
        self.server_name() != config::get().server_name
    }
    fn is_local(&self) -> bool {
        self.server_name() == config::get().server_name
    }
}

impl IsRemoteOrLocal for OwnedRoomAliasId {
    fn is_remote(&self) -> bool {
        self.server_name() != config::get().server_name
    }
    fn is_local(&self) -> bool {
        self.server_name() == config::get().server_name
    }
}

impl IsRemoteOrLocal for ServerName {
    fn is_remote(&self) -> bool {
        self != config::get().server_name
    }
    fn is_local(&self) -> bool {
        self == config::get().server_name
    }
}
