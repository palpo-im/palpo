use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use diesel::deserialize::FromSql;
use diesel::serialize::ToSql;
use diesel::{AsExpression, FromSqlRow, pg, sql_types};
use salvo::prelude::*;
use serde::{Deserialize, Serialize};

/// A timestamp represented as the number of milliseconds since the unix epoch.
#[derive(
    ToSchema,
    FromSqlRow,
    AsExpression,
    Default,
    Clone,
    Copy,
    Hash,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Deserialize,
    Serialize,
)]
#[diesel(sql_type = sql_types::Bigint)]
#[allow(clippy::exhaustive_structs)]
#[serde(transparent)]
pub struct UnixMillis(pub u64);
impl fmt::Display for UnixMillis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl UnixMillis {
    /// Creates a new `UnixMillis` from the given `SystemTime`, if it is not
    /// before the unix epoch, or too large to be represented.
    pub fn from_system_time(time: SystemTime) -> Option<Self> {
        let duration = time.duration_since(UNIX_EPOCH).ok()?;
        let millis = duration.as_millis().try_into().ok()?;
        Some(Self(millis))
    }

    /// The current system time in milliseconds since the unix epoch.
    pub fn now() -> Self {
        Self::from_system_time(SystemTime::now()).expect("date out of range")
    }

    /// Creates a new `SystemTime` from `self`, if it can be represented.
    pub fn to_system_time(self) -> Option<SystemTime> {
        UNIX_EPOCH.checked_add(Duration::from_millis(self.0))
    }

    /// Get the time since the unix epoch in milliseconds.
    pub fn get(&self) -> u64 {
        self.0
    }

    /// Get time since the unix epoch in seconds.
    pub fn as_secs(&self) -> u64 {
        self.0 / 1000
    }
}

impl fmt::Debug for UnixMillis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The default Debug impl would put the inner value on its own line if the
        // formatter's alternate mode is enabled, which bloats debug strings
        // unnecessarily
        write!(f, "UnixMillis({})", self.0)
    }
}
impl FromSql<sql_types::BigInt, pg::Pg> for UnixMillis {
    fn from_sql(bytes: diesel::pg::PgValue<'_>) -> diesel::deserialize::Result<Self> {
        let value = <i64 as diesel::deserialize::FromSql<
            diesel::sql_types::BigInt,
            diesel::pg::Pg,
        >>::from_sql(bytes)?;
        Ok(Self(value as u64))
    }
}

impl ToSql<sql_types::BigInt, pg::Pg> for UnixMillis {
    fn to_sql(
        &self,
        out: &mut diesel::serialize::Output<'_, '_, pg::Pg>,
    ) -> diesel::serialize::Result {
        ToSql::<sql_types::BigInt, pg::Pg>::to_sql(&(self.0 as i64), &mut out.reborrow())
    }
}

/// A timestamp represented as the number of seconds since the unix epoch.
#[derive(
    ToSchema,
    FromSqlRow,
    AsExpression,
    Clone,
    Copy,
    Hash,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Deserialize,
    Serialize,
)]
#[diesel(sql_type = sql_types::Bigint)]
#[allow(clippy::exhaustive_structs)]
#[serde(transparent)]
pub struct UnixSeconds(pub u64);

impl UnixSeconds {
    /// Creates a new `UnixMillis` from the given `SystemTime`, if it is not
    /// before the unix epoch, or too large to be represented.
    pub fn from_system_time(time: SystemTime) -> Option<Self> {
        let duration = time.duration_since(UNIX_EPOCH).ok()?;
        Some(Self(duration.as_secs()))
    }

    /// The current system-time as seconds since the unix epoch.
    pub fn now() -> Self {
        Self::from_system_time(SystemTime::now()).expect("date out of range")
    }

    /// Creates a new `SystemTime` from `self`, if it can be represented.
    pub fn to_system_time(self) -> Option<SystemTime> {
        UNIX_EPOCH.checked_add(Duration::from_secs(self.0))
    }

    /// Get time since the unix epoch in seconds.
    pub fn get(&self) -> u64 {
        self.0
    }
}

impl fmt::Debug for UnixSeconds {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The default Debug impl would put the inner value on its own line if the
        // formatter's alternate mode is enabled, which bloats debug strings
        // unnecessarily
        write!(f, "UnixSeconds({})", self.0)
    }
}
impl FromSql<sql_types::BigInt, pg::Pg> for UnixSeconds {
    fn from_sql(bytes: diesel::pg::PgValue<'_>) -> diesel::deserialize::Result<Self> {
        let value = <i64 as diesel::deserialize::FromSql<
            diesel::sql_types::BigInt,
            diesel::pg::Pg,
        >>::from_sql(bytes)?;
        Ok(Self(value as u64))
    }
}

impl ToSql<sql_types::BigInt, pg::Pg> for UnixSeconds {
    fn to_sql(
        &self,
        out: &mut diesel::serialize::Output<'_, '_, pg::Pg>,
    ) -> diesel::serialize::Result {
        ToSql::<sql_types::BigInt, pg::Pg>::to_sql(&(self.0 as i64), &mut out.reborrow())
    }
}

// #[cfg(test)]
// mod tests {
//     use std::time::{Duration, UNIX_EPOCH};

//     use serde::{Deserialize, Serialize};
//     use serde_json::json;

//     use super::{UnixMillis, UnixSeconds};

//     #[derive(Clone, Debug, Deserialize, Serialize)]
//     struct SystemTimeTest {
//         millis: UnixMillis,
//         secs: UnixSeconds,
//     }

//     #[test]
//     fn deserialize() {
//         let json = json!({ "millis": 3000, "secs": 60 });

//         let time = serde_json::from_value::<SystemTimeTest>(json).unwrap();
//         assert_eq!(
//             time.millis.to_system_time(),
//             Some(UNIX_EPOCH + Duration::from_millis(3000))
//         );
//         assert_eq!(time.secs.to_system_time(), Some(UNIX_EPOCH +
// Duration::from_secs(60)));     }

//     #[test]
//     fn serialize() {
//         let request = SystemTimeTest {
//             millis: UnixMillis::from_system_time(UNIX_EPOCH +
// Duration::new(2, 0)).unwrap(),             secs: UnixSeconds(u0),
//         };

//         assert_eq!(
//             serde_json::to_value(request).unwrap(),
//             json!({ "millis": 2000, "secs": 0 })
//         );
//     }
// }
