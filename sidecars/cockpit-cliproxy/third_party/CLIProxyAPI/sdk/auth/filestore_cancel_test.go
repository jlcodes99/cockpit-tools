package auth

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"testing"
	"time"

	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
)

func TestFileTokenStore_SaveCancelledWhileWaitingDoesNotWrite(t *testing.T) {
	store := NewFileTokenStore()
	dir := t.TempDir()
	store.SetBaseDir(dir)
	path := filepath.Join(dir, "auth.json")
	old := `{"access_token":"old"}`
	if err := os.WriteFile(path, []byte(old), 0o600); err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	store.mu.Lock()
	done := make(chan error, 1)
	go func() {
		_, err := store.Save(ctx, &cliproxyauth.Auth{
			ID: "auth.json", FileName: "auth.json", Metadata: map[string]any{"access_token": "new"},
		})
		done <- err
	}()
	cancel()
	store.mu.Unlock()
	select {
	case err := <-done:
		if !errors.Is(err, context.Canceled) {
			t.Fatalf("Save() = %v, want context.Canceled", err)
		}
	case <-time.After(time.Second):
		t.Fatal("cancelled save did not return")
	}
	if raw, err := os.ReadFile(path); err != nil || string(raw) != old {
		t.Fatalf("cancelled save changed credentials: %s, %v", raw, err)
	}
}
