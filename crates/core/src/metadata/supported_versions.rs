use std::collections::{BTreeMap, BTreeSet};

use crate::serde::StringEnum;
use crate::{MatrixVersion, PrivOwnedStr};

/// The list of Matrix versions and features supported by a homeserver.
#[derive(Debug, Clone)]
#[allow(clippy::exhaustive_structs)]
pub struct SupportedVersions {
    /// The Matrix versions that are supported by the homeserver.
    ///
    /// This set contains only known versions.
    pub versions: BTreeSet<MatrixVersion>,

    /// The features that are supported by the homeserver.
    ///
    /// This matches the `unstable_features` field of the `/versions` endpoint, without the boolean
    /// value.
    pub features: BTreeSet<FeatureFlag>,
}

impl SupportedVersions {
    /// Construct a `SupportedVersions` from the parts of a `/versions` response.
    ///
    /// Matrix versions that can't be parsed to a `MatrixVersion`, and features with the boolean
    /// value set to `false` are discarded.
    pub fn from_parts(versions: &[String], unstable_features: &BTreeMap<String, bool>) -> Self {
        Self {
            versions: versions
                .iter()
                .flat_map(|s| s.parse::<MatrixVersion>())
                .collect(),
            features: unstable_features
                .iter()
                .filter(|(_, enabled)| **enabled)
                .map(|(feature, _)| feature.as_str().into())
                .collect(),
        }
    }
}

/// The Matrix features supported by Palpo.
///
/// Features that are not behind a cargo feature are features that are part of the Matrix
/// specification and that Palpo still supports, like the unstable version of an endpoint or a
/// stable feature. Features behind a cargo feature are only supported when this feature is enabled.
#[doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/doc/string_enum.md"))]
#[derive(Clone, StringEnum, Hash)]
#[non_exhaustive]
pub enum FeatureFlag {
    /// `fi.mau.msc2246` ([MSC])
    ///
    /// Asynchronous media uploads.
    ///
    /// [MSC]: https://github.com/matrix-org/matrix-spec-proposals/pull/2246
    #[palpo_enum(rename = "fi.mau.msc2246")]
    Msc2246,

    /// `org.matrix.msc2432` ([MSC])
    ///
    /// Updated semantics for publishing room aliases.
    ///
    /// [MSC]: https://github.com/matrix-org/matrix-spec-proposals/pull/2432
    #[palpo_enum(rename = "org.matrix.msc2432")]
    Msc2432,

    /// `fi.mau.msc2659` ([MSC])
    ///
    /// Application service ping endpoint.
    ///
    /// [MSC]: https://github.com/matrix-org/matrix-spec-proposals/pull/2659
    #[palpo_enum(rename = "fi.mau.msc2659")]
    Msc2659,

    /// `fi.mau.msc2659` ([MSC])
    ///
    /// Stable version of the application service ping endpoint.
    ///
    /// [MSC]: https://github.com/matrix-org/matrix-spec-proposals/pull/2659
    #[palpo_enum(rename = "fi.mau.msc2659.stable")]
    Msc2659Stable,

    /// `uk.half-shot.msc2666.query_mutual_rooms` ([MSC])
    ///
    /// Get rooms in common with another user.
    ///
    /// [MSC]: https://github.com/matrix-org/matrix-spec-proposals/pull/2666
    #[cfg(feature = "unstable-msc2666")]
    #[palpo_enum(rename = "uk.half-shot.msc2666.query_mutual_rooms")]
    Msc2666,

    /// `org.matrix.msc3030` ([MSC])
    ///
    /// Jump to date API endpoint.
    ///
    /// [MSC]: https://github.com/matrix-org/matrix-spec-proposals/pull/3030
    #[palpo_enum(rename = "org.matrix.msc3030")]
    Msc3030,

    /// `org.matrix.msc3882` ([MSC])
    ///
    /// Allow an existing session to sign in a new session.
    ///
    /// [MSC]: https://github.com/matrix-org/matrix-spec-proposals/pull/3882
    #[palpo_enum(rename = "org.matrix.msc3882")]
    Msc3882,

    /// `org.matrix.msc3916` ([MSC])
    ///
    /// Authentication for media.
    ///
    /// [MSC]: https://github.com/matrix-org/matrix-spec-proposals/pull/3916
    #[palpo_enum(rename = "org.matrix.msc3916")]
    Msc3916,

    /// `org.matrix.msc3916.stable` ([MSC])
    ///
    /// Stable version of authentication for media.
    ///
    /// [MSC]: https://github.com/matrix-org/matrix-spec-proposals/pull/3916
    #[palpo_enum(rename = "org.matrix.msc3916.stable")]
    Msc3916Stable,

    /// `org.matrix.msc4108` ([MSC])
    ///
    /// Mechanism to allow OIDC sign in and E2EE set up via QR code.
    ///
    /// [MSC]: https://github.com/matrix-org/matrix-spec-proposals/pull/4108
    #[cfg(feature = "unstable-msc4108")]
    #[palpo_enum(rename = "org.matrix.msc4108")]
    Msc4108,

    /// `org.matrix.msc4140` ([MSC])
    ///
    /// Delayed events.
    ///
    /// [MSC]: https://github.com/matrix-org/matrix-spec-proposals/pull/4140
    #[cfg(feature = "unstable-msc4140")]
    #[palpo_enum(rename = "org.matrix.msc4140")]
    Msc4140,

    /// `org.matrix.simplified_msc3575` ([MSC])
    ///
    /// Simplified Sliding Sync.
    ///
    /// [MSC]: https://github.com/matrix-org/matrix-spec-proposals/pull/4186
    #[cfg(feature = "unstable-msc4186")]
    #[palpo_enum(rename = "org.matrix.simplified_msc3575")]
    Msc4186,

    /// `org.matrix.msc4380_invite_permission_config` ([MSC])
    ///
    /// Invite Blocking.
    ///
    /// [MSC]: https://github.com/matrix-org/matrix-spec-proposals/pull/4380
    #[cfg(feature = "unstable-msc4380")]
    #[palpo_enum(rename = "org.matrix.msc4380")]
    Msc4380,

    #[doc(hidden)]
    _Custom(PrivOwnedStr),
}
