use salvo::prelude::*;

use crate::core::client::retention::RetentionConfigurationResBody;
use crate::{AuthArgs, DepotExt, JsonResult, MatrixError, config, json_ok, room};

pub(super) fn router() -> Router {
    Router::with_path("org.matrix.msc1763/retention/configuration").get(configuration)
}

#[endpoint]
async fn configuration(
    _aa: AuthArgs,
    depot: &mut Depot,
) -> JsonResult<RetentionConfigurationResBody> {
    if !crate::retention::enabled() {
        return Err(MatrixError::not_found("Message retention is disabled").into());
    }
    let authed = depot.authed_info()?;
    let conf = &config::get().retention;
    let mut policies = std::collections::BTreeMap::new();
    for (key, policy) in &conf.policies {
        if key == "*"
            || room::user::is_joined(authed.user_id(), &crate::core::RoomId::parse(key)?).await?
        {
            policies.insert(key.clone(), policy.clone());
        }
    }
    json_ok(RetentionConfigurationResBody {
        policies,
        limits: conf.limits.clone(),
    })
}
