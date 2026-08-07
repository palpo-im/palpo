use palpo_core::events::relation::{InReplyTo, Replacement, Thread};
use palpo_core::events::room::message::{
    Relation, RoomMessageEventContent, RoomMessageEventContentWithoutRelation,
};
use palpo_core::owned_event_id;

#[test]
fn thread_accessor_returns_only_thread_relations() {
    let mut content = RoomMessageEventContent::text_plain("Thread reply");
    content.relates_to = Some(Relation::Thread(Thread::plain(
        owned_event_id!("$root:example.org"),
        owned_event_id!("$latest:example.org"),
    )));

    let thread = content.thread().expect("thread relation");
    assert_eq!(thread.event_id, "$root:example.org");
    assert_eq!(
        thread
            .in_reply_to
            .as_ref()
            .map(|reply| reply.event_id.as_str()),
        Some("$latest:example.org")
    );
    assert!(thread.is_falling_back);

    content.relates_to = Some(Relation::Reply {
        in_reply_to: InReplyTo::new(owned_event_id!("$reply:example.org")),
    });
    assert!(content.thread().is_none());

    content.relates_to = Some(Relation::Replacement(Replacement::new(
        owned_event_id!("$original:example.org"),
        RoomMessageEventContentWithoutRelation::from(RoomMessageEventContent::text_plain("Edit")),
    )));
    assert!(content.thread().is_none());

    content.relates_to = None;
    assert!(content.thread().is_none());
}
