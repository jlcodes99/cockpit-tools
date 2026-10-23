package auth

import "testing"

type sharedXAIExecutor struct {
	*replaceAwareExecutor
	store *int
}

func (e *sharedXAIExecutor) SharesExecutionSessionsWith(other ProviderExecutor) bool {
	next, ok := other.(*sharedXAIExecutor)
	return ok && e.store == next.store
}

func TestXAIExecutorRebindKeepsOnlySharedSessions(t *testing.T) {
	for _, shared := range []bool{true, false} {
		firstStore, secondStore := 1, 2
		previous := &sharedXAIExecutor{replaceAwareExecutor: &replaceAwareExecutor{id: "xai"}, store: &firstStore}
		next := &sharedXAIExecutor{replaceAwareExecutor: &replaceAwareExecutor{id: "xai"}, store: &secondStore}
		if shared {
			next.store = previous.store
		}
		manager := NewManager(nil, nil, nil)
		manager.RegisterExecutor(previous)
		manager.RegisterExecutor(next)
		closed := previous.ClosedSessionIDs()
		if shared && len(closed) != 0 {
			t.Fatalf("shared sessions closed: %v", closed)
		}
		if !shared && (len(closed) != 1 || closed[0] != CloseAllExecutionSessionsID) {
			t.Fatalf("replaced independent sessions remain open: %v", closed)
		}
	}
}
