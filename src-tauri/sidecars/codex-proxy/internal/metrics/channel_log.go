package metrics

import (
	"sync"
	"time"
)

// ChannelLog 单次上游请求日志
type ChannelLog struct {
	RequestID     string    `json:"requestId"` // 请求唯一标识
	ChannelIndex  int       `json:"-"`         // 创建时的渠道索引（不序列化，仅用于内部验证）
	Timestamp     time.Time `json:"timestamp"`
	Model         string    `json:"model"`                   // 实际使用的模型（重定向后）
	OriginalModel string    `json:"originalModel,omitempty"` // 原始请求模型（仅当重定向时有值）
	Operation     string    `json:"operation,omitempty"`     // Images 端点（generations/edits/variations）
	StatusCode    int       `json:"statusCode"`
	DurationMs    int64     `json:"durationMs"`
	Success       bool      `json:"success"`
	KeyMask       string    `json:"keyMask"`
	BaseURL       string    `json:"baseUrl"`
	ErrorInfo     string    `json:"errorInfo"`
	IsRetry       bool      `json:"isRetry"`
	InterfaceType string    `json:"interfaceType"`           // 接口类型（Messages/Responses/Gemini）
	RequestSource string    `json:"requestSource,omitempty"` // 请求来源（proxy/capability_test）

	// 请求生命周期状态
	Status      string     `json:"status"`                // pending/connecting/first_byte/streaming/completed/failed/cancelled
	StartTime   time.Time  `json:"startTime"`             // 请求开始时间
	ConnectedAt *time.Time `json:"connectedAt,omitempty"` // 连接建立时间
	FirstByteAt *time.Time `json:"firstByteAt,omitempty"` // 首字节到达时间
	CompletedAt *time.Time `json:"completedAt,omitempty"` // 请求完成时间
}

const (
	RequestSourceProxy          = "proxy"
	RequestSourceCapabilityTest = "capability_test"
	maxChannelLogs              = 50

	// 请求状态常量
	StatusPending    = "pending"
	StatusConnecting = "connecting"
	StatusFirstByte  = "first_byte"
	StatusStreaming  = "streaming"
	StatusCompleted  = "completed"
	StatusFailed     = "failed"
	StatusCancelled  = "cancelled"
)

func isTerminalStatus(status string) bool {
	return status == StatusCompleted || status == StatusFailed || status == StatusCancelled
}

// ChannelLogStore 渠道日志存储（内存环形缓冲区）
type ChannelLogStore struct {
	mu               sync.RWMutex
	logs             map[int][]*ChannelLog // key: channelIndex
	requestLocations map[string]int        // requestID -> current channelIndex；仅跟踪在途请求
}

func NewChannelLogStore() *ChannelLogStore {
	return &ChannelLogStore{
		logs:             make(map[int][]*ChannelLog),
		requestLocations: make(map[string]int),
	}
}

func (s *ChannelLogStore) Record(channelIndex int, log *ChannelLog) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if log != nil && log.RequestID != "" {
		log.ChannelIndex = channelIndex
		if !isTerminalStatus(log.Status) {
			s.requestLocations[log.RequestID] = channelIndex
		} else {
			delete(s.requestLocations, log.RequestID)
		}
	}
	s.logs[channelIndex] = append(s.logs[channelIndex], log)
	if len(s.logs[channelIndex]) > maxChannelLogs {
		s.logs[channelIndex] = s.logs[channelIndex][len(s.logs[channelIndex])-maxChannelLogs:]
	}
}

// RemoveAndShift 删除指定渠道日志，并将其后的渠道日志索引前移一位，保持与删除后的渠道切片索引一致。
func (s *ChannelLogStore) RemoveAndShift(channelIndex int) {
	s.mu.Lock()
	defer s.mu.Unlock()

	if len(s.logs) == 0 && len(s.requestLocations) == 0 {
		return
	}

	// 先修正所有在途请求的索引：
	// - 指向被删除渠道的请求直接移除（包括已被环形缓冲淘汰但仍在途的请求）
	// - 指向其后渠道的请求索引前移一位
	for requestID, idx := range s.requestLocations {
		switch {
		case idx == channelIndex:
			delete(s.requestLocations, requestID)
		case idx > channelIndex:
			s.requestLocations[requestID] = idx - 1
		}
	}

	if len(s.logs) == 0 {
		return
	}

	shifted := make(map[int][]*ChannelLog, len(s.logs))
	for idx, logs := range s.logs {
		switch {
		case idx == channelIndex:
			continue
		case idx > channelIndex:
			for _, log := range logs {
				if log != nil {
					log.ChannelIndex = idx - 1
				}
			}
			shifted[idx-1] = logs
		default:
			shifted[idx] = logs
		}
	}

	s.logs = shifted
}

// ClearAll 清除所有渠道日志，仅用于需要整体重置日志缓存的场景。
func (s *ChannelLogStore) ClearAll() {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.logs = make(map[int][]*ChannelLog)
	s.requestLocations = make(map[string]int)
}

func (s *ChannelLogStore) Get(channelIndex int) []*ChannelLog {
	s.mu.RLock()
	defer s.mu.RUnlock()
	src := s.logs[channelIndex]
	if len(src) == 0 {
		return nil
	}
	// 返回深拷贝，按时间倒序（最新在前），避免并发修改问题
	result := make([]*ChannelLog, len(src))
	for i, j := 0, len(src)-1; j >= 0; i, j = i+1, j-1 {
		// 深拷贝每个日志对象
		logCopy := *src[j]
		result[i] = &logCopy
	}
	return result
}

// UpdateStatus 描述 Update 的结果
type UpdateStatus int

const (
	UpdateFound UpdateStatus = iota
	UpdateMissingEvicted
	UpdateMissingDeleted
)

// Update 更新指定请求日志（通过 RequestID 匹配）
// 优先使用在途请求索引定位；若请求已不在途，则区分为淘汰或删除。
// 返回值为 (状态, 当前实际渠道索引)。
func (s *ChannelLogStore) Update(channelIndex int, requestID string, updateFn func(*ChannelLog)) (UpdateStatus, int) {
	s.mu.Lock()
	defer s.mu.Unlock()

	if requestID == "" {
		return UpdateMissingDeleted, -1
	}

	actualIndex, tracking := s.requestLocations[requestID]
	if !tracking {
		return UpdateMissingDeleted, -1
	}

	logs, ok := s.logs[actualIndex]
	if !ok {
		delete(s.requestLocations, requestID)
		return UpdateMissingDeleted, -1
	}

	for i := range logs {
		if logs[i].RequestID == requestID {
			updateFn(logs[i])
			if isTerminalStatus(logs[i].Status) {
				delete(s.requestLocations, requestID)
			}
			return UpdateFound, actualIndex
		}
	}

	// 仍被标记为在途，但已不在缓冲区中，说明是环形缓冲淘汰。
	return UpdateMissingEvicted, actualIndex
}
