package helps

import (
	"context"
	"net/http"
	"strings"
	"testing"

	internallogging "github.com/router-for-me/CLIProxyAPI/v7/internal/logging"
)

func TestClassifyTurnStateValueMatchesRelayBaseline(t *testing.T) {
	cases := []struct {
		name   string
		value  string
		length int
		class  string
	}{
		{"empty", "", 0, TurnStateClassMissing},
		{"blank", "   ", 0, TurnStateClassMissing},
		{"normal-292", strings.Repeat("x", 292), 292, TurnStateClassNormal},
		{"normal-332", strings.Repeat("x", 332), 332, TurnStateClassNormal},
		{"suspected-312", strings.Repeat("x", 312), 312, TurnStateClassSuspected},
		{"abnormal-short", strings.Repeat("x", 200), 200, TurnStateClassAbnormal},
		{"trimmed", " " + strings.Repeat("x", 332) + " ", 332, TurnStateClassNormal},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			length, class := ClassifyTurnStateValue(tc.value)
			if length != tc.length || class != tc.class {
				t.Fatalf("ClassifyTurnStateValue() = (%d, %q), want (%d, %q)", length, class, tc.length, tc.class)
			}
		})
	}
}

func TestTurnStateObservationRoundTripByRequestID(t *testing.T) {
	ctx := internallogging.WithRequestID(context.Background(), "turn-state-round-trip")
	headers := http.Header{}
	headers.Set(TurnStateHeaderName, strings.Repeat("x", 312))

	ObserveTurnStateFromHeaders(ctx, http.StatusOK, headers)

	observation, ok := TakeTurnStateObservation("turn-state-round-trip")
	if !ok {
		t.Fatal("expected observation for request id")
	}
	if observation.Length != 312 || observation.Class != TurnStateClassSuspected || observation.Status != http.StatusOK {
		t.Fatalf("unexpected observation: %+v", observation)
	}
	// 取出后即删除，避免同一请求重复上报。
	if _, ok := TakeTurnStateObservation("turn-state-round-trip"); ok {
		t.Fatal("observation should be consumed after Take")
	}
}

func TestTurnStateObservationMarksMissingHeader(t *testing.T) {
	ctx := internallogging.WithRequestID(context.Background(), "turn-state-missing")
	ObserveTurnStateFromHeaders(ctx, http.StatusForbidden, http.Header{})

	observation, ok := TakeTurnStateObservation("turn-state-missing")
	if !ok {
		t.Fatal("expected observation for request id")
	}
	if observation.Class != TurnStateClassMissing || observation.Length != 0 || observation.Status != http.StatusForbidden {
		t.Fatalf("unexpected observation: %+v", observation)
	}
}
