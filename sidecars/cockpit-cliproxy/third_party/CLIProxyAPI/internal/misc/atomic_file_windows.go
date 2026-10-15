package misc

import (
	"errors"
	"os"
	"time"

	"golang.org/x/sys/windows"
)

func renameAuthFile(oldPath, newPath string) error {
	// Windows readers (including os.ReadFile) can briefly deny replacement.
	// Retry only these transient errors, preserving the old file on failure.
	// os.Rename uses MOVEFILE_REPLACE_EXISTING; never remove the destination.
	deadline := time.Now().Add(500 * time.Millisecond)
	for {
		err := os.Rename(oldPath, newPath)
		if err == nil || (!errors.Is(err, windows.ERROR_SHARING_VIOLATION) && !errors.Is(err, windows.ERROR_ACCESS_DENIED)) {
			return err
		}
		if time.Now().After(deadline) {
			return err
		}
		time.Sleep(10 * time.Millisecond)
	}
}
