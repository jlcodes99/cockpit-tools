package main

import (
	"context"

	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
)

// Enforce authorization before reserve, backup, concurrency and affinity
// policies. Out-of-scope accounts must never influence a policy's fallback.
type apiKeyScopeSelector struct {
	manifest *manifest
	fallback coreauth.Selector
}

func (s *apiKeyScopeSelector) Pick(ctx context.Context, provider, model string, opts cliproxyexecutor.Options, auths []*coreauth.Auth) (*coreauth.Auth, error) {
	scoped := (&cockpitSelector{manifest: s.manifest}).filterAuthsForAPIKeyScope(ctx, auths)
	if len(scoped) == 0 && len(auths) > 0 {
		err := noAuthAvailableError(nil)
		return nil, s.ReportAuthSelectionFailure(ctx, provider, model, auths, err)
	}
	return s.fallback.Pick(ctx, provider, model, opts, scoped)
}

func (s *apiKeyScopeSelector) ReportAuthSelectionFailure(ctx context.Context, provider, model string, auths []*coreauth.Auth, err error) error {
	if reporter, ok := s.fallback.(coreauth.AuthSelectionFailureReporter); ok {
		return reporter.ReportAuthSelectionFailure(ctx, provider, model, auths, err)
	}
	return err
}

func (s *apiKeyScopeSelector) OnResult(result coreauth.Result) {
	forwardAuthSelectionResult(s.fallback, result)
}

func (s *apiKeyScopeSelector) Stop() {
	if stoppable, ok := s.fallback.(coreauth.StoppableSelector); ok {
		stoppable.Stop()
	}
}
