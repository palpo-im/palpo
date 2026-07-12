use std::str::FromStr;

use crate::MatrixError;
use crate::core::Seqnum;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatchToken {
    Live {
        stream_ordering: Seqnum,
    },
    Historic {
        stream_ordering: Seqnum,
        topological_ordering: i64,
    },
}

impl BatchToken {
    pub fn new_live(stream_ordering: Seqnum) -> Self {
        Self::Live { stream_ordering }
    }
    pub fn new_historic(stream_ordering: Seqnum, topological_ordering: i64) -> Self {
        Self::Historic {
            stream_ordering,
            topological_ordering,
        }
    }
    pub fn event_sn(&self) -> Seqnum {
        match self {
            BatchToken::Live { stream_ordering } => stream_ordering.abs(),
            BatchToken::Historic {
                stream_ordering, ..
            } => stream_ordering.abs(),
        }
    }
    pub fn stream_ordering(&self) -> Seqnum {
        match self {
            BatchToken::Live { stream_ordering } => *stream_ordering,
            BatchToken::Historic {
                stream_ordering, ..
            } => *stream_ordering,
        }
    }
    pub fn topological_ordering(&self) -> Option<i64> {
        match self {
            BatchToken::Live { .. } => None,
            BatchToken::Historic {
                topological_ordering,
                ..
            } => Some(*topological_ordering),
        }
    }

    pub const LIVE_MIN: Self = Self::Live { stream_ordering: 0 };
    pub const LIVE_MAX: Self = Self::Live {
        stream_ordering: Seqnum::MAX,
    };
}

// Live tokens start with an "s" followed by the `stream_ordering` of the event
// that comes before the position of the token. Said another way:
// `stream_ordering` uniquely identifies a persisted event. The live token
// means "the position just after the event identified by `stream_ordering`".
// An example token is:

//     s2633508

// ---

// Historic tokens start with a "t" followed by the `depth`
// (`topological_ordering` in the event graph) of the event that comes before
// the position of the token, followed by "-", followed by the
// `stream_ordering` of the event that comes before the position of the token.
// An example token is:

//     t426-2633508

// ---
impl FromStr for BatchToken {
    type Err = MatrixError;
    fn from_str(input: &str) -> Result<Self, Self::Err> {
        if let Some(stripped) = input.strip_prefix('s') {
            let stream_ordering: Seqnum = stripped.parse().map_err(|_| {
                MatrixError::invalid_param("invalid batch token: cannot parse stream ordering")
            })?;
            Ok(BatchToken::Live { stream_ordering })
        } else if let Some(stripped) = input.strip_prefix('t') {
            let parts: Vec<&str> = stripped.splitn(2, '-').collect();
            if parts.len() != 2 {
                return Err(MatrixError::invalid_param(
                    "invalid batch token: missing '-' separator",
                ));
            }
            let topological_ordering: i64 = parts[0].parse().map_err(|_| {
                MatrixError::invalid_param("invalid batch token: cannot parse topological ordering")
            })?;
            let stream_ordering: Seqnum = parts[1].parse().map_err(|_| {
                MatrixError::invalid_param("invalid batch token: cannot parse stream ordering")
            })?;
            Ok(BatchToken::Historic {
                stream_ordering,
                topological_ordering,
            })
        } else if let Ok(stream_ordering) = input.parse::<Seqnum>() {
            // Backward compatibility: older builds (pre-#69) and some paths
            // serialized the raw `stream_ordering` without the leading 's'
            // (e.g. "128"). Clients persist these tokens in local storage and
            // replay them on back-pagination long after the server is upgraded,
            // so reject-on-missing-prefix would brick history loading until
            // every client cleared its cache. Treat a bare integer as a live
            // token, which is exactly what those builds meant by it.
            Ok(BatchToken::Live { stream_ordering })
        } else {
            Err(MatrixError::invalid_param(
                "invalid batch token: must start with 's' or 't'",
            ))
        }
    }
}

impl std::fmt::Display for BatchToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BatchToken::Live { stream_ordering } => write!(f, "s{}", stream_ordering),
            BatchToken::Historic {
                stream_ordering,
                topological_ordering,
            } => write!(f, "t{}-{}", topological_ordering, stream_ordering),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_live_and_historic_tokens() {
        assert_eq!(
            "s2633508".parse::<BatchToken>().unwrap(),
            BatchToken::Live {
                stream_ordering: 2633508
            }
        );
        assert_eq!(
            "t426-2633508".parse::<BatchToken>().unwrap(),
            BatchToken::Historic {
                stream_ordering: 2633508,
                topological_ordering: 426
            }
        );
    }

    #[test]
    fn accepts_bare_numeric_token_for_backward_compat() {
        // Tokens minted by pre-#69 builds (and replayed from client storage)
        // have no 's' prefix; they must still resolve to a live token.
        assert_eq!(
            "128".parse::<BatchToken>().unwrap(),
            BatchToken::Live {
                stream_ordering: 128
            }
        );
    }

    #[test]
    fn rejects_non_numeric_garbage() {
        assert!("garbage".parse::<BatchToken>().is_err());
        assert!("".parse::<BatchToken>().is_err());
    }
}
