package cliproxy

import (
	"context"
	"errors"
	"fmt"
	"strings"
	"time"

	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/usage"
	log "github.com/sirupsen/logrus"
)

// StartRuntime starts the embedded executor runtime without binding the SDK HTTP server.
func (s *Service) StartRuntime(ctx context.Context) error {
	if s == nil {
		return fmt.Errorf("cliproxy: service is nil")
	}
	if ctx == nil {
		ctx = context.Background()
	}
	usage.StartDefault(ctx)
	shutdownCtx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	defer func() {
		if errShutdown := s.Shutdown(shutdownCtx); errShutdown != nil {
			log.Errorf("service shutdown returned error: %v", errShutdown)
		}
	}()
	if errEnsure := s.ensureAuthDir(); errEnsure != nil {
		return errEnsure
	}
	s.applyRetryConfig(s.cfg)
	s.configureCooldownStateStore(s.cfg)
	if s.coreManager != nil {
		if errLoad := s.coreManager.Load(ctx); errLoad != nil {
			log.Warnf("failed to load auth store: %v", errLoad)
		}
		s.coreManager.SetConfig(s.cfg)
		s.coreManager.SetOAuthModelAlias(s.cfg.OAuthModelAlias)
	}
	if s.tokenProvider != nil {
		if _, errLoad := s.tokenProvider.Load(ctx, s.cfg); errLoad != nil && !errors.Is(errLoad, context.Canceled) {
			return errLoad
		}
	}
	if s.apiKeyProvider != nil {
		if _, errLoad := s.apiKeyProvider.Load(ctx, s.cfg); errLoad != nil && !errors.Is(errLoad, context.Canceled) {
			return errLoad
		}
	}
	s.ensureWebsocketGateway()
	s.registerAvailableExecutors(ctx, executorRegistrationOptions{includeBaseline: true})
	if s.hooks.OnBeforeStart != nil {
		s.hooks.OnBeforeStart(s.cfg)
	}
	if s.hooks.OnAfterStart != nil {
		s.hooks.OnAfterStart(s)
	}
	if s.coreManager != nil {
		s.coreManager.StartAutoRefresh(context.Background(), 15*time.Minute)
	}
	<-ctx.Done()
	return ctx.Err()
}

// RebindRuntimeExecutors refreshes provider executors for embedded hosts.
func (s *Service) RebindRuntimeExecutors() {
	if s == nil {
		return
	}
	s.registerAvailableExecutors(context.Background(), executorRegistrationOptions{includeBaseline: true, forceReplaceAuths: true})
	if s.coreManager != nil {
		s.registerExecutorsForAuths(s.coreManager.List(), true)
	}
}

// UpsertRuntimeAuth registers or updates an auth entry in the embedded runtime.
func (s *Service) UpsertRuntimeAuth(ctx context.Context, auth *coreauth.Auth) (*coreauth.Auth, error) {
	if s == nil || s.coreManager == nil {
		return nil, fmt.Errorf("cliproxy: core auth manager is not configured")
	}
	if auth == nil || strings.TrimSpace(auth.ID) == "" {
		return nil, fmt.Errorf("cliproxy: auth id is required")
	}
	auth = auth.Clone()
	s.ensureExecutorsForAuthWithContext(ctx, auth, false)
	var registered *coreauth.Auth
	var err error
	if existing, ok := s.coreManager.GetByID(auth.ID); ok && existing != nil {
		auth.CreatedAt = existing.CreatedAt
		registered, err = s.coreManager.Update(ctx, auth)
	} else {
		registered, err = s.coreManager.Register(ctx, auth)
	}
	if err != nil {
		return nil, err
	}
	if registered == nil {
		registered = auth
	}
	s.registerModelsForAuth(ctx, registered)
	s.coreManager.ReconcileRegistryModelStates(ctx, registered.ID)
	s.coreManager.RefreshSchedulerEntry(registered.ID)
	return registered, nil
}

func (s *Service) Execute(ctx context.Context, providers []string, req cliproxyexecutor.Request, opts cliproxyexecutor.Options) (cliproxyexecutor.Response, error) {
	if s == nil || s.coreManager == nil {
		return cliproxyexecutor.Response{}, fmt.Errorf("cliproxy: core auth manager is not configured")
	}
	return s.coreManager.Execute(ctx, providers, req, opts)
}

func (s *Service) ExecuteStream(ctx context.Context, providers []string, req cliproxyexecutor.Request, opts cliproxyexecutor.Options) (*cliproxyexecutor.StreamResult, error) {
	if s == nil || s.coreManager == nil {
		return nil, fmt.Errorf("cliproxy: core auth manager is not configured")
	}
	return s.coreManager.ExecuteStream(ctx, providers, req, opts)
}
