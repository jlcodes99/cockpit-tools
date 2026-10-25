package executor

import (
	"context"
	"errors"
	"net/http"
	"testing"
)

func TestContinuityOnlyTrustedUnsentPreparationCanReleaseAnAccountPin(t *testing.T) {
	for _, tc := range []struct {
		name, want string
		unsent     bool
		observed   bool
	}{
		{name: "local-preparation", unsent: true, want: "continuity_upstream_not_sent"},
		{name: "uncertain-transport", want: "continuity_upstream_failed"},
		{name: "observed-upstream", unsent: true, observed: true, want: "continuity_upstream_failed"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			hook := &continuityTestHook{}
			ctx, _, err := PrepareContinuity(WithContinuityHook(context.Background(), hook), ContinuityPrepareRequest{Body: []byte(`{"input":[]}`)})
			if err != nil {
				t.Fatal(err)
			}
			attempt, _, _, err := BindContinuity(ctx, "account-a", nil, []byte(`{"input":[]}`))
			if err != nil {
				t.Fatal(err)
			}
			if tc.observed {
				if err := attempt.Observe(ctx, http.Header{}, nil); err != nil {
					t.Fatal(err)
				}
			}
			cause := errors.New("synthetic preparation or transport failure")
			if tc.unsent {
				err = attempt.FinishNotSent(ctx, cause)
			} else {
				err = attempt.Finish(ctx, cause)
			}
			if err != nil {
				t.Fatal(err)
			}
			if len(hook.finished) != 1 || hook.finished[0].Status != "failed" || hook.finished[0].ErrorCode != tc.want {
				t.Fatalf("finish=%+v, want failed with %s", hook.finished, tc.want)
			}
		})
	}
}
