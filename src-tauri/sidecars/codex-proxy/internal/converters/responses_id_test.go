package converters

import (
	"testing"
	"time"
)

func TestGenerateResponseID_UsesCurrentTimestamp(t *testing.T) {
	first := generateResponseID()
	time.Sleep(2 * time.Millisecond)
	second := generateResponseID()

	if first == "resp_0" || second == "resp_0" {
		t.Fatalf("response id should not use placeholder timestamp: first=%s second=%s", first, second)
	}
	if first == second {
		t.Fatalf("response ids should change across calls: first=%s second=%s", first, second)
	}
}
