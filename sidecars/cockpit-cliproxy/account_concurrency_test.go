package main

import (
	"context"
	"errors"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/gin-gonic/gin"

	internallogging "github.com/router-for-me/CLIProxyAPI/v7/internal/logging"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/config"
)

// orderedAuthSelector 按固定偏好顺序返回候选账号，用于模拟调度策略的排序结果。
type orderedAuthSelector struct {
	order []string
}

func (s *orderedAuthSelector) Pick(_ context.Context, _, _ string, _ cliproxyexecutor.Options, auths []*coreauth.Auth) (*coreauth.Auth, error) {
	for _, want := range s.order {
		for _, auth := range auths {
			if auth != nil && auth.ID == want {
				return auth, nil
			}
		}
	}
	if len(auths) == 0 {
		return nil, nil
	}
	return auths[0], nil
}

func accountConcurrencyRequestContext(requestID string) context.Context {
	return internallogging.WithRequestID(context.Background(), requestID)
}

func testAuth(id string) *coreauth.Auth {
	return &coreauth.Auth{ID: id, Provider: "codex", Status: coreauth.StatusActive}
}

func TestAccountSlotSelectorFailsOverWhenPreferredAccountIsFull(t *testing.T) {
	tracker := newRequestUsageTracker()
	if !tracker.tryReserveAccountSlot("existing-request", "auth-a", 1) {
		t.Fatal("expected initial reservation on auth-a")
	}
	m := &manifest{MaxAccountConcurrency: 1}
	selector := &accountSlotSelector{
		manifest: m,
		tracker:  tracker,
		fallback: &orderedAuthSelector{order: []string{"auth-a", "auth-b"}},
	}

	selected, err := selector.Pick(
		accountConcurrencyRequestContext("new-request"),
		"codex",
		"gpt-5.4",
		cliproxyexecutor.Options{},
		[]*coreauth.Auth{testAuth("auth-a"), testAuth("auth-b")},
	)
	if err != nil {
		t.Fatalf("Pick: %v", err)
	}
	if selected == nil || selected.ID != "auth-b" {
		t.Fatalf("expected failover to auth-b, got %#v", selected)
	}
	if got := tracker.accountInFlightCount("auth-b"); got != 1 {
		t.Fatalf("expected auth-b in-flight=1, got %d", got)
	}
	if got := tracker.accountInFlightCount("auth-a"); got != 1 {
		t.Fatalf("expected auth-a in-flight to stay 1, got %d", got)
	}
}

func TestAccountConcurrencySelectorWaitsForBoundAccount(t *testing.T) {
	tracker := newRequestUsageTracker()
	if !tracker.tryReserveAccountSlot("in-flight", "auth-a", 1) {
		t.Fatal("expected initial reservation on auth-a")
	}
	m := &manifest{MaxAccountConcurrency: 1, AccountConcurrencyWaitMs: 5000}
	selector := &accountConcurrencySelector{
		manifest: m,
		tracker:  tracker,
		locale:   "en",
		fallback: &orderedAuthSelector{order: []string{"auth-a"}},
	}

	go func() {
		time.Sleep(80 * time.Millisecond)
		tracker.releaseAccountSlots("in-flight")
	}()

	startedAt := time.Now()
	selected, err := selector.Pick(
		accountConcurrencyRequestContext("waiting-request"),
		"codex",
		"gpt-5.4",
		cliproxyexecutor.Options{},
		[]*coreauth.Auth{testAuth("auth-a")},
	)
	if err != nil {
		t.Fatalf("Pick: %v", err)
	}
	if selected == nil || selected.ID != "auth-a" {
		t.Fatalf("expected auth-a after waiting, got %#v", selected)
	}
	if elapsed := time.Since(startedAt); elapsed < 50*time.Millisecond {
		t.Fatalf("expected Pick to wait for the slot, elapsed=%s", elapsed)
	}
	if got := tracker.accountInFlightCount("auth-a"); got != 1 {
		t.Fatalf("expected auth-a in-flight=1 after acquiring, got %d", got)
	}
}

func TestAccountConcurrencySelectorRejectsWhenWaitDisabled(t *testing.T) {
	tracker := newRequestUsageTracker()
	if !tracker.tryReserveAccountSlot("in-flight", "auth-a", 1) {
		t.Fatal("expected initial reservation on auth-a")
	}
	m := &manifest{MaxAccountConcurrency: 1, AccountConcurrencyWaitMs: 0}
	selector := &accountConcurrencySelector{
		manifest: m,
		tracker:  tracker,
		locale:   "en",
		fallback: &orderedAuthSelector{order: []string{"auth-a"}},
	}

	startedAt := time.Now()
	_, err := selector.Pick(
		accountConcurrencyRequestContext("rejected-request"),
		"codex",
		"gpt-5.4",
		cliproxyexecutor.Options{},
		[]*coreauth.Auth{testAuth("auth-a")},
	)
	if err == nil {
		t.Fatal("expected concurrency error when wait is disabled")
	}
	if elapsed := time.Since(startedAt); elapsed > time.Second {
		t.Fatalf("expected immediate rejection, elapsed=%s", elapsed)
	}
	var authErr *coreauth.Error
	if !errors.As(err, &authErr) {
		t.Fatalf("expected coreauth.Error, got %#v", err)
	}
	if authErr.Code != "account_concurrency_exceeded" {
		t.Fatalf("unexpected error code %q", authErr.Code)
	}
	if authErr.HTTPStatus != http.StatusTooManyRequests {
		t.Fatalf("expected 429, got %d", authErr.HTTPStatus)
	}
	if authErr.Retryable {
		t.Fatal("expected non-retryable concurrency error")
	}
}

func TestAccountConcurrencySelectorWaitsWhenAllCandidatesAreFull(t *testing.T) {
	tracker := newRequestUsageTracker()
	if !tracker.tryReserveAccountSlot("in-flight", "auth-a", 1) {
		t.Fatal("expected initial reservation on auth-a")
	}
	m := &manifest{MaxAccountConcurrency: 1, AccountConcurrencyWaitMs: 5000}
	inner := &accountSlotSelector{
		manifest: m,
		tracker:  tracker,
		fallback: &orderedAuthSelector{order: []string{"auth-a"}},
	}
	selector := &accountConcurrencySelector{
		manifest: m,
		tracker:  tracker,
		locale:   "en",
		fallback: inner,
	}

	go func() {
		time.Sleep(80 * time.Millisecond)
		tracker.releaseAccountSlots("in-flight")
	}()

	selected, err := selector.Pick(
		accountConcurrencyRequestContext("queued-request"),
		"codex",
		"gpt-5.4",
		cliproxyexecutor.Options{},
		[]*coreauth.Auth{testAuth("auth-a")},
	)
	if err != nil {
		t.Fatalf("Pick: %v", err)
	}
	if selected == nil || selected.ID != "auth-a" {
		t.Fatalf("expected auth-a after queueing, got %#v", selected)
	}
}

func TestAccountConcurrencySelectorTimesOutWhenSlotNeverFrees(t *testing.T) {
	tracker := newRequestUsageTracker()
	if !tracker.tryReserveAccountSlot("in-flight", "auth-a", 1) {
		t.Fatal("expected initial reservation on auth-a")
	}
	m := &manifest{MaxAccountConcurrency: 1, AccountConcurrencyWaitMs: 60}
	selector := &accountConcurrencySelector{
		manifest: m,
		tracker:  tracker,
		locale:   "en",
		fallback: &orderedAuthSelector{order: []string{"auth-a"}},
	}

	_, err := selector.Pick(
		accountConcurrencyRequestContext("timeout-request"),
		"codex",
		"gpt-5.4",
		cliproxyexecutor.Options{},
		[]*coreauth.Auth{testAuth("auth-a")},
	)
	if err == nil {
		t.Fatal("expected timeout error")
	}
	var authErr *coreauth.Error
	if !errors.As(err, &authErr) || authErr.HTTPStatus != http.StatusTooManyRequests {
		t.Fatalf("expected 429 concurrency error, got %#v", err)
	}
}

func TestAccountConcurrencySelectorSkipsTrackingWithoutRequestID(t *testing.T) {
	tracker := newRequestUsageTracker()
	m := &manifest{MaxAccountConcurrency: 1}
	selector := &accountConcurrencySelector{
		manifest: m,
		tracker:  tracker,
		locale:   "en",
		fallback: &orderedAuthSelector{order: []string{"auth-a"}},
	}
	// 背景任务没有 requestID：不应占用槽位，也不应被限制。
	for i := 0; i < 3; i++ {
		if _, err := selector.Pick(
			context.Background(),
			"codex",
			"gpt-5.4",
			cliproxyexecutor.Options{},
			[]*coreauth.Auth{testAuth("auth-a")},
		); err != nil {
			t.Fatalf("Pick: %v", err)
		}
	}
	if got := tracker.accountInFlightCount("auth-a"); got != 0 {
		t.Fatalf("expected no reservation without request id, got %d", got)
	}
}

func TestAccountConcurrencyDisabledKeepsSelectorChainUnchanged(t *testing.T) {
	m := &manifest{MaxAccountConcurrency: 0}
	tracker := newRequestUsageTracker()
	base := &orderedAuthSelector{order: []string{"auth-a"}}
	if accountConcurrencyEnabled(m, tracker) {
		t.Fatal("expected concurrency gate to stay disabled for limit 0")
	}
	if chain := buildCoreAuthSelectorWithConcurrency(nil, base, nil, nil, tracker); chain != coreauth.Selector(base) {
		t.Fatalf("expected unchanged selector when disabled, got %#v", chain)
	}
	if accountConcurrencyEnabled(m, nil) {
		t.Fatal("expected concurrency gate to stay disabled without tracker")
	}
}

func TestAccountConcurrencyWrapsSelectorChainWhenEnabled(t *testing.T) {
	m := &manifest{MaxAccountConcurrency: 2, AccountConcurrencyWaitMs: 1000}
	tracker := newRequestUsageTracker()
	if !accountConcurrencyEnabled(m, tracker) {
		t.Fatal("expected concurrency gate to be enabled")
	}
	chain := buildCoreAuthSelectorWithConcurrency(nil, &orderedAuthSelector{order: []string{"auth-a"}}, m, nil, tracker)
	scope, ok := chain.(*apiKeyScopeSelector)
	if !ok {
		t.Fatalf("expected authorization scope at the outermost layer, got %#v", chain)
	}
	concurrency, ok := scope.fallback.(*accountConcurrencySelector)
	if !ok {
		t.Fatalf("expected concurrency gate immediately inside authorization scope, got %#v", scope.fallback)
	}
	if _, ok := concurrency.fallback.(*quotaCooldownSelector); !ok {
		t.Fatalf("expected quota cooldown selector below the concurrency gate, got %#v", chain)
	}
}

func TestReleaseAccountSlotsFreesSlotForNextRequest(t *testing.T) {
	tracker := newRequestUsageTracker()
	if !tracker.tryReserveAccountSlot("first", "auth-a", 1) {
		t.Fatal("expected first reservation")
	}
	if tracker.tryReserveAccountSlot("second", "auth-a", 1) {
		t.Fatal("expected second reservation to be rejected while first is in flight")
	}
	tracker.releaseAccountSlots("first")
	if got := tracker.accountInFlightCount("auth-a"); got != 0 {
		t.Fatalf("expected in-flight=0 after release, got %d", got)
	}
	if !tracker.tryReserveAccountSlot("second", "auth-a", 1) {
		t.Fatal("expected reservation to succeed after release")
	}
}

func TestRequestMiddlewareReleasesAccountSlots(t *testing.T) {
	gin.SetMode(gin.TestMode)
	tracker := newRequestUsageTracker()
	policy := &requestPolicy{manifest: &manifest{}, tracker: tracker}
	router := gin.New()
	router.Use(policy.middleware())
	router.POST("/v1/responses", func(c *gin.Context) {
		requestID := internallogging.GetRequestID(c.Request.Context())
		if requestID == "" {
			t.Error("expected request id in context")
		}
		if !tracker.tryReserveAccountSlot(requestID, "auth-a", 1) {
			t.Error("expected slot reservation inside handler")
		}
		c.JSON(http.StatusOK, gin.H{"ok": true})
	})

	recorder := httptest.NewRecorder()
	request := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader("{}"))
	router.ServeHTTP(recorder, request)

	if got := tracker.accountInFlightCount("auth-a"); got != 0 {
		t.Fatalf("expected account slot released after request finished, got %d", got)
	}
}

// TestAccountConcurrencyGateThroughAuthManager 走真实的鉴权管理器与完整选择器链，
// 验证「账号并发数」在主要请求路径上真正生效：第二个并发请求被挡下，
// 释放槽位后可以再次选中同一账号。
func TestAccountConcurrencyGateThroughAuthManager(t *testing.T) {
	tempDir := t.TempDir()
	authDir := filepath.Join(tempDir, "auths")
	if err := os.MkdirAll(authDir, 0o755); err != nil {
		t.Fatalf("create auth dir: %v", err)
	}
	configPath := filepath.Join(tempDir, "config.json")
	if err := os.WriteFile(configPath, []byte(`{}`), 0o644); err != nil {
		t.Fatalf("write config: %v", err)
	}
	authFile := "single-account.json"
	if err := os.WriteFile(filepath.Join(authDir, authFile), []byte(`{
  "type":"codex",
  "email":"single@example.com",
  "access_token":"single-token",
  "account_id":"acct-single"
}`), 0o600); err != nil {
		t.Fatalf("write auth file: %v", err)
	}

	account := &accountSpec{
		ID:       "single-account",
		Email:    "single@example.com",
		AuthID:   authFile,
		AuthKind: "oauth",
	}
	m := &manifest{
		Accounts:                 []accountSpec{*account},
		ModelIDs:                 []string{"gpt-5.4"},
		MaxAccountConcurrency:    1,
		AccountConcurrencyWaitMs: 0,
		accountByID:              map[string]*accountSpec{"single-account": account},
		accountByAuthID:          map[string]*accountSpec{strings.ToLower(authFile): account},
		accountByAPIKey:          map[string]*accountSpec{},
		accountByChatGPT:         map[string]*accountSpec{"acct-single": account},
		accountByEmail:           map[string]*accountSpec{"single@example.com": account},
	}
	cfg := &config.Config{AuthDir: authDir}
	tracker := newRequestUsageTracker()
	manager := buildCoreAuthManager(cfg, &cockpitSelector{manifest: m}, &authHook{manifest: m}, m, nil, tracker)
	m.authManager = manager

	runtime, err := newSidecarRuntime(context.Background(), configPath, cfg, m, manager)
	if err != nil {
		t.Fatalf("newSidecarRuntime: %v", err)
	}
	defer runtime.Stop()

	firstCtx := internallogging.WithRequestID(context.Background(), "req-first")
	selected, err := manager.SelectAuth(firstCtx, "codex", "gpt-5.4", cliproxyexecutor.Options{})
	if err != nil {
		t.Fatalf("first SelectAuth: %v", err)
	}
	if selected == nil {
		t.Fatal("first SelectAuth returned no auth")
	}
	if got := tracker.accountInFlightCount(strings.ToLower(authFile)); got != 1 && tracker.accountInFlightCount(selected.ID) != 1 {
		t.Fatalf("expected one in-flight slot after first selection, got %d", got)
	}

	secondCtx := internallogging.WithRequestID(context.Background(), "req-second")
	_, err = manager.SelectAuth(secondCtx, "codex", "gpt-5.4", cliproxyexecutor.Options{})
	if err == nil {
		t.Fatal("expected the second concurrent request to be blocked by the account concurrency limit")
	}
	var authErr *coreauth.Error
	if !errors.As(err, &authErr) {
		t.Fatalf("expected coreauth.Error, got %#v", err)
	}
	if authErr.Code != "account_concurrency_exceeded" {
		t.Fatalf("unexpected error code %q (message=%s)", authErr.Code, authErr.Message)
	}
	if authErr.HTTPStatus != http.StatusTooManyRequests {
		t.Fatalf("expected 429, got %d", authErr.HTTPStatus)
	}

	// 请求结束（真实路径由中间件 defer 触发）后槽位归还，账号应重新可用。
	tracker.releaseAccountSlots("req-first")
	thirdCtx := internallogging.WithRequestID(context.Background(), "req-third")
	selectedAgain, err := manager.SelectAuth(thirdCtx, "codex", "gpt-5.4", cliproxyexecutor.Options{})
	if err != nil {
		t.Fatalf("SelectAuth after release: %v", err)
	}
	if selectedAgain == nil {
		t.Fatal("expected the account to be selectable again after the slot was released")
	}
}

// TestAccountConcurrencyGateCoversSessionAffinityHit 验证最容易被漏掉的路径：
// 会话亲和命中缓存时不会调用内层选择器，因此闸门必须挂在整条链最外层才能覆盖。
func TestAccountConcurrencyGateCoversSessionAffinityHit(t *testing.T) {
	tempDir := t.TempDir()
	authDir := filepath.Join(tempDir, "auths")
	if err := os.MkdirAll(authDir, 0o755); err != nil {
		t.Fatalf("create auth dir: %v", err)
	}
	configPath := filepath.Join(tempDir, "config.json")
	if err := os.WriteFile(configPath, []byte(`{}`), 0o644); err != nil {
		t.Fatalf("write config: %v", err)
	}
	authFile := "affinity-account.json"
	if err := os.WriteFile(filepath.Join(authDir, authFile), []byte(`{
  "type":"codex",
  "email":"affinity@example.com",
  "access_token":"affinity-token",
  "account_id":"acct-affinity"
}`), 0o600); err != nil {
		t.Fatalf("write auth file: %v", err)
	}
	secondAuthFile := "affinity-idle-account.json"
	if err := os.WriteFile(filepath.Join(authDir, secondAuthFile), []byte(`{
  "type":"codex",
  "email":"affinity-idle@example.com",
  "access_token":"affinity-idle-token",
  "account_id":"acct-affinity-idle"
}`), 0o600); err != nil {
		t.Fatalf("write second auth file: %v", err)
	}

	account := &accountSpec{
		ID:       "affinity-account",
		Email:    "affinity@example.com",
		AuthID:   authFile,
		AuthKind: "oauth",
	}
	idleAccount := &accountSpec{
		ID:       "affinity-idle-account",
		Email:    "affinity-idle@example.com",
		AuthID:   secondAuthFile,
		AuthKind: "oauth",
	}
	m := &manifest{
		Accounts:                 []accountSpec{*account, *idleAccount},
		ModelIDs:                 []string{"gpt-5.4"},
		MaxAccountConcurrency:    1,
		AccountConcurrencyWaitMs: 0,
		accountByID: map[string]*accountSpec{
			"affinity-account":      account,
			"affinity-idle-account": idleAccount,
		},
		accountByAuthID: map[string]*accountSpec{
			strings.ToLower(authFile):       account,
			strings.ToLower(secondAuthFile): idleAccount,
		},
		accountByAPIKey:  map[string]*accountSpec{},
		accountByChatGPT: map[string]*accountSpec{"acct-affinity": account},
		accountByEmail:   map[string]*accountSpec{"affinity@example.com": account},
	}
	cfg := &config.Config{AuthDir: authDir}
	cfg.Routing.SessionAffinity = true
	cfg.Routing.SessionAffinityTTL = time.Minute.String()

	tracker := newRequestUsageTracker()
	manager := buildCoreAuthManager(cfg, &cockpitSelector{manifest: m}, &authHook{manifest: m}, m, nil, tracker)
	m.authManager = manager

	runtime, err := newSidecarRuntime(context.Background(), configPath, cfg, m, manager)
	if err != nil {
		t.Fatalf("newSidecarRuntime: %v", err)
	}
	defer runtime.Stop()

	headers := make(http.Header)
	headers.Set("X-Session-ID", "affinity-session")
	opts := cliproxyexecutor.Options{Headers: headers}

	firstCtx := internallogging.WithRequestID(context.Background(), "affinity-first")
	if _, err := manager.SelectAuth(firstCtx, "codex", "gpt-5.4", opts); err != nil {
		t.Fatalf("first SelectAuth: %v", err)
	}

	// 同一个会话的第二个并发请求：命中会话亲和缓存，必须留在已绑定账号上等待/被拒绝，
	// 不能因为池里还有空闲账号就换号（换号会破坏会话粘性，也会绕过并发上限）。
	secondCtx := internallogging.WithRequestID(context.Background(), "affinity-second")
	_, err = manager.SelectAuth(secondCtx, "codex", "gpt-5.4", opts)
	if err == nil {
		t.Fatal("expected the affinity-bound session to stay on its account and be blocked, instead it was reselected")
	}
	var authErr *coreauth.Error
	if !errors.As(err, &authErr) || authErr.Code != "account_concurrency_exceeded" {
		t.Fatalf("expected account_concurrency_exceeded, got %#v", err)
	}

	tracker.releaseAccountSlots("affinity-first")
	thirdCtx := internallogging.WithRequestID(context.Background(), "affinity-third")
	if _, err := manager.SelectAuth(thirdCtx, "codex", "gpt-5.4", opts); err != nil {
		t.Fatalf("SelectAuth after release: %v", err)
	}
}
