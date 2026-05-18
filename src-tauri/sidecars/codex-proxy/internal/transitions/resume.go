package transitions

// ManualResumeResult 描述一次手动 resume 编排结果。
type ManualResumeResult struct {
	RestoredCount int
}

// RestoreAllAndReset 收口“恢复全部 disabled keys + 重置渠道 breaker”的编排。
func RestoreAllAndReset(
	restoreAllKeys func() (int, error),
	resetChannelRuntime func(),
) (ManualResumeResult, error) {
	result := ManualResumeResult{}
	restoredCount, err := restoreAllKeys()
	if err != nil {
		return result, err
	}
	resetChannelRuntime()
	result.RestoredCount = restoredCount
	return result, nil
}
