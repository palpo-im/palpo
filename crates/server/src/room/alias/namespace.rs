//! C-S application-service alias authority. Vendor administration remains an
//! explicit separate operation; an ordinary admin access token is still C-S.
use crate::appservice::{NamespaceRegex, RegistrationInfo};
use crate::core::RoomAliasId;
use crate::core::appservice::Registration;
use crate::{AppResult, MatrixError, data};

/// Return whether this authenticated AS controls the alias's namespace.
/// Creation by an AS requires its own namespace. For deletion, absence of an
/// own match merely falls back to ordinary C-S permission; only another AS's
/// exclusive namespace is a prohibition. No caller may override another AS's
/// exclusive match by supplying an overlapping nonexclusive pattern.
pub(super) async fn check(
    alias: &RoomAliasId,
    caller: Option<&RegistrationInfo>,
    creating: bool,
) -> AppResult<bool> {
    let caller_id = caller.map(|info| info.registration.id.as_str());
    let mut own_namespace = false;
    // Disabled registrations still reserve their exclusive names: pausing a
    // bridge is not an ownership transfer to people or other appservices.
    for saved in data::appservice::all_registrations().await? {
        let registration: Registration = saved.try_into()?;
        let aliases = NamespaceRegex::try_from(registration.namespaces.aliases)?;
        if Some(registration.id.as_str()) == caller_id {
            own_namespace = aliases.is_match(alias.as_str());
        } else if aliases.is_exclusive_match(alias.as_str()) {
            return Err(MatrixError::exclusive(
                "Room alias belongs to another application service's exclusive namespace",
            )
            .into());
        }
    }
    if creating && caller.is_some() && !own_namespace {
        return Err(MatrixError::exclusive(
            "Application service cannot create an alias outside its namespace",
        )
        .into());
    }
    Ok(own_namespace)
}
