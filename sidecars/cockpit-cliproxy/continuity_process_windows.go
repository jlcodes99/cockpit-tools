//go:build windows

package main

import (
	"os/exec"
	"syscall"
)

func configureContinuityProcess(cmd *exec.Cmd) {
	cmd.SysProcAttr = &syscall.SysProcAttr{CreationFlags: 0x08000000, HideWindow: true}
}
