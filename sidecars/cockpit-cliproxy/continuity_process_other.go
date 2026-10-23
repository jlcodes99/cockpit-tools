//go:build !windows

package main

import "os/exec"

func configureContinuityProcess(cmd *exec.Cmd) {}
