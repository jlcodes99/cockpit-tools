package helps

import (
	"context"
	"net/http"
	"strings"
	"sync"
	"time"

	internallogging "github.com/router-for-me/CLIProxyAPI/v7/internal/logging"
)

// TurnStateObservation 记录一次上游响应里 X-Codex-Turn-State 的可观测信息。
// 只保留长度与分类，绝不保存 state 原文。
type TurnStateObservation struct {
	Length     int
	Class      string
	Status     int
	ObservedAt time.Time
}

const (
	// TurnStateHeaderName 是上游返回的 turn state 响应头。
	TurnStateHeaderName = "x-codex-turn-state"

	// 292/332 为正常长度；312 直接作为「疑似风控」信号；其它长度或缺失视为异常。
	TurnStateClassNormal    = "normal"
	TurnStateClassSuspected = "suspected"
	TurnStateClassAbnormal  = "abnormal"
	TurnStateClassMissing   = "missing"

	turnStateObserverTTL      = 10 * time.Minute
	turnStateObserverMaxItems = 4096
)

// ClassifyTurnStateValue 按观测规则对 state 长度分类。state 内容始终按不透明处理，
// 这里只读长度，不做结构或签名校验。
func ClassifyTurnStateValue(value string) (int, string) {
	length := len(strings.TrimSpace(value))
	switch length {
	case 0:
		return 0, TurnStateClassMissing
	case 292, 332:
		return length, TurnStateClassNormal
	case 312:
		return length, TurnStateClassSuspected
	default:
		return length, TurnStateClassAbnormal
	}
}

type turnStateObserver struct {
	mu      sync.Mutex
	entries map[string]TurnStateObservation
}

var globalTurnStateObserver = &turnStateObserver{
	entries: make(map[string]TurnStateObservation),
}

// ObserveTurnStateFromHeaders 在收到上游响应后记录一次 state 观测，按 request ID
// 存放，供 usage 上报时取出。缺失该响应头同样记录为 missing。
func ObserveTurnStateFromHeaders(ctx context.Context, status int, headers http.Header) {
	requestID := strings.TrimSpace(internallogging.GetRequestID(ctx))
	if requestID == "" {
		return
	}

	length := 0
	class := TurnStateClassMissing
	if headers != nil {
		if values := headers.Values(TurnStateHeaderName); len(values) > 0 {
			length, class = ClassifyTurnStateValue(values[0])
		}
	}

	now := time.Now()
	globalTurnStateObserver.mu.Lock()
	defer globalTurnStateObserver.mu.Unlock()
	if len(globalTurnStateObserver.entries) >= turnStateObserverMaxItems {
		pruneTurnStateObservationsLocked(now)
	}
	globalTurnStateObserver.entries[requestID] = TurnStateObservation{
		Length:     length,
		Class:      class,
		Status:     status,
		ObservedAt: now,
	}
}

// TakeTurnStateObservation 取出并移除某个请求的观测结果；过期条目按未观测处理。
func TakeTurnStateObservation(requestID string) (TurnStateObservation, bool) {
	requestID = strings.TrimSpace(requestID)
	if requestID == "" {
		return TurnStateObservation{}, false
	}
	now := time.Now()
	globalTurnStateObserver.mu.Lock()
	defer globalTurnStateObserver.mu.Unlock()
	observation, ok := globalTurnStateObserver.entries[requestID]
	if !ok {
		return TurnStateObservation{}, false
	}
	delete(globalTurnStateObserver.entries, requestID)
	if now.Sub(observation.ObservedAt) > turnStateObserverTTL {
		return TurnStateObservation{}, false
	}
	return observation, true
}

func pruneTurnStateObservationsLocked(now time.Time) {
	for key, observation := range globalTurnStateObserver.entries {
		if now.Sub(observation.ObservedAt) > turnStateObserverTTL {
			delete(globalTurnStateObserver.entries, key)
		}
	}
	if len(globalTurnStateObserver.entries) >= turnStateObserverMaxItems {
		// 极端情况下直接清空，观察结果只用于统计，丢失不影响业务请求。
		globalTurnStateObserver.entries = make(map[string]TurnStateObservation)
	}
}
