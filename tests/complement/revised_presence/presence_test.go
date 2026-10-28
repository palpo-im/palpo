package revised_presence_tests

import (
	"net/http"
	"net/url"
	"testing"
	"time"

	"github.com/matrix-org/complement"
	"github.com/matrix-org/complement/client"
	"github.com/matrix-org/complement/helpers"
	"github.com/matrix-org/complement/must"
	"github.com/matrix-org/gomatrixserverlib/spec"
	"github.com/tidwall/gjson"
)

const prefix = "org.continuwuity.presence_v2.msc4532"
const sharing = "org.continuwuity.presence_v2.msc4495.presence.sharing"

func TestMain(m *testing.M) { complement.TestMain(m, "palpo_revised_presence") }

func body(t *testing.T, res *http.Response) gjson.Result {
	t.Helper()
	defer res.Body.Close()
	if res.StatusCode != http.StatusOK {
		t.Fatalf("HTTP %d: %s", res.StatusCode, must.ParseJSON(t, res.Body).Raw)
	}
	return must.ParseJSON(t, res.Body)
}

func accountData(t *testing.T, c *client.CSAPI, event string, content map[string]interface{}) {
	t.Helper()
	c.MustDo(t, "PUT", []string{"_matrix", "client", "v3", "user", c.UserID, "account_data", event},
		client.WithJSONBody(t, content)).Body.Close()
}

func setPresence(t *testing.T, c *client.CSAPI, state string) {
	t.Helper()
	c.MustDo(t, "PUT", []string{"_matrix", "client", "v3", "presence", c.UserID, "status"},
		client.WithJSONBody(t, map[string]interface{}{"presence": state})).Body.Close()
}

func sync(t *testing.T, c *client.CSAPI, since string, revised bool) gjson.Result {
	t.Helper()
	query := url.Values{"timeout": {"1000"}}
	if since != "" {
		query.Set("since", since)
	}
	if revised {
		query.Set(prefix+".revised_presence", "true")
	}
	return body(t, c.MustDo(t, "GET", []string{"_matrix", "client", "v3", "sync"}, client.WithQueries(query)))
}

func awaitPresence(t *testing.T, c *client.CSAPI, since, sender, state, message, icon string, revised bool) string {
	t.Helper()
	deadline := time.Now().Add(60 * time.Second)
	var last string
	for time.Now().Before(deadline) {
		res := sync(t, c, since, revised)
		since = res.Get("next_batch").Str
		for _, event := range res.Get("presence.events").Array() {
			if event.Get("sender").Str != sender {
				continue
			}
			content := event.Get("content")
			last = content.Raw
			status := content.Get(client.GjsonEscape(prefix + ".status"))
			if content.Get("presence").Str == state && content.Get("status_msg").Str == message &&
				status.Get("msg").Str == message && status.Get("example\\.icon").Str == icon {
				if icon == "" && status.Get("example\\.icon").Exists() {
					t.Fatalf("cleared status extension survived federation: %s", content.Raw)
				}
				return since
			}
		}
	}
	t.Fatalf("remote presence did not reach state=%s message=%q icon=%q; last=%s", state, message, icon, last)
	return since
}

// Two actual homeservers exchange signed federation requests. Both directions
// must pass; no mocked EDU injection or success-rate threshold is used.
func TestRevisedPresenceFederation(t *testing.T) {
	d := complement.Deploy(t, 2)
	defer d.Destroy(t)
	for _, direction := range [][2]string{{"hs1", "hs2"}, {"hs2", "hs1"}} {
		t.Run(direction[0]+"_to_"+direction[1], func(t *testing.T) {
			sender := d.Register(t, direction[0], helpers.RegistrationOpts{})
			receiver := d.Register(t, direction[1], helpers.RegistrationOpts{})
			excluded := d.Register(t, direction[1], helpers.RegistrationOpts{})
			for _, c := range []*client.CSAPI{sender, receiver} {
				versions := body(t, c.MustDo(t, "GET", []string{"_matrix", "client", "versions"}))
				if !versions.Get("unstable_features." + client.GjsonEscape(prefix)).Bool() {
					t.Fatal("MSC4532 must be enabled on both deployed homeservers")
				}
			}
			room := sender.MustCreateRoom(t, map[string]interface{}{"preset": "public_chat", "room_version": "10"})
			via := []spec.ServerName{d.GetFullyQualifiedHomeserverName(t, direction[0])}
			receiver.MustJoinRoom(t, room, via)
			excluded.MustJoinRoom(t, room, via)
			sender.MustSyncUntil(t, client.SyncReq{}, client.SyncJoinedTo(receiver.UserID, room), client.SyncJoinedTo(excluded.UserID, room))
			since := sync(t, receiver, "", true).Get("next_batch").Str
			excludedSince := sync(t, excluded, "", true).Get("next_batch").Str
			accountData(t, sender, sharing, map[string]interface{}{"users": map[string]string{receiver.UserID: "allow", excluded.UserID: "deny"}})
			accountData(t, sender, prefix+".presence.persistent", map[string]interface{}{
				"state_override": prefix + ".busy", "status": map[string]interface{}{"msg": "federated status", "example.icon": "moon"},
			})
			setPresence(t, sender, prefix+".active")
			since = awaitPresence(t, receiver, since, sender.UserID, prefix+".busy", "federated status", "moon", true)

			// An old client on the receiving server sees a compatible state and message.
			legacy := body(t, receiver.MustDo(t, "GET", []string{"_matrix", "client", "v3", "presence", sender.UserID, "status"}))
			if legacy.Get("presence").Str != "unavailable" || legacy.Get("status_msg").Str != "federated status" {
				t.Fatalf("legacy projection mismatch: %s", legacy.Raw)
			}
			awaitPresence(t, receiver, "", sender.UserID, "unavailable", "federated status", "moon", false)

			// The receiver's positive result is a federation-delivery barrier. A user
			// sharing the same room but not authorized must see neither sync nor GET.
			res := sync(t, excluded, excludedSince, true)
			for _, event := range res.Get("presence.events").Array() {
				if event.Get("sender").Str == sender.UserID {
					t.Fatalf("presence leaked to an excluded room member: %s", event.Raw)
				}
			}
			denied := excluded.Do(t, "GET", []string{"_matrix", "client", "v3", "presence", sender.UserID, "status"})
			denied.Body.Close()
			if denied.StatusCode != http.StatusForbidden && denied.StatusCode != http.StatusUnauthorized {
				t.Fatalf("excluded presence GET returned HTTP %d", denied.StatusCode)
			}

			accountData(t, sender, prefix+".presence.persistent", map[string]interface{}{})
			since = awaitPresence(t, receiver, since, sender.UserID, prefix+".active", "", "", true)
			setPresence(t, sender, prefix+".idle")
			since = awaitPresence(t, receiver, since, sender.UserID, prefix+".idle", "", "", true)
			setPresence(t, sender, "offline")
			awaitPresence(t, receiver, since, sender.UserID, "offline", "", "", true)
		})
	}
}
