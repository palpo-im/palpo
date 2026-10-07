package mixed_tests

import (
	"fmt"
	"net/http"
	"net/url"
	"strings"
	"testing"
	"time"

	"github.com/matrix-org/complement"
	"github.com/matrix-org/complement/client"
	"github.com/matrix-org/complement/helpers"
	"github.com/matrix-org/complement/must"
	"github.com/matrix-org/gomatrixserverlib/spec"
	"github.com/tidwall/gjson"
)

var directoryPath = []string{"_matrix", "client", "v3", "publicRooms"}

func jsonResponse(t *testing.T, res *http.Response, status int) gjson.Result {
	t.Helper()
	defer res.Body.Close()
	body := must.ParseJSON(t, res.Body)
	if res.StatusCode != status {
		t.Fatalf("HTTP %d, want %d: %s", res.StatusCode, status, body.Raw)
	}
	return body
}

func roomIDs(t *testing.T, body gjson.Result) map[string]bool {
	t.Helper()
	chunk := body.Get("chunk")
	if !chunk.IsArray() {
		t.Fatalf("directory response has no chunk array: %s", body.Raw)
	}
	ids := make(map[string]bool)
	for _, room := range chunk.Array() {
		id := room.Get("room_id").Str
		if id == "" || ids[id] {
			t.Fatalf("empty or duplicate room ID in directory: %s", body.Raw)
		}
		ids[id] = true
	}
	return ids
}

// hs1 queries hs2 before sharing a room. The runner reverses the images so
// both implementations must make real outbound discovery requests.
func TestMixedPublicRooms(t *testing.T) {
	d := complement.Deploy(t, 2)
	t.Cleanup(func() { d.Destroy(t) })
	query := d.Register(t, "hs1", helpers.RegistrationOpts{})
	owner := d.Register(t, "hs2", helpers.RegistrationOpts{})
	server := string(d.GetFullyQualifiedHomeserverName(t, "hs2"))
	marker := fmt.Sprintf("mixed-directory-%s", owner.UserID)
	public := make(map[string]bool)
	for i := 0; i < 3; i++ {
		id := owner.MustCreateRoom(t, map[string]interface{}{
			"preset": "public_chat", "visibility": "public",
			"room_version": "10", "name": fmt.Sprintf("%s-%d", marker, i),
			"topic": marker + "-topic",
		})
		public[id] = true
		// Dirty deployments are reused by other tests; unpublish our fixtures.
		t.Cleanup(func() {
			owner.MustDo(t, "PUT", []string{"_matrix", "client", "v3", "directory", "list", "room", id},
				client.WithJSONBody(t, map[string]interface{}{"visibility": "private"})).Body.Close()
		})
	}
	private := owner.MustCreateRoom(t, map[string]interface{}{
		"preset": "private_chat", "visibility": "private", "room_version": "10", "name": marker + "-private",
	})

	request := func(t *testing.T, method, term, since string, limit int) gjson.Result {
		t.Helper()
		if method == "GET" {
			values := url.Values{"server": {server}, "limit": {fmt.Sprint(limit)}}
			if since != "" {
				values.Set("since", since)
			}
			return jsonResponse(t, query.Do(t, method, directoryPath, client.WithQueries(values)), 200)
		}
		body := map[string]interface{}{"limit": limit,
			"filter": map[string]interface{}{"generic_search_term": term}}
		if since != "" {
			body["since"] = since
		}
		return jsonResponse(t, query.Do(t, method, directoryPath,
			client.WithQueries(url.Values{"server": {server}}), client.WithJSONBody(t, body)), 200)
	}

	for _, method := range []string{"GET", "POST"} {
		t.Run(method+" remote listing and privacy", func(t *testing.T) {
			body := request(t, method, marker, "", 1000)
			ids := roomIDs(t, body)
			for id := range public {
				if !ids[id] {
					t.Errorf("remote public room %s missing", id)
				}
			}
			for _, entry := range body.Get("chunk").Array() {
				if !public[entry.Get("room_id").Str] {
					continue
				}
				if !strings.HasPrefix(entry.Get("name").Str, marker) || entry.Get("topic").Str != marker+"-topic" || entry.Get("num_joined_members").Int() != 1 {
					t.Errorf("remote directory metadata mismatch: %s", entry.Raw)
				}
				for _, key := range []string{"world_readable", "guest_can_join"} {
					value := entry.Get(key)
					if value.Type != gjson.True && value.Type != gjson.False {
						t.Errorf("missing boolean %s: %s", key, entry.Raw)
					}
				}
			}
			if ids[private] {
				t.Error("private room leaked into remote directory")
			}
		})
		t.Run(method+" invalid remote pagination", func(t *testing.T) {
			var res *http.Response
			if method == "GET" {
				res = query.Do(t, method, directoryPath, client.WithQueries(url.Values{"server": {server}, "since": {"not-a-token"}}))
			} else {
				res = query.Do(t, method, directoryPath, client.WithQueries(url.Values{"server": {server}}),
					client.WithJSONBody(t, map[string]interface{}{"since": "not-a-token"}))
			}
			// Peers differ in how they map malformed remote tokens (400, 500
			// or 502). Require a Matrix error without a successful chunk;
			// success paths above separately verify federation authentication.
			status := res.StatusCode
			if status != 400 && status != 500 && status != 502 {
				defer res.Body.Close()
				t.Fatalf("invalid remote token returned unexpected HTTP %d", status)
			}
			body := jsonResponse(t, res, status)
			message := body.Get("error").Str
			if body.Get("errcode").Str == "" || body.Get("chunk").Exists() || strings.Contains(message, "missing field") || strings.Contains(message, "error decoding response body") {
				t.Fatalf("remote error was decoded as a successful directory: %s", body.Raw)
			}
		})
		t.Run(method+" pagination", func(t *testing.T) {
			seen, tokens := make(map[string]bool), make(map[string]bool)
			since := ""
			// GET has no filter: allow unrelated rooms in a dirty deployment.
			for page := 0; page < 100; page++ {
				body := request(t, method, marker, since, 1)
				ids := roomIDs(t, body)
				if len(ids) > 1 {
					t.Fatal("remote limit was not honored")
				}
				for id := range ids {
					if seen[id] || id == private {
						t.Fatalf("duplicate or private room on page %d: %s", page, id)
					}
					seen[id] = true
				}
				since = body.Get("next_batch").Str
				if since == "" {
					for id := range public {
						if !seen[id] {
							t.Errorf("pagination missed remote room %s", id)
						}
					}
					return
				}
				if tokens[since] {
					t.Fatal("remote pagination token repeated")
				}
				tokens[since] = true
			}
			t.Fatal("remote pagination did not terminate")
		})
	}
	for _, term := range []string{marker, marker + "-topic", marker + "-absent"} {
		t.Run("POST filter "+term, func(t *testing.T) {
			ids := roomIDs(t, request(t, "POST", term, "", 1000))
			want := len(public)
			if term == marker+"-absent" {
				want = 0
			}
			if len(ids) != want {
				t.Fatalf("filter %q returned %d rooms, want %d", term, len(ids), want)
			}
			for id := range ids {
				if !public[id] {
					t.Errorf("filter returned unrelated room %s", id)
				}
			}
		})
	}
	t.Run("remote visibility change", func(t *testing.T) {
		var id string
		for id = range public {
			break
		}
		owner.MustDo(t, "PUT", []string{"_matrix", "client", "v3", "directory", "list", "room", id},
			client.WithJSONBody(t, map[string]interface{}{"visibility": "private"})).Body.Close()
		if roomIDs(t, request(t, "POST", marker, "", 1000))[id] {
			t.Fatal("unpublished remote room is still listed")
		}
	})
}

func TestMixedRemoteProfile(t *testing.T) {
	d := complement.Deploy(t, 2)
	defer d.Destroy(t)
	query := d.Register(t, "hs1", helpers.RegistrationOpts{})
	owner := d.Register(t, "hs2", helpers.RegistrationOpts{})
	name := "Remote 用户 🐯"
	avatar := "mxc://" + string(d.GetFullyQualifiedHomeserverName(t, "hs2")) + "/profile-avatar"
	owner.MustSetDisplayName(t, name)
	owner.MustDo(t, "PUT", []string{"_matrix", "client", "v3", "profile", owner.UserID, "avatar_url"},
		client.WithJSONBody(t, map[string]interface{}{"avatar_url": avatar})).Body.Close()
	for _, field := range []string{"", "displayname", "avatar_url"} {
		t.Run("query before room join "+field, func(t *testing.T) {
			path := []string{"_matrix", "client", "v3", "profile", owner.UserID}
			if field != "" {
				path = append(path, field)
			}
			body := jsonResponse(t, query.Do(t, "GET", path), 200)
			if field != "avatar_url" && body.Get("displayname").Str != name {
				t.Fatalf("remote displayname mismatch: %s", body.Raw)
			}
			if field != "displayname" && body.Get("avatar_url").Str != avatar {
				t.Fatalf("remote avatar mismatch: %s", body.Raw)
			}
		})
	}
}

func TestMixedRemoteAlias(t *testing.T) {
	d := complement.Deploy(t, 2)
	defer d.Destroy(t)
	query := d.Register(t, "hs1", helpers.RegistrationOpts{})
	owner := d.Register(t, "hs2", helpers.RegistrationOpts{})
	server := d.GetFullyQualifiedHomeserverName(t, "hs2")
	alias := fmt.Sprintf("#mixed-用户-%s:%s", owner.DeviceID, server)
	room := owner.MustCreateRoom(t, map[string]interface{}{"preset": "public_chat", "room_version": "10"})
	path := []string{"_matrix", "client", "v3", "directory", "room", alias}
	owner.MustDo(t, "PUT", path, client.WithJSONBody(t, map[string]interface{}{"room_id": room})).Body.Close()
	t.Run("resolve and join by remote alias", func(t *testing.T) {
		body := jsonResponse(t, query.Do(t, "GET", path), 200)
		if body.Get("room_id").Str != room || !body.Get("servers").IsArray() {
			t.Fatalf("invalid remote alias resolution: %s", body.Raw)
		}
		if got := query.MustJoinRoom(t, alias, []spec.ServerName{server}); got != room {
			t.Fatalf("joined %s, want %s", got, room)
		}
	})
	t.Run("unknown remote alias", func(t *testing.T) {
		missing := fmt.Sprintf("#mixed-missing-%s:%s", owner.DeviceID, server)
		body := jsonResponse(t, query.Do(t, "GET", []string{"_matrix", "client", "v3", "directory", "room", missing}), 404)
		if body.Get("errcode").Str != "M_NOT_FOUND" {
			t.Fatalf("remote alias error was masked: %s", body.Raw)
		}
	})
	t.Run("deleted remote alias", func(t *testing.T) {
		owner.MustDo(t, "DELETE", path).Body.Close()
		body := jsonResponse(t, query.Do(t, "GET", path), 404)
		if body.Get("errcode").Str != "M_NOT_FOUND" {
			t.Fatalf("deleted remote alias error was masked: %s", body.Raw)
		}
	})
}

// Verify the fixtures actually require federation authentication. Otherwise an
// unsigned sender could pass the directory regression against permissive peers.
func TestMixedFederationAuthentication(t *testing.T) {
	d := complement.Deploy(t, 2)
	defer d.Destroy(t)
	httpClient := &http.Client{Transport: d.RoundTripper(), Timeout: 10 * time.Second}
	server := d.GetFullyQualifiedHomeserverName(t, "hs2")
	t.Run("unsigned POST publicRooms is rejected", func(t *testing.T) {
		req, err := http.NewRequest("POST", "https://"+string(server)+"/_matrix/federation/v1/publicRooms", strings.NewReader("{}"))
		if err != nil {
			t.Fatal(err)
		}
		req.Header.Set("Content-Type", "application/json")
		res, err := httpClient.Do(req)
		if err != nil {
			t.Fatal(err)
		}
		defer res.Body.Close()
		body := must.ParseJSON(t, res.Body)
		if res.StatusCode != 401 && res.StatusCode != 403 {
			t.Fatalf("peer accepted unsigned directory request: HTTP %d: %s", res.StatusCode, body.Raw)
		}
		if body.Get("errcode").Str == "" || body.Get("chunk").Exists() {
			t.Fatalf("expected Matrix authentication error: %s", body.Raw)
		}
	})
}
