package models

import (
	"reflect"
	"testing"
)

func TestReserveFallbackUsesLunaCapabilitiesForEachClientVersion(t *testing.T) {
	for _, version := range []string{"", "0.98.0", "0.146.0"} {
		t.Run(version, func(t *testing.T) {
			response := BuildResponseForClient([]map[string]any{{"id": "gpt-5.6-luna"}, {"id": "gpt-reserve"}}, nil, false, version)
			models := response["models"].([]map[string]any)
			if len(models) != 2 {
				t.Fatalf("models = %d, want 2", len(models))
			}
			luna, reserve := models[0], models[1]
			if reserve["slug"] != "gpt-reserve" || reserve["visibility"] != "list" || reserve["display_name"] != "GPT-5.6 Reserve" {
				t.Fatalf("Reserve identity/visibility = %v/%v", reserve["slug"], reserve["visibility"])
			}
			for field, value := range luna {
				if field != "slug" && field != "visibility" && field != "display_name" && !reflect.DeepEqual(value, reserve[field]) {
					t.Errorf("Reserve field %s differs from Luna", field)
				}
			}
			// Reserve 继承 Luna 模板的上下文，就必须同时继承它的 90% 压缩阈值，
			// 目录里不允许出现「有窗口但没压缩阈值」的声明。
			if got, want := intModelValue(reserve, "auto_compact_token_limit"), intModelValue(luna, "auto_compact_token_limit"); got != 272000*90/100 || got != want {
				t.Fatalf("Reserve auto_compact_token_limit = %d, want Luna value %d", got, want)
			}
		})
	}
}

func TestReserveFallbackPrefersDedicatedTemplateWhenAvailable(t *testing.T) {
	templates := map[string]map[string]any{
		"gpt-5.6-luna": {"context_window": 272000},
		"gpt-reserve":  {"context_window": 123456},
	}
	got, ok := lookupCodexClientModelTemplate(templates, "gpt-reserve")
	if !ok || got["context_window"] != 123456 {
		t.Fatalf("dedicated Reserve metadata must win: %#v", got)
	}
}
