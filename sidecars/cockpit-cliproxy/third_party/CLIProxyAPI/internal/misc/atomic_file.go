package misc

import (
	"fmt"
	"os"
	"path/filepath"
	"regexp"
)

// authPathTraversal matches a literal ".." path segment (bounded by path
// separators or string edges) in a raw, uncleaned path. Segment-exact matching
// keeps legitimate names like "account..backup.json" working while refusing
// genuinely traversing input before filepath.Clean can hide it.
var authPathTraversal = regexp.MustCompile(`(^|[\\/])\.\.([\\/]|$)`)

// WriteFileAtomic writes data to path atomically: it first writes to a
// temporary file in the same directory, fsyncs it, then renames it over the
// destination. Concurrent readers therefore always see either the complete
// previous contents or the complete new contents, never a truncated file.
// The file watcher reads auth JSON files while refreshes and metadata syncs
// rewrite them; non-atomic writes produced empty reads ("unexpected end of
// JSON input") and spurious reloads.
func WriteFileAtomic(path string, data []byte, perm os.FileMode) error {
	// Reject traversal segments in the raw path: filepath.Clean would resolve
	// "/safe/../outside" into "/outside" and silently lose the traversal, so
	// the check must run before normalization.
	if authPathTraversal.MatchString(path) {
		return fmt.Errorf("refusing to write auth file with traversing path: %s", path)
	}
	// Normalize the destination before touching the filesystem. This keeps the
	// temp file, chmod and rename all operating on one canonical path and
	// collapses any redundant separators or dot segments in caller input.
	path = filepath.Clean(path)
	dir := filepath.Dir(path)
	tmp, err := os.CreateTemp(dir, "."+filepath.Base(path)+".*.tmp")
	if err != nil {
		return fmt.Errorf("create temp file for %s: %w", path, err)
	}
	tmpName := tmp.Name()
	defer func() { _ = os.Remove(tmpName) }()

	if _, err = tmp.Write(data); err != nil {
		_ = tmp.Close()
		return fmt.Errorf("write temp file for %s: %w", path, err)
	}
	if err = tmp.Sync(); err != nil {
		_ = tmp.Close()
		return fmt.Errorf("sync temp file for %s: %w", path, err)
	}
	if err = tmp.Close(); err != nil {
		return fmt.Errorf("close temp file for %s: %w", path, err)
	}
	if err = os.Chmod(tmpName, perm); err != nil {
		return fmt.Errorf("chmod temp file for %s: %w", path, err)
	}
	if err = os.Rename(tmpName, path); err != nil {
		return fmt.Errorf("rename temp file to %s: %w", path, err)
	}
	return nil
}
