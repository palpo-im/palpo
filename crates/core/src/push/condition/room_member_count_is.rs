use std::fmt;
use std::ops::{Bound, RangeBounds, RangeFrom, RangeTo, RangeToInclusive};
use std::str::FromStr;

use salvo::oapi::ToSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// One of `==`, `<`, `>`, `>=` or `<=`.
///
/// Used by `RoomMemberCountIs`. Defaults to `==`.
#[derive(ToSchema, Copy, Clone, Debug, Default, Eq, PartialEq)]
#[allow(clippy::exhaustive_enums)]
pub enum ComparisonOperator {
    /// Equals
    #[default]
    Eq,

    /// Less than
    Lt,

    /// Greater than
    Gt,

    /// Greater or equal
    Ge,

    /// Less or equal
    Le,
}

/// A decimal integer optionally prefixed by one of `==`, `<`, `>`, `>=` or
/// `<=`.
///
/// A prefix of `<` matches rooms where the member count is strictly less than
/// the given number and so forth. If no prefix is present, this parameter
/// defaults to `==`.
///
/// Can be constructed from a number or a range:
/// ```
/// use palpo_core::push::RoomMemberCountIs;
///
/// // equivalent to `is: "3"` or `is: "==3"`
/// let exact = RoomMemberCountIs::from(3);
///
/// // equivalent to `is: ">=3"`
/// let greater_or_equal = RoomMemberCountIs::from(3..);
///
/// // equivalent to `is: "<3"`
/// let less = RoomMemberCountIs::from(..3);
///
/// // equivalent to `is: "<=3"`
/// let less_or_equal = RoomMemberCountIs::from(..=3);
///
/// // An exclusive range can be constructed with `RoomMemberCountIs::gt`:
/// // (equivalent to `is: ">3"`)
/// let greater = RoomMemberCountIs::gt(3);
/// ```
#[derive(ToSchema, Copy, Clone, Debug, Eq, PartialEq)]
#[allow(clippy::exhaustive_structs)]
pub struct RoomMemberCountIs {
    /// One of `==`, `<`, `>`, `>=`, `<=`, or no prefix.
    pub prefix: ComparisonOperator,

    /// The number of people in the room.
    pub count: u64,
}

impl RoomMemberCountIs {
    /// Creates an instance of `RoomMemberCount` equivalent to `<X`,
    /// where X is the specified member count.
    pub fn gt(count: u64) -> Self {
        RoomMemberCountIs {
            prefix: ComparisonOperator::Gt,
            count,
        }
    }
}

impl From<u64> for RoomMemberCountIs {
    fn from(x: u64) -> Self {
        RoomMemberCountIs {
            prefix: ComparisonOperator::Eq,
            count: x,
        }
    }
}

impl From<RangeFrom<u64>> for RoomMemberCountIs {
    fn from(x: RangeFrom<u64>) -> Self {
        RoomMemberCountIs {
            prefix: ComparisonOperator::Ge,
            count: x.start,
        }
    }
}

impl From<RangeTo<u64>> for RoomMemberCountIs {
    fn from(x: RangeTo<u64>) -> Self {
        RoomMemberCountIs {
            prefix: ComparisonOperator::Lt,
            count: x.end,
        }
    }
}

impl From<RangeToInclusive<u64>> for RoomMemberCountIs {
    fn from(x: RangeToInclusive<u64>) -> Self {
        RoomMemberCountIs {
            prefix: ComparisonOperator::Le,
            count: x.end,
        }
    }
}

impl fmt::Display for RoomMemberCountIs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use ComparisonOperator as Op;

        let prefix = match self.prefix {
            Op::Eq => "",
            Op::Lt => "<",
            Op::Gt => ">",
            Op::Ge => ">=",
            Op::Le => "<=",
        };

        write!(f, "{prefix}{}", self.count)
    }
}

impl Serialize for RoomMemberCountIs {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let s = self.to_string();
        s.serialize(serializer)
    }
}

impl FromStr for RoomMemberCountIs {
    type Err = std::num::ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        use ComparisonOperator as Op;

        let (prefix, count_str) = match s {
            s if s.starts_with("<=") => (Op::Le, &s[2..]),
            s if s.starts_with('<') => (Op::Lt, &s[1..]),
            s if s.starts_with(">=") => (Op::Ge, &s[2..]),
            s if s.starts_with('>') => (Op::Gt, &s[1..]),
            s if s.starts_with("==") => (Op::Eq, &s[2..]),
            s => (Op::Eq, s),
        };

        Ok(RoomMemberCountIs {
            prefix,
            count: u64::from_str(count_str)?,
        })
    }
}

impl<'de> Deserialize<'de> for RoomMemberCountIs {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = crate::serde::deserialize_cow_str(deserializer)?;
        FromStr::from_str(&s).map_err(serde::de::Error::custom)
    }
}

impl RangeBounds<u64> for RoomMemberCountIs {
    fn start_bound(&self) -> Bound<&u64> {
        use ComparisonOperator as Op;

        match self.prefix {
            Op::Eq => Bound::Included(&self.count),
            Op::Lt | Op::Le => Bound::Unbounded,
            Op::Gt => Bound::Excluded(&self.count),
            Op::Ge => Bound::Included(&self.count),
        }
    }

    fn end_bound(&self) -> Bound<&u64> {
        use ComparisonOperator as Op;

        match self.prefix {
            Op::Eq => Bound::Included(&self.count),
            Op::Gt | Op::Ge => Bound::Unbounded,
            Op::Lt => Bound::Excluded(&self.count),
            Op::Le => Bound::Included(&self.count),
        }
    }
}

// #[cfg(test)]
// mod tests {
//     use std::ops::RangeBounds;

//     use super::RoomMemberCountIs;

//     #[test]
//     fn eq_range_contains_its_own_count() {
//         let count = u2;
//         let range = RoomMemberCountIs::from(count);

//         assert!(range.contains(&count));
//     }

//     #[test]
//     fn ge_range_contains_large_number() {
//         let range = RoomMemberCountIs::from(u2..);
//         let large_number = 9001;

//         assert!(range.contains(&large_number));
//     }

//     #[test]
//     fn gt_range_does_not_contain_initial_point() {
//         let range = RoomMemberCountIs::gt(2);
//         let initial_point = u2;

//         assert!(!range.contains(&initial_point));
//     }
// }
