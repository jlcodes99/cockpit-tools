package helps

import (
	"context"
	"testing"

	coreexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	"github.com/tidwall/gjson"
)

func TestApplyServiceTierDefault(t *testing.T) {
	for _, tc := range []struct {
		name, tier, original, body, want string
		configured                       bool
	}{
		{"unconfigured", "", `{}`, `{"service_tier":"priority"}`, "priority", false},
		{"fast", "priority", `{}`, `{}`, "priority", true},
		{"standard-removes-startup-default", "", `{}`, `{"service_tier":"priority"}`, "", true},
		{"explicit-default", "priority", `{"service_tier":"default"}`, `{"service_tier":"default"}`, "default", true},
		{"explicit-flex", "priority", `{"service_tier":"flex"}`, `{"service_tier":"flex"}`, "flex", true},
		{"translated-explicit-default", "priority", `{"service_tier":"default"}`, `{}`, "", true},
		{"explicit-fast-in-standard", "", `{"service_tier":"priority"}`, `{"service_tier":"priority"}`, "priority", true},
	} {
		t.Run(tc.name, func(t *testing.T) {
			ctx := context.Background()
			if tc.configured {
				ctx = coreexecutor.SnapshotServiceTierDefault(coreexecutor.WithServiceTierDefaultProvider(ctx, func() string { return tc.tier }))
			}
			got := ApplyServiceTierDefault(ctx, []byte(tc.body), []byte(tc.original))
			if tier := gjson.GetBytes(got, "service_tier").String(); tier != tc.want {
				t.Fatalf("tier = %q, want %q", tier, tc.want)
			}
		})
	}
}
