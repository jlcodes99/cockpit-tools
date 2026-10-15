//go:build !windows

package misc

import "os"

func renameAuthFile(oldPath, newPath string) error {
	return os.Rename(oldPath, newPath)
}
