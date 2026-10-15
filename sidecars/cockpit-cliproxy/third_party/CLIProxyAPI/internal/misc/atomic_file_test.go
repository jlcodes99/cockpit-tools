package misc

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"sync"
	"testing"
)

// TestWriteFileAtomic_ConcurrentReadersNeverSeePartialContent hammers a single
// destination with writers while readers continuously parse it. Before atomic
// writes, auth files were truncated in place, so a reader could observe an
// empty or half-written JSON document; this is the regression that produced
// "unexpected end of JSON input" flakes in the sidecar tests.
func TestWriteFileAtomic_ConcurrentReadersNeverSeePartialContent(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "auth.json")

	payloadFor := func(tag string, fill int) []byte {
		raw, err := json.Marshal(map[string]any{
			"tag":    tag,
			"tokens": strings.Repeat("x", fill),
		})
		if err != nil {
			t.Fatalf("marshal payload: %v", err)
		}
		return append(raw, '\n')
	}
	payloads := [][]byte{payloadFor("a", 64), payloadFor("b", 512), payloadFor("c", 8)}

	if err := WriteFileAtomic(path, payloads[0], 0o600); err != nil {
		t.Fatalf("initial write: %v", err)
	}

	var wg sync.WaitGroup
	errCh := make(chan error, 16)

	// Writer: keep replacing the file with payloads of different sizes so a
	// torn write is immediately visible to readers.
	wg.Add(1)
	go func() {
		defer wg.Done()
		for i := 0; i < 300; i++ {
			if err := WriteFileAtomic(path, payloads[i%len(payloads)], 0o600); err != nil {
				errCh <- fmt.Errorf("writer iteration %d: %w", i, err)
				return
			}
		}
	}()

	// Readers: every observation must be a complete, valid JSON document equal
	// to one of the known payloads.
	valid := make(map[string]struct{}, len(payloads))
	for _, p := range payloads {
		valid[string(p)] = struct{}{}
	}
	for i := 0; i < 4; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for j := 0; j < 500; j++ {
				raw, err := os.ReadFile(path)
				if err != nil {
					errCh <- fmt.Errorf("read: %w", err)
					return
				}
				if len(raw) == 0 {
					errCh <- fmt.Errorf("read empty file")
					return
				}
				var doc map[string]any
				if err := json.Unmarshal(raw, &doc); err != nil {
					errCh <- fmt.Errorf("read partial JSON %q: %w", string(raw), err)
					return
				}
				if _, ok := valid[string(raw)]; !ok {
					errCh <- fmt.Errorf("read unexpected content %q", string(raw))
					return
				}
			}
		}()
	}

	wg.Wait()
	close(errCh)
	for err := range errCh {
		t.Error(err)
	}
}

// TestWriteFileAtomic_RejectsTraversalSegments verifies the traversal guard
// matches whole ".." segments only: genuinely traversing paths are refused,
// while names that merely contain ".." inside a segment keep working.
func TestWriteFileAtomic_RejectsTraversalSegments(t *testing.T) {
	dir := t.TempDir()
	payload := []byte(`{"ok":true}`)

	rejected := []string{
		// Build raw strings instead of filepath.Join, which would clean the
		// ".." away before WriteFileAtomic ever sees it.
		dir + "/sub/../auth.json",
		dir + "/../escape.json",
		"../outside.json",
		`..\outside.json`,
	}
	for _, p := range rejected {
		if err := WriteFileAtomic(p, payload, 0o600); err == nil {
			t.Errorf("expected rejection for traversing path %q", p)
		} else if _, statErr := os.Stat(p); !os.IsNotExist(statErr) {
			t.Errorf("rejected path %q still created a file", p)
		}
	}

	accepted := filepath.Join(dir, "account..backup.json")
	if err := WriteFileAtomic(accepted, payload, 0o600); err != nil {
		t.Errorf("legitimate dotted name rejected: %v", err)
	}
}

func TestWriteFileAtomic_PreservesRelativeSymlinkAndTightensPermissions(t *testing.T) {
	dir := t.TempDir()
	ownerDir := filepath.Join(dir, "owner")
	if err := os.Mkdir(ownerDir, 0o700); err != nil {
		t.Fatal(err)
	}
	target := filepath.Join(ownerDir, "auth.json")
	if err := os.WriteFile(target, []byte(`{"v":1}`), 0o644); err != nil {
		t.Fatal(err)
	}
	link := filepath.Join(dir, "linked.json")
	linkTarget := filepath.Join("owner", "auth.json")
	if err := os.Symlink(linkTarget, link); err != nil {
		t.Skipf("symlinks unavailable: %v", err)
	}
	if err := WriteFileAtomic(link, []byte(`{"v":2}`), 0o600); err != nil {
		t.Fatal(err)
	}
	if got, err := os.Readlink(link); err != nil || got != linkTarget {
		t.Fatalf("relative link changed: %q, %v", got, err)
	}
	if raw, err := os.ReadFile(target); err != nil || string(raw) != `{"v":2}` {
		t.Fatalf("linked contents = %s, %v", raw, err)
	}
	if info, err := os.Stat(target); err != nil || (runtime.GOOS != "windows" && info.Mode().Perm() != 0o600) {
		t.Fatalf("linked permissions not restricted: %v, %v", info, err)
	}
}

func TestWriteFileAtomic_PreservesDanglingSymlinkOnFailure(t *testing.T) {
	dir := t.TempDir()
	link := filepath.Join(dir, "linked.json")
	if err := os.Symlink("missing.json", link); err != nil {
		t.Skipf("symlinks unavailable: %v", err)
	}
	if err := WriteFileAtomic(link, []byte(`{"v":2}`), 0o600); err == nil {
		t.Fatal("expected failure resolving dangling credential link")
	}
	if target, err := os.Readlink(link); err != nil || target != "missing.json" {
		t.Fatalf("dangling link changed: %q, %v", target, err)
	}
	if entries, err := os.ReadDir(dir); err != nil || len(entries) != 1 {
		t.Fatalf("failure left temporary files: %v, %v", entries, err)
	}
}

func TestWriteFileAtomic_PermissionsAndOverwrite(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "auth.json")

	if err := WriteFileAtomic(path, []byte(`{"v":1}`), 0o600); err != nil {
		t.Fatalf("first write: %v", err)
	}
	if err := WriteFileAtomic(path, []byte(`{"v":2}`), 0o600); err != nil {
		t.Fatalf("overwrite: %v", err)
	}
	raw, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read back: %v", err)
	}
	if string(raw) != `{"v":2}` {
		t.Fatalf("unexpected content %q", string(raw))
	}
	info, err := os.Stat(path)
	if err != nil {
		t.Fatalf("stat: %v", err)
	}
	if got := info.Mode().Perm(); runtime.GOOS != "windows" && got != 0o600 {
		t.Fatalf("perm = %o, want 600", got)
	}
	// No temp files should be left behind.
	entries, err := os.ReadDir(dir)
	if err != nil {
		t.Fatalf("readdir: %v", err)
	}
	if len(entries) != 1 {
		names := make([]string, 0, len(entries))
		for _, e := range entries {
			names = append(names, e.Name())
		}
		t.Fatalf("leftover files in dir: %v", names)
	}
}

func TestWriteFileAtomic_PreservesLinkedDestinationAndFailedWrite(t *testing.T) {
	dir := t.TempDir()
	target := filepath.Join(dir, "target.json")
	link := filepath.Join(dir, "linked.json")
	if err := os.WriteFile(target, []byte(`{"v":1}`), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(target, link); err != nil {
		t.Skipf("symlinks unavailable: %v", err)
	}
	if err := WriteFileAtomic(link, []byte(`{"v":2}`), 0o600); err != nil {
		t.Fatal(err)
	}
	if info, err := os.Lstat(link); err != nil || info.Mode()&os.ModeSymlink == 0 {
		t.Fatal("credential link was replaced")
	}
	if raw, err := os.ReadFile(target); err != nil || string(raw) != `{"v":2}` {
		t.Fatalf("target not updated: %s %v", raw, err)
	}
	if err := WriteFileAtomic(dir, []byte("invalid"), 0o600); err == nil {
		t.Fatal("directory replacement unexpectedly succeeded")
	}
	entries, err := os.ReadDir(dir)
	if err != nil || len(entries) != 2 {
		t.Fatalf("failed replacement left temporary files: %v %v", entries, err)
	}
}
