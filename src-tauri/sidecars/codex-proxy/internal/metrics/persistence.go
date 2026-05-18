package metrics

import (
	"time"
)

// PersistenceStore 持久化存储接口
type PersistenceStore interface {
	// AddRecord 添加记录到写入缓冲区（非阻塞）
	AddRecord(record PersistentRecord)

	// LoadRecords 加载指定时间范围内的记录
	LoadRecords(since time.Time, apiType string) ([]PersistentRecord, error)

	// LoadLatestTimestamps 从全量历史记录中查询每个 key 的最后成功/失败时间
	// 用于启动时补全超出 24h 窗口的时间戳
	LoadLatestTimestamps(apiType string) (map[string]*KeyLatestTimestamps, error)

	// LoadCircuitStates 加载指定 API 类型的熔断状态
	LoadCircuitStates(apiType string) (map[string]*PersistentCircuitState, error)

	// UpsertCircuitState 写入或更新熔断状态
	UpsertCircuitState(state PersistentCircuitState) error

	// QueryAggregatedHistory 查询聚合历史数据（按时间桶分组）
	// 用于 >24h 的长时间范围查询（1周/1月），内存中仅保留 24h
	QueryAggregatedHistory(apiType string, since time.Time, intervalSeconds int64, metricsKey string, baseURL string) ([]AggregatedBucket, error)

	// CleanupOldRecords 清理过期数据
	CleanupOldRecords(before time.Time) (int64, error)

	// DeleteRecordsByMetricsKeys 按 metrics_key 和 api_type 批量删除记录（用于删除渠道时清理数据）
	// apiType: 接口类型（messages/responses/gemini），避免误删其他接口的数据
	DeleteRecordsByMetricsKeys(metricsKeys []string, apiType string) (int64, error)

	// DeleteCircuitStatesByMetricsKeys 按 metrics_key 和 api_type 批量删除熔断状态
	DeleteCircuitStatesByMetricsKeys(metricsKeys []string, apiType string) (int64, error)

	// Close 关闭存储（会先刷新缓冲区）
	Close() error
}

// KeyLatestTimestamps 每个 key 的最后成功/失败时间
type KeyLatestTimestamps struct {
	BaseURL       string
	KeyMask       string
	LastSuccessAt *time.Time
	LastFailureAt *time.Time
}

// PersistentCircuitState 持久化熔断状态结构
type PersistentCircuitState struct {
	MetricsKey          string
	BaseURL             string
	KeyMask             string
	APIType             string
	CircuitState        string
	CircuitOpenedAt     *time.Time
	HalfOpenAt          *time.Time
	NextRetryAt         *time.Time
	BackoffLevel        int
	HalfOpenSuccesses   int
	ConsecutiveFailures int64
}

// PersistentRecord 持久化记录结构
type PersistentRecord struct {
	MetricsKey          string       // hash(baseURL + apiKey)
	BaseURL             string       // 上游 BaseURL
	KeyMask             string       // 脱敏的 API Key
	Timestamp           time.Time    // 请求时间
	Success             bool         // 是否成功
	FailureClass        FailureClass // 失败分类（用于重建 breaker 窗口）
	InputTokens         int64        // 输入 Token 数
	OutputTokens        int64        // 输出 Token 数
	CacheCreationTokens int64        // 缓存创建 Token
	CacheReadTokens     int64        // 缓存读取 Token
	Model               string       // 请求模型
	APIType             string       // "messages"、"responses"、"gemini" 或 "chat"
}
