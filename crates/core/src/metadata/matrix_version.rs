use std::cmp::Ordering;
use std::fmt::{self, Display};
use std::str::FromStr;

use salvo::prelude::*;
use serde::Serialize;

use crate::RoomVersionId;
use crate::error::UnknownVersionError;

/// The Matrix versions Palpo currently understands to exist.
///
/// Matrix, since fall 2021, has a quarterly release schedule, using a global
/// `vX.Y` versioning scheme.
///
/// Every new minor version denotes stable support for endpoints in a
/// *relatively* backwards-compatible manner.
///
/// Matrix has a deprecation policy, read more about it here: <https://spec.matrix.org/latest/#deprecation-policy>.
///
/// Palpo keeps track of when endpoints are added, deprecated, and removed.
/// It'll automatically select the right endpoint stability variation to use
/// depending on which Matrix versions you
/// pass to [`try_into_http_request`](super::OutgoingRequest::try_into_http_request), see its
/// respective documentation for more information.
#[derive(ToSchema, Serialize, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MatrixVersion {
    /// Matrix 1.0 was a release prior to the global versioning system and does not correspond to a
    /// version of the Matrix specification.
    ///
    /// It matches the following per-API versions:
    ///
    /// * Client-Server API: r0.5.0 to r0.6.1
    /// * Identity Service API: r0.2.0 to r0.3.0
    ///
    /// The other APIs are not supported because they do not have a `GET /versions` endpoint.
    ///
    /// See <https://spec.matrix.org/latest/#legacy-versioning>.
    V1_0,

    /// Version 1.1 of the Matrix specification, released in Q4 2021.
    ///
    /// See <https://spec.matrix.org/v1.1/>.
    V1_1,

    /// Version 1.2 of the Matrix specification, released in Q1 2022.
    ///
    /// See <https://spec.matrix.org/v1.2/>.
    V1_2,

    /// Version 1.3 of the Matrix specification, released in Q2 2022.
    ///
    /// See <https://spec.matrix.org/v1.3/>.
    V1_3,

    /// Version 1.4 of the Matrix specification, released in Q3 2022.
    ///
    /// See <https://spec.matrix.org/v1.4/>.
    V1_4,

    /// Version 1.5 of the Matrix specification, released in Q4 2022.
    ///
    /// See <https://spec.matrix.org/v1.5/>.
    V1_5,

    /// Version 1.6 of the Matrix specification, released in Q1 2023.
    ///
    /// See <https://spec.matrix.org/v1.6/>.
    V1_6,

    /// Version 1.7 of the Matrix specification, released in Q2 2023.
    ///
    /// See <https://spec.matrix.org/v1.7/>.
    V1_7,

    /// Version 1.8 of the Matrix specification, released in Q3 2023.
    ///
    /// See <https://spec.matrix.org/v1.8/>.
    V1_8,

    /// Version 1.9 of the Matrix specification, released in Q4 2023.
    ///
    /// See <https://spec.matrix.org/v1.9/>.
    V1_9,

    /// Version 1.10 of the Matrix specification, released in Q1 2024.
    ///
    /// See <https://spec.matrix.org/v1.10/>.
    V1_10,

    /// Version 1.11 of the Matrix specification, released in Q2 2024.
    ///
    /// See <https://spec.matrix.org/v1.11/>.
    V1_11,

    /// Version 1.12 of the Matrix specification, released in Q3 2024.
    ///
    /// See <https://spec.matrix.org/v1.12/>.
    V1_12,

    /// Version 1.13 of the Matrix specification, released in Q4 2024.
    ///
    /// See <https://spec.matrix.org/v1.13/>.
    V1_13,

    /// Version 1.14 of the Matrix specification, released in Q1 2025.
    ///
    /// See <https://spec.matrix.org/v1.14/>.
    V1_14,

    /// Version 1.15 of the Matrix specification, released in Q2 2025.
    ///
    /// See <https://spec.matrix.org/v1.15/>.
    V1_15,

    /// Version 1.16 of the Matrix specification, released in Q3 2025.
    ///
    /// See <https://spec.matrix.org/v1.17/>.
    V1_16,

    /// Version 1.17 of the Matrix specification, released in Q4 2025.
    ///
    /// See <https://spec.matrix.org/v1.17/>.
    V1_17,
}

impl TryFrom<&str> for MatrixVersion {
    type Error = UnknownVersionError;

    fn try_from(value: &str) -> Result<MatrixVersion, Self::Error> {
        use MatrixVersion::*;

        Ok(match value {
            "v1.0" |
            // Additional definitions according to https://spec.matrix.org/latest/#legacy-versioning
            "r0.5.0" | "r0.6.0" | "r0.6.1" => V1_0,
            "v1.1" => V1_1,
            "v1.2" => V1_2,
            "v1.3" => V1_3,
            "v1.4" => V1_4,
            "v1.5" => V1_5,
            "v1.6" => V1_6,
            "v1.7" => V1_7,
            "v1.8" => V1_8,
            "v1.9" => V1_9,
            "v1.10" => V1_10,
            "v1.11" => V1_11,
            "v1.12" => V1_12,
            "v1.13" => V1_13,
            "v1.14" => V1_14,
            "v1.15" => V1_15,
            "v1.16" => V1_16,
            "v1.17" => V1_17,
            _ => return Err(UnknownVersionError),
        })
    }
}

impl FromStr for MatrixVersion {
    type Err = UnknownVersionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_from(s)
    }
}

impl MatrixVersion {
    /// Checks whether a version is compatible with another.
    ///
    /// A is compatible with B as long as B is equal or less, so long as A and B
    /// have the same major versions.
    ///
    /// For example, v1.2 is compatible with v1.1, as it is likely only some
    /// additions of endpoints on top of v1.1, but v1.1 would not be
    /// compatible with v1.2, as v1.1 cannot represent all of v1.2, in a
    /// manner similar to set theory.
    ///
    /// Warning: Matrix has a deprecation policy, and Matrix versioning is not
    /// as straight-forward as this function makes it out to be. This
    /// function only exists to prune major version differences, and
    /// versions too new for `self`.
    ///
    /// This (considering if major versions are the same) is equivalent to a
    /// `self >= other` check.
    pub fn is_superset_of(self, other: Self) -> bool {
        let (major_l, minor_l) = self.into_parts();
        let (major_r, minor_r) = other.into_parts();
        major_l == major_r && minor_l >= minor_r
    }

    /// Get a string representation of this Matrix version.
    ///
    /// This is the string that can be found in the response to one of the `GET /versions`
    /// endpoints. Parsing this string will give the same variant.
    ///
    /// Returns `None` for [`MatrixVersion::V1_0`] because it can match several per-API versions.
    pub const fn as_str(self) -> Option<&'static str> {
        let string = match self {
            MatrixVersion::V1_0 => return None,
            MatrixVersion::V1_1 => "v1.1",
            MatrixVersion::V1_2 => "v1.2",
            MatrixVersion::V1_3 => "v1.3",
            MatrixVersion::V1_4 => "v1.4",
            MatrixVersion::V1_5 => "v1.5",
            MatrixVersion::V1_6 => "v1.6",
            MatrixVersion::V1_7 => "v1.7",
            MatrixVersion::V1_8 => "v1.8",
            MatrixVersion::V1_9 => "v1.9",
            MatrixVersion::V1_10 => "v1.10",
            MatrixVersion::V1_11 => "v1.11",
            MatrixVersion::V1_12 => "v1.12",
            MatrixVersion::V1_13 => "v1.13",
            MatrixVersion::V1_14 => "v1.14",
            MatrixVersion::V1_15 => "v1.15",
            MatrixVersion::V1_16 => "v1.16",
            MatrixVersion::V1_17 => "v1.17",
        };

        Some(string)
    }

    /// Decompose the Matrix version into its major and minor number.
    pub const fn into_parts(self) -> (u8, u8) {
        match self {
            MatrixVersion::V1_0 => (1, 0),
            MatrixVersion::V1_1 => (1, 1),
            MatrixVersion::V1_2 => (1, 2),
            MatrixVersion::V1_3 => (1, 3),
            MatrixVersion::V1_4 => (1, 4),
            MatrixVersion::V1_5 => (1, 5),
            MatrixVersion::V1_6 => (1, 6),
            MatrixVersion::V1_7 => (1, 7),
            MatrixVersion::V1_8 => (1, 8),
            MatrixVersion::V1_9 => (1, 9),
            MatrixVersion::V1_10 => (1, 10),
            MatrixVersion::V1_11 => (1, 11),
            MatrixVersion::V1_12 => (1, 12),
            MatrixVersion::V1_13 => (1, 13),
            MatrixVersion::V1_14 => (1, 14),
            MatrixVersion::V1_15 => (1, 15),
            MatrixVersion::V1_16 => (1, 16),
            MatrixVersion::V1_17 => (1, 17),
        }
    }

    /// Try to turn a pair of (major, minor) version components back into a
    /// `MatrixVersion`.
    pub const fn from_parts(major: u8, minor: u8) -> Result<Self, UnknownVersionError> {
        match (major, minor) {
            (1, 0) => Ok(MatrixVersion::V1_0),
            (1, 1) => Ok(MatrixVersion::V1_1),
            (1, 2) => Ok(MatrixVersion::V1_2),
            (1, 3) => Ok(MatrixVersion::V1_3),
            (1, 4) => Ok(MatrixVersion::V1_4),
            (1, 5) => Ok(MatrixVersion::V1_5),
            (1, 6) => Ok(MatrixVersion::V1_6),
            (1, 7) => Ok(MatrixVersion::V1_7),
            (1, 8) => Ok(MatrixVersion::V1_8),
            (1, 9) => Ok(MatrixVersion::V1_9),
            (1, 10) => Ok(MatrixVersion::V1_10),
            (1, 11) => Ok(MatrixVersion::V1_11),
            (1, 12) => Ok(MatrixVersion::V1_12),
            (1, 13) => Ok(MatrixVersion::V1_13),
            (1, 14) => Ok(MatrixVersion::V1_14),
            (1, 15) => Ok(MatrixVersion::V1_15),
            (1, 16) => Ok(MatrixVersion::V1_16),
            (1, 17) => Ok(MatrixVersion::V1_17),
            _ => Err(UnknownVersionError),
        }
    }

    /// Constructor for use by the `metadata!` macro.
    ///
    /// Accepts string literals and parses them.
    #[doc(hidden)]
    pub const fn from_lit(lit: &'static str) -> Self {
        use konst::{result, string};

        let major: u8;
        let minor: u8;

        let mut split = string::split(lit, ".");

        // Get major version
        let major_str = match split.next() {
            Some(s) => s,
            None => panic!("could not find major version"),
        };

        major = match u8::from_str_radix(major_str, 10) {
            Ok(v) => v,
            Err(_) => panic!("major version is not a valid number"),
        };

        // Get minor version
        let minor_str = match split.next() {
            Some(s) => s,
            None => panic!("could not find dot to denote second number"),
        };

        minor = match u8::from_str_radix(minor_str, 10) {
            Ok(v) => v,
            Err(_) => panic!("minor version is not a valid number"),
        };

        // Check there's no more content
        if split.next().is_some() {
            panic!("version literal contains more than one dot")
        }

        result::unwrap_or_else!(Self::from_parts(major, minor), |_| panic!(
            "not a valid version literal"
        ))
    }

    // Internal function to do ordering in const-fn contexts
    pub(crate) const fn const_ord(&self, other: &Self) -> Ordering {
        let self_parts = self.into_parts();
        let other_parts = other.into_parts();

        use konst::primitive::cmp::cmp_u8;

        let major_ord = cmp_u8(self_parts.0, other_parts.0);
        if major_ord.is_ne() {
            major_ord
        } else {
            cmp_u8(self_parts.1, other_parts.1)
        }
    }

    // Internal function to check if this version is the legacy (v1.0) version in
    // const-fn contexts
    pub(crate) const fn is_legacy(&self) -> bool {
        let self_parts = self.into_parts();

        use konst::primitive::cmp::cmp_u8;

        cmp_u8(self_parts.0, 1).is_eq() && cmp_u8(self_parts.1, 0).is_eq()
    }

    /// Get the default [`RoomVersionId`] for this `MatrixVersion`.
    pub fn default_room_version(&self) -> RoomVersionId {
        match self {
            // <https://spec.matrix.org/historical/index.html#complete-list-of-room-versions>
            MatrixVersion::V1_0
            // <https://spec.matrix.org/v1.1/rooms/#complete-list-of-room-versions>
            | MatrixVersion::V1_1
            // <https://spec.matrix.org/v1.2/rooms/#complete-list-of-room-versions>
            | MatrixVersion::V1_2 => RoomVersionId::V6,
            // <https://spec.matrix.org/v1.3/rooms/#complete-list-of-room-versions>
            MatrixVersion::V1_3
            // <https://spec.matrix.org/v1.4/rooms/#complete-list-of-room-versions>
            | MatrixVersion::V1_4
            // <https://spec.matrix.org/v1.5/rooms/#complete-list-of-room-versions>
            | MatrixVersion::V1_5 => RoomVersionId::V9,
            // <https://spec.matrix.org/v1.6/rooms/#complete-list-of-room-versions>
            MatrixVersion::V1_6
            // <https://spec.matrix.org/v1.7/rooms/#complete-list-of-room-versions>
            | MatrixVersion::V1_7
            // <https://spec.matrix.org/v1.8/rooms/#complete-list-of-room-versions>
            | MatrixVersion::V1_8
            // <https://spec.matrix.org/v1.9/rooms/#complete-list-of-room-versions>
            | MatrixVersion::V1_9
             // <https://spec.matrix.org/v1.10/rooms/#complete-list-of-room-versions>
            | MatrixVersion::V1_10
            // <https://spec.matrix.org/v1.11/rooms/#complete-list-of-room-versions>
            | MatrixVersion::V1_11
            // <https://spec.matrix.org/v1.12/rooms/#complete-list-of-room-versions>
            | MatrixVersion::V1_12
            // <https://spec.matrix.org/v1.13/rooms/#complete-list-of-room-versions>
            | MatrixVersion::V1_13 => RoomVersionId::V10,
            // <https://spec.matrix.org/v1.14/rooms/#complete-list-of-room-versions>
            | MatrixVersion::V1_14
            // <https://spec.matrix.org/v1.15/rooms/#complete-list-of-room-versions>
            | MatrixVersion::V1_15 => RoomVersionId::V11,
            // <https://spec.matrix.org/v1.17/rooms/#complete-list-of-room-versions>
            MatrixVersion::V1_16
            // <https://spec.matrix.org/v1.17/rooms/#complete-list-of-room-versions>
            | MatrixVersion::V1_17 => RoomVersionId::V12,
        }
    }
}

impl Display for MatrixVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (major, minor) = self.into_parts();
        f.write_str(&format!("v{major}.{minor}"))
    }
}
