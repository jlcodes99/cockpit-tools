package main

import (
	"encoding/json"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
)

func TestSidecarRejectsOfficialBindingBeforeSourceOAuthRegistration(t *testing.T) {
	for _, tc := range []struct {
		name   string
		update func(map[string]any)
	}{
		{"active source binding", func(m map[string]any) { m["officialAccountBindings"] = map[string]string{"source": "registration"} }},
		{"paused binding", func(m map[string]any) {
			m["officialAccountBindings"] = map[string]string{"source": "registration"}
			m["pausedAccountIds"] = []string{"source"}
		}},
		{"empty grant does not become local", func(m map[string]any) { m["officialAccountBindings"] = map[string]string{"source": ""} }},
		{"missing source does not become local", func(m map[string]any) { m["officialAccountBindings"] = map[string]string{"missing": "registration"} }},
		{"legacy account", func(m map[string]any) {
			m["accounts"] = []map[string]any{{"id": "chatgpt_plan_registration", "upstreamApiKey": "unusable-marker"}}
		}},
		{"legacy scope without account", func(m map[string]any) {
			m["apiKeys"] = []map[string]any{{"id": "key", "key": "client", "enabled": true, "accountIds": []string{"chatgpt_plan_registration"}}}
		}},
		{"legacy image scope", func(m map[string]any) {
			m["apiKeys"] = []map[string]any{{"id": "key", "key": "client", "enabled": true, "accountIds": []string{"source"}, "imageGenerationAccountIds": []string{"chatgpt_plan_registration"}}}
		}},
		{"legacy route", func(m map[string]any) {
			m["apiKeys"] = []map[string]any{{"id": "key", "modelRouting": map[string]any{"routes": []map[string]any{{"providerAccountId": "chatgpt_plan_registration"}}}}}
		}},
		{"legacy paused member", func(m map[string]any) { m["pausedAccountIds"] = []string{"chatgpt_plan_registration"} }},
	} {
		t.Run(tc.name, func(t *testing.T) {
			payload := map[string]any{
				"accounts": []map[string]any{{"id": "source", "authId": "source.json", "authKind": "oauth"}},
				"apiKeys":  []map[string]any{{"id": "key", "key": "client", "enabled": true, "accountIds": []string{"source"}}},
			}
			tc.update(payload)
			body, err := json.Marshal(payload)
			if err != nil {
				t.Fatal(err)
			}
			path := filepath.Join(t.TempDir(), "manifest.json")
			if err := os.WriteFile(path, body, 0600); err != nil {
				t.Fatal(err)
			}
			loaded, err := loadManifest(path)
			if loaded != nil || err == nil || !strings.HasPrefix(err.Error(), officialAccountNativeRequired+":") {
				t.Fatalf("official route was accepted as source OAuth: manifest=%#v err=%v", loaded, err)
			}
			if strings.Contains(err.Error(), "registration") || strings.Contains(err.Error(), "unusable-marker") {
				t.Fatalf("error exposed grant or credential data: %v", err)
			}
		})
	}
}

func TestSidecarLocalSourceMembersRemainCompatibleWithEmptyOfficialBindings(t *testing.T) {
	for _, bindings := range []string{"", `,"officialAccountBindings":{}`, `,"officialAccountBindings":null`} {
		m := loadAutomaticRoutingManifest(t, `{"accounts":[{"id":"source","authId":"source.json","authKind":"oauth"}],"pausedAccountIds":["source"],"apiKeys":[{"id":"key","key":"client","enabled":true,"accountIds":["source"]}]`+bindings+`}`)
		if len(m.OfficialAccountBindings) != 0 || len(m.Accounts) != 1 || m.Accounts[0].ID != "source" || !reflect.DeepEqual(m.APIKeys[0].AccountIDs, []string{"source"}) || !reflect.DeepEqual(m.PausedAccountIDs, []string{"source"}) {
			t.Fatalf("local service IDs or scopes changed: %#v", m)
		}
	}
}
