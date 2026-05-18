package metrics

import (
	"database/sql"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
)

func TestMigrateMetricsKeysToIdentity_MigratesRecordsAndCircuitStates(t *testing.T) {
	store, err := NewSQLiteStore(&SQLiteStoreConfig{
		DBPath:        filepath.Join(t.TempDir(), "metrics.db"),
		RetentionDays: 30,
	})
	if err != nil {
		t.Fatalf("NewSQLiteStore() error = %v", err)
	}
	defer store.Close()

	baseURL := "https://example.com"
	apiKey := "sk-test"
	legacyKey := GenerateMetricsKey(baseURL, apiKey)
	identityKey := GenerateMetricsIdentityKey(baseURL, apiKey, "claude")
	if legacyKey == identityKey {
		t.Fatalf("expected legacy and identity keys to differ")
	}

	now := time.Now().Unix()
	if _, err := store.db.Exec(`
		INSERT INTO request_records (
			metrics_key, base_url, key_mask, timestamp, success, failure_class,
			input_tokens, output_tokens, cache_creation_tokens, cache_read_tokens, api_type, model
		) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
	`, legacyKey, baseURL, "sk-***", now, 1, "", 10, 20, 0, 0, "messages", "claude-3"); err != nil {
		t.Fatalf("insert request_records: %v", err)
	}
	if _, err := store.db.Exec(`
		INSERT INTO circuit_states (
			metrics_key, api_type, base_url, key_mask, circuit_state,
			circuit_opened_at, half_open_at, next_retry_at,
			backoff_level, half_open_successes, consecutive_failures, updated_at
		) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
	`, legacyKey, "messages", baseURL, "sk-***", "open", now, nil, now+60, 2, 0, 3, now); err != nil {
		t.Fatalf("insert circuit_states: %v", err)
	}

	cfg := config.Config{
		Upstream: []config.UpstreamConfig{{
			Name:    "legacy-messages",
			BaseURL: baseURL,
			APIKeys: []string{apiKey},
		}},
	}

	if err := store.MigrateMetricsKeysToIdentity(cfg); err != nil {
		t.Fatalf("MigrateMetricsKeysToIdentity() error = %v", err)
	}

	version, err := store.schemaVersion()
	if err != nil {
		t.Fatalf("schemaVersion() error = %v", err)
	}
	if version != 3 {
		t.Fatalf("schemaVersion = %d, want 3", version)
	}

	var migratedRecordCount int
	if err := store.db.QueryRow(`SELECT COUNT(*) FROM request_records WHERE metrics_key = ?`, identityKey).Scan(&migratedRecordCount); err != nil {
		t.Fatalf("count migrated request_records: %v", err)
	}
	if migratedRecordCount != 1 {
		t.Fatalf("migrated request_records count = %d, want 1", migratedRecordCount)
	}
	var legacyRecordCount int
	if err := store.db.QueryRow(`SELECT COUNT(*) FROM request_records WHERE metrics_key = ?`, legacyKey).Scan(&legacyRecordCount); err != nil {
		t.Fatalf("count legacy request_records: %v", err)
	}
	if legacyRecordCount != 0 {
		t.Fatalf("legacy request_records count = %d, want 0", legacyRecordCount)
	}

	var stateKey, stateBaseURL, stateValue string
	if err := store.db.QueryRow(`SELECT metrics_key, base_url, circuit_state FROM circuit_states WHERE api_type = 'messages'`).Scan(&stateKey, &stateBaseURL, &stateValue); err != nil {
		t.Fatalf("load migrated circuit_state: %v", err)
	}
	if stateKey != identityKey {
		t.Fatalf("circuit_state metrics_key = %s, want %s", stateKey, identityKey)
	}
	if stateBaseURL != baseURL+"/v1" {
		t.Fatalf("circuit_state base_url = %s, want %s", stateBaseURL, baseURL+"/v1")
	}
	if stateValue != "open" {
		t.Fatalf("circuit_state = %s, want open", stateValue)
	}
}

func TestMigrateMetricsKeysToIdentity_MergesCircuitStatesBySeverity(t *testing.T) {
	store, err := NewSQLiteStore(&SQLiteStoreConfig{
		DBPath:        filepath.Join(t.TempDir(), "metrics.db"),
		RetentionDays: 30,
	})
	if err != nil {
		t.Fatalf("NewSQLiteStore() error = %v", err)
	}
	defer store.Close()

	baseURL := "https://example.com"
	apiKey := "sk-test"
	legacyKey := GenerateMetricsKey(baseURL, apiKey)
	identityKey := GenerateMetricsIdentityKey(baseURL, apiKey, "claude")
	now := time.Now().Unix()

	if _, err := store.db.Exec(`
		INSERT INTO circuit_states (
			metrics_key, api_type, base_url, key_mask, circuit_state,
			circuit_opened_at, half_open_at, next_retry_at,
			backoff_level, half_open_successes, consecutive_failures, updated_at
		) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
	`, legacyKey, "messages", baseURL, "sk-***", "open", now, nil, now+120, 4, 0, 5, now); err != nil {
		t.Fatalf("insert legacy circuit_state: %v", err)
	}
	if _, err := store.db.Exec(`
		INSERT INTO circuit_states (
			metrics_key, api_type, base_url, key_mask, circuit_state,
			circuit_opened_at, half_open_at, next_retry_at,
			backoff_level, half_open_successes, consecutive_failures, updated_at
		) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
	`, identityKey, "messages", baseURL+"/v1", "sk-***", "half_open", now-60, now-30, now+30, 1, 1, 1, now-10); err != nil {
		t.Fatalf("insert identity circuit_state: %v", err)
	}

	cfg := config.Config{
		Upstream: []config.UpstreamConfig{{
			Name:    "legacy-messages",
			BaseURL: baseURL,
			APIKeys: []string{apiKey},
		}},
	}

	if err := store.MigrateMetricsKeysToIdentity(cfg); err != nil {
		t.Fatalf("MigrateMetricsKeysToIdentity() error = %v", err)
	}

	rows, err := store.db.Query(`SELECT metrics_key, circuit_state, backoff_level, consecutive_failures FROM circuit_states WHERE api_type = 'messages'`)
	if err != nil {
		t.Fatalf("query circuit_states: %v", err)
	}
	defer rows.Close()

	count := 0
	for rows.Next() {
		var key, state string
		var backoffLevel int
		var failures int64
		if err := rows.Scan(&key, &state, &backoffLevel, &failures); err != nil {
			t.Fatalf("scan circuit_state: %v", err)
		}
		count++
		if key != identityKey {
			t.Fatalf("merged metrics_key = %s, want %s", key, identityKey)
		}
		if state != "open" {
			t.Fatalf("merged circuit_state = %s, want open", state)
		}
		if backoffLevel != 4 {
			t.Fatalf("merged backoff_level = %d, want 4", backoffLevel)
		}
		if failures != 5 {
			t.Fatalf("merged consecutive_failures = %d, want 5", failures)
		}
	}
	if err := rows.Err(); err != nil {
		t.Fatalf("rows err: %v", err)
	}
	if count != 1 {
		t.Fatalf("merged circuit_state rows = %d, want 1", count)
	}
}

func TestMigrateMetricsKeysToIdentity_MigratesHashSuffixLegacyRows(t *testing.T) {
	store, err := NewSQLiteStore(&SQLiteStoreConfig{
		DBPath:        filepath.Join(t.TempDir(), "metrics.db"),
		RetentionDays: 30,
	})
	if err != nil {
		t.Fatalf("NewSQLiteStore() error = %v", err)
	}
	defer store.Close()

	baseURL := "https://example.com/#"
	apiKey := "sk-test"
	legacyKey := GenerateMetricsKey("https://example.com/#", apiKey)
	identityKey := GenerateMetricsIdentityKey(baseURL, apiKey, "claude")
	now := time.Now().Unix()

	mustExecMigrationTest(t, store.db, `
		INSERT INTO request_records (
			metrics_key, base_url, key_mask, timestamp, success, failure_class,
			input_tokens, output_tokens, cache_creation_tokens, cache_read_tokens, api_type, model
		) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
	`, legacyKey, "https://example.com/#", "sk-***", now, 1, "", 10, 20, 0, 0, "messages", "claude-3")
	mustExecMigrationTest(t, store.db, `
		INSERT INTO circuit_states (
			metrics_key, api_type, base_url, key_mask, circuit_state,
			circuit_opened_at, half_open_at, next_retry_at,
			backoff_level, half_open_successes, consecutive_failures, updated_at
		) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
	`, legacyKey, "messages", "https://example.com/#", "sk-***", "half_open", nil, now-30, now+60, 1, 1, 1, now)

	cfg := config.Config{
		Upstream: []config.UpstreamConfig{{
			Name:    "hash-channel",
			BaseURL: baseURL,
			APIKeys: []string{apiKey},
		}},
	}

	if err := store.MigrateMetricsKeysToIdentity(cfg); err != nil {
		t.Fatalf("MigrateMetricsKeysToIdentity() error = %v", err)
	}

	var migratedRecordCount int
	if err := store.db.QueryRow(`SELECT COUNT(*) FROM request_records WHERE metrics_key = ?`, identityKey).Scan(&migratedRecordCount); err != nil {
		t.Fatalf("count migrated request_records: %v", err)
	}
	if migratedRecordCount != 1 {
		t.Fatalf("migrated request_records count = %d, want 1", migratedRecordCount)
	}

	var stateKey, stateBaseURL string
	if err := store.db.QueryRow(`SELECT metrics_key, base_url FROM circuit_states WHERE api_type = 'messages'`).Scan(&stateKey, &stateBaseURL); err != nil {
		t.Fatalf("load migrated circuit_state: %v", err)
	}
	if stateKey != identityKey {
		t.Fatalf("circuit_state metrics_key = %s, want %s", stateKey, identityKey)
	}
	if stateBaseURL != "https://example.com#" {
		t.Fatalf("circuit_state base_url = %s, want %s", stateBaseURL, "https://example.com#")
	}
}

func TestMigrateMetricsKeysToIdentity_MigratesHistoricalAPIKeyRows(t *testing.T) {
	store, err := NewSQLiteStore(&SQLiteStoreConfig{
		DBPath:        filepath.Join(t.TempDir(), "metrics.db"),
		RetentionDays: 30,
	})
	if err != nil {
		t.Fatalf("NewSQLiteStore() error = %v", err)
	}
	defer store.Close()

	baseURL := "https://example.com"
	historicalKey := "sk-history"
	legacyKey := GenerateMetricsKey(baseURL, historicalKey)
	identityKey := GenerateMetricsIdentityKey(baseURL, historicalKey, "claude")
	now := time.Now().Unix()

	mustExecMigrationTest(t, store.db, `
		INSERT INTO request_records (
			metrics_key, base_url, key_mask, timestamp, success, failure_class,
			input_tokens, output_tokens, cache_creation_tokens, cache_read_tokens, api_type, model
		) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
	`, legacyKey, baseURL, "sk-***", now, 1, "", 10, 20, 0, 0, "messages", "claude-3")
	mustExecMigrationTest(t, store.db, `
		INSERT INTO circuit_states (
			metrics_key, api_type, base_url, key_mask, circuit_state,
			circuit_opened_at, half_open_at, next_retry_at,
			backoff_level, half_open_successes, consecutive_failures, updated_at
		) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
	`, legacyKey, "messages", baseURL, "sk-***", "open", now, nil, now+60, 2, 0, 3, now)

	cfg := config.Config{
		Upstream: []config.UpstreamConfig{{
			Name:              "legacy-messages",
			BaseURL:           baseURL,
			HistoricalAPIKeys: []string{historicalKey},
			ServiceType:       "claude",
		}},
	}

	if err := store.MigrateMetricsKeysToIdentity(cfg); err != nil {
		t.Fatalf("MigrateMetricsKeysToIdentity() error = %v", err)
	}

	var migratedRecordCount int
	if err := store.db.QueryRow(`SELECT COUNT(*) FROM request_records WHERE metrics_key = ?`, identityKey).Scan(&migratedRecordCount); err != nil {
		t.Fatalf("count migrated request_records: %v", err)
	}
	if migratedRecordCount != 1 {
		t.Fatalf("migrated request_records count = %d, want 1", migratedRecordCount)
	}

	var stateKey string
	if err := store.db.QueryRow(`SELECT metrics_key FROM circuit_states WHERE api_type = 'messages'`).Scan(&stateKey); err != nil {
		t.Fatalf("load migrated circuit_state: %v", err)
	}
	if stateKey != identityKey {
		t.Fatalf("circuit_state metrics_key = %s, want %s", stateKey, identityKey)
	}
}

func TestMigrateMetricsKeysToIdentity_IdempotentWhenAlreadyMigrated(t *testing.T) {
	store, err := NewSQLiteStore(&SQLiteStoreConfig{
		DBPath:        filepath.Join(t.TempDir(), "metrics.db"),
		RetentionDays: 30,
	})
	if err != nil {
		t.Fatalf("NewSQLiteStore() error = %v", err)
	}
	defer store.Close()

	if _, err := store.db.Exec("PRAGMA user_version = 3"); err != nil {
		t.Fatalf("set user_version: %v", err)
	}

	cfg := config.Config{}
	if err := store.MigrateMetricsKeysToIdentity(cfg); err != nil {
		t.Fatalf("MigrateMetricsKeysToIdentity() error = %v", err)
	}
}

func TestMigrateMetricsKeysToIdentity_IgnoresConflictingMappingsWithoutLegacyRows(t *testing.T) {
	store, err := NewSQLiteStore(&SQLiteStoreConfig{
		DBPath:        filepath.Join(t.TempDir(), "metrics.db"),
		RetentionDays: 30,
	})
	if err != nil {
		t.Fatalf("NewSQLiteStore() error = %v", err)
	}
	defer store.Close()

	cfg := config.Config{
		Upstream: []config.UpstreamConfig{{
			Name:        "claude-shared",
			BaseURL:     "https://shared.example.com",
			APIKeys:     []string{"sk-shared"},
			ServiceType: "claude",
		}, {
			Name:        "gemini-shared",
			BaseURL:     "https://shared.example.com",
			APIKeys:     []string{"sk-shared"},
			ServiceType: "gemini",
		}},
	}

	if err := store.MigrateMetricsKeysToIdentity(cfg); err != nil {
		t.Fatalf("MigrateMetricsKeysToIdentity() error = %v", err)
	}

	version, err := store.schemaVersion()
	if err != nil {
		t.Fatalf("schemaVersion() error = %v", err)
	}
	if version != 3 {
		t.Fatalf("schemaVersion = %d, want 3", version)
	}
}

func TestMigrateMetricsKeysToIdentity_FailsWhenConflictingLegacyRowsExist(t *testing.T) {
	store, err := NewSQLiteStore(&SQLiteStoreConfig{
		DBPath:        filepath.Join(t.TempDir(), "metrics.db"),
		RetentionDays: 30,
	})
	if err != nil {
		t.Fatalf("NewSQLiteStore() error = %v", err)
	}
	defer store.Close()

	legacyKey := GenerateMetricsKey("https://shared.example.com", "sk-shared")
	now := time.Now().Unix()
	mustExecMigrationTest(t, store.db, `
		INSERT INTO request_records (
			metrics_key, base_url, key_mask, timestamp, success, failure_class,
			input_tokens, output_tokens, cache_creation_tokens, cache_read_tokens, api_type, model
		) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
	`, legacyKey, "https://shared.example.com", "sk-***", now, 1, "", 1, 1, 0, 0, "messages", "claude-3")

	cfg := config.Config{
		Upstream: []config.UpstreamConfig{{
			Name:        "claude-shared",
			BaseURL:     "https://shared.example.com",
			APIKeys:     []string{"sk-shared"},
			ServiceType: "claude",
		}, {
			Name:        "gemini-shared",
			BaseURL:     "https://shared.example.com",
			APIKeys:     []string{"sk-shared"},
			ServiceType: "gemini",
		}},
	}

	err = store.MigrateMetricsKeysToIdentity(cfg)
	if err == nil {
		t.Fatal("MigrateMetricsKeysToIdentity() error = nil, want conflict error")
	}
	if !strings.Contains(err.Error(), "映射到多个 identity target") {
		t.Fatalf("MigrateMetricsKeysToIdentity() error = %v, want conflict detail", err)
	}
}

func TestMigrateMetricsKeysToIdentity_LeavesUnmappedLegacyRowsUntouched(t *testing.T) {
	store, err := NewSQLiteStore(&SQLiteStoreConfig{
		DBPath:        filepath.Join(t.TempDir(), "metrics.db"),
		RetentionDays: 30,
	})
	if err != nil {
		t.Fatalf("NewSQLiteStore() error = %v", err)
	}
	defer store.Close()

	baseURL := "https://orphan.example.com"
	apiKey := "sk-orphan"
	legacyKey := GenerateMetricsKey(baseURL, apiKey)
	now := time.Now().Unix()

	mustExecMigrationTest(t, store.db, `
		INSERT INTO request_records (
			metrics_key, base_url, key_mask, timestamp, success, failure_class,
			input_tokens, output_tokens, cache_creation_tokens, cache_read_tokens, api_type, model
		) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
	`, legacyKey, baseURL, "sk-***", now, 1, "", 10, 20, 0, 0, "messages", "claude-3")

	cfg := config.Config{}
	if err := store.MigrateMetricsKeysToIdentity(cfg); err != nil {
		t.Fatalf("MigrateMetricsKeysToIdentity() error = %v", err)
	}

	var legacyCount int
	if err := store.db.QueryRow(`SELECT COUNT(*) FROM request_records WHERE metrics_key = ?`, legacyKey).Scan(&legacyCount); err != nil {
		t.Fatalf("count legacy request_records: %v", err)
	}
	if legacyCount != 1 {
		t.Fatalf("legacy request_records count = %d, want 1", legacyCount)
	}
}

func TestSchemaVersion(t *testing.T) {
	store, err := NewSQLiteStore(&SQLiteStoreConfig{
		DBPath:        filepath.Join(t.TempDir(), "metrics.db"),
		RetentionDays: 30,
	})
	if err != nil {
		t.Fatalf("NewSQLiteStore() error = %v", err)
	}
	defer store.Close()

	version, err := store.schemaVersion()
	if err != nil {
		t.Fatalf("schemaVersion() error = %v", err)
	}
	if version != 2 {
		t.Fatalf("schemaVersion = %d, want 2", version)
	}
}

func mustExecMigrationTest(t *testing.T, db *sql.DB, query string, args ...any) {
	t.Helper()
	if _, err := db.Exec(query, args...); err != nil {
		t.Fatalf("exec %q: %v", query, err)
	}
}
