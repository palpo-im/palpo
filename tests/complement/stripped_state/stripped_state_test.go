package stripped_state_tests

import (
	"net/http"
	"net/url"
	"strings"
	"testing"

	"github.com/matrix-org/complement"
	"github.com/matrix-org/complement/client"
	"github.com/matrix-org/complement/helpers"
	"github.com/matrix-org/complement/must"
	"github.com/tidwall/gjson"
)

func TestMain(m *testing.M) { complement.TestMain(m, "palpo_stripped_state") }

func parsed(t *testing.T, res *http.Response) gjson.Result {
	t.Helper()
	defer res.Body.Close()
	if res.StatusCode != http.StatusOK {
		t.Fatalf("HTTP %d: %s", res.StatusCode, must.ParseJSON(t, res.Body).Raw)
	}
	return must.ParseJSON(t, res.Body)
}

func TestStrippedStateFederation(t *testing.T) {
	deployment := complement.Deploy(t, 2)
	defer deployment.Destroy(t)
	reader := deployment.Register(t, "hs1", helpers.RegistrationOpts{})
	owner := deployment.Register(t, "hs2", helpers.RegistrationOpts{})
	server := strings.SplitN(owner.UserID, ":", 2)[1]
	room := owner.MustCreateRoom(t, map[string]interface{}{
		"preset": "public_chat", "name": "Remote stripped preview", "room_alias_name": "preview",
	})
	for _, child := range []string{"!one:example.org", "!two:example.org"} {
		owner.MustDo(t, "PUT", []string{"_matrix", "client", "v3", "rooms", room, "state", "m.space.child", child},
			client.WithJSONBody(t, map[string]interface{}{"via": []string{server}})).Body.Close()
	}
	selection := map[string]interface{}{"state": []string{"m.room.name", "m.space.child"}, "num_joined_members": true}
	query := func(id string, vias []string) gjson.Result {
		return parsed(t, reader.MustDo(t, "POST", []string{"_matrix", "client", "unstable", "org.continuwuity.msc4560.stripped_state", id},
			client.WithQueries(url.Values{"via": vias}), client.WithJSONBody(t, selection)))
	}
	check := func(body gjson.Result) {
		t.Helper()
		if body.Get("via").Str != server || body.Get("num_joined_members").Int() != 1 || len(body.Get("state").Array()) != 3 {
			t.Fatalf("unexpected remote preview: %s", body.Raw)
		}
		keys := make(map[string]bool)
		for _, event := range body.Get("state").Array() {
			if len(event.Map()) != 4 || event.Get("event_id").Exists() || event.Get("signatures").Exists() {
				t.Fatalf("full PDU leaked: %s", event.Raw)
			}
			keys[event.Get("state_key").Str] = true
		}
		if !keys["!one:example.org"] || !keys["!two:example.org"] {
			t.Fatalf("missing space state keys: %s", body.Raw)
		}
	}
	check(query(room, []string{server}))
	check(query("#preview:"+server, nil))
	// More than four earlier failures must not prevent the last supplied via
	// from succeeding. The reader is not resident in this room.
	check(query(room, []string{"one.invalid", "two.invalid", "three.invalid", "four.invalid", "five.invalid", server}))
	private := owner.MustCreateRoom(t, map[string]interface{}{"preset": "private_chat", "name": "private"})
	res := reader.Do(t, "POST", []string{"_matrix", "client", "unstable", "org.continuwuity.msc4560.stripped_state", private},
		client.WithQueries(url.Values{"via": {server}}), client.WithJSONBody(t, selection))
	res.Body.Close()
	if res.StatusCode != http.StatusForbidden && res.StatusCode != http.StatusNotFound {
		t.Fatalf("private preview returned HTTP %d", res.StatusCode)
	}
}
