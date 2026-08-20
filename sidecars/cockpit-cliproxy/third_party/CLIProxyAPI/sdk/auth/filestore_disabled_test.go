package auth

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"

	emptyauth "github.com/router-for-me/CLIProxyAPI/v7/internal/auth/empty"
	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
)

type testTokenStorage struct {
	meta map[string]any
}

type failingTokenStorage struct{}

func (*failingTokenStorage) SaveTokenToFile(authFilePath string) error {
	if err := os.WriteFile(authFilePath, []byte(`{"partial":`), 0o600); err != nil {
		return err
	}
	return errors.New("injected token write failure")
}

func (s *testTokenStorage) SetMetadata(meta map[string]any) { s.meta = meta }

func (s *testTokenStorage) SaveTokenToFile(authFilePath string) error {
	raw, err := json.Marshal(s.meta)
	if err != nil {
		return err
	}
	return os.WriteFile(authFilePath, raw, 0o600)
}

func TestFileTokenStore_Save_DisabledPersistsFlagForTokenStorage(t *testing.T) {
	ctx := context.Background()
	baseDir := t.TempDir()
	path := filepath.Join(baseDir, "disabled.json")

	if err := os.WriteFile(path, []byte(`{"type":"test","disabled":true}`), 0o600); err != nil {
		t.Fatalf("seed auth file: %v", err)
	}

	store := NewFileTokenStore()
	store.SetBaseDir(baseDir)
	storage := &testTokenStorage{}

	auth := &cliproxyauth.Auth{
		ID:       "disabled.json",
		Provider: "test",
		FileName: "disabled.json",
		Disabled: true,
		Storage:  storage,
		Metadata: map[string]any{"type": "test"},
	}

	if _, err := store.Save(ctx, auth); err != nil {
		t.Fatalf("Save() error: %v", err)
	}

	raw, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read auth file: %v", err)
	}
	var meta map[string]any
	if err := json.Unmarshal(raw, &meta); err != nil {
		t.Fatalf("unmarshal auth file: %v", err)
	}
	if disabled, _ := meta["disabled"].(bool); !disabled {
		t.Fatalf("disabled=%v, want true (raw=%s)", meta["disabled"], string(raw))
	}
}

func TestFileTokenStore_SaveFailurePreservesExistingAuthFile(t *testing.T) {
	ctx := context.Background()
	baseDir := t.TempDir()
	path := filepath.Join(baseDir, "existing.json")
	original := []byte(`{"type":"codex","access_token":"still-valid"}`)
	if err := os.WriteFile(path, original, 0o600); err != nil {
		t.Fatalf("seed auth file: %v", err)
	}

	store := NewFileTokenStore()
	store.SetBaseDir(baseDir)
	_, err := store.Save(ctx, &cliproxyauth.Auth{
		ID:       "existing.json",
		Provider: "codex",
		FileName: "existing.json",
		Storage:  &failingTokenStorage{},
		Metadata: map[string]any{"type": "codex"},
	})
	if err == nil || !strings.Contains(err.Error(), "injected token write failure") {
		t.Fatalf("Save() error = %v, want injected failure", err)
	}
	got, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read preserved auth file: %v", err)
	}
	if !bytes.Equal(got, original) {
		t.Fatalf("failed save changed live auth file: got=%s want=%s", got, original)
	}
}

func TestFileTokenStore_EmptyStoragePreservesNoOpBehavior(t *testing.T) {
	for _, seedExisting := range []bool{true, false} {
		name := "absent target"
		if seedExisting {
			name = "existing target"
		}
		t.Run(name, func(t *testing.T) {
			baseDir := t.TempDir()
			path := filepath.Join(baseDir, "empty.json")
			original := []byte(`{"type":"codex","access_token":"keep-me"}`)
			if seedExisting {
				if err := os.WriteFile(path, original, 0o600); err != nil {
					t.Fatalf("seed auth file: %v", err)
				}
			}

			store := NewFileTokenStore()
			store.SetBaseDir(baseDir)
			storage := &emptyauth.EmptyStorage{}
			auth := &cliproxyauth.Auth{
				ID:       "empty.json",
				Provider: "empty",
				FileName: "empty.json",
				Storage:  storage,
				Metadata: map[string]any{"type": "empty"},
			}
			savedPath, err := store.Save(context.Background(), auth)
			if err != nil {
				t.Fatalf("Save() error: %v", err)
			}
			if savedPath != path {
				t.Fatalf("Save() path = %q, want %q", savedPath, path)
			}
			if storage.Type != "empty" {
				t.Fatalf("EmptyStorage type = %q, want empty", storage.Type)
			}
			if auth.Attributes["path"] != path {
				t.Fatalf("auth path attribute = %q, want %q", auth.Attributes["path"], path)
			}

			got, err := os.ReadFile(path)
			if seedExisting {
				if err != nil {
					t.Fatalf("read preserved auth file: %v", err)
				}
				if !bytes.Equal(got, original) {
					t.Fatalf("empty storage changed live auth file: got=%s want=%s", got, original)
				}
			} else if !os.IsNotExist(err) {
				t.Fatalf("empty storage created absent target or returned unexpected error: data=%q err=%v", got, err)
			}
		})
	}
}
