package transitions

import "github.com/jlcodes99/cockpit-tools/codex-proxy/internal/metrics"

// RestoreAndHalfOpenResult 描述一次恢复编排结果。
type RestoreAndHalfOpenResult struct {
	RestoredKeys     []string
	ActivatedChannel bool
}

// RestoreDisabledKeysAndActivate 收口“恢复 disabled keys + half-open + 激活渠道”的编排。
func RestoreDisabledKeysAndActivate(
	restoreDisabledKeys func([]string) ([]string, error),
	moveKeyToHalfOpen func(baseURL, apiKey string),
	setChannelStatus func(string) error,
	shouldActivateChannel func() bool,
	keysToRestore []string,
) (RestoreAndHalfOpenResult, error) {
	_ = (*metrics.MetricsManager)(nil)
	result := RestoreAndHalfOpenResult{}
	if len(keysToRestore) == 0 {
		return result, nil
	}

	restoredKeys, err := restoreDisabledKeys(keysToRestore)
	if err != nil {
		return result, err
	}
	if len(restoredKeys) == 0 {
		return result, nil
	}

	for _, apiKey := range restoredKeys {
		moveKeyToHalfOpen("", apiKey)
	}

	result.RestoredKeys = restoredKeys
	if shouldActivateChannel != nil && shouldActivateChannel() {
		if err := setChannelStatus("active"); err != nil {
			return result, err
		}
		result.ActivatedChannel = true
	}
	return result, nil
}
