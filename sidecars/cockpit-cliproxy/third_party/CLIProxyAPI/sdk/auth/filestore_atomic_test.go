package auth

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"runtime"
	"testing"

	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
)

func TestFileTokenStoreMetadataSaveAtomicallyReplacesExisting(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "account.json")
	if err := os.WriteFile(path, []byte(`{"type":"codex","state":"old"}`), 0600); err != nil {
		t.Fatal(err)
	}
	before, err := os.Stat(path)
	if err != nil {
		t.Fatal(err)
	}
	store := NewFileTokenStore()
	store.SetBaseDir(dir)
	auth := &cliproxyauth.Auth{ID: "account.json", Provider: "codex", FileName: "account.json", Metadata: map[string]any{"type": "codex", "state": "new"}}
	if _, err := store.Save(context.Background(), auth); err != nil {
		t.Fatal(err)
	}
	after, err := os.Stat(path)
	if err != nil {
		t.Fatal(err)
	}
	if runtime.GOOS != "windows" && os.SameFile(before, after) {
		t.Fatal("credential metadata was overwritten in place")
	}
	raw, err := os.ReadFile(path)
	if err != nil || !json.Valid(raw) {
		t.Fatalf("replacement is not complete JSON: %v", err)
	}
	var payload map[string]any
	if err := json.Unmarshal(raw, &payload); err != nil || payload["state"] != "new" {
		t.Fatalf("replacement payload invalid: %v", err)
	}
}
