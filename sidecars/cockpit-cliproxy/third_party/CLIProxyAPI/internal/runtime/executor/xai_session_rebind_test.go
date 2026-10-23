package executor

import (
	"testing"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
)

func TestXAIExecutorSharingRequiresBothSessionStores(t *testing.T) {
	previous, next := NewXAIWebsocketsExecutor(&config.Config{}), NewXAIWebsocketsExecutor(&config.Config{})
	if !previous.SharesExecutionSessionsWith(next) {
		t.Fatal("default XAI stores should be shared")
	}
	if !previous.SharesExecutionSessionsWith(NewXAIAutoExecutor(&config.Config{})) {
		t.Fatal("auto executor must expose the shared stores")
	}
	next.store = &codexWebsocketSessionStore{sessions: make(map[string]*codexWebsocketSession)}
	if previous.SharesExecutionSessionsWith(next) {
		t.Fatal("different transport stores treated as shared")
	}
	next.store = previous.store
	next.idStore = &xaiWebsocketIDStateStore{sessions: make(map[string]*xaiWebsocketIDState)}
	if previous.SharesExecutionSessionsWith(next) {
		t.Fatal("different transcript stores treated as shared")
	}
}

func TestXAISharedExecutorAccountRemovalStillClosesSessions(t *testing.T) {
	e := NewXAIWebsocketsExecutor(&config.Config{})
	const id = "xai-removal-rebind-test"
	sess := e.getOrCreateSession(id)
	sess.connMu.Lock()
	sess.authID = id
	sess.connMu.Unlock()
	getXAIWebsocketIDState(e.idStore, id)
	CloseXAIWebsocketSessionsForAuthID(id, "auth_removed")
	e.store.mu.Lock()
	remaining := e.store.sessions[id]
	e.store.mu.Unlock()
	e.idStore.mu.Lock()
	state := e.idStore.sessions[id]
	e.idStore.mu.Unlock()
	if remaining != nil || state != nil {
		t.Fatal("removed account retained transport or transcript session")
	}
}
