package main

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"sync"
	"time"
)

// The Go gateway owns the worker. Persistent gateways therefore keep continuity
// after the GUI quits; closing the parent pipe also handles an ungraceful exit.
func startContinuityWorker(ctx context.Context, spec *continuityServiceSpec, m *manifest) (*continuityClient, error) {
	if spec == nil || spec.Version != 1 || !filepath.IsAbs(spec.Executable) || !filepath.IsAbs(spec.ConfigPath) || len(spec.Token) != 64 {
		return nil, errors.New("invalid continuity worker configuration")
	}
	info, err := os.Lstat(spec.ConfigPath)
	if err != nil || !info.Mode().IsRegular() || info.Mode()&os.ModeSymlink != 0 || info.Size() > 8<<20 {
		return nil, errors.New("continuity worker configuration is unavailable")
	}
	cmd := exec.Command(spec.Executable, "--codex-continuity-bridge", spec.ConfigPath)
	configureContinuityProcess(cmd)
	stdin, err := cmd.StdinPipe()
	if err != nil {
		return nil, errors.New("continuity worker pipe is unavailable")
	}
	stdout, err := cmd.StdoutPipe()
	if err != nil {
		_ = stdin.Close()
		return nil, errors.New("continuity worker pipe is unavailable")
	}
	cmd.Stderr = io.Discard
	if err = cmd.Start(); err != nil {
		_ = stdin.Close()
		_ = stdout.Close()
		return nil, errors.New("continuity worker failed to start")
	}
	done := make(chan struct{})
	go func() { _ = cmd.Wait(); close(done) }()
	var once sync.Once
	stop := func() {
		once.Do(func() {
			_ = stdin.Close()
			select {
			case <-done:
			case <-time.After(2 * time.Second):
				_ = cmd.Process.Kill()
				select {
				case <-done:
				case <-time.After(time.Second):
				}
			}
		})
	}
	type readyResult struct {
		endpoint string
		err      error
	}
	ready := make(chan readyResult, 1)
	go func() {
		scanner := bufio.NewScanner(stdout)
		scanner.Buffer(make([]byte, 1024), 64<<10)
		if !scanner.Scan() {
			ready <- readyResult{err: errors.New("continuity worker did not become ready")}
			return
		}
		var response struct {
			Protocol int    `json:"protocol"`
			Endpoint string `json:"endpoint"`
		}
		if json.Unmarshal(scanner.Bytes(), &response) != nil || response.Protocol != 1 {
			ready <- readyResult{err: errors.New("invalid continuity worker handshake")}
			return
		}
		ready <- readyResult{endpoint: response.Endpoint}
		// No request payloads or diagnostics are accepted on this private ready pipe.
		for scanner.Scan() {
		}
	}()
	timer := time.NewTimer(10 * time.Second)
	defer timer.Stop()
	select {
	case result := <-ready:
		if result.err != nil {
			stop()
			return nil, result.err
		}
		client, errClient := newContinuityClient(result.endpoint, spec.Token, m)
		if errClient != nil {
			stop()
			return nil, errClient
		}
		client.closeWorker = stop
		go func() {
			select {
			case <-ctx.Done():
				stop()
			case <-done:
			}
		}()
		return client, nil
	case <-ctx.Done():
		stop()
		return nil, ctx.Err()
	case <-timer.C:
		stop()
		return nil, errors.New("continuity worker startup timed out")
	}
}
