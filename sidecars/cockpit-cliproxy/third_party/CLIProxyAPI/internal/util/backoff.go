package util

import (
	"context"
	"math/rand"
	"time"
)

func SleepContext(ctx context.Context, delay time.Duration) error {
	if delay <= 0 {
		return nil
	}
	timer := time.NewTimer(delay)
	defer timer.Stop()
	select {
	case <-timer.C:
		return nil
	case <-ctx.Done():
		return ctx.Err()
	}
}

func BackoffDelay(attempt int, base, max time.Duration) time.Duration {
	if attempt < 1 {
		attempt = 1
	}
	if base <= 0 {
		return 0
	}
	delay := base
	for i := 1; i < attempt; i++ {
		if delay >= max && max > 0 {
			return max
		}
		delay *= 2
	}
	if max > 0 && delay > max {
		delay = max
	}
	if delay > 0 {
		delay += time.Duration(rand.Int63n(int64(delay / 4)))
		if max > 0 && delay > max {
			delay = max
		}
	}
	return delay
}
