package helps

import (
	"context"
	"errors"
	"testing"
	"time"

	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/usage"
)

type codexUnsentUsageSink struct{ records chan usage.Record }

func (s codexUnsentUsageSink) HandleUsage(_ context.Context, record usage.Record) {
	if s.records != nil && record.Model == "codex-unsent-usage-regression" {
		s.records <- record
	}
}

func TestCodexUnsentFailureDoesNotCreateUsageRecordOrConsumeLaterOutcome(t *testing.T) {
	const pluginName = "codex-unsent-usage-regression"
	records := make(chan usage.Record, 8)
	usage.RegisterNamedPlugin(pluginName, codexUnsentUsageSink{records})
	t.Cleanup(func() { usage.RegisterNamedPlugin(pluginName, codexUnsentUsageSink{}) })
	usage.StartDefault(context.Background())
	ctx := cliproxyexecutor.WithUpstreamAttemptTracker(context.Background())
	reporter := NewUsageReporter(ctx, "codex", pluginName, nil)
	local := errors.New("local candidate rejected before upstream send")
	reporter.PublishFailure(ctx, local)
	reporter.TrackFailure(ctx, &local)
	reporter.PublishFailureWithDetail(ctx, usage.Detail{}, local)

	// A later real send must still publish exactly one outcome. A sentinel on
	// the same ordered queue proves all earlier failures have been processed.
	cliproxyexecutor.MarkUpstreamAttempt(ctx)
	reporter.PublishFailure(ctx, errors.New("real upstream quota rejection"))
	usage.PublishRecord(ctx, usage.Record{Model: pluginName, Source: "sentinel"})
	count := 0
	for {
		select {
		case record := <-records:
			if record.Source == "sentinel" {
				if count != 1 {
					t.Fatalf("failed usage records = %d, want only the actual upstream failure", count)
				}
				return
			}
			count++
			if !record.Failed || record.Fail.Body != "real upstream quota rejection" {
				t.Fatalf("unsent failure leaked into usage sink: %+v", record)
			}
		case <-time.After(3 * time.Second):
			t.Fatal("usage queue did not publish the actual upstream outcome")
		}
	}
}
