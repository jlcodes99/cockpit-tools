package cliproxy

import (
	"context"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/runtime/executor"
)

// Both the standalone server and Cockpit's embedded runtime own an updater.
// Its context is canceled on startup failure as well as ordinary shutdown.
func (s *Service) startXAIVersionUpdater(ctx context.Context) context.CancelFunc {
	updaterCtx, cancel := context.WithCancel(ctx)
	s.cfgMu.RLock()
	proxyURL := ""
	if s.cfg != nil {
		proxyURL = s.cfg.ProxyURL
	}
	s.cfgMu.RUnlock()
	executor.StartXAIVersionUpdater(updaterCtx, proxyURL)
	return cancel
}
