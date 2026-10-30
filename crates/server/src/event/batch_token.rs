use std::str::FromStr;

use crate::MatrixError;
use crate::core::Seqnum;

/// Sync-only response identities never change the event stream component.
pub(crate) fn split_sync_token(input: &str) -> Result<(&str, Option<i64>), MatrixError> {
    let mut parts = input.split('_');
    let base = parts.next().unwrap_or_default();
    let mut invite = None;
    let mut window_seen = false;
    for part in parts {
        if let Some(id) = part.strip_prefix('i') {
            let id =
                id.parse::<i64>().ok().filter(|id| *id > 0).ok_or_else(|| {
                    MatrixError::invalid_param("invalid invitation delivery token")
                })?;
            if invite.replace(id).is_some() {
                return Err(MatrixError::invalid_param(
                    "duplicate invitation delivery token",
                ));
            }
        } else if let Some(id) = part.strip_prefix('w')
            && !window_seen
            && !id.is_empty()
            && id.len() <= 64
            && id.bytes().all(|b| b.is_ascii_alphanumeric())
        {
            window_seen = true;
        } else {
            return Err(MatrixError::invalid_param("invalid sync response token"));
        }
    }
    Ok((base, invite))
}

#[derive(Clone, Debug, Default)]
pub struct SyncPosition {
    pub(crate) event_sn: Seqnum,
    pub(crate) invite_batch: Option<i64>,
    pub(crate) original: Option<String>,
}

impl From<Seqnum> for SyncPosition {
    fn from(event_sn: Seqnum) -> Self {
        Self {
            event_sn,
            ..Self::default()
        }
    }
}

impl FromStr for SyncPosition {
    type Err = MatrixError;
    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let (base, invite_batch) = split_sync_token(input)?;
        let event_sn = base
            .parse()
            .map_err(|_| MatrixError::invalid_param("invalid sync position"))?;
        Ok(Self {
            event_sn,
            invite_batch,
            original: Some(input.to_owned()),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatchToken {
    /// A V3 sync publication position, distinct from an event's stored identity.
    Publication {
        stream_ordering: Seqnum,
    },
    /// A topology seek qualified by its original publication window.
    /// This is a messages-page cursor, never a global sync position.
    PublicationPage {
        lower: Option<Seqnum>,
        upper: Option<Seqnum>,
        topological_ordering: i64,
        stream_ordering: Seqnum,
        identity_lower: Option<Seqnum>,
        identity_upper: Option<Seqnum>,
        child_lower: Option<Seqnum>,
        child_upper: Option<Seqnum>,
    },
    Live {
        stream_ordering: Seqnum,
    },
    Historic {
        stream_ordering: Seqnum,
        topological_ordering: i64,
    },
}

impl BatchToken {
    pub fn new_publication(stream_ordering: Seqnum) -> Self {
        Self::Publication { stream_ordering }
    }
    pub fn new_publication_page(
        lower: Option<Seqnum>,
        upper: Option<Seqnum>,
        topological_ordering: i64,
        stream_ordering: Seqnum,
    ) -> Self {
        Self::new_publication_page_with_bounds(
            lower,
            upper,
            topological_ordering,
            stream_ordering,
            None,
            None,
            None,
            None,
        )
    }
    pub fn new_publication_page_with_identity(
        lower: Option<Seqnum>,
        upper: Option<Seqnum>,
        topological_ordering: i64,
        stream_ordering: Seqnum,
        identity_lower: Option<Seqnum>,
        identity_upper: Option<Seqnum>,
    ) -> Self {
        Self::new_publication_page_with_bounds(
            lower,
            upper,
            topological_ordering,
            stream_ordering,
            identity_lower,
            identity_upper,
            None,
            None,
        )
    }
    pub fn new_publication_page_with_bounds(
        lower: Option<Seqnum>,
        upper: Option<Seqnum>,
        topological_ordering: i64,
        stream_ordering: Seqnum,
        identity_lower: Option<Seqnum>,
        identity_upper: Option<Seqnum>,
        child_lower: Option<Seqnum>,
        child_upper: Option<Seqnum>,
    ) -> Self {
        Self::PublicationPage {
            lower,
            upper,
            topological_ordering,
            stream_ordering,
            identity_lower,
            identity_upper,
            child_lower,
            child_upper,
        }
    }
    pub fn is_publication_page(&self) -> bool {
        matches!(self, Self::PublicationPage { .. })
    }
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
            BatchToken::Publication { stream_ordering } => stream_ordering.abs(),
            BatchToken::PublicationPage {
                stream_ordering, ..
            } => stream_ordering.abs(),
            BatchToken::Live { stream_ordering } => stream_ordering.abs(),
            BatchToken::Historic {
                stream_ordering, ..
            } => stream_ordering.abs(),
        }
    }
    pub fn stream_ordering(&self) -> Seqnum {
        match self {
            BatchToken::Publication { stream_ordering } => *stream_ordering,
            BatchToken::PublicationPage {
                stream_ordering, ..
            } => *stream_ordering,
            BatchToken::Live { stream_ordering } => *stream_ordering,
            BatchToken::Historic {
                stream_ordering, ..
            } => *stream_ordering,
        }
    }
    pub fn topological_ordering(&self) -> Option<i64> {
        match self {
            BatchToken::Publication { .. } | BatchToken::Live { .. } => None,
            BatchToken::PublicationPage {
                topological_ordering,
                ..
            }
            | BatchToken::Historic {
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
// Publication tokens start with "p" and carry a nonnegative sync publication position.
// Qualified messages pages use q<lower|n>.<upper|n>.<depth>.<signed identity seek>;
// With signed legacy identity bounds, append .<identity lower|n>.<identity upper|n>.
// Absolute child identity bounds add .<child lower|n>.<child upper|n>.
// Publication, signed stream identity and absolute child identity are independent
// half-open intervals. Display uses q4/q6/q8 as required by the present bounds.
// They deliberately retain their own variant: this number must not be used as
// an event identity bound by a generic timeline reader.
impl FromStr for BatchToken {
    type Err = MatrixError;
    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let (input, _) = split_sync_token(input)?;
        if let Some(stripped) = input.strip_prefix('p') {
            let stream_ordering: Seqnum = stripped.parse().map_err(|_| {
                MatrixError::invalid_param("invalid batch token: cannot parse publication ordering")
            })?;
            if stream_ordering < 0 {
                return Err(MatrixError::invalid_param(
                    "invalid batch token: publication ordering is out of range",
                ));
            }
            Ok(BatchToken::Publication { stream_ordering })
        } else if let Some(stripped) = input.strip_prefix('q') {
            let invalid = || MatrixError::invalid_param("invalid publication page token");
            let parse_bound = |input: &str| -> Result<Option<Seqnum>, MatrixError> {
                if input == "n" {
                    Ok(None)
                } else {
                    let value: Seqnum = input.parse().map_err(|_| invalid())?;
                    if value < 0 {
                        return Err(invalid());
                    }
                    Ok(Some(value))
                }
            };
            let mut parts = stripped.split('.');
            let lower = parse_bound(parts.next().ok_or_else(invalid)?)?;
            let upper = parse_bound(parts.next().ok_or_else(invalid)?)?;
            let topological_ordering: i64 = parts
                .next()
                .ok_or_else(invalid)?
                .parse()
                .map_err(|_| invalid())?;
            let stream_ordering: Seqnum = parts
                .next()
                .ok_or_else(invalid)?
                .parse()
                .map_err(|_| invalid())?;
            let parse_identity = |input: &str| -> Result<Option<Seqnum>, MatrixError> {
                if input == "n" {
                    Ok(None)
                } else {
                    input.parse().map(Some).map_err(|_| invalid())
                }
            };
            let (identity_lower, identity_upper, child_lower, child_upper) = match (
                parts.next(),
                parts.next(),
                parts.next(),
                parts.next(),
                parts.next(),
            ) {
                (None, None, None, None, None) => (None, None, None, None),
                (Some(lower), Some(upper), None, None, None) => {
                    (parse_identity(lower)?, parse_identity(upper)?, None, None)
                }
                (Some(il), Some(iu), Some(cl), Some(cu), None) => (
                    parse_identity(il)?,
                    parse_identity(iu)?,
                    parse_bound(cl)?,
                    parse_bound(cu)?,
                ),
                _ => return Err(invalid()),
            };
            if matches!((lower, upper), (Some(lower), Some(upper)) if lower > upper)
                || matches!((identity_lower, identity_upper), (Some(lower), Some(upper)) if lower > upper)
                || matches!((child_lower, child_upper), (Some(lower), Some(upper)) if lower > upper)
                || topological_ordering < 0
                || stream_ordering == Seqnum::MIN
            {
                return Err(invalid());
            }
            Ok(Self::new_publication_page_with_bounds(
                lower,
                upper,
                topological_ordering,
                stream_ordering,
                identity_lower,
                identity_upper,
                child_lower,
                child_upper,
            ))
        } else if let Some(stripped) = input.strip_prefix('s') {
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
                "invalid batch token: must start with 'p', 'q', 's' or 't'",
            ))
        }
    }
}

impl std::fmt::Display for BatchToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BatchToken::Publication { stream_ordering } => write!(f, "p{}", stream_ordering),
            BatchToken::PublicationPage {
                lower,
                upper,
                topological_ordering,
                stream_ordering,
                identity_lower,
                identity_upper,
                child_lower,
                child_upper,
            } => {
                match lower {
                    Some(lower) => write!(f, "q{lower}.")?,
                    None => write!(f, "qn.")?,
                }
                match upper {
                    Some(upper) => write!(f, "{upper}.")?,
                    None => write!(f, "n.")?,
                }
                write!(f, "{topological_ordering}.{stream_ordering}")?;
                if identity_lower.is_some()
                    || identity_upper.is_some()
                    || child_lower.is_some()
                    || child_upper.is_some()
                {
                    match identity_lower {
                        Some(lower) => write!(f, ".{lower}")?,
                        None => write!(f, ".n")?,
                    }
                    match identity_upper {
                        Some(upper) => write!(f, ".{upper}")?,
                        None => write!(f, ".n")?,
                    }
                }
                if child_lower.is_some() || child_upper.is_some() {
                    match child_lower {
                        Some(lower) => write!(f, ".{lower}")?,
                        None => write!(f, ".n")?,
                    }
                    match child_upper {
                        Some(upper) => write!(f, ".{upper}")?,
                        None => write!(f, ".n")?,
                    }
                }
                Ok(())
            }
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

    #[test]
    fn response_markers_preserve_the_event_cursor() {
        let live: BatchToken = "s42_i19".parse().unwrap();
        assert_eq!(live, BatchToken::new_live(42));
        let sliding: SyncPosition = "42_i19_wWindow123".parse().unwrap();
        assert_eq!(sliding.event_sn, 42);
        assert_eq!(sliding.invite_batch, Some(19));
        assert_eq!(sliding.original.as_deref(), Some("42_i19_wWindow123"));
        for invalid in [
            "42_i0",
            "42_i-1",
            "42_i19_i20",
            "42_w",
            "42_wA_wB",
            "42_x1",
            "42_i99999999999999999999",
        ] {
            assert!(invalid.parse::<SyncPosition>().is_err(), "{invalid}");
        }
    }

    #[test]
    fn publication_tokens_roundtrip_without_becoming_event_identity_tokens() {
        for position in [0, 1, Seqnum::MAX] {
            let token = BatchToken::new_publication(position);
            assert_eq!(token.to_string(), format!("p{position}"));
            assert_eq!(token.to_string().parse::<BatchToken>().unwrap(), token);
            assert_eq!(token.stream_ordering(), position);
            assert_eq!(token.event_sn(), position.abs());
            assert_eq!(token.topological_ordering(), None);
            assert_ne!(token, BatchToken::new_live(position));
        }
        // Signed numeric parsing follows the existing live-token convention.
        assert_eq!("p+42".parse::<BatchToken>().unwrap().to_string(), "p42");
    }

    #[test]
    fn publication_response_suffixes_preserve_typed_cursor_and_invite_batch() {
        for input in ["p42_i19_wWindow123", "p42_wWindow123_i19"] {
            let (base, invite_batch) = split_sync_token(input).unwrap();
            assert_eq!(base, "p42");
            assert_eq!(invite_batch, Some(19));
            assert_eq!(
                input.parse::<BatchToken>().unwrap(),
                BatchToken::new_publication(42)
            );
        }
        assert!("p-42_i19_wWindow123".parse::<BatchToken>().is_err());
        // V5's separate bare-integer position format does not silently change.
        assert!("p42_i19_wWindow123".parse::<SyncPosition>().is_err());
    }

    #[test]
    fn publication_parser_rejects_overflow_and_malformed_tokens() {
        for input in [
            "p",
            "p-",
            "p+",
            "pp1",
            "P1",
            "p1.0",
            "p1-2",
            "p 1",
            " p1",
            "p1 ",
            "p9223372036854775808",
            "p-9223372036854775809",
            "p-9223372036854775808",
            "p-1",
            "p-42",
            "p1_i0",
            "p1_i-1",
            "p1_i19_i20",
            "p1_w",
            "p1_wA_wB",
            "p1_x1",
            "p1_i99999999999999999999",
            "p1_wA-B",
        ] {
            assert!(input.parse::<BatchToken>().is_err(), "{input}");
        }
        for input in [
            format!("p1_w{}", "A".repeat(65)),
            format!("p{}_i19", Seqnum::MIN),
        ] {
            assert!(input.parse::<BatchToken>().is_err(), "{input}");
        }
    }

    #[test]
    fn publication_support_preserves_legacy_token_values_and_rejections() {
        for (input, position) in [
            ("s0", 0),
            ("s-42", -42),
            ("s+42", 42),
            ("42", 42),
            ("-42", -42),
            ("+42", 42),
        ] {
            let token: BatchToken = input.parse().unwrap();
            assert_eq!(token, BatchToken::new_live(position));
            assert_eq!(token.stream_ordering(), position);
            assert_eq!(token.event_sn(), position.abs());
        }
        let historic: BatchToken = "t426--42_i19_wWindow123".parse().unwrap();
        assert_eq!(historic, BatchToken::new_historic(-42, 426));
        assert_eq!(historic.topological_ordering(), Some(426));
        assert_eq!(
            historic.to_string().parse::<BatchToken>().unwrap(),
            historic
        );
        // The new publication-only lower bound must not alter old parsing.
        for input in [Seqnum::MIN.to_string(), format!("s{}", Seqnum::MIN)] {
            assert_eq!(
                input.parse::<BatchToken>().unwrap(),
                BatchToken::new_live(Seqnum::MIN)
            );
        }
        assert_eq!(
            format!("t0-{}", Seqnum::MIN).parse::<BatchToken>().unwrap(),
            BatchToken::new_historic(Seqnum::MIN, 0)
        );
        for input in [
            "s",
            "s-",
            "t",
            "t1",
            "t-1-42",
            "t1-no",
            "t1-",
            "128garbage",
            " 128",
        ] {
            assert!(input.parse::<BatchToken>().is_err(), "{input}");
        }
    }

    #[test]
    fn publication_pages_roundtrip_signed_historic_seeks_and_optional_windows() {
        for (lower, upper) in [
            (None, None),
            (Some(0), None),
            (None, Some(0)),
            (Some(0), Some(0)),
            (Some(3), Some(9)),
            (Some(Seqnum::MAX), Some(Seqnum::MAX)),
        ] {
            for seek in [0, 1, -1, Seqnum::MIN + 1, Seqnum::MAX] {
                let token = BatchToken::new_publication_page(lower, upper, 42, seek);
                assert!(token.is_publication_page());
                assert_eq!(token.to_string().parse::<BatchToken>().unwrap(), token);
                assert_eq!(token.event_sn(), seek.abs());
                assert_eq!(token.stream_ordering(), seek);
                assert_eq!(token.topological_ordering(), Some(42));
            }
        }
        assert_eq!(
            BatchToken::new_publication_page(None, None, 0, -7).to_string(),
            "qn.n.0.-7"
        );
        assert_eq!(
            BatchToken::new_publication_page(Some(0), None, 0, -7).to_string(),
            "q0.n.0.-7"
        );
        assert_ne!(
            "qn.n.0.-7".parse::<BatchToken>().unwrap(),
            "q0.n.0.-7".parse::<BatchToken>().unwrap()
        );
        assert!(!BatchToken::new_publication(0).is_publication_page());
        assert!(!BatchToken::new_live(0).is_publication_page());
        assert!(!BatchToken::new_historic(0, 0).is_publication_page());
    }

    #[test]
    fn publication_page_suffixes_preserve_window_and_seek() {
        for input in ["q0.19.42.-7_i19_wWindow123", "q0.19.42.-7_wWindow123_i19"] {
            let (base, invite) = split_sync_token(input).unwrap();
            assert_eq!(base, "q0.19.42.-7");
            assert_eq!(invite, Some(19));
            assert_eq!(
                input.parse::<BatchToken>().unwrap(),
                BatchToken::new_publication_page(Some(0), Some(19), 42, -7)
            );
        }
        assert!(
            "q0.19.42.-7_i19_wWindow123"
                .parse::<SyncPosition>()
                .is_err()
        );
    }

    #[test]
    fn publication_page_parser_rejects_invalid_windows_and_historic_seeks() {
        for input in [
            "q",
            "q0.1.2",
            "q0.1.2.3.4",
            "q.1.2.3",
            "q0..2.3",
            "qN.n.0.1",
            "q-1.n.0.1",
            "qn.-1.0.1",
            "q2.1.0.1",
            "qn.n.-1.1",
            "qn.n.0.-9223372036854775808",
            "q9223372036854775808.n.0.1",
            "qn.9223372036854775808.0.1",
            "qn.n.9223372036854775808.1",
            "qn.n.0.9223372036854775808",
            "qn.n.0.-9223372036854775809",
            "qn.n.n.1",
            "qn.n.0.n",
            "q0.1.2.3 ",
            " q0.1.2.3",
            "q0.1.2.3_i0",
            "q0.1.2.3_i1_i2",
            "q0.1.2.3_wA_wB",
        ] {
            assert!(input.parse::<BatchToken>().is_err(), "{input}");
        }
        // An empty window is meaningful and distinct from a reversed window.
        assert!("q19.19.0.-1".parse::<BatchToken>().is_ok());
        assert!("qn.n.9223372036854775807.0".parse::<BatchToken>().is_ok());
    }

    #[test]
    fn publication_pages_preserve_independent_signed_identity_windows() {
        for (identity_lower, identity_upper) in [
            (Some(Seqnum::MIN), Some(Seqnum::MAX)),
            (Some(-9), Some(-1)),
            (Some(-9), Some(0)),
            (Some(0), Some(0)),
            (Some(Seqnum::MIN), Some(Seqnum::MIN)),
            (None, Some(-1)),
            (Some(-1), None),
            (None, None),
        ] {
            let token = BatchToken::new_publication_page_with_identity(
                Some(10),
                Some(20),
                42,
                -7,
                identity_lower,
                identity_upper,
            );
            assert_eq!(token.to_string().parse::<BatchToken>().unwrap(), token);
            assert_eq!(token.event_sn(), 7);
            assert_eq!(token.stream_ordering(), -7);
            assert_eq!(token.topological_ordering(), Some(42));
            assert!(matches!(
                token,
                BatchToken::PublicationPage {
                    lower: Some(10),
                    upper: Some(20),
                    identity_lower: lower,
                    identity_upper: upper,
                    ..
                } if lower == identity_lower && upper == identity_upper
            ));
        }
        let old = BatchToken::new_publication_page(Some(10), Some(20), 42, -7);
        assert_eq!(old.to_string(), "q10.20.42.-7");
        assert_eq!(
            old,
            BatchToken::new_publication_page_with_identity(Some(10), Some(20), 42, -7, None, None,)
        );
        assert_eq!("q10.20.42.-7.n.n".parse::<BatchToken>().unwrap(), old);
        assert_eq!(
            "qn.n.0.1.-9.n".parse::<BatchToken>().unwrap().to_string(),
            "qn.n.0.1.-9.n"
        );
        assert_eq!(
            "q10.20.42.-7.n.+9"
                .parse::<BatchToken>()
                .unwrap()
                .to_string(),
            "q10.20.42.-7.n.9"
        );
        assert!("q10.10.42.-7.-9.-9".parse::<BatchToken>().is_ok());
    }

    #[test]
    fn publication_identity_windows_preserve_response_suffixes() {
        let input = "q10.20.42.-7.-9.9_i19_wWindow123";
        let (base, invite_batch) = split_sync_token(input).unwrap();
        assert_eq!(base, "q10.20.42.-7.-9.9");
        assert_eq!(invite_batch, Some(19));
        assert_eq!(
            input.parse::<BatchToken>().unwrap(),
            BatchToken::new_publication_page_with_identity(
                Some(10),
                Some(20),
                42,
                -7,
                Some(-9),
                Some(9),
            )
        );
        assert_eq!(input.parse::<BatchToken>().unwrap().to_string(), base);
    }

    #[test]
    fn publication_page_parser_rejects_malformed_or_reversed_identity_windows() {
        for input in [
            "q10.20.42.-7.n",
            "q10.20.42.-7.n.n.n",
            "q10.20.42.-7..n",
            "q10.20.42.-7.n.",
            "q10.20.42.-7.x.9",
            "q10.20.42.-7.n.x",
            "q10.20.42.-7.9.-9",
            "q10.20.42.-7.-1.-2",
            "q10.20.42.-7.-9223372036854775807.-9223372036854775808",
            "q10.20.42.-7.-9223372036854775809.n",
            "q10.20.42.-7.n.9223372036854775808",
            "q10.20.42.-7. 1.9",
            "q10.20.42.-7.1.9 ",
            "q-1.20.42.-7.-9.9",
            "q20.10.42.-7.-9.9",
            "q10.20.-1.-7.-9.9",
            "q10.20.42.-9223372036854775808.-9.9",
        ] {
            assert!(input.parse::<BatchToken>().is_err(), "{input}");
        }
    }

    #[test]
    fn publication_pages_preserve_three_independent_window_axes() {
        for (child_lower, child_upper) in [
            (Some(0), None),
            (None, Some(0)),
            (Some(0), Some(Seqnum::MAX)),
            (Some(Seqnum::MAX), Some(Seqnum::MAX)),
            (Some(Seqnum::MAX), None),
            (None, None),
        ] {
            let token = BatchToken::new_publication_page_with_bounds(
                Some(10),
                Some(20),
                42,
                -7,
                Some(Seqnum::MIN),
                Some(-1),
                child_lower,
                child_upper,
            );
            assert_eq!(token.to_string().parse::<BatchToken>().unwrap(), token);
            assert_eq!(token.event_sn(), 7);
            assert!(matches!(
                token,
                BatchToken::PublicationPage {
                    lower: Some(10), upper: Some(20),
                    identity_lower: Some(Seqnum::MIN), identity_upper: Some(-1),
                    child_lower: cl, child_upper: cu, ..
                } if cl == child_lower && cu == child_upper
            ));
        }
        let zero_child = BatchToken::new_publication_page_with_bounds(
            None,
            None,
            42,
            -7,
            None,
            None,
            Some(0),
            None,
        );
        assert_eq!(zero_child.to_string(), "qn.n.42.-7.n.n.0.n");
        assert_ne!(
            zero_child,
            BatchToken::new_publication_page(None, None, 42, -7)
        );
        assert_eq!(
            "q10.20.42.-7.-8.-2.100.200"
                .parse::<BatchToken>()
                .unwrap()
                .to_string(),
            "q10.20.42.-7.-8.-2.100.200",
            "separate axes need not numerically overlap"
        );
        assert!("q10.10.42.-7.-8.-8.0.0".parse::<BatchToken>().is_ok());
    }

    #[test]
    fn publication_child_windows_preserve_legacy_forms_and_response_suffixes() {
        let old = BatchToken::new_publication_page(Some(10), Some(20), 42, -7);
        assert_eq!("q10.20.42.-7.n.n.n.n".parse::<BatchToken>().unwrap(), old);
        assert_eq!(
            "q10.20.42.-7.n.n.n.n"
                .parse::<BatchToken>()
                .unwrap()
                .to_string(),
            "q10.20.42.-7"
        );
        assert_eq!(
            "q10.20.42.-7.-8.-2.n.n"
                .parse::<BatchToken>()
                .unwrap()
                .to_string(),
            "q10.20.42.-7.-8.-2"
        );
        let input = "q10.20.42.-7.-8.-2.100.200_i19_wWindow123";
        let (base, invite_batch) = split_sync_token(input).unwrap();
        assert_eq!(invite_batch, Some(19));
        let token = input.parse::<BatchToken>().unwrap();
        assert_eq!(token.to_string(), base);
        assert_eq!(
            token,
            BatchToken::new_publication_page_with_bounds(
                Some(10),
                Some(20),
                42,
                -7,
                Some(-8),
                Some(-2),
                Some(100),
                Some(200),
            )
        );
    }

    #[test]
    fn publication_page_parser_rejects_invalid_child_windows_and_arity() {
        for input in [
            "qn.n.0.1.n.n.0",
            "qn.n.0.1.n.n.0.1.n",
            "qn.n.0.1.n.n.-1.n",
            "qn.n.0.1.n.n.n.-1",
            "qn.n.0.1.n.n.2.1",
            "qn.n.0.1.n.n.9223372036854775808.n",
            "qn.n.0.1.n.n.n.9223372036854775808",
            "qn.n.0.1.n.n.-9223372036854775808.n",
            "qn.n.0.1.n.n.x.1",
            "qn.n.0.1.n.n.0.",
            "qn.n.0.1.n.n..1",
            "qn.n.0.1.2.1.0.10",
            "q2.1.0.1.n.n.0.10",
            "qn.n.0.-9223372036854775808.n.n.0.10",
        ] {
            assert!(input.parse::<BatchToken>().is_err(), "{input}");
        }
    }
}
