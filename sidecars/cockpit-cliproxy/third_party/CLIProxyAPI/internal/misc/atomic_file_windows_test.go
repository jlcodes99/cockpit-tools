package misc

import (
	"os"
	"path/filepath"
	"testing"
	"time"
)

func TestWriteFileAtomic_WindowsRetriesOpenReader(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "auth.json")
	if err := os.WriteFile(path, []byte(`{"v":1}`), 0o600); err != nil {
		t.Fatal(err)
	}
	reader, err := os.Open(path)
	if err != nil {
		t.Fatal(err)
	}
	closed := make(chan error, 1)
	go func() {
		time.Sleep(50 * time.Millisecond)
		closed <- reader.Close()
	}()
	errWrite := WriteFileAtomic(path, []byte(`{"v":2}`), 0o600)
	if errClose := <-closed; errClose != nil {
		t.Fatal(errClose)
	}
	if errWrite != nil {
		t.Fatalf("replace while a reader briefly holds the file: %v", errWrite)
	}
	if raw, errRead := os.ReadFile(path); errRead != nil || string(raw) != `{"v":2}` {
		t.Fatalf("replacement = %s, %v", raw, errRead)
	}
}

func TestWriteFileAtomic_WindowsPreservesLockedDestination(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "auth.json")
	if err := os.WriteFile(path, []byte(`{"v":1}`), 0o600); err != nil {
		t.Fatal(err)
	}
	reader, err := os.Open(path)
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = reader.Close() }()
	started := time.Now()
	if errWrite := WriteFileAtomic(path, []byte(`{"v":2}`), 0o600); errWrite == nil {
		t.Fatal("expected replacement failure while destination is locked")
	}
	if time.Since(started) > 3*time.Second {
		t.Fatal("replacement retries exceeded the bounded wait")
	}
	if raw, errRead := os.ReadFile(path); errRead != nil || string(raw) != `{"v":1}` {
		t.Fatalf("old contents changed: %s, %v", raw, errRead)
	}
	if entries, errRead := os.ReadDir(dir); errRead != nil || len(entries) != 1 {
		t.Fatalf("failed replacement left temporary files: %v, %v", entries, errRead)
	}
}
