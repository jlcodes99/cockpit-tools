package main

import (
	"context"
	"crypto/sha256"

	"encoding/json"
	"errors"

	"fmt"

	"math/rand"

	"net/http"

	"os"

	"path/filepath"

	"sort"

	"strings"
	"sync"

	"time"

	internallogging "github.com/router-for-me/CLIProxyAPI/v7/internal/logging"

	sdkauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/auth"

	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	coreusage "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/usage"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/config"
)

type cockpitSelector struct {
	manifest   *manifest
	emitter    *eventEmitter
	locale     string
	quota      *quotaReserveStateStore
	priorities *apiKeyPriorityStateStore
	tracker    *requestUsageTracker
	mu         sync.Mutex
	cursor     int
}

type recordingSelector struct {
	inner    coreauth.Selector
	manifest *manifest
	tracker  *requestUsageTracker
}

type imageRequestSelector struct {
	imageFallback coreauth.Selector
	fallback      coreauth.Selector
}

// modelExclusionSelector filters account-level model exclusions before wrappers
// such as session affinity can reuse a cached account binding.
type modelExclusionSelector struct {
	manifest *manifest
	fallback coreauth.Selector
}

func (s *imageRequestSelector) Pick(ctx context.Context, provider, model string, opts cliproxyexecutor.Options, auths []*coreauth.Auth) (*coreauth.Auth, error) {
	requestKind, _ := ctx.Value(requestKindContextKey).(string)
	if isImageRequestKind(requestKind) && s.imageFallback != nil {
		return s.imageFallback.Pick(ctx, provider, model, opts, auths)
	}
	if s.fallback == nil {
		return nil, fmt.Errorf("image request selector fallback is not initialized")
	}
	return s.fallback.Pick(ctx, provider, model, opts, auths)
}

func (s *imageRequestSelector) Stop() {
	if stoppable, ok := s.fallback.(coreauth.StoppableSelector); ok {
		stoppable.Stop()
	}
}

func (s *imageRequestSelector) ReportAuthSelectionFailure(ctx context.Context, provider, model string, candidates []*coreauth.Auth, err error) error {
	if reporter, ok := s.imageFallback.(coreauth.AuthSelectionFailureReporter); ok {
		return reporter.ReportAuthSelectionFailure(ctx, provider, model, candidates, err)
	}
	if reporter, ok := s.fallback.(coreauth.AuthSelectionFailureReporter); ok {
		return reporter.ReportAuthSelectionFailure(ctx, provider, model, candidates, err)
	}
	return err
}

func (s *modelExclusionSelector) Pick(ctx context.Context, provider, model string, opts cliproxyexecutor.Options, auths []*coreauth.Auth) (*coreauth.Auth, error) {
	if s == nil || s.fallback == nil {
		return nil, fmt.Errorf("model exclusion selector is not initialized")
	}
	if strings.TrimSpace(model) == "" {
		return s.fallback.Pick(ctx, provider, model, opts, auths)
	}
	filtered := make([]*coreauth.Auth, 0, len(auths))
	for _, auth := range auths {
		if authModelExcluded(s.manifest, auth, model) {
			continue
		}
		filtered = append(filtered, auth)
	}
	if len(filtered) == 0 {
		return nil, noAuthAvailableError(nil)
	}
	return s.fallback.Pick(ctx, provider, model, opts, filtered)
}

func (s *modelExclusionSelector) Stop() {
	if s == nil || s.fallback == nil {
		return
	}
	if stoppable, ok := s.fallback.(coreauth.StoppableSelector); ok {
		stoppable.Stop()
	}
}

func (s *modelExclusionSelector) ReportAuthSelectionFailure(ctx context.Context, provider, model string, candidates []*coreauth.Auth, err error) error {
	if reporter, ok := s.fallback.(coreauth.AuthSelectionFailureReporter); ok {
		return reporter.ReportAuthSelectionFailure(ctx, provider, model, candidates, err)
	}
	return err
}

func (s *recordingSelector) Pick(ctx context.Context, provider, model string, opts cliproxyexecutor.Options, auths []*coreauth.Auth) (*coreauth.Auth, error) {
	auth, err := s.inner.Pick(ctx, provider, model, opts, auths)
	if err != nil || auth == nil || s.tracker == nil {
		return auth, err
	}
	s.tracker.recordSelectedAccount(internallogging.GetRequestID(ctx), accountForAuthInManifest(s.manifest, auth), auth.ID)
	return auth, nil
}

func (s *recordingSelector) Stop() {
	if stoppable, ok := s.inner.(coreauth.StoppableSelector); ok {
		stoppable.Stop()
	}
}

func (s *recordingSelector) ReportAuthSelectionFailure(ctx context.Context, provider, model string, candidates []*coreauth.Auth, err error) error {
	if reporter, ok := s.inner.(coreauth.AuthSelectionFailureReporter); ok {
		return reporter.ReportAuthSelectionFailure(ctx, provider, model, candidates, err)
	}
	return err
}

type quotaReserveSelector struct {
	manifest *manifest
	fallback coreauth.Selector
	quota    *quotaReserveStateStore
}

type backupAccountSelector struct {
	manifest *manifest
	fallback coreauth.Selector
}

func quotaReserveSnapshotsFromManifest(m *manifest) map[string]quotaReserveSnapshot {
	snapshots := make(map[string]quotaReserveSnapshot)
	if m == nil {
		return snapshots
	}
	for index := range m.Accounts {
		account := &m.Accounts[index]
		if account.QuotaReserve == nil || strings.TrimSpace(account.ID) == "" {
			continue
		}
		reserve := account.QuotaReserve
		snapshots[account.ID] = quotaReserveSnapshot{
			SnapshotUpdatedAtUnixSeconds: reserve.SnapshotUpdatedAtUnixSeconds,
			HourlyRemainingPercent:       reserve.HourlyRemainingPercent,
			WeeklyRemainingPercent:       reserve.WeeklyRemainingPercent,
			HourlyWindowPresent:          reserve.HourlyWindowPresent,
			WeeklyWindowPresent:          reserve.WeeklyWindowPresent,
		}
	}
	return snapshots
}

func newQuotaReserveStateStore(path string, m *manifest) *quotaReserveStateStore {
	store := &quotaReserveStateStore{path: strings.TrimSpace(path)}
	store.snapshot.Store(quotaReserveSnapshotsFromManifest(m))
	return store
}

func (s *quotaReserveStateStore) load() error {
	if s == nil || s.path == "" {
		return nil
	}
	content, err := os.ReadFile(s.path)
	if err != nil {
		return err
	}
	hash := sha256.Sum256(content)
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.hasHash && hash == s.lastHash {
		return nil
	}
	var state quotaReserveStateFile
	if err := json.Unmarshal(content, &state); err != nil {
		return err
	}
	if state.Accounts == nil {
		state.Accounts = make(map[string]quotaReserveSnapshot)
	}
	normalized := make(map[string]quotaReserveSnapshot, len(state.Accounts))
	for accountID, snapshot := range state.Accounts {
		accountID = strings.TrimSpace(accountID)
		if accountID != "" {
			normalized[accountID] = snapshot
		}
	}
	s.snapshot.Store(normalized)
	s.lastHash = hash
	s.hasHash = true
	return nil
}

func (s *quotaReserveStateStore) start(ctx context.Context, emitter *eventEmitter) {
	if s == nil || s.path == "" {
		return
	}
	go func() {
		ticker := time.NewTicker(time.Second)
		defer ticker.Stop()
		lastError := ""
		for {
			if err := s.load(); err != nil {
				message := err.Error()
				if message != lastError && emitter != nil {
					emitter.emit(map[string]any{
						"type":    "quota_reserve_state_error",
						"message": message,
					})
				}
				lastError = message
			} else {
				lastError = ""
			}
			select {
			case <-ctx.Done():
				return
			case <-ticker.C:
			}
		}
	}()
}

func (s *quotaReserveStateStore) forAccount(accountID string) *quotaReserveSnapshot {
	if s == nil {
		return nil
	}
	loaded := s.snapshot.Load()
	snapshots, ok := loaded.(map[string]quotaReserveSnapshot)
	if !ok {
		return nil
	}
	snapshot, ok := snapshots[strings.TrimSpace(accountID)]
	if !ok {
		return nil
	}
	return &snapshot
}

func (s *quotaReserveSelector) Pick(ctx context.Context, provider, model string, opts cliproxyexecutor.Options, auths []*coreauth.Auth) (*coreauth.Auth, error) {
	if s == nil || s.fallback == nil {
		return nil, fmt.Errorf("quota reserve selector is not initialized")
	}
	if s.manifest == nil {
		return s.fallback.Pick(ctx, provider, model, opts, auths)
	}

	now := time.Now()
	var filtered []*coreauth.Auth
	quotaReserveReasons := make([]string, 0)
	availableAfterReserve := 0
	for index, auth := range auths {
		if !authAvailable(auth, model, now) {
			if filtered != nil {
				filtered = append(filtered, auth)
			}
			continue
		}
		reason := quotaReserveBlockReasonWithState(
			accountForAuthInManifest(s.manifest, auth),
			s.quota,
			now,
		)
		if reason == "" {
			availableAfterReserve++
			if filtered != nil {
				filtered = append(filtered, auth)
			}
			continue
		}
		if filtered == nil {
			filtered = append(make([]*coreauth.Auth, 0, len(auths)-1), auths[:index]...)
		}
		quotaReserveReasons = append(quotaReserveReasons, reason)
	}
	if filtered == nil {
		return s.fallback.Pick(ctx, provider, model, opts, auths)
	}
	if availableAfterReserve == 0 {
		return nil, noAuthAvailableError(quotaReserveReasons)
	}
	return s.fallback.Pick(ctx, provider, model, opts, filtered)
}

func (s *quotaReserveSelector) Stop() {
	if s == nil || s.fallback == nil {
		return
	}
	if stoppable, ok := s.fallback.(coreauth.StoppableSelector); ok {
		stoppable.Stop()
	}
}

func (s *quotaReserveSelector) ReportAuthSelectionFailure(ctx context.Context, provider, model string, candidates []*coreauth.Auth, err error) error {
	if reporter, ok := s.fallback.(coreauth.AuthSelectionFailureReporter); ok {
		return reporter.ReportAuthSelectionFailure(ctx, provider, model, candidates, err)
	}
	return err
}

func (s *backupAccountSelector) Pick(ctx context.Context, provider, model string, opts cliproxyexecutor.Options, auths []*coreauth.Auth) (*coreauth.Auth, error) {
	if s == nil || s.fallback == nil {
		return nil, fmt.Errorf("backup account selector is not initialized")
	}
	if s.manifest == nil {
		return s.fallback.Pick(ctx, provider, model, opts, auths)
	}

	now := time.Now()
	preferred := make([]*coreauth.Auth, 0)
	regular := make([]*coreauth.Auth, 0, len(auths))
	backup := make([]*coreauth.Auth, 0)
	preferredAvailable := false
	regularAvailable := false
	for _, auth := range auths {
		switch s.authUsagePriority(auth) {
		case 1:
			preferred = append(preferred, auth)
			if authAvailable(auth, model, now) {
				preferredAvailable = true
			}
		case -1:
			backup = append(backup, auth)
		default:
			regular = append(regular, auth)
			if authAvailable(auth, model, now) {
				regularAvailable = true
			}
		}
	}

	if preferredAvailable {
		return s.fallback.Pick(ctx, provider, model, opts, preferred)
	}
	if regularAvailable || len(backup) == 0 {
		return s.fallback.Pick(ctx, provider, model, opts, regular)
	}
	return s.fallback.Pick(ctx, provider, model, opts, backup)
}

func (s *backupAccountSelector) authUsagePriority(auth *coreauth.Auth) int {
	account := accountForAuthInManifest(s.manifest, auth)
	if account == nil {
		return 0
	}
	for _, rule := range s.manifest.CustomRoutingRules {
		if rule.AccountID == account.ID {
			if rule.IsPreferred {
				return 1
			}
			if rule.IsBackup {
				return -1
			}
			return 0
		}
	}
	return 0
}

func (s *backupAccountSelector) Stop() {
	if s == nil || s.fallback == nil {
		return
	}
	if stoppable, ok := s.fallback.(coreauth.StoppableSelector); ok {
		stoppable.Stop()
	}
}

func (s *backupAccountSelector) ReportAuthSelectionFailure(ctx context.Context, provider, model string, candidates []*coreauth.Auth, err error) error {
	if reporter, ok := s.fallback.(coreauth.AuthSelectionFailureReporter); ok {
		return reporter.ReportAuthSelectionFailure(ctx, provider, model, candidates, err)
	}
	return err
}

func (s *cockpitSelector) Pick(ctx context.Context, provider, model string, opts cliproxyexecutor.Options, auths []*coreauth.Auth) (*coreauth.Auth, error) {
	_ = opts
	selectionStats := authPoolSelectionStats{candidateAuths: len(auths)}
	auths = s.filterAuthsForAPIKeyScope(ctx, auths)
	selectionStats.scopedAuths = len(auths)
	requestKind, _ := ctx.Value(requestKindContextKey).(string)
	if isImageRequestKind(requestKind) {
		beforeImagePolicy := len(auths)
		beforeImageAuths := append([]*coreauth.Auth(nil), auths...)
		auths = s.filterAuthsForImageGeneration(auths)
		selectionStats.imagePolicyBlockedAuths = beforeImagePolicy - len(auths)
		allowed := make(map[*coreauth.Auth]struct{}, len(auths))
		for _, auth := range auths {
			allowed[auth] = struct{}{}
		}
		for _, auth := range beforeImageAuths {
			if _, ok := allowed[auth]; !ok {
				selectionStats.members = append(selectionStats.members, poolMemberDiagnostic(auth, s.accountForAuth(auth), false, "image_policy_blocked", "image generation is disabled for this account"))
			}
		}
		if len(auths) == 0 {
			err := authPoolUnavailableError(s.locale, selectionStats, "image generation is disabled for all selected accounts")
			s.emitAuthPoolUnavailable(ctx, provider, model, selectionStats, err)
			return nil, err
		}
	}
	now := time.Now()
	available := make([]*coreauth.Auth, 0, len(auths))
	quotaReserveReasons := make([]string, 0)
	for _, auth := range auths {
		if !authAvailable(auth, model, now) {
			selectionStats.unavailableAuths++
			reasonCode, reasonMessage := unavailableAuthDiagnostic(auth, model, now)
			selectionStats.members = append(selectionStats.members, poolMemberDiagnostic(auth, s.accountForAuth(auth), false, reasonCode, reasonMessage))
			continue
		}
		if authModelExcluded(s.manifest, auth, model) {
			selectionStats.modelExcludedAuths++
			selectionStats.members = append(selectionStats.members, poolMemberDiagnostic(auth, s.accountForAuth(auth), false, "model_excluded", "model is excluded for this account"))
			continue
		}
		if reason := quotaReserveBlockReasonWithState(s.accountForAuth(auth), s.quota, now); reason != "" {
			selectionStats.quotaReservedAuths++
			quotaReserveReasons = append(quotaReserveReasons, reason)
			selectionStats.members = append(selectionStats.members, poolMemberDiagnostic(auth, s.accountForAuth(auth), false, "quota_reserved", reason))
			continue
		}
		available = append(available, auth)
		selectionStats.members = append(selectionStats.members, poolMemberDiagnostic(auth, s.accountForAuth(auth), true, "available", "account is eligible for routing"))
	}
	selectionStats.availableAuths = len(available)
	if len(available) == 0 {
		err := authPoolUnavailableError(s.locale, selectionStats, noAuthAvailableError(quotaReserveReasons).Error())
		s.emitAuthPoolUnavailable(ctx, provider, model, selectionStats, err)
		return nil, err
	}

	s.mu.Lock()
	start := s.cursor
	s.cursor++
	s.mu.Unlock()

	ordered := s.orderAuths(available, start)
	if isImageRequestKind(requestKind) {
		ordered = s.orderImageAuths(available, start)
	} else {
		ordered = s.prioritizeAuthsForAPIKey(ctx, ordered)
	}
	if len(ordered) == 0 {
		err := authPoolUnavailableError(s.locale, selectionStats, noAuthAvailableError(quotaReserveReasons).Error())
		s.emitAuthPoolUnavailable(ctx, provider, model, selectionStats, err)
		return nil, err
	}
	if isImageRequestKind(requestKind) && s.tracker != nil {
		requestID := internallogging.GetRequestID(ctx)
		for {
			changed := s.tracker.imageJobChangeSignal()
			for _, candidate := range s.orderImageAuths(available, start) {
				if s.tracker.tryReserveImageJob(requestID, candidate.ID, s.maxConcurrentImageRequests()) {
					s.emitAuthSelected(ctx, candidate, provider, model, len(auths), len(available))
					return candidate, nil
				}
			}
			select {
			case <-ctx.Done():
				return nil, ctx.Err()
			case <-changed:
			}
		}
	}
	selected := ordered[0]
	s.emitAuthSelected(ctx, selected, provider, model, len(auths), len(available))
	return selected, nil
}

// ReportAuthSelectionFailure handles failures raised by the manager's
// availability pass, which runs before cockpitSelector.Pick. It intentionally
// reuses the same account-level policy checks as Pick so the UI receives a
// useful diagnostic for the real request path as well.
func (s *cockpitSelector) ReportAuthSelectionFailure(ctx context.Context, provider, model string, candidates []*coreauth.Auth, err error) error {
	if s == nil {
		return err
	}
	if ctx == nil {
		ctx = context.Background()
	}
	stats := authPoolSelectionStats{candidateAuths: len(candidates)}
	auths := s.filterAuthsForAPIKeyScope(ctx, candidates)
	stats.scopedAuths = len(auths)
	requestKind, _ := ctx.Value(requestKindContextKey).(string)
	if isImageRequestKind(requestKind) {
		before := len(auths)
		beforeImage := append([]*coreauth.Auth(nil), auths...)
		auths = s.filterAuthsForImageGeneration(auths)
		stats.imagePolicyBlockedAuths = before - len(auths)
		allowed := make(map[*coreauth.Auth]struct{}, len(auths))
		for _, auth := range auths {
			allowed[auth] = struct{}{}
		}
		for _, auth := range beforeImage {
			if _, ok := allowed[auth]; !ok {
				stats.members = append(stats.members, poolMemberDiagnostic(auth, s.accountForAuth(auth), false, "image_policy_blocked", "image generation is disabled for this account"))
			}
		}
	}
	now := time.Now()
	for _, auth := range auths {
		if !authAvailable(auth, model, now) {
			stats.unavailableAuths++
			code, message := unavailableAuthDiagnostic(auth, model, now)
			stats.members = append(stats.members, poolMemberDiagnostic(auth, s.accountForAuth(auth), false, code, message))
			continue
		}
		if authModelExcluded(s.manifest, auth, model) {
			stats.modelExcludedAuths++
			stats.members = append(stats.members, poolMemberDiagnostic(auth, s.accountForAuth(auth), false, "model_excluded", "model is excluded for this account"))
			continue
		}
		if reason := quotaReserveBlockReasonWithState(s.accountForAuth(auth), s.quota, now); reason != "" {
			stats.quotaReservedAuths++
			stats.members = append(stats.members, poolMemberDiagnostic(auth, s.accountForAuth(auth), false, "quota_reserved", reason))
			continue
		}
		stats.availableAuths++
		stats.members = append(stats.members, poolMemberDiagnostic(auth, s.accountForAuth(auth), true, "available", "account passed Cockpit policy checks; manager availability rejected it"))
	}
	// The manager may reject a candidate because of runtime cooldown state that
	// is not represented in Cockpit's policy layer. Do not claim success for
	// those entries; attach the manager's reason to make the discrepancy clear.
	if stats.availableAuths > 0 && len(stats.members) > 0 {
		managerReason := "manager availability check rejected this account"
		if err != nil && strings.TrimSpace(err.Error()) != "" {
			managerReason = strings.TrimSpace(err.Error())
		}
		for index := range stats.members {
			member := &stats.members[index]
			if member.Available {
				member.Available = false
				member.ReasonCode = "manager_unavailable"
				member.ReasonMessage = managerReason
				stats.unavailableAuths++
				stats.availableAuths--
			}
		}
	}
	detail := "no auth available"
	if err != nil && strings.TrimSpace(err.Error()) != "" {
		detail = strings.TrimSpace(err.Error())
	}
	diagnosticErr := authPoolUnavailableError(s.locale, stats, detail)
	var authErr *coreauth.Error
	if errors.As(err, &authErr) && authErr != nil && strings.TrimSpace(authErr.Code) != "" {
		diagnosticErr.Code = authErr.Code
	}
	s.emitAuthPoolUnavailable(ctx, provider, model, stats, diagnosticErr)
	return diagnosticErr
}

// filterAuthsForImageGeneration enforces the member-level policy after API Key
// scope filtering and before quota/round-robin selection. This lets an image
// request fall through to another allowed account instead of reaching upstream
// with an account that explicitly disabled image_generation.
func (s *cockpitSelector) filterAuthsForImageGeneration(auths []*coreauth.Auth) []*coreauth.Auth {
	if s == nil || s.manifest == nil {
		return auths
	}
	filtered := make([]*coreauth.Auth, 0, len(auths))
	for _, auth := range auths {
		account := s.accountForAuth(auth)
		if account == nil || imageGenerationAllowedForAccount(account) {
			filtered = append(filtered, auth)
		}
	}
	return filtered
}

func imageGenerationAllowedForAccount(account *accountSpec) bool {
	if account == nil {
		return true
	}
	policy := strings.ToLower(strings.TrimSpace(account.ImageGenerationPolicy))
	if policy == "disabled" {
		return false
	}
	if strings.EqualFold(account.AuthKind, "api_key") {
		return policy == "enabled"
	}
	// Free OAuth cannot gain hosted image_generation through a local override.
	if strings.Contains(strings.ToLower(strings.TrimSpace(account.PlanType)), "free") {
		return false
	}
	return true
}

func (s *cockpitSelector) prioritizeAuthsForAPIKey(ctx context.Context, auths []*coreauth.Auth) []*coreauth.Auth {
	if s == nil || ctx == nil || len(auths) <= 1 || s.priorities == nil {
		return auths
	}
	spec, _ := ctx.Value(clientAPIKeyContextKey).(*apiKeySpec)
	if spec == nil {
		return auths
	}
	priorityAccountIDs := s.priorities.priorityAccountIDs(spec.ID)
	if len(priorityAccountIDs) == 0 {
		return auths
	}

	ordered := make([]*coreauth.Auth, 0, len(auths))
	selected := make(map[*coreauth.Auth]struct{}, len(priorityAccountIDs))
	for _, priorityAccountID := range priorityAccountIDs {
		for _, auth := range auths {
			account := s.accountForAuth(auth)
			if account == nil || account.ID != priorityAccountID {
				continue
			}
			if _, alreadySelected := selected[auth]; alreadySelected {
				break
			}
			ordered = append(ordered, auth)
			selected[auth] = struct{}{}
			break
		}
	}
	if len(ordered) == 0 {
		return auths
	}
	for _, auth := range auths {
		if _, alreadySelected := selected[auth]; !alreadySelected {
			ordered = append(ordered, auth)
		}
	}
	return ordered
}

func (s *cockpitSelector) filterAuthsForAPIKeyScope(ctx context.Context, auths []*coreauth.Auth) []*coreauth.Auth {
	if s == nil || s.manifest == nil || ctx == nil {
		return auths
	}
	spec, _ := ctx.Value(clientAPIKeyContextKey).(*apiKeySpec)
	if spec == nil || len(spec.AccountIDs) == 0 {
		return auths
	}

	allowedAccountIDs := make(map[string]struct{}, len(spec.AccountIDs))
	for _, accountID := range spec.AccountIDs {
		if accountID = strings.TrimSpace(accountID); accountID != "" {
			allowedAccountIDs[accountID] = struct{}{}
		}
	}
	if len(allowedAccountIDs) == 0 {
		return nil
	}

	scoped := make([]*coreauth.Auth, 0, len(auths))
	for _, auth := range auths {
		account := s.accountForAuth(auth)
		if account == nil {
			continue
		}
		if _, allowed := allowedAccountIDs[account.ID]; allowed {
			scoped = append(scoped, auth)
		}
	}
	return scoped
}

func (s *cockpitSelector) maxConcurrentImageRequests() int {
	if s == nil || s.manifest == nil || s.manifest.MaxConcurrentImageRequests < 1 {
		return 1
	}
	return s.manifest.MaxConcurrentImageRequests
}

func isImageRequestKind(requestKind string) bool {
	switch strings.TrimSpace(requestKind) {
	case "image_generation", "image_edit":
		return true
	default:
		return false
	}
}

func (s *cockpitSelector) orderImageAuths(auths []*coreauth.Auth, start int) []*coreauth.Auth {
	if len(auths) <= 1 || s == nil || s.tracker == nil {
		return s.orderAuths(auths, start)
	}
	out := append([]*coreauth.Auth(nil), auths...)
	sort.SliceStable(out, func(i, j int) bool {
		leftJobs := s.tracker.imageInFlightCount(out[i].ID)
		rightJobs := s.tracker.imageInFlightCount(out[j].ID)
		if leftJobs != rightJobs {
			return leftJobs < rightJobs
		}
		return s.rotatedIndex(s.accountForAuth(out[i]), start) < s.rotatedIndex(s.accountForAuth(out[j]), start)
	})
	return out
}

func quotaReserveBlockReason(account *accountSpec, now time.Time) string {
	return quotaReserveBlockReasonWithSnapshot(account, quotaReserveSnapshotFromSpec(account), now)
}

func quotaReserveBlockReasonWithState(account *accountSpec, state *quotaReserveStateStore, now time.Time) string {
	var snapshot *quotaReserveSnapshot
	if account != nil && state != nil {
		snapshot = state.forAccount(account.ID)
	}
	if snapshot == nil {
		snapshot = quotaReserveSnapshotFromSpec(account)
	}
	return quotaReserveBlockReasonWithSnapshot(account, snapshot, now)
}

func quotaReserveSnapshotFromSpec(account *accountSpec) *quotaReserveSnapshot {
	if account == nil || account.QuotaReserve == nil {
		return nil
	}
	reserve := account.QuotaReserve
	return &quotaReserveSnapshot{
		SnapshotUpdatedAtUnixSeconds: reserve.SnapshotUpdatedAtUnixSeconds,
		HourlyRemainingPercent:       reserve.HourlyRemainingPercent,
		WeeklyRemainingPercent:       reserve.WeeklyRemainingPercent,
		HourlyWindowPresent:          reserve.HourlyWindowPresent,
		WeeklyWindowPresent:          reserve.WeeklyWindowPresent,
	}
}

func quotaReserveBlockReasonWithSnapshot(account *accountSpec, snapshot *quotaReserveSnapshot, now time.Time) string {
	if account == nil || account.QuotaReserve == nil {
		return ""
	}

	reserve := account.QuotaReserve
	if snapshot == nil {
		return quotaReserveAccountReason(account, []string{"quota snapshot unknown"})
	}
	if reason := quotaReserveSnapshotBlockReason(snapshot.SnapshotUpdatedAtUnixSeconds, now); reason != "" {
		return quotaReserveAccountReason(account, []string{reason})
	}

	reasons := make([]string, 0, 2)
	if reason := quotaReserveWindowBlockReason(
		"5h",
		reserve.HourlyThresholdPercent,
		snapshot.HourlyRemainingPercent,
		snapshot.HourlyWindowPresent,
	); reason != "" {
		reasons = append(reasons, reason)
	}
	if reason := quotaReserveWindowBlockReason(
		"weekly",
		reserve.WeeklyThresholdPercent,
		snapshot.WeeklyRemainingPercent,
		snapshot.WeeklyWindowPresent,
	); reason != "" {
		reasons = append(reasons, reason)
	}
	if len(reasons) == 0 {
		return ""
	}
	return quotaReserveAccountReason(account, reasons)
}

func quotaReserveSnapshotBlockReason(updatedAt *int64, now time.Time) string {
	if updatedAt == nil {
		return "quota snapshot timestamp unknown"
	}

	nowUnix := now.Unix()
	if *updatedAt <= 0 || *updatedAt > nowUnix {
		return "quota snapshot timestamp invalid"
	}
	if nowUnix-*updatedAt > int64(quotaReserveMaxSnapshotAge/time.Second) {
		return "quota snapshot stale"
	}
	return ""
}

func quotaReserveAccountReason(account *accountSpec, reasons []string) string {
	accountLabel := strings.TrimSpace(account.Email)
	if accountLabel == "" {
		accountLabel = strings.TrimSpace(account.ID)
	}
	if accountLabel == "" {
		accountLabel = "unknown account"
	}
	return fmt.Sprintf("%s (%s)", accountLabel, strings.Join(reasons, ", "))
}

func quotaReserveWindowBlockReason(window string, threshold, remaining *int, present *bool) string {
	if present != nil && !*present {
		return ""
	}
	if threshold == nil || *threshold < 1 || *threshold > 100 {
		return fmt.Sprintf("%s reserve threshold unknown", window)
	}
	if remaining == nil || *remaining < 0 || *remaining > 100 {
		return fmt.Sprintf("%s remaining quota unknown; reserve %d%%", window, *threshold)
	}
	if *remaining <= *threshold {
		return fmt.Sprintf("%s remaining %d%% <= reserve %d%%", window, *remaining, *threshold)
	}
	return ""
}

func noAuthAvailableError(quotaReserveReasons []string) error {
	if len(quotaReserveReasons) == 0 {
		return fmt.Errorf("no auth available")
	}

	const maxReasons = 3
	reasons := quotaReserveReasons
	if len(reasons) > maxReasons {
		reasons = reasons[:maxReasons]
	}
	detail := strings.Join(reasons, "; ")
	if omitted := len(quotaReserveReasons) - len(reasons); omitted > 0 {
		detail = fmt.Sprintf("%s; and %d more", detail, omitted)
	}
	return fmt.Errorf(
		"no auth available: bound OAuth quota reserve blocked %d auth(s): %s",
		len(quotaReserveReasons),
		detail,
	)
}

func authAvailable(auth *coreauth.Auth, model string, now time.Time) bool {
	if auth == nil || auth.Disabled || auth.Status == coreauth.StatusDisabled {
		return false
	}
	if model != "" && len(auth.ModelStates) > 0 {
		state := auth.ModelStates[model]
		if state == nil {
			state = auth.ModelStates[resolveBaseModelKey(model)]
		}
		if state != nil {
			if state.Status == coreauth.StatusDisabled {
				return false
			}
			if runtimeAvailabilityBlocked(state.Unavailable, state.Quota.Exceeded, state.NextRetryAfter, state.Quota.NextRecoverAt, now) {
				return false
			}
		}
	}
	if runtimeAvailabilityBlocked(auth.Unavailable, auth.Quota.Exceeded, auth.NextRetryAfter, auth.Quota.NextRecoverAt, now) {
		return false
	}
	return true
}

func runtimeAvailabilityBlocked(unavailable, quotaExceeded bool, nextRetryAfter, nextRecoverAt, now time.Time) bool {
	if !unavailable && !quotaExceeded {
		return false
	}
	hasRecoveryTime := !nextRetryAfter.IsZero() || !nextRecoverAt.IsZero()
	if nextRetryAfter.After(now) || nextRecoverAt.After(now) {
		return true
	}
	return !hasRecoveryTime
}

func resolveBaseModelKey(model string) string {
	model = strings.TrimSpace(model)
	for i := len(model) - 1; i >= 0; i-- {
		if model[i] == '-' && i+len("-2006-01-02") == len(model) && hasDateSnapshotSuffix(model[i:]) {
			return model[:i]
		}
	}
	return model
}

func (s *cockpitSelector) orderAuths(auths []*coreauth.Auth, start int) []*coreauth.Auth {
	if len(auths) <= 1 || s == nil || s.manifest == nil {
		return auths
	}
	strategy := strings.TrimSpace(strings.ToLower(s.manifest.RoutingStrategy))
	if strategy == "random" {
		out := append([]*coreauth.Auth(nil), auths...)
		rand.Shuffle(len(out), func(i, j int) {
			out[i], out[j] = out[j], out[i]
		})
		return out
	}
	if strategy == "custom" {
		return s.orderCustom(auths, start)
	}
	out := append([]*coreauth.Auth(nil), auths...)
	sort.SliceStable(out, func(i, j int) bool {
		left := s.accountForAuth(out[i])
		right := s.accountForAuth(out[j])
		if compareAccountSpecs(left, right, strategy) != 0 {
			return compareAccountSpecs(left, right, strategy) < 0
		}
		return s.rotatedIndex(left, start) < s.rotatedIndex(right, start)
	})
	return out
}

func compareAccountSpecs(left, right *accountSpec, strategy string) int {
	switch strategy {
	case "quota_high_first":
		if cmp := compareIntPtrDesc(valueInt(left, "quota"), valueInt(right, "quota")); cmp != 0 {
			return cmp
		}
		return compareIntPtrDesc(valueInt(left, "plan"), valueInt(right, "plan"))
	case "quota_low_first":
		if cmp := compareIntPtrAsc(valueInt(left, "quota"), valueInt(right, "quota")); cmp != 0 {
			return cmp
		}
		return compareIntPtrDesc(valueInt(left, "plan"), valueInt(right, "plan"))
	case "plan_low_first":
		if cmp := compareIntPtrAsc(valueInt(left, "plan"), valueInt(right, "plan")); cmp != 0 {
			return cmp
		}
		return compareIntPtrDesc(valueInt(left, "quota"), valueInt(right, "quota"))
	case "expiry_soon_first":
		if cmp := compareInt64PtrAsc(valueInt64(left), valueInt64(right)); cmp != 0 {
			return cmp
		}
		if cmp := compareIntPtrDesc(valueInt(left, "plan"), valueInt(right, "plan")); cmp != 0 {
			return cmp
		}
		return compareIntPtrDesc(valueInt(left, "quota"), valueInt(right, "quota"))
	case "plan_high_first":
		fallthrough
	case "auto":
		fallthrough
	default:
		if cmp := compareIntPtrDesc(valueInt(left, "plan"), valueInt(right, "plan")); cmp != 0 {
			return cmp
		}
		return compareIntPtrDesc(valueInt(left, "quota"), valueInt(right, "quota"))
	}
}

func valueInt(account *accountSpec, kind string) *int {
	if account == nil {
		return nil
	}
	if kind == "quota" {
		return account.RemainingQuota
	}
	return account.PlanRank
}

func valueInt64(account *accountSpec) *int64 {
	if account == nil {
		return nil
	}
	return account.SubscriptionExpiryMS
}

func compareIntPtrDesc(left, right *int) int {
	switch {
	case left != nil && right != nil:
		return *right - *left
	case left != nil:
		return -1
	case right != nil:
		return 1
	default:
		return 0
	}
}

func compareIntPtrAsc(left, right *int) int {
	switch {
	case left != nil && right != nil:
		return *left - *right
	case left != nil:
		return -1
	case right != nil:
		return 1
	default:
		return 0
	}
}

func compareInt64PtrAsc(left, right *int64) int {
	switch {
	case left != nil && right != nil:
		if *left < *right {
			return -1
		}
		if *left > *right {
			return 1
		}
		return 0
	case left != nil:
		return -1
	case right != nil:
		return 1
	default:
		return 0
	}
}

func (s *cockpitSelector) orderCustom(auths []*coreauth.Auth, start int) []*coreauth.Auth {
	rules := make(map[string]customRoutingRule)
	for _, rule := range s.manifest.CustomRoutingRules {
		if strings.TrimSpace(rule.AccountID) == "" {
			continue
		}
		if rule.Weight <= 0 {
			rule.Weight = 1
		}
		rules[rule.AccountID] = rule
	}
	groups := make(map[int][]*coreauth.Auth)
	priorities := make([]int, 0)
	seenPriority := make(map[int]struct{})
	for _, auth := range auths {
		account := s.accountForAuth(auth)
		priority := 0
		if account != nil {
			priority = rules[account.ID].Priority
		}
		groups[priority] = append(groups[priority], auth)
		if _, ok := seenPriority[priority]; !ok {
			seenPriority[priority] = struct{}{}
			priorities = append(priorities, priority)
		}
	}
	sort.Sort(sort.Reverse(sort.IntSlice(priorities)))
	out := make([]*coreauth.Auth, 0, len(auths))
	for _, priority := range priorities {
		group := groups[priority]
		out = append(out, weightedOrder(group, rules, s, start)...)
	}
	return out
}

func weightedOrder(group []*coreauth.Auth, rules map[string]customRoutingRule, selector *cockpitSelector, start int) []*coreauth.Auth {
	if len(group) <= 1 {
		return group
	}
	total := 0
	weights := make([]int, len(group))
	for i, auth := range group {
		weight := 1
		if account := selector.accountForAuth(auth); account != nil {
			if rule, ok := rules[account.ID]; ok && rule.Weight > 0 {
				weight = rule.Weight
			}
		}
		weights[i] = weight
		total += weight
	}
	slot := start % total
	first := 0
	for i, weight := range weights {
		if slot < weight {
			first = i
			break
		}
		slot -= weight
	}
	out := make([]*coreauth.Auth, 0, len(group))
	for offset := 0; offset < len(group); offset++ {
		out = append(out, group[(first+offset)%len(group)])
	}
	return out
}

func (s *cockpitSelector) accountForAuth(auth *coreauth.Auth) *accountSpec {
	if s == nil {
		return nil
	}
	return accountForAuthInManifest(s.manifest, auth)
}

func accountForAuthInManifest(m *manifest, auth *coreauth.Auth) *accountSpec {
	if m == nil || auth == nil {
		return nil
	}
	if auth.ID != "" {
		if account := m.accountByAuthID[strings.ToLower(auth.ID)]; account != nil {
			return account
		}
		base := strings.TrimSuffix(filepath.Base(auth.ID), filepath.Ext(auth.ID))
		if account := m.accountByID[base]; account != nil {
			return account
		}
	}
	if auth.Attributes != nil {
		if key := strings.TrimSpace(auth.Attributes["api_key"]); key != "" {
			return m.accountByAPIKey[key]
		}
	}
	return nil
}

func (s *cockpitSelector) emitAuthSelected(ctx context.Context, auth *coreauth.Auth, provider, model string, candidateAuths, availableAuths int) {
	if s == nil || s.emitter == nil || auth == nil {
		return
	}
	if ctx == nil {
		ctx = context.Background()
	}
	spec, _ := ctx.Value(clientAPIKeyContextKey).(*apiKeySpec)
	requestKind, _ := ctx.Value(requestKindContextKey).(string)
	if requestKind == "" {
		requestKind = requestKindFromPath(internallogging.GetEndpoint(ctx))
	}
	requestModel, _ := ctx.Value(requestModelContextKey).(string)
	if strings.TrimSpace(requestModel) != "" {
		model = requestModel
	}
	account := s.accountForAuth(auth)
	routingStrategy := ""
	if s.manifest != nil {
		routingStrategy = strings.TrimSpace(s.manifest.RoutingStrategy)
	}
	s.emitter.emit(requestDiagnosticPayload{
		Type:            "auth_selected",
		RequestID:       internallogging.GetRequestID(ctx),
		RequestKind:     requestKind,
		Model:           model,
		APIKeyID:        stringFromAPIKey(spec, "id"),
		APIKeyLabel:     stringFromAPIKey(spec, "label"),
		CandidateAuths:  candidateAuths,
		AvailableAuths:  availableAuths,
		RoutingStrategy: routingStrategy,
		Provider:        provider,
		AuthID:          auth.ID,
		AccountID:       stringFromAccount(account, "id"),
		AccountEmail:    stringFromAccount(account, "email"),
	})
}

// authPoolSelectionStats explains why a request could not reach an account.
// These are pool-level counters: they must not be attributed to a specific OAuth account.
type authPoolSelectionStats struct {
	candidateAuths          int
	scopedAuths             int
	availableAuths          int
	unavailableAuths        int
	modelExcludedAuths      int
	quotaReservedAuths      int
	imagePolicyBlockedAuths int
	members                 []authPoolMemberDiagnostic
}

// authPoolMemberDiagnostic contains the selector's actual per-account decision.
type authPoolMemberDiagnostic struct {
	AccountID     string `json:"accountId"`
	AccountEmail  string `json:"accountEmail,omitempty"`
	Available     bool   `json:"available"`
	ReasonCode    string `json:"reasonCode"`
	ReasonMessage string `json:"reasonMessage"`
}

func poolMemberDiagnostic(auth *coreauth.Auth, account *accountSpec, available bool, code, message string) authPoolMemberDiagnostic {
	item := authPoolMemberDiagnostic{Available: available, ReasonCode: code, ReasonMessage: message}
	if account != nil {
		item.AccountID = strings.TrimSpace(account.ID)
		item.AccountEmail = strings.TrimSpace(account.Email)
	}
	if item.AccountID == "" && auth != nil {
		item.AccountID = strings.TrimSpace(auth.ID)
	}
	return item
}

func authPoolUnavailableError(locale string, stats authPoolSelectionStats, detail string) *coreauth.Error {
	detail = strings.TrimSpace(detail)
	if detail == "" || detail == "no auth available" {
		detail = "no auth available"
	}
	chinese := strings.HasPrefix(strings.ToLower(strings.TrimSpace(locale)), "zh")
	var message string
	if chinese {
		message = fmt.Sprintf("账号池没有可用账号：候选 %d 个，不可用 %d 个，模型排除 %d 个，额度保留拦截 %d 个，生图策略拦截 %d 个。请前往 Cockpit Tools 查看账号池诊断详情。",
			stats.candidateAuths, stats.unavailableAuths, stats.modelExcludedAuths, stats.quotaReservedAuths, stats.imagePolicyBlockedAuths)
	} else {
		message = fmt.Sprintf("No available account: candidates=%d, unavailable=%d, model_excluded=%d, quota_reserved=%d, image_policy_blocked=%d. Check account pool diagnostics in Cockpit Tools.",
			stats.candidateAuths, stats.unavailableAuths, stats.modelExcludedAuths, stats.quotaReservedAuths, stats.imagePolicyBlockedAuths)
	}
	if detail != "no auth available" && !chinese {
		message += " " + detail
	}
	return &coreauth.Error{Code: "auth_unavailable", Message: message, HTTPStatus: http.StatusServiceUnavailable, Retryable: true}
}

// emitAuthPoolUnavailable reports selection failures that happen before an auth is chosen.
// The host stores this separately from account health so the UI can show a pool issue
// even though the event correctly has no accountId.
func (s *cockpitSelector) emitAuthPoolUnavailable(
	ctx context.Context,
	provider string,
	model string,
	stats authPoolSelectionStats,
	err error,
) {
	if s == nil || s.emitter == nil {
		return
	}
	if ctx == nil {
		ctx = context.Background()
	}
	spec, _ := ctx.Value(clientAPIKeyContextKey).(*apiKeySpec)
	requestKind, _ := ctx.Value(requestKindContextKey).(string)
	if requestKind == "" {
		requestKind = requestKindFromPath(internallogging.GetEndpoint(ctx))
	}
	if requestModel, _ := ctx.Value(requestModelContextKey).(string); strings.TrimSpace(requestModel) != "" {
		model = strings.TrimSpace(requestModel)
	}
	errorMessage := "no auth available"
	if err != nil && strings.TrimSpace(err.Error()) != "" {
		errorMessage = strings.TrimSpace(err.Error())
	}
	errorCode := "auth_unavailable"
	var authErr *coreauth.Error
	if errors.As(err, &authErr) && authErr != nil && strings.TrimSpace(authErr.Code) != "" {
		errorCode = strings.TrimSpace(authErr.Code)
	}
	s.emitter.emit(requestDiagnosticPayload{
		Type:                    "auth_pool_result",
		RequestID:               internallogging.GetRequestID(ctx),
		RequestKind:             requestKind,
		Model:                   model,
		APIKeyID:                stringFromAPIKey(spec, "id"),
		APIKeyLabel:             stringFromAPIKey(spec, "label"),
		Provider:                provider,
		ErrorCode:               errorCode,
		ErrorMessage:            errorMessage,
		CandidateAuths:          stats.candidateAuths,
		ScopedAuths:             stats.scopedAuths,
		AvailableAuths:          stats.availableAuths,
		UnavailableAuths:        stats.unavailableAuths,
		ModelExcludedAuths:      stats.modelExcludedAuths,
		QuotaReservedAuths:      stats.quotaReservedAuths,
		ImagePolicyBlockedAuths: stats.imagePolicyBlockedAuths,
		AccountStatuses:         stats.members,
	})
}

func unavailableAuthDiagnostic(auth *coreauth.Auth, model string, now time.Time) (string, string) {
	if auth == nil {
		return "auth_missing", "auth record is missing"
	}
	if auth.Disabled || auth.Status == coreauth.StatusDisabled {
		return "disabled", "account is disabled"
	}
	if model != "" && len(auth.ModelStates) > 0 {
		state := auth.ModelStates[model]
		if state == nil {
			state = auth.ModelStates[resolveBaseModelKey(model)]
		}
		if state != nil {
			if state.Status == coreauth.StatusDisabled {
				return "model_disabled", "model is disabled for this account"
			}
			if runtimeAvailabilityBlocked(state.Unavailable, state.Quota.Exceeded, state.NextRetryAfter, state.Quota.NextRecoverAt, now) {
				if !state.Quota.NextRecoverAt.IsZero() && state.Quota.NextRecoverAt.After(now) {
					return "model_quota_cooldown", fmt.Sprintf("model quota cooldown until %s", state.Quota.NextRecoverAt.UTC().Format(time.RFC3339))
				}
				if !state.NextRetryAfter.IsZero() && state.NextRetryAfter.After(now) {
					return "model_cooldown", fmt.Sprintf("model cooldown until %s", state.NextRetryAfter.UTC().Format(time.RFC3339))
				}
				if strings.TrimSpace(state.Quota.Reason) != "" {
					return "model_quota_exceeded", state.Quota.Reason
				}
				return "model_unavailable", "model is temporarily unavailable"
			}
		}
	}
	if auth.Unavailable && !auth.NextRetryAfter.IsZero() && auth.NextRetryAfter.After(now) {
		return "account_cooldown", fmt.Sprintf("account cooldown until %s", auth.NextRetryAfter.UTC().Format(time.RFC3339))
	}
	if auth.Quota.Exceeded {
		if !auth.Quota.NextRecoverAt.IsZero() && auth.Quota.NextRecoverAt.After(now) {
			return "quota_cooldown", fmt.Sprintf("quota cooldown until %s", auth.Quota.NextRecoverAt.UTC().Format(time.RFC3339))
		}
		if strings.TrimSpace(auth.Quota.Reason) != "" {
			return "quota_exceeded", auth.Quota.Reason
		}
		return "quota_exceeded", "account quota is unavailable"
	}
	if auth.LastError != nil && auth.LastError.Message != "" {
		code := strings.TrimSpace(auth.LastError.Code)
		if code == "" {
			code = "auth_error"
		}
		return code, auth.LastError.Message
	}
	if auth.StatusMessage != "" {
		return "unavailable", auth.StatusMessage
	}
	return "unavailable", "account is temporarily unavailable"
}

func (s *cockpitSelector) rotatedIndex(account *accountSpec, start int) int {
	if s == nil || s.manifest == nil || account == nil {
		return 1 << 30
	}
	index, ok := s.manifest.originalIndexByID[account.ID]
	if !ok || len(s.manifest.Accounts) == 0 {
		return 1 << 30
	}
	total := len(s.manifest.Accounts)
	return (index - (start % total) + total) % total
}

type usagePlugin struct {
	manifest *manifest
	tracker  *requestUsageTracker
}

func (p *usagePlugin) HandleUsage(ctx context.Context, record coreusage.Record) {
	if p == nil || p.tracker == nil {
		return
	}
	if ctx == nil {
		ctx = context.Background()
	}
	spec, _ := ctx.Value(clientAPIKeyContextKey).(*apiKeySpec)
	if spec == nil && p.manifest != nil && strings.TrimSpace(record.APIKey) != "" {
		spec = p.manifest.apiKeyByValue[strings.TrimSpace(record.APIKey)]
	}
	account := p.accountForRecord(record)
	requestKind, _ := ctx.Value(requestKindContextKey).(string)
	if strings.TrimSpace(requestKind) == "" {
		requestKind = requestKindFromPath(internallogging.GetEndpoint(ctx))
	}
	if strings.TrimSpace(requestKind) == "" {
		requestKind = "other"
	}
	requestModel, _ := ctx.Value(requestModelContextKey).(string)
	model := strings.TrimSpace(record.Model)
	if model == "" {
		model = strings.TrimSpace(requestModel)
	}
	alias := strings.TrimSpace(record.Alias)
	if alias == "" {
		alias = strings.TrimSpace(requestModel)
	}
	status := record.Fail.StatusCode
	success := !record.Failed
	p.tracker.record(usagePayload{
		Type:             "usage",
		RequestID:        internallogging.GetRequestID(ctx),
		Provider:         record.Provider,
		Model:            model,
		Alias:            alias,
		AccountID:        stringFromAccount(account, "id"),
		AccountEmail:     stringFromAccount(account, "email"),
		AuthID:           record.AuthID,
		APIKeyID:         stringFromAPIKey(spec, "id"),
		APIKeyLabel:      stringFromAPIKey(spec, "label"),
		ClientInstanceID: clientInstanceIDFromContext(ctx),
		RequestKind:      requestKind,
		ServiceTier:      normalizedUsageServiceTier(record.ServiceTier),
		ReasoningEffort:  strings.TrimSpace(record.ReasoningEffort),
		Success:          success,
		Status:           status,
		ErrorCategory:    errorCategory(status, record.Fail.Body, success),
		ErrorMessage:     strings.TrimSpace(record.Fail.Body),
		LatencyMS:        record.Latency.Milliseconds(),
		Usage: usageDetails{
			InputTokens:     record.Detail.InputTokens,
			OutputTokens:    record.Detail.OutputTokens,
			ReasoningTokens: record.Detail.ReasoningTokens,
			CachedTokens:    record.Detail.CachedTokens,
			TotalTokens:     record.Detail.TotalTokens,
			TokenBreakdown:  record.Detail.TokenBreakdown,
		},
		RequestedAtMS: record.RequestedAt.UnixMilli(),
	})
}

func (p *usagePlugin) accountForRecord(record coreusage.Record) *accountSpec {
	if p == nil || p.manifest == nil {
		return nil
	}
	if record.AuthID != "" {
		if account := p.manifest.accountByAuthID[strings.ToLower(record.AuthID)]; account != nil {
			return account
		}
		base := strings.TrimSuffix(filepath.Base(record.AuthID), filepath.Ext(record.AuthID))
		if account := p.manifest.accountByID[base]; account != nil {
			return account
		}
	}
	if record.APIKey != "" {
		return p.manifest.accountByAPIKey[record.APIKey]
	}
	return nil
}

func stringFromAccount(account *accountSpec, field string) string {
	if account == nil {
		return ""
	}
	if field == "email" {
		return account.Email
	}
	return account.ID
}

func stringFromAPIKey(spec *apiKeySpec, field string) string {
	if spec == nil {
		return ""
	}
	if field == "label" {
		return spec.Label
	}
	return spec.ID
}

func errorCategory(status int, body string, success bool) string {
	if success {
		return ""
	}
	lower := strings.ToLower(body)
	switch {
	case strings.Contains(lower, "upstream timed out in stream_open") ||
		strings.Contains(lower, "phase=execute_stream upstream timed out in stream_open") ||
		strings.Contains(lower, "stream_open"):
		return "upstream_first_byte_timeout"
	case strings.Contains(lower, "upstream timed out in stream_idle") ||
		strings.Contains(lower, "stream_idle"):
		return "upstream_stream_timeout"
	case strings.Contains(lower, "upstream timed out") ||
		strings.Contains(lower, "request_timeout") ||
		strings.Contains(lower, "deadline exceeded"):
		return "upstream_stream_timeout"
	case strings.Contains(lower, "downstream_client_closed") ||
		strings.Contains(lower, "stream_client_gone") ||
		strings.Contains(lower, "client_gone") ||
		strings.Contains(lower, "client canceled") ||
		strings.Contains(lower, "client disconnected") ||
		strings.Contains(lower, "client closed") ||
		strings.Contains(lower, "broken pipe") ||
		strings.Contains(lower, "connection reset") ||
		strings.Contains(lower, "connection aborted") ||
		strings.Contains(lower, "unexpected eof"):
		return "client_canceled"
	case strings.Contains(lower, "context canceled"):
		if status >= http.StatusInternalServerError || status == http.StatusRequestTimeout {
			return "gateway_context_canceled"
		}
		return "client_canceled"
	case strings.Contains(lower, "upstream_response_failed") ||
		strings.Contains(lower, "codex upstream response.failed") ||
		strings.Contains(lower, "last_event=response.failed"):
		return "upstream_response_failed"
	case status == http.StatusUnauthorized || status == http.StatusForbidden:
		return "auth_failed"
	case status == http.StatusNotFound:
		return "model_not_available"
	case status == http.StatusTooManyRequests || strings.Contains(lower, "quota") || strings.Contains(lower, "rate limit"):
		return "quota_or_rate_limit"
	case status >= 500:
		return "upstream_error"
	default:
		return "request_failed"
	}
}

type authHook struct {
	manifest *manifest
	emitter  *eventEmitter
}

func (h *authHook) OnAuthRegistered(_ context.Context, auth *coreauth.Auth) {
	h.emit("auth_registered", auth)
}

func (h *authHook) OnAuthUpdated(_ context.Context, auth *coreauth.Auth) {
	h.emit("auth_updated", auth)
}

func (h *authHook) OnResult(ctx context.Context, result coreauth.Result) {
	if h == nil || h.emitter == nil {
		return
	}
	if ctx == nil {
		ctx = context.Background()
	}
	spec, _ := ctx.Value(clientAPIKeyContextKey).(*apiKeySpec)
	requestKind, _ := ctx.Value(requestKindContextKey).(string)
	if requestKind == "" {
		requestKind = requestKindFromPath(internallogging.GetEndpoint(ctx))
	}
	model := strings.TrimSpace(result.Model)
	if requestModel, _ := ctx.Value(requestModelContextKey).(string); strings.TrimSpace(requestModel) != "" {
		model = strings.TrimSpace(requestModel)
	}
	account := h.accountForAuthID(result.AuthID)
	status := 0
	errorCode := ""
	errorMessage := ""
	retryable := false
	var retryablePtr *bool
	if result.Error != nil {
		status = result.Error.HTTPStatus
		errorCode = result.Error.Code
		errorMessage = result.Error.Message
		retryable = result.Error.Retryable
		retryablePtr = &retryable
	}
	retryAfterMS := int64(0)
	if result.RetryAfter != nil {
		retryAfterMS = result.RetryAfter.Milliseconds()
	}
	success := result.Success
	var authAvailable *bool
	if result.AuthStateKnown {
		value := result.AuthAvailable
		authAvailable = &value
	}
	nextRetryAtMS := int64(0)
	if !result.NextRetryAt.IsZero() {
		nextRetryAtMS = result.NextRetryAt.UnixMilli()
	}
	h.emitter.emit(requestDiagnosticPayload{
		Type:            "auth_result",
		RequestID:       internallogging.GetRequestID(ctx),
		Provider:        result.Provider,
		Model:           model,
		AuthID:          result.AuthID,
		AccountID:       stringFromAccount(account, "id"),
		AccountEmail:    stringFromAccount(account, "email"),
		APIKeyID:        stringFromAPIKey(spec, "id"),
		APIKeyLabel:     stringFromAPIKey(spec, "label"),
		RequestKind:     requestKind,
		Success:         &success,
		HTTPStatus:      status,
		ErrorCode:       errorCode,
		ErrorMessage:    errorMessage,
		Retryable:       retryablePtr,
		RetryAfterMS:    retryAfterMS,
		AuthAvailable:   authAvailable,
		NextRetryAtMS:   nextRetryAtMS,
		AuthStateReason: result.AuthStateReason,
	})
}

func (h *authHook) accountForAuthID(authID string) *accountSpec {
	if h == nil || h.manifest == nil {
		return nil
	}
	authID = strings.TrimSpace(authID)
	if authID == "" {
		return nil
	}
	if account := h.manifest.accountByAuthID[strings.ToLower(authID)]; account != nil {
		return account
	}
	base := strings.TrimSuffix(filepath.Base(authID), filepath.Ext(authID))
	return h.manifest.accountByID[base]
}

func (h *authHook) emit(eventType string, auth *coreauth.Auth) {
	if h == nil || h.emitter == nil || auth == nil {
		return
	}
	h.emitter.emit(map[string]any{
		"type":     eventType,
		"authId":   auth.ID,
		"provider": auth.Provider,
		"label":    auth.Label,
		"status":   string(auth.Status),
		"disabled": auth.Disabled,
	})
}

func buildCoreAuthSelector(cfg *config.Config, selector coreauth.Selector, m *manifest, quota *quotaReserveStateStore) coreauth.Selector {
	if selector == nil {
		selector = &coreauth.RoundRobinSelector{}
	}
	if cfg != nil && cfg.Routing.SessionAffinity {
		imageFallback := selector
		ttl := time.Hour
		if parsed, err := time.ParseDuration(strings.TrimSpace(cfg.Routing.SessionAffinityTTL)); err == nil && parsed > 0 {
			ttl = parsed
		}
		// Session affinity + per-client-key namespace, with image requests bypassing affinity.
		selector = coreauth.NewSessionAffinitySelectorWithConfig(coreauth.SessionAffinityConfig{
			Fallback: selector,
			TTL:      ttl,
		})
		selector = &cockpitSessionAffinitySelector{inner: selector}
		selector = &imageRequestSelector{
			imageFallback: imageFallback,
			fallback:      selector,
		}
	}
	if m != nil {
		selector = &backupAccountSelector{manifest: m, fallback: selector}
		selector = &quotaReserveSelector{manifest: m, fallback: selector, quota: quota}
		selector = &modelExclusionSelector{manifest: m, fallback: selector}
	}
	return selector
}

func buildCoreAuthManager(cfg *config.Config, selector coreauth.Selector, hook coreauth.Hook, m *manifest, quota *quotaReserveStateStore, tracker *requestUsageTracker) *coreauth.Manager {
	tokenStore := sdkauth.GetTokenStore()
	if dirSetter, ok := tokenStore.(interface{ SetBaseDir(string) }); ok && cfg != nil {
		dirSetter.SetBaseDir(cfg.AuthDir)
	}
	selector = buildCoreAuthSelector(cfg, selector, m, quota)
	if tracker != nil {
		selector = &recordingSelector{inner: selector, manifest: m, tracker: tracker}
	}
	return coreauth.NewManager(tokenStore, selector, hook)
}

type cockpitSessionAffinitySelector struct {
	inner coreauth.Selector
}

func (s *cockpitSessionAffinitySelector) Pick(ctx context.Context, provider, model string, opts cliproxyexecutor.Options, auths []*coreauth.Auth) (*coreauth.Auth, error) {
	if s == nil || s.inner == nil {
		return nil, errors.New("session affinity selector is unavailable")
	}
	if spec, _ := ctx.Value(clientAPIKeyContextKey).(*apiKeySpec); spec != nil && strings.TrimSpace(spec.ID) != "" {
		metadata := make(map[string]any, len(opts.Metadata)+1)
		for key, value := range opts.Metadata {
			metadata[key] = value
		}
		metadata[cliproxyexecutor.CallerScopeMetadataKey] = spec.ID
		opts.Metadata = metadata
	}
	return s.inner.Pick(ctx, provider, model, opts, auths)
}

func (s *cockpitSessionAffinitySelector) ReportAuthSelectionFailure(ctx context.Context, provider, model string, candidates []*coreauth.Auth, err error) error {
	if reporter, ok := s.inner.(coreauth.AuthSelectionFailureReporter); ok {
		return reporter.ReportAuthSelectionFailure(ctx, provider, model, candidates, err)
	}
	return err
}
