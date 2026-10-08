package main

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"strings"
	"time"

	internallogging "github.com/router-for-me/CLIProxyAPI/v7/internal/logging"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
)

const (
	// defaultAccountConcurrencyMaxWaiting 同时排队等待账号并发槽位的请求上限，避免无限堆积。
	defaultAccountConcurrencyMaxWaiting = 100
	// accountConcurrencyWaitPollInterval 等待槽位时的兜底轮询间隔；正常情况下由槽位释放通知唤醒。
	accountConcurrencyWaitPollInterval = 500 * time.Millisecond
)

// errAccountConcurrencyFull 是自由选号路径的内部信号：所有候选账号都已被并发上限占满。
// 它不会直接返回给客户端，外层 accountConcurrencySelector 会据此转入等待。
var errAccountConcurrencyFull = errors.New("account concurrency limit reached")

func accountConcurrencyRequestID(ctx context.Context) string {
	if ctx == nil {
		return ""
	}
	return strings.TrimSpace(internallogging.GetRequestID(ctx))
}

func excludeAuths(auths []*coreauth.Auth, excluded map[*coreauth.Auth]struct{}) []*coreauth.Auth {
	remaining := make([]*coreauth.Auth, 0, len(auths))
	for _, auth := range auths {
		if auth == nil {
			continue
		}
		if _, skip := excluded[auth]; skip {
			continue
		}
		remaining = append(remaining, auth)
	}
	return remaining
}

// accountSlotSelector 在「自由选号」路径上占用账号并发槽位。
//
// 它位于会话亲和选择器内部：命中会话亲和缓存时不会走到这里，因此不会把同一个会话
// 拆分到其它账号（缓存命中由外层 accountConcurrencySelector 负责等待原账号）。
// 自由选号时按候选顺序逐个试抢槽位，抢不到就换下一个候选账号，避免空闲账号被浪费。
type accountSlotSelector struct {
	manifest *manifest
	tracker  *requestUsageTracker
	fallback coreauth.Selector
}

func (s *accountSlotSelector) maxConcurrency() int {
	if s == nil || s.manifest == nil || s.manifest.MaxAccountConcurrency < 1 {
		return 0
	}
	return s.manifest.MaxAccountConcurrency
}

func (s *accountSlotSelector) Pick(ctx context.Context, provider, model string, opts cliproxyexecutor.Options, auths []*coreauth.Auth) (*coreauth.Auth, error) {
	if s == nil || s.fallback == nil {
		return nil, errors.New("account slot selector is not initialized")
	}
	maxConcurrency := s.maxConcurrency()
	requestID := accountConcurrencyRequestID(ctx)
	if maxConcurrency <= 0 || s.tracker == nil || requestID == "" || len(auths) == 0 {
		return s.fallback.Pick(ctx, provider, model, opts, auths)
	}
	remaining := auths
	excluded := make(map[*coreauth.Auth]struct{}, len(auths))
	for len(remaining) > 0 {
		auth, err := s.fallback.Pick(ctx, provider, model, opts, remaining)
		if err != nil || auth == nil {
			return auth, err
		}
		if s.tracker.tryReserveAccountSlot(requestID, auth.ID, maxConcurrency) {
			return auth, nil
		}
		excluded[auth] = struct{}{}
		remaining = excludeAuths(auths, excluded)
	}
	return nil, errAccountConcurrencyFull
}

func (s *accountSlotSelector) Stop() {
	if s == nil || s.fallback == nil {
		return
	}
	if stoppable, ok := s.fallback.(coreauth.StoppableSelector); ok {
		stoppable.Stop()
	}
}

func (s *accountSlotSelector) OnResult(result coreauth.Result) {
	if s == nil {
		return
	}
	forwardAuthSelectionResult(s.fallback, result)
}

func (s *accountSlotSelector) ReportAuthSelectionFailure(ctx context.Context, provider, model string, candidates []*coreauth.Auth, err error) error {
	if s == nil || s.fallback == nil {
		return err
	}
	if reporter, ok := s.fallback.(coreauth.AuthSelectionFailureReporter); ok {
		return reporter.ReportAuthSelectionFailure(ctx, provider, model, candidates, err)
	}
	return err
}

// accountConcurrencySelector 是账号并发闸门的最外层入口。
//
// 放在最外层是因为会话亲和命中缓存时会直接返回绑定账号、不会调用内层选择器，
// 只有包住整条链才能覆盖这类请求。
type accountConcurrencySelector struct {
	manifest *manifest
	tracker  *requestUsageTracker
	locale   string
	fallback coreauth.Selector
}

func (s *accountConcurrencySelector) maxConcurrency() int {
	if s == nil || s.manifest == nil || s.manifest.MaxAccountConcurrency < 1 {
		return 0
	}
	return s.manifest.MaxAccountConcurrency
}

func (s *accountConcurrencySelector) waitDuration() time.Duration {
	if s == nil || s.manifest == nil || s.manifest.AccountConcurrencyWaitMs <= 0 {
		return 0
	}
	return time.Duration(s.manifest.AccountConcurrencyWaitMs) * time.Millisecond
}

func (s *accountConcurrencySelector) Pick(ctx context.Context, provider, model string, opts cliproxyexecutor.Options, auths []*coreauth.Auth) (*coreauth.Auth, error) {
	if s == nil || s.fallback == nil {
		return nil, errors.New("account concurrency selector is not initialized")
	}
	maxConcurrency := s.maxConcurrency()
	requestID := accountConcurrencyRequestID(ctx)
	if maxConcurrency <= 0 || s.tracker == nil || requestID == "" {
		return s.fallback.Pick(ctx, provider, model, opts, auths)
	}

	auth, err := s.fallback.Pick(ctx, provider, model, opts, auths)
	if err != nil {
		if errors.Is(err, errAccountConcurrencyFull) {
			return s.waitForAnyAccount(ctx, auths, requestID, maxConcurrency)
		}
		return nil, err
	}
	if auth == nil {
		return nil, nil
	}
	if s.tracker.tryReserveAccountSlot(requestID, auth.ID, maxConcurrency) {
		return auth, nil
	}
	// 到这里说明本次选择没有经过自由选号（会话亲和缓存命中）：
	// 同一会话必须尽量留在原账号，因此只等待该账号释放槽位。
	return s.waitForAnyAccount(ctx, []*coreauth.Auth{auth}, requestID, maxConcurrency)
}

// waitForAnyAccount 在候选账号中等待一个可用槽位；候选按传入顺序尝试。
func (s *accountConcurrencySelector) waitForAnyAccount(
	ctx context.Context,
	candidates []*coreauth.Auth,
	requestID string,
	maxConcurrency int,
) (*coreauth.Auth, error) {
	target := firstAccountSpec(s.manifest, candidates)
	if s.tracker == nil {
		return nil, s.concurrencyExceededError(target, 0)
	}
	if len(candidates) == 0 {
		return nil, s.concurrencyExceededError(target, 0)
	}
	if !s.tracker.tryBeginAccountWait(defaultAccountConcurrencyMaxWaiting) {
		return nil, s.concurrencyExceededError(target, 0)
	}
	defer s.tracker.endAccountWait()

	wait := s.waitDuration()
	startedAt := time.Now()
	deadline := startedAt.Add(wait)
	for {
		// 先取变更信号再尝试占位，避免释放与等待之间的竞态导致漏唤醒。
		changed := s.tracker.accountConcurrencyChangeSignal()
		for _, auth := range candidates {
			if auth == nil {
				continue
			}
			if s.tracker.tryReserveAccountSlot(requestID, auth.ID, maxConcurrency) {
				return auth, nil
			}
		}
		if wait <= 0 || !time.Now().Before(deadline) {
			return nil, s.concurrencyExceededError(target, time.Since(startedAt))
		}
		if err := waitForAccountConcurrencyChange(ctx, changed, deadline); err != nil {
			return nil, err
		}
	}
}

func waitForAccountConcurrencyChange(ctx context.Context, changed <-chan struct{}, deadline time.Time) error {
	timeout := time.Until(deadline)
	if timeout > accountConcurrencyWaitPollInterval {
		timeout = accountConcurrencyWaitPollInterval
	}
	if timeout <= 0 {
		return nil
	}
	timer := time.NewTimer(timeout)
	defer timer.Stop()
	select {
	case <-ctx.Done():
		return ctx.Err()
	case <-changed:
		return nil
	case <-timer.C:
		return nil
	}
}

// concurrencyExceededError 返回客户端可读的并发超限错误。
// Retryable=false：重试同一个已被占满的账号不会成功，避免上层继续放大等待。
func (s *accountConcurrencySelector) concurrencyExceededError(account *accountSpec, waited time.Duration) *coreauth.Error {
	limit := s.maxConcurrency()
	waitedSeconds := waited.Seconds()
	chinese := strings.HasPrefix(strings.ToLower(strings.TrimSpace(s.locale)), "zh")
	var message string
	if chinese {
		message = fmt.Sprintf(
			"账号并发已达上限：并发上限为 %d，等待 %.1f 秒后仍没有可用槽位。请在「Codex API 服务 → 调度选项」调整账号并发数，或稍后重试。",
			limit,
			waitedSeconds,
		)
	} else {
		message = fmt.Sprintf(
			"Account concurrency limit reached: limit is %d and no slot became available after waiting %.1fs. Adjust the account concurrency limit in Codex API Service → Scheduling options, or retry later.",
			limit,
			waitedSeconds,
		)
	}
	if account != nil {
		if email := strings.TrimSpace(account.Email); email != "" {
			message += fmt.Sprintf(" (%s)", email)
		} else if id := strings.TrimSpace(account.ID); id != "" {
			message += fmt.Sprintf(" (%s)", id)
		}
	}
	return &coreauth.Error{
		Code:       "account_concurrency_exceeded",
		Message:    message,
		HTTPStatus: http.StatusTooManyRequests,
		Retryable:  false,
	}
}

func (s *accountConcurrencySelector) Stop() {
	if s == nil || s.fallback == nil {
		return
	}
	if stoppable, ok := s.fallback.(coreauth.StoppableSelector); ok {
		stoppable.Stop()
	}
}

func (s *accountConcurrencySelector) OnResult(result coreauth.Result) {
	if s == nil {
		return
	}
	forwardAuthSelectionResult(s.fallback, result)
}

func (s *accountConcurrencySelector) ReportAuthSelectionFailure(ctx context.Context, provider, model string, candidates []*coreauth.Auth, err error) error {
	if s == nil || s.fallback == nil {
		return err
	}
	if reporter, ok := s.fallback.(coreauth.AuthSelectionFailureReporter); ok {
		return reporter.ReportAuthSelectionFailure(ctx, provider, model, candidates, err)
	}
	return err
}

func firstAccountSpec(m *manifest, candidates []*coreauth.Auth) *accountSpec {
	for _, auth := range candidates {
		if auth == nil {
			continue
		}
		if account := accountForAuthInManifest(m, auth); account != nil {
			return account
		}
	}
	return nil
}
