package main

import (
	"context"

	"errors"
	"flag"

	"net"
	"net/http"

	"os"
	"os/signal"
	"path/filepath"

	"strconv"
	"strings"

	"syscall"
	"time"

	codexlive "github.com/router-for-me/CLIProxyAPI/v7/internal/client/codex/live"

	sdkhandlers "github.com/router-for-me/CLIProxyAPI/v7/sdk/api/handlers"
	sdkopenai "github.com/router-for-me/CLIProxyAPI/v7/sdk/api/handlers/openai"

	coreusage "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/usage"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/config"

	_ "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator/builtin"
)

func runRelayHTTPServer(ctx context.Context, cfg *config.Config, handler http.Handler, emitter *eventEmitter) error {
	host := "127.0.0.1"
	port := 0
	if cfg != nil {
		if strings.TrimSpace(cfg.Host) != "" {
			host = strings.TrimSpace(cfg.Host)
		}
		port = cfg.Port
	}
	listener, err := net.Listen("tcp", net.JoinHostPort(host, strconv.Itoa(port)))
	if err != nil {
		return err
	}
	server := &http.Server{
		Handler:           handler,
		ReadHeaderTimeout: 30 * time.Second,
	}
	errCh := make(chan error, 1)
	go func() {
		if serveErr := server.Serve(listener); serveErr != nil && !errors.Is(serveErr, http.ErrServerClosed) {
			errCh <- serveErr
			return
		}
		errCh <- nil
	}()
	if emitter != nil {
		readyPort := port
		if tcpAddr, ok := listener.Addr().(*net.TCPAddr); ok {
			readyPort = tcpAddr.Port
		}
		emitter.emit(map[string]any{"type": "ready", "port": readyPort, "host": host})
	}
	select {
	case <-ctx.Done():
		shutdownCtx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
		defer cancel()
		_ = server.Shutdown(shutdownCtx)
		return ctx.Err()
	case serveErr := <-errCh:
		return serveErr
	}
}

func monitorParentProcess(ctx context.Context, parentPID int, cancel context.CancelFunc, emitter *eventEmitter) {
	if parentPID <= 0 || parentPID == os.Getpid() {
		return
	}
	monitorParentProcessPlatform(ctx, parentPID, cancel, emitter)
}

func normalizeCockpitLocale(locale string) string {
	locale = strings.TrimSpace(locale)
	if locale == "" {
		return "en"
	}
	return locale
}

func main() {
	ignoreBrokenPipeSignal()
	configPath := flag.String("config", "", "CLIProxyAPI config file")
	manifestPath := flag.String("manifest", "", "Cockpit sidecar manifest file")
	quotaReserveStatePath := flag.String("quota-reserve-state", "", "Cockpit OAuth quota reserve state file")
	quotaPoolStatePath := flag.String("quota-pool-state", "", "Cockpit account-pool quota state file")
	parentPID := flag.Int("parent-pid", 0, "Cockpit Tools parent process id")
	flag.Parse()

	emitter := &eventEmitter{}
	if strings.TrimSpace(*configPath) == "" || strings.TrimSpace(*manifestPath) == "" {
		emitter.emit(map[string]any{"type": "error", "message": "missing --config or --manifest"})
		os.Exit(2)
	}

	emitter.emitStartupStage("resolve_config_path")
	absConfigPath, err := filepath.Abs(*configPath)
	if err != nil {
		emitter.emit(map[string]any{"type": "error", "message": err.Error()})
		os.Exit(2)
	}
	emitter.emitStartupStage("load_config")
	cfg, err := config.LoadConfig(absConfigPath)
	if err != nil {
		emitter.emit(map[string]any{"type": "error", "message": err.Error()})
		os.Exit(2)
	}
	emitter.emitStartupStage("load_manifest")
	m, err := loadManifest(*manifestPath)
	if err != nil {
		emitter.emit(map[string]any{"type": "error", "message": err.Error()})
		os.Exit(2)
	}
	emitter.emitStartupStage("init_runtime")
	quotaState := newQuotaReserveStateStore(*quotaReserveStatePath, m)
	if err := quotaState.load(); err != nil {
		emitter.emit(map[string]any{
			"type":    "quota_reserve_state_error",
			"message": err.Error(),
		})
	}

	usageTracker := newRequestUsageTracker()
	tokenLimiter := newAPIKeyTokenLimiter(m)
	policy := &requestPolicy{
		manifest:     m,
		emitter:      emitter,
		tracker:      usageTracker,
		tokenLimiter: tokenLimiter,
	}
	hook := &authHook{manifest: m, emitter: emitter}
	priorityState := newAPIKeyPriorityStateStore(*manifestPath)
	selector := &cockpitSelector{
		manifest:   m,
		emitter:    emitter,
		locale:     normalizeCockpitLocale(m.Locale),
		quota:      quotaState,
		priorities: priorityState,
		tracker:    usageTracker,
	}
	coreManager := buildCoreAuthManager(cfg, selector, hook, m, quotaState, usageTracker)

	signalCtx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()
	ctx, cancel := context.WithCancel(signalCtx)
	defer cancel()
	quotaState.start(ctx, emitter)
	monitorParentProcess(ctx, *parentPID, cancel, emitter)

	coreusage.RegisterPlugin(&usagePlugin{manifest: m, tracker: usageTracker})

	runtime, err := newSidecarRuntime(ctx, absConfigPath, cfg, m, coreManager)
	if err != nil {
		emitter.emit(map[string]any{"type": "error", "message": err.Error()})
		os.Exit(1)
	}
	defer runtime.Stop()
	emitter.emitStartupStage("start_http_server")

	// Reuse the same coreManager so WS upgrades share OAuth pool, routing and
	// session affinity with POST /v1/responses.
	var sdkCfg *config.SDKConfig
	if cfg != nil {
		sdkCfg = &cfg.SDKConfig
	}
	baseHandlers := sdkhandlers.NewBaseAPIHandlers(sdkCfg, coreManager)
	responsesHandler := sdkopenai.NewOpenAIResponsesAPIHandler(baseHandlers)
	liveHandler := codexlive.NewHandler(coreManager, cfg)
	defer liveHandler.Close()
	relay := &relayServer{
		runtime:            runtime,
		cfg:                cfg,
		manifest:           m,
		authManager:        coreManager,
		emitter:            emitter,
		policy:             policy,
		responsesWebsocket: responsesHandler.ResponsesWebsocket,
		codexLive:          liveHandler,
		quotaPoolStatePath: *quotaPoolStatePath,
	}
	if err := runRelayHTTPServer(ctx, cfg, relay.router(), emitter); err != nil && !errors.Is(err, context.Canceled) {
		emitter.emit(map[string]any{"type": "error", "message": err.Error()})
		os.Exit(1)
	}
}
