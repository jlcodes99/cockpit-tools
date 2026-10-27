package main

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"io"
	"os"
	"strings"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/registry"
)

const maxHostCodexClientModelsBytes = 8 << 20

func loadManifestCodexClientModels(m *manifest) error {
	if m == nil {
		return nil
	}
	path := strings.TrimSpace(m.CodexClientModelsPath)
	hash := strings.TrimSpace(m.CodexClientModelsHash)
	if path == "" && hash == "" {
		return nil
	}
	if path == "" || hash == "" {
		return fmt.Errorf("host Codex client catalog requires both codexClientModelsPath and codexClientModelsHash")
	}
	expectedHash, err := hex.DecodeString(hash)
	if err != nil || len(expectedHash) != sha256.Size {
		return fmt.Errorf("host Codex client catalog codexClientModelsHash must be a 64-character SHA256 hex digest")
	}
	file, err := os.Open(path)
	if err != nil {
		return fmt.Errorf("read host Codex client catalog %q: %w", path, err)
	}
	defer file.Close()
	data, err := io.ReadAll(io.LimitReader(file, maxHostCodexClientModelsBytes+1))
	if err != nil {
		return fmt.Errorf("read host Codex client catalog %q: %w", path, err)
	}
	if len(data) > maxHostCodexClientModelsBytes {
		return fmt.Errorf("host Codex client catalog %q exceeds 8 MiB", path)
	}
	actualHash := sha256.Sum256(data)
	if !bytes.Equal(actualHash[:], expectedHash) {
		return fmt.Errorf("host Codex client catalog %q SHA256 mismatch", path)
	}
	if _, err := registry.LoadCodexClientModelsJSON(data); err != nil {
		return fmt.Errorf("load host Codex client catalog %q: %w", path, err)
	}
	m.codexClientModelsLoaded = true
	return nil
}
