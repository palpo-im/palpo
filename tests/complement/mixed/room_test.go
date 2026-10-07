package mixed_tests

import (
	"fmt"
	"net/url"
	"testing"

	"github.com/matrix-org/complement"
	"github.com/matrix-org/complement/b"
	"github.com/matrix-org/complement/client"
	"github.com/matrix-org/complement/helpers"
	"github.com/matrix-org/gomatrixserverlib/spec"
	"github.com/tidwall/gjson"
)

// Cover real server-to-server PDUs and EDUs, not just HTTP success on the
// sending client. Each assertion waits for the remote user's incremental sync.
func TestMixedRoomLifecycle(t *testing.T) {
	d := complement.Deploy(t, 2)
	defer d.Destroy(t)
	owner := d.Register(t, "hs2", helpers.RegistrationOpts{})
	remote := d.Register(t, "hs1", helpers.RegistrationOpts{})
	server := d.GetFullyQualifiedHomeserverName(t, "hs2")
	room := owner.MustCreateRoom(t, map[string]interface{}{
		"preset": "private_chat", "room_version": "10",
	})
	var ownerSince, remoteSince string
	t.Run("remote invite and join", func(t *testing.T) {
		owner.MustInviteRoom(t, room, remote.UserID)
		remoteSince = remote.MustSyncUntil(t, client.SyncReq{}, client.SyncInvitedTo(remote.UserID, room))
		remote.MustJoinRoom(t, room, []spec.ServerName{server})
		ownerSince = owner.MustSyncUntil(t, client.SyncReq{}, client.SyncJoinedTo(remote.UserID, room))
		remoteSince = remote.MustSyncUntil(t, client.SyncReq{Since: remoteSince}, client.SyncJoinedTo(remote.UserID, room))
	})
	// Fatal setup errors must not leave subsequent subtests running on a room
	// they never joined. This is a precondition, not a skip.
	if t.Failed() {
		t.Fatal("remote invite/join setup failed")
	}
	var event string
	t.Run("message and event fetch", func(t *testing.T) {
		event = remote.SendEventSynced(t, room, b.Event{Type: "m.room.message", Content: map[string]interface{}{
			"msgtype": "m.text", "body": "Mixed federation 用户 🐯",
		}})
		ownerSince = owner.MustSyncUntil(t, client.SyncReq{Since: ownerSince}, client.SyncTimelineHasEventID(room, event))
		body := owner.MustGetEvent(t, room, event)
		if body.Get("sender").Str != remote.UserID || body.Get("content.body").Str != "Mixed federation 用户 🐯" {
			t.Fatalf("remote message content mismatch: %s", body.Raw)
		}
	})
	if t.Failed() {
		t.Fatal("remote message setup failed")
	}
	t.Run("encrypted payload relay", func(t *testing.T) {
		// Homeservers relay opaque encrypted content; client-side decryption is
		// deliberately outside this test's claim of coverage.
		content := map[string]interface{}{
			"algorithm": "m.megolm.v1.aes-sha2", "ciphertext": "opaque-mixed-ciphertext",
			"sender_key": "mixed-sender-key", "session_id": "mixed-session", "device_id": owner.DeviceID,
		}
		encrypted := owner.SendEventSynced(t, room, b.Event{Type: "m.room.encrypted", Content: content})
		remoteSince = remote.MustSyncUntil(t, client.SyncReq{Since: remoteSince}, client.SyncTimelineHasEventID(room, encrypted))
		body := remote.MustGetEvent(t, room, encrypted)
		for key, want := range content {
			if body.Get("content."+key).Str != want {
				t.Errorf("encrypted field %s was changed: %s", key, body.Raw)
			}
		}
	})
	t.Run("read receipt EDU", func(t *testing.T) {
		owner.MustDo(t, "POST", []string{"_matrix", "client", "v3", "rooms", room, "receipt", "m.read", event},
			client.WithJSONBody(t, map[string]interface{}{})).Body.Close()
		remoteSince = remote.MustSyncUntil(t, client.SyncReq{Since: remoteSince}, client.SyncEphemeralHas(room, func(ev gjson.Result) bool {
			return ev.Get("type").Str == "m.receipt" &&
				ev.Get("content."+client.GjsonEscape(event)+".m\\.read."+client.GjsonEscape(owner.UserID)).Exists()
		}))
	})
	t.Run("remote state update", func(t *testing.T) {
		key := ""
		state := owner.SendEventSynced(t, room, b.Event{Type: "m.room.topic", StateKey: &key,
			Content: map[string]interface{}{"topic": "Remote topic 用户"}})
		remoteSince = remote.MustSyncUntil(t, client.SyncReq{Since: remoteSince}, client.SyncTimelineHasEventID(room, state))
		if got := remote.MustGetStateEventContent(t, room, "m.room.topic", "").Get("topic").Str; got != "Remote topic 用户" {
			t.Fatalf("remote state content = %q", got)
		}
	})
	for _, field := range []string{"displayname", "avatar_url"} {
		t.Run("membership profile update "+field, func(t *testing.T) {
			value := "Updated remote 用户"
			if field == "avatar_url" {
				value = "mxc://" + string(d.GetFullyQualifiedHomeserverName(t, "hs1")) + "/updated-avatar"
			}
			remote.MustDo(t, "PUT", []string{"_matrix", "client", "v3", "profile", remote.UserID, field},
				client.WithJSONBody(t, map[string]interface{}{field: value})).Body.Close()
			ownerSince = owner.MustSyncUntil(t, client.SyncReq{Since: ownerSince}, client.SyncTimelineHas(room, func(ev gjson.Result) bool {
				return ev.Get("type").Str == "m.room.member" && ev.Get("state_key").Str == remote.UserID &&
					ev.Get("content.membership").Str == "join" && ev.Get("content."+field).Str == value
			}))
			body := owner.MustGetStateEventContent(t, room, "m.room.member", remote.UserID)
			if body.Get(field).Str != value {
				t.Fatalf("remote membership profile mismatch: %s", body.Raw)
			}
		})
	}
	t.Run("remote redaction", func(t *testing.T) {
		redaction := remote.MustSendRedaction(t, room, map[string]interface{}{"reason": "mixed regression"}, event)
		ownerSince = owner.MustSyncUntil(t, client.SyncReq{Since: ownerSince}, client.SyncTimelineHasEventID(room, redaction))
		body := owner.MustGetEvent(t, room, event)
		if body.Get("content.body").Exists() || !body.Get("unsigned.redacted_because").Exists() {
			t.Fatalf("remote event was not redacted: %s", body.Raw)
		}
	})
	t.Run("remote leave and rejoin", func(t *testing.T) {
		remote.MustLeaveRoom(t, room)
		ownerSince = owner.MustSyncUntil(t, client.SyncReq{Since: ownerSince}, client.SyncLeftFrom(remote.UserID, room))
		remoteSince = remote.MustSyncUntil(t, client.SyncReq{Since: remoteSince}, client.SyncLeftFrom(remote.UserID, room))
		owner.MustInviteRoom(t, room, remote.UserID)
		remote.MustJoinRoom(t, room, []spec.ServerName{server})
		ownerSince = owner.MustSyncUntil(t, client.SyncReq{Since: ownerSince}, client.SyncJoinedTo(remote.UserID, room))
		remoteSince = remote.MustSyncUntil(t, client.SyncReq{Since: remoteSince}, client.SyncJoinedTo(remote.UserID, room))
	})
	t.Run("remote kick", func(t *testing.T) {
		owner.MustDo(t, "POST", []string{"_matrix", "client", "v3", "rooms", room, "kick"},
			client.WithJSONBody(t, map[string]interface{}{"user_id": remote.UserID, "reason": "mixed regression"})).Body.Close()
		remote.MustSyncUntil(t, client.SyncReq{Since: remoteSince}, client.SyncLeftFrom(remote.UserID, room))
	})
}

func TestMixedRemoteHierarchy(t *testing.T) {
	d := complement.Deploy(t, 2)
	defer d.Destroy(t)
	query := d.Register(t, "hs1", helpers.RegistrationOpts{})
	owner := d.Register(t, "hs2", helpers.RegistrationOpts{})
	server := d.GetFullyQualifiedHomeserverName(t, "hs2")
	space := owner.MustCreateRoom(t, map[string]interface{}{
		"preset": "public_chat", "room_version": "10", "name": "Mixed remote space",
		"creation_content": map[string]interface{}{"type": "m.space"},
	})
	public := owner.MustCreateRoom(t, map[string]interface{}{"preset": "public_chat", "room_version": "10"})
	private := owner.MustCreateRoom(t, map[string]interface{}{"preset": "private_chat", "room_version": "10"})
	for _, child := range []string{public, private} {
		owner.SendEventSynced(t, space, b.Event{Type: "m.space.child", StateKey: &child,
			Content: map[string]interface{}{"via": []string{string(server)}, "suggested": child == public}})
	}
	// Joining the root teaches hs1 a valid via server; hs1 has never joined
	// either child, so traversing them still requires remote hierarchy requests.
	query.MustJoinRoom(t, space, []spec.ServerName{server})
	for _, suggested := range []bool{false, true} {
		t.Run(fmt.Sprintf("suggested_only=%t", suggested), func(t *testing.T) {
			body := jsonResponse(t, query.Do(t, "GET", []string{"_matrix", "client", "v1", "rooms", space, "hierarchy"},
				client.WithQueries(url.Values{"suggested_only": {fmt.Sprint(suggested)}})), 200)
			ids := make(map[string]bool)
			if !body.Get("rooms").IsArray() {
				t.Fatalf("hierarchy has no rooms array: %s", body.Raw)
			}
			for _, room := range body.Get("rooms").Array() {
				ids[room.Get("room_id").Str] = true
			}
			if !ids[space] || !ids[public] || ids[private] {
				t.Fatalf("remote hierarchy missing accessible rooms or leaking private room: %s", body.Raw)
			}
		})
	}
}
