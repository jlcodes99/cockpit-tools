package helps

import (
	"context"

	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"
)

// ApplyServiceTierDefault replaces only the host default after static payload
// rules. Client-supplied tiers remain authoritative, and unconfigured hosts are
// unchanged. The original payload must not contain defaults injected by the host.
func ApplyServiceTierDefault(ctx context.Context, body, original []byte) []byte {
	tier, ok := cliproxyexecutor.ServiceTierDefaultSnapshot(ctx)
	if !ok || len(body) == 0 || gjson.GetBytes(original, "service_tier").Exists() {
		return body
	}
	var updated []byte
	var err error
	if tier == "" {
		updated, err = sjson.DeleteBytes(body, "service_tier")
	} else {
		updated, err = sjson.SetBytes(body, "service_tier", tier)
	}
	if err != nil {
		return body
	}
	return updated
}
