//! Publication points select live events; qualified pages retain topology and
//! the frozen publication window without reinterpreting historical tokens.
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use indexmap::IndexMap;

use crate::core::client::filter::RoomEventFilter;
use crate::core::identifiers::*;
use crate::core::{Direction, Seqnum};
use crate::data::connect;
use crate::data::schema::*;
use crate::event::BatchToken;
use crate::{AppResult, MatrixError, SnPduEvent};

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Window {
    pub lower: Option<Seqnum>,
    pub upper: Option<Seqnum>,
    /// Legacy /messages signed stream-ordering interval.
    pub identity_lower: Option<Seqnum>,
    pub identity_upper: Option<Seqnum>,
    /// Legacy /relations absolute child identity interval.
    pub child_lower: Option<Seqnum>,
    pub child_upper: Option<Seqnum>,
}

impl Window {
    fn intersect(&mut self, other: Self) {
        self.lower = self.lower.into_iter().chain(other.lower).max();
        self.upper = self.upper.into_iter().chain(other.upper).min();
        self.identity_lower = self
            .identity_lower
            .into_iter()
            .chain(other.identity_lower)
            .max();
        self.identity_upper = self
            .identity_upper
            .into_iter()
            .chain(other.identity_upper)
            .min();
        self.child_lower = self.child_lower.into_iter().chain(other.child_lower).max();
        self.child_upper = self.child_upper.into_iter().chain(other.child_upper).min();
    }
    fn point(&mut self, position: Seqnum, lower: bool) -> AppResult<()> {
        if position < 0 {
            return Err(MatrixError::invalid_param("negative publication position").into());
        }
        self.intersect(if lower {
            Self {
                lower: Some(position),
                upper: None,
                ..Self::default()
            }
        } else {
            Self {
                lower: None,
                upper: Some(position),
                ..Self::default()
            }
        });
        Ok(())
    }
    pub(crate) fn empty(self) -> bool {
        self.lower.zip(self.upper).is_some_and(|(l, u)| l >= u)
            || self
                .identity_lower
                .zip(self.identity_upper)
                .is_some_and(|(l, u)| l >= u)
            || self
                .child_lower
                .zip(self.child_upper)
                .is_some_and(|(l, u)| l >= u)
    }
}

pub(crate) struct Page {
    pub events: IndexMap<Seqnum, SnPduEvent>,
    pub end: Option<BatchToken>,
    window: Window,
}

impl Page {
    pub fn replace_events(&mut self, events: IndexMap<Seqnum, SnPduEvent>, direction: Direction) {
        self.events = events;
        self.end = page_end(self.window, &self.events, direction);
    }
}

fn page_end(
    window: Window,
    events: &IndexMap<Seqnum, SnPduEvent>,
    direction: Direction,
) -> Option<BatchToken> {
    events.last().map(|(_, pdu)| {
        let historic = if direction == Direction::Backward {
            pdu.prev_historic_token()
        } else {
            pdu.historic_token()
        };
        BatchToken::new_publication_page_with_bounds(
            window.lower,
            window.upper,
            historic.topological_ordering().expect("historic cursor"),
            historic.stream_ordering(),
            window.identity_lower,
            window.identity_upper,
            window.child_lower,
            window.child_upper,
        )
    })
}

pub(crate) fn uses_publication(token: BatchToken) -> bool {
    matches!(
        token,
        BatchToken::Publication { .. } | BatchToken::PublicationPage { .. }
    )
}

/// Each cursor retains its own coordinate domain. Legacy live cursors select
/// signed stream identities even when the other endpoint is a publication point.
/// Qualified pages retain all windows independently across endpoint handoffs.
pub(crate) fn bound(
    token: BatchToken,
    from: bool,
    direction: Direction,
    window: &mut Window,
) -> AppResult<Option<BatchToken>> {
    match token {
        BatchToken::Publication { stream_ordering } => {
            window.point(stream_ordering, (direction == Direction::Forward) == from)?;
            Ok(None)
        }
        BatchToken::Live { stream_ordering } => {
            let lower = (direction == Direction::Forward) == from;
            window.intersect(if lower {
                Window {
                    identity_lower: Some(stream_ordering),
                    ..Window::default()
                }
            } else {
                Window {
                    identity_upper: Some(stream_ordering),
                    ..Window::default()
                }
            });
            Ok(None)
        }
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
            window.intersect(Window {
                lower,
                upper,
                identity_lower,
                identity_upper,
                child_lower,
                child_upper,
            });
            Ok(Some(BatchToken::new_historic(
                stream_ordering,
                topological_ordering,
            )))
        }
        historic @ BatchToken::Historic { .. } => Ok(Some(historic)),
    }
}

pub(crate) async fn page(
    user_id: &UserId,
    room_id: &RoomId,
    from: BatchToken,
    to: Option<BatchToken>,
    filter: &RoomEventFilter,
    limit: usize,
    direction: Direction,
) -> AppResult<Page> {
    let mut window = Window::default();
    let historic_from = bound(from, true, direction, &mut window)?;
    let historic_to = to
        .map(|to| bound(to, false, direction, &mut window))
        .transpose()?
        .flatten();
    if window.empty() || limit == 0 {
        return Ok(Page {
            events: IndexMap::new(),
            end: None,
            window,
        });
    }
    let events = super::topolo::load_publication_window(
        Some(user_id),
        room_id,
        historic_from,
        historic_to,
        limit,
        Some(filter),
        (direction, window),
    )
    .await?;
    let end = page_end(window, &events, direction);
    Ok(Page {
        events,
        end,
        window,
    })
}

pub(crate) async fn position(room_id: &RoomId, identity: Seqnum) -> AppResult<Option<Seqnum>> {
    Ok(event_points::table
        .inner_join(
            events::table.on(events::id
                .eq(event_points::event_id)
                .and(events::room_id.eq(event_points::room_id))),
        )
        .filter(event_points::room_id.eq(room_id))
        .filter(event_points::event_sn.eq(identity))
        .filter(events::is_outlier.eq(false))
        .filter(events::soft_failed.eq(false))
        .filter(events::is_rejected.eq(false))
        .filter(events::stream_ordering.ge(0))
        .select(event_points::live_published_sn)
        .first::<Option<Seqnum>>(&mut connect().await?)
        .await
        .optional()?
        .flatten())
}

pub(crate) async fn has_history(room_id: &RoomId) -> AppResult<bool> {
    Ok(diesel::select(diesel::dsl::exists(
        events::table
            .filter(events::room_id.eq(room_id))
            .filter(events::stream_ordering.lt(0))
            .filter(events::is_outlier.eq(false))
            .filter(events::soft_failed.eq(false))
            .filter(events::is_rejected.eq(false)),
    ))
    .get_result(&mut connect().await?)
    .await?)
}

/// Gaps are publication-window metadata even when their PDU is filtered out.
pub(crate) async fn gaps(
    room: &RoomId,
    lower: Option<Seqnum>,
    upper: Option<Seqnum>,
) -> AppResult<Vec<Seqnum>> {
    #[derive(diesel::QueryableByName)]
    struct Gap {
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        position: Seqnum,
    }
    let gaps = diesel::sql_query("SELECT p.live_published_sn AS position FROM timeline_gaps g JOIN event_points p ON p.event_id=g.event_id AND p.room_id=g.room_id JOIN events e ON e.id=p.event_id AND e.room_id=p.room_id WHERE g.room_id=$1 AND p.live_published_sn IS NOT NULL AND ($2::bigint IS NULL OR p.live_published_sn >= $2) AND ($3::bigint IS NULL OR p.live_published_sn < $3) AND e.stream_ordering>=0 AND NOT e.is_outlier AND NOT e.soft_failed AND NOT e.is_rejected")
        .bind::<diesel::sql_types::Text,_>(room.as_str())
        .bind::<diesel::sql_types::Nullable<diesel::sql_types::BigInt>,_>(lower)
        .bind::<diesel::sql_types::Nullable<diesel::sql_types::BigInt>,_>(upper)
        .load::<Gap>(&mut connect().await?).await?;
    Ok(gaps.into_iter().map(|gap| gap.position).collect())
}
