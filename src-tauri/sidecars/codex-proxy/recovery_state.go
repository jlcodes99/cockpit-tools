package main

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"time"
)

const scheduledRecoveryStateFile = ".config/scheduled_recovery_state.json"

type scheduledRecoveryState struct {
	LastCheckUTC string `json:"lastCheckUtc,omitempty"`
}

func loadScheduledRecoveryLastCheck(path string) (time.Time, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return time.Time{}, nil
		}
		return time.Time{}, fmt.Errorf("读取恢复检查状态失败: %w", err)
	}
	if len(data) == 0 {
		return time.Time{}, nil
	}

	var state scheduledRecoveryState
	if err := json.Unmarshal(data, &state); err != nil {
		return time.Time{}, fmt.Errorf("解析恢复检查状态失败: %w", err)
	}
	if state.LastCheckUTC == "" {
		return time.Time{}, nil
	}

	checkedAt, err := time.Parse(time.RFC3339Nano, state.LastCheckUTC)
	if err != nil {
		return time.Time{}, fmt.Errorf("解析恢复检查时间失败: %w", err)
	}
	return checkedAt.UTC(), nil
}

func saveScheduledRecoveryLastCheck(path string, checkedAt time.Time) error {
	state := scheduledRecoveryState{LastCheckUTC: checkedAt.UTC().Format(time.RFC3339Nano)}
	data, err := json.Marshal(state)
	if err != nil {
		return fmt.Errorf("序列化恢复检查状态失败: %w", err)
	}
	if err := os.MkdirAll(filepath.Dir(path), 0755); err != nil {
		return fmt.Errorf("创建恢复检查状态目录失败: %w", err)
	}
	if err := os.WriteFile(path, append(data, '\n'), 0644); err != nil {
		return fmt.Errorf("写入恢复检查状态失败: %w", err)
	}
	return nil
}
