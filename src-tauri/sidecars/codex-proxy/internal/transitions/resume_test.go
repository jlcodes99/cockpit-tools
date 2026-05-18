package transitions

import "testing"

func TestRestoreAllAndReset(t *testing.T) {
	resetCalled := false
	result, err := RestoreAllAndReset(
		func() (int, error) { return 2, nil },
		func() { resetCalled = true },
	)
	if err != nil {
		t.Fatalf("RestoreAllAndReset() error = %v", err)
	}
	if !resetCalled {
		t.Fatal("resetCalled = false, want true")
	}
	if result.RestoredCount != 2 {
		t.Fatalf("RestoredCount = %d, want 2", result.RestoredCount)
	}
}
