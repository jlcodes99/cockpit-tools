package helps

import (
	"bytes"
	"crypto/sha256"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"

	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"
)

const codexTargetRecoveryBody = `{"model":"gpt-5.4","prompt_cache_key":"targeted-session","input":[{"type":"reasoning","encrypted_content":"good-one","summary":[{"type":"summary_text","text":"keep reasoning"}]},{"type":"compaction","encrypted_content":"good-two","summary":"keep compaction"},{"type":"function_call_output","encrypted_content":"foreign","output":"keep result"}]}`

func TestCodexEncryptedRecoveryPreciseTargetSelection(t *testing.T) {
	for _, tc := range []struct {
		name, param, message string
		removed              int
	}{
		{name: "unmatched named content", message: "The encrypted content missing could not be verified"},
		{name: "unmatched named with generic input", param: "input", message: "The encrypted content missing could not be verified"},
		{name: "empty named content", message: "The encrypted content '' could not be verified"},
		{name: "nonexistent field", param: "input[9].encrypted_content", message: "The encrypted content good-one could not be verified"},
		{name: "nonreasoning item", param: "input[2].encrypted_content", message: "The encrypted content good-one could not be verified"},
		{name: "unrelated field is authoritative", param: "previous_response_id", message: "The encrypted content good-one could not be verified"},
		{name: "different field on reasoning", param: "input[0].id"},
		{name: "named matching content", message: "The encrypted content good-one could not be verified", removed: 1},
		{name: "quoted matching content", message: "The encrypted content 'good-one' could not be verified", removed: 1},
		{name: "case insensitive wording", message: "The Encrypted Content good-one Could Not Be Verified", removed: 1},
		{name: "field location wins", param: "input.1.encrypted_content", message: "The encrypted content good-one could not be verified", removed: 1},
		{name: "generic no location", message: "Encrypted content could not be verified", removed: 2},
		{name: "generic input location", param: "input", message: "Encrypted content could not be verified", removed: 2},
	} {
		t.Run(tc.name, func(t *testing.T) {
			errorBody := []byte(`{"error":{}}`)
			// Preserve the exact error field shape in the fixture.
			errorBody, _ = setRecoveryTargetError(errorBody, tc.param, tc.message)
			body := []byte(codexTargetRecoveryBody)
			next, removed, err := stripCodexRejectedContent(body, rejectedCodexContentDigests(body, errorBody))
			if err != nil || len(removed) != tc.removed {
				t.Fatalf("removed=%d want=%d err=%v next=%s", len(removed), tc.removed, err, next)
			}
			if tc.removed == 0 && !bytes.Equal(body, next) {
				t.Fatalf("unmatched rejection changed history: %s", next)
			}
			if tc.name == "field location wins" && (len(removed) != 1 || removed[0] != sha256.Sum256([]byte("good-two"))) {
				t.Fatal("message overrode explicit field target")
			}
			if !bytes.Contains(next, []byte("keep result")) {
				t.Fatal("nonreasoning history lost")
			}
		})
	}
}

func TestCodexEncryptedRecoveryUnmatchedHTTPResponseIsUnchanged(t *testing.T) {
	for _, tc := range []struct{ name, param, message string }{
		{name: "unmatched named", message: "The encrypted content missing could not be verified"},
		{name: "unrelated param", param: "previous_response_id", message: "Encrypted content could not be verified"},
		{name: "nonreasoning param", param: "input[2].encrypted_content", message: "Encrypted content could not be verified"},
		{name: "out of range", param: "input[9].encrypted_content", message: "Encrypted content could not be verified"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			errorBody, _ := setRecoveryTargetError([]byte(`{"error":{}}`), tc.param, tc.message)
			var requests atomic.Int32
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) {
				requests.Add(1)
				actual, _ := io.ReadAll(req.Body)
				if string(actual) != codexTargetRecoveryBody {
					t.Errorf("original request history changed: %s", actual)
				}
				w.Header().Set("X-Fixture", "preserve")
				w.WriteHeader(http.StatusBadRequest)
				_, _ = w.Write(errorBody)
			}))
			defer server.Close()
			body := []byte(codexTargetRecoveryBody)
			req, _ := http.NewRequestWithContext(t.Context(), "POST", server.URL+"/responses", bytes.NewReader(body))
			req.Header.Set("Authorization", "Bearer fixture")
			auth := &cliproxyauth.Auth{ID: t.Name(), Metadata: map[string]any{"access_token": "fixture"}}
			recovery := NewCodexHTTPEncryptedRecovery(auth, false, req, body)
			callbacks := 0
			resp, next, err := recovery.Do(t.Context(), nil, server.Client(), req, body, func(int, []byte) error { callbacks++; return nil })
			if err != nil {
				t.Fatal(err)
			}
			defer resp.Body.Close()
			actual, err := io.ReadAll(resp.Body)
			if err != nil || !bytes.Equal(actual, errorBody) || resp.StatusCode != 400 || resp.Header.Get("X-Fixture") != "preserve" {
				t.Fatalf("original error response changed: status=%d body=%s err=%v", resp.StatusCode, actual, err)
			}
			if requests.Load() != 1 || callbacks != 0 || !bytes.Equal(next, body) || len(recovery.knownDigests()) != 0 {
				t.Fatalf("unmatched rejection retried/changed lineage: requests=%d callbacks=%d body=%s", requests.Load(), callbacks, next)
			}
			replay, err := req.GetBody()
			if err != nil {
				t.Fatal(err)
			}
			replayBody, _ := io.ReadAll(replay)
			_ = replay.Close()
			if !bytes.Equal(replayBody, body) || req.ContentLength != int64(len(body)) || req.Header.Get("Authorization") != "Bearer fixture" {
				t.Fatal("unmatched error mutated replayable request")
			}
			followup, _ := http.NewRequestWithContext(t.Context(), "POST", server.URL+"/responses", strings.NewReader(string(body)))
			future, err := NewCodexHTTPEncryptedRecovery(auth, false, followup, body).PrepareRequest(followup, body)
			if err != nil || !bytes.Equal(future, body) {
				t.Fatal("unmatched target polluted followup lineage")
			}
		})
	}
}

func TestCodexEncryptedRecoveryGenericInputStillRetriesOnce(t *testing.T) {
	body := []byte(codexTargetRecoveryBody)
	var requests atomic.Int32
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) {
		attempt := requests.Add(1)
		actual, _ := io.ReadAll(req.Body)
		if attempt == 1 {
			w.WriteHeader(400)
			_, _ = io.WriteString(w, `{"error":{"code":"invalid_encrypted_content","param":"input","message":"Encrypted content could not be verified"}}`)
			return
		}
		if gjson.GetBytes(actual, "input.0.encrypted_content").Exists() || !bytes.Contains(actual, []byte("keep reasoning")) || !bytes.Contains(actual, []byte("keep compaction")) || !bytes.Contains(actual, []byte("keep result")) {
			t.Errorf("generic recovery malformed history: %s", actual)
		}
		_, _ = io.WriteString(w, "ok")
	}))
	defer server.Close()
	req, _ := http.NewRequestWithContext(t.Context(), "POST", server.URL+"/responses", bytes.NewReader(body))
	auth := &cliproxyauth.Auth{ID: t.Name(), Metadata: map[string]any{"access_token": "fixture"}}
	recovery := NewCodexHTTPEncryptedRecovery(auth, false, req, body)
	callbacks := 0
	resp, _, err := recovery.Do(t.Context(), nil, server.Client(), req, body, func(int, []byte) error { callbacks++; return nil })
	if err != nil {
		t.Fatal(err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != 200 || requests.Load() != 2 || callbacks != 1 || len(recovery.knownDigests()) != 2 {
		t.Fatalf("generic rejection no longer recovered once: status=%d requests=%d callbacks=%d", resp.StatusCode, requests.Load(), callbacks)
	}
}

func setRecoveryTargetError(body []byte, param, message string) ([]byte, error) {
	body, err := sjson.SetBytes(body, "error.code", "invalid_encrypted_content")
	if err != nil {
		return nil, err
	}
	body, err = sjson.SetBytes(body, "error.param", param)
	if err != nil {
		return nil, err
	}
	return sjson.SetBytes(body, "error.message", message)
}

func TestCodexEncryptedRecoveryNamedTargetsPreserveOriginalUnicode(t *testing.T) {
	for _, tc := range []struct {
		name, prefix, content, messageName string
		matched                            bool
	}{
		{name: "expanded lowercase prefix", prefix: strings.Repeat("Ⱥ", 64), content: "bad", messageName: "bad", matched: true},
		{name: "shortened lowercase prefix", prefix: strings.Repeat("İ", 64), content: "bad", messageName: "bad", matched: true},
		{name: "unicode ciphertext", prefix: "Ⱥİ", content: "密文Ⱥİ字", messageName: "密文Ⱥİ字", matched: true},
		{name: "quoted unicode ciphertext", prefix: strings.Repeat("Ⱥ", 64), content: "密文Ⱥİ字", messageName: "'密文Ⱥİ字'", matched: true},
		{name: "double quoted unicode ciphertext", prefix: strings.Repeat("İ", 64), content: "密文Ⱥİ字", messageName: `"密文Ⱥİ字"`, matched: true},
		{name: "ciphertext case is significant", prefix: "Ⱥİ", content: "Ⱥİ", messageName: "ⱥi"},
		{name: "unmatched unicode ciphertext", prefix: strings.Repeat("Ⱥ", 64), content: "密文", messageName: "其它密文"},
		{name: "empty quoted unicode prefix", prefix: strings.Repeat("Ⱥ", 64), content: "bad", messageName: "''"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			body, err := sjson.SetBytes([]byte(`{"input":[{"type":"reasoning","summary":[{"text":"keep"}]}]}`), "input.0.encrypted_content", tc.content)
			if err != nil {
				t.Fatal(err)
			}
			message := tc.prefix + "The Encrypted Content " + tc.messageName + " Could Not Be Verified"
			errorBody, err := setRecoveryTargetError([]byte(`{"error":{}}`), "", message)
			if err != nil {
				t.Fatal(err)
			}
			next, removed, err := stripCodexRejectedContent(body, rejectedCodexContentDigests(body, errorBody))
			want := 0
			if tc.matched {
				want = 1
			}
			if err != nil || len(removed) != want {
				t.Fatalf("unicode target mismatch: removed=%d want=%d err=%v body=%s", len(removed), want, err, next)
			}
			if !tc.matched && !bytes.Equal(body, next) {
				t.Fatalf("unmatched unicode target purged history: %s", next)
			}
			if tc.matched && gjson.GetBytes(next, "input.0.summary.0.text").String() != "keep" {
				t.Fatal("unicode recovery lost summary")
			}
		})
	}
}
