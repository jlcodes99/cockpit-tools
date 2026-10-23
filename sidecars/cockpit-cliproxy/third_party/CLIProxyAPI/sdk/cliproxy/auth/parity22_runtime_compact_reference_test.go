package auth

import (
	"context"
	"encoding/json"
	"net/http"
	"os"
	"reflect"
	"testing"
	"time"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/registry"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
)

// Exercise the default, non-Home conductor, including its final cooldown state.
func TestParity22RuntimeCompactReference(t *testing.T) {
	actual := make([]map[string]any, 0)
	for _, status := range []int{500, 502, 503, 401, 403, 429} {
		e := &compactTestExecutor{compactErr: compactTestStatusError{code: status, msg: http.StatusText(status)}}
		m := NewManager(nil, nil, nil)
		if m.HomeEnabled() {
			t.Fatal("fixture must use the deployed default non-Home path")
		}
		m.RegisterExecutor(e)
		for _, id := range []string{"parity22-a", "parity22-b"} {
			if _, err := m.Register(context.Background(), &Auth{ID: id, Provider: e.Identifier(), Status: StatusActive}); err != nil {
				t.Fatal(err)
			}
			registry.GetGlobalRegistry().RegisterClient(id, e.Identifier(), []*registry.ModelInfo{{ID: "parity22-model"}})
		}
		_, err := m.Execute(context.Background(), []string{e.Identifier()}, cliproxyexecutor.Request{Model: "parity22-model", Payload: []byte(`{"model":"parity22-model","input":[]}`)}, cliproxyexecutor.Options{Alt: "responses/compact"})
		if err == nil {
			t.Fatal("expected fixture failure")
		}
		cooled := false
		for _, id := range []string{"parity22-a", "parity22-b"} {
			a, _ := m.GetByID(id)
			if a.NextRetryAfter.After(time.Now()) {
				cooled = true
			}
			for _, state := range a.ModelStates {
				if state != nil && state.NextRetryAfter.After(time.Now()) {
					cooled = true
				}
			}
			registry.GetGlobalRegistry().UnregisterClient(id)
		}
		actual = append(actual, map[string]any{"status": status, "calls": e.calls, "cooled": cooled})
	}
	data, err := json.MarshalIndent(actual, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	path := os.Getenv("PARITY22_RUNTIME_COMPACT_REFERENCE_OUT")
	if path != "" {
		if err := os.WriteFile(path, append(data, '\n'), 0600); err != nil {
			t.Fatal(err)
		}
		return
	}
	want, err := os.ReadFile("../../../../../../../src-tauri/src/modules/codex_native_api/fixtures/parity22_runtime_compact_go.json")
	if err != nil {
		t.Fatal(err)
	}
	var gotJSON, wantJSON any
	_ = json.Unmarshal(data, &gotJSON)
	if err := json.Unmarshal(want, &wantJSON); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(gotJSON, wantJSON) {
		t.Fatalf("runtime compact reference changed: %s", data)
	}
}
