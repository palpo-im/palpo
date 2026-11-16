package csapi_tests

import (
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"
)

// publicRoomsContain searches a complete directory traversal. A missing room
// returns false so the caller can retry publication from the first page.
func publicRoomsContain(first *http.Response, roomID string, next func(string) (*http.Response, error)) (bool, error) {
	seen := make(map[string]bool)
	res := first
	for page := 0; page < 100; page++ {
		var body struct {
			Chunk *[]struct {
				RoomID string `json:"room_id"`
			} `json:"chunk"`
			NextBatch string `json:"next_batch"`
		}
		err := func() error {
			defer res.Body.Close()
			if res.StatusCode != http.StatusOK {
				return fmt.Errorf("publicRooms page returned HTTP %d", res.StatusCode)
			}
			decoder := json.NewDecoder(io.LimitReader(res.Body, 4<<20))
			if err := decoder.Decode(&body); err != nil {
				return fmt.Errorf("invalid publicRooms page: %w", err)
			}
			var trailing interface{}
			if err := decoder.Decode(&trailing); err != io.EOF {
				return fmt.Errorf("publicRooms page has trailing data")
			}
			if body.Chunk == nil {
				return fmt.Errorf("publicRooms page has no chunk array")
			}
			return nil
		}()
		if err != nil {
			return false, err
		}
		for _, room := range *body.Chunk {
			if room.RoomID == roomID {
				return true, nil
			}
		}
		if body.NextBatch == "" {
			return false, nil
		}
		if seen[body.NextBatch] {
			return false, fmt.Errorf("publicRooms pagination repeated token %q", body.NextBatch)
		}
		seen[body.NextBatch] = true
		if page == 99 {
			return false, fmt.Errorf("publicRooms pagination exceeded 100 pages")
		}
		res, err = next(body.NextBatch)
		if err != nil {
			return false, err
		}
	}
	panic("unreachable")
}

// No Docker is needed: run this file directly before the Complement image build.
func TestPublicRoomsPagination(t *testing.T) {
	t.Run("finds room after default first page", func(t *testing.T) {
		const token = "opaque/+="
		requests := 0
		server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			requests++
			if requests == 1 {
				rooms := make([]map[string]string, 10)
				for i := range rooms {
					rooms[i] = map[string]string{"room_id": fmt.Sprintf("!%d:test", i)}
				}
				json.NewEncoder(w).Encode(map[string]interface{}{"chunk": rooms, "next_batch": token})
				return
			}
			if r.URL.Query().Get("since") != token {
				t.Errorf("pagination token was not preserved: %s", r.URL.RawQuery)
			}
			io.WriteString(w, `{"chunk":[{"room_id":"!wanted:test"}]}`)
		}))
		defer server.Close()
		first, err := http.Get(server.URL)
		if err != nil {
			t.Fatal(err)
		}
		found, err := publicRoomsContain(first, "!wanted:test", func(since string) (*http.Response, error) {
			return http.Get(server.URL + "?" + url.Values{"since": {since}}.Encode())
		})
		if err != nil || !found || requests != 2 {
			t.Fatalf("found=%v, requests=%d, err=%v", found, requests, err)
		}
	})

	response := func(body string) *http.Response {
		return &http.Response{StatusCode: http.StatusOK, Body: io.NopCloser(strings.NewReader(body))}
	}
	t.Run("first page", func(t *testing.T) {
		found, err := publicRoomsContain(response(`{"chunk":[{"room_id":"!wanted:test"}],"next_batch":"unused"}`), "!wanted:test", func(string) (*http.Response, error) {
			t.Fatal("must stop once found")
			return nil, nil
		})
		if err != nil || !found {
			t.Fatalf("found=%v, err=%v", found, err)
		}
	})
	t.Run("missing room remains a failure", func(t *testing.T) {
		for attempt := 0; attempt < 2; attempt++ {
			found, err := publicRoomsContain(response(`{"chunk":[],"next_batch":"last"}`), "!missing:test", func(string) (*http.Response, error) {
				return response(`{"chunk":[]}`), nil
			})
			if found || err != nil {
				t.Fatalf("found=%v, err=%v", found, err)
			}
		}
	})
	for _, body := range []string{`{}`, `{"chunk":null}`, `{"chunk":{}}`, `{"chunk":[]`, `{"chunk":[]} {}`} {
		t.Run("invalid "+body, func(t *testing.T) {
			if _, err := publicRoomsContain(response(body), "!wanted:test", nil); err == nil {
				t.Fatal("accepted invalid response")
			}
		})
	}
	t.Run("HTTP error", func(t *testing.T) {
		res := response(`{"chunk":[]}`)
		res.StatusCode = http.StatusInternalServerError
		if _, err := publicRoomsContain(res, "!wanted:test", nil); err == nil {
			t.Fatal("accepted failed response")
		}
	})
	t.Run("transport error", func(t *testing.T) {
		if _, err := publicRoomsContain(response(`{"chunk":[],"next_batch":"next"}`), "!wanted:test", func(string) (*http.Response, error) {
			return nil, fmt.Errorf("network failure")
		}); err == nil {
			t.Fatal("accepted failed pagination")
		}
	})
	t.Run("cycle", func(t *testing.T) {
		if _, err := publicRoomsContain(response(`{"chunk":[],"next_batch":"same"}`), "!wanted:test", func(string) (*http.Response, error) {
			return response(`{"chunk":[],"next_batch":"same"}`), nil
		}); err == nil {
			t.Fatal("accepted cyclic pagination")
		}
	})
	t.Run("page bound", func(t *testing.T) {
		calls := 0
		_, err := publicRoomsContain(response(`{"chunk":[],"next_batch":"start"}`), "!wanted:test", func(string) (*http.Response, error) {
			calls++
			return response(fmt.Sprintf(`{"chunk":[],"next_batch":"%d"}`, calls)), nil
		})
		if err == nil || calls != 99 {
			t.Fatalf("calls=%d, err=%v", calls, err)
		}
	})
}
