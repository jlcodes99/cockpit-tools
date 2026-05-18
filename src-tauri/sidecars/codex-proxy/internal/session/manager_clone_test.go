package session

import (
	"testing"
	"time"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/types"
)

func TestGetSession_ReturnsClonedSession(t *testing.T) {
	sm := NewSessionManager(time.Hour, 100, 100000)
	sess, err := sm.GetOrCreateSession("")
	if err != nil {
		t.Fatalf("GetOrCreateSession() err = %v", err)
	}

	if err := sm.AppendMessage(sess.ID, types.ResponsesItem{
		Type: "message",
		Role: "user",
		Content: []types.ContentBlock{{
			Type: "input_text",
			Text: "hello",
		}},
	}, 1); err != nil {
		t.Fatalf("AppendMessage() err = %v", err)
	}
	if err := sm.UpdateLastResponseID(sess.ID, "resp_1"); err != nil {
		t.Fatalf("UpdateLastResponseID() err = %v", err)
	}

	cloned, err := sm.GetSession(sess.ID)
	if err != nil {
		t.Fatalf("GetSession() err = %v", err)
	}

	cloned.LastResponseID = "resp_modified"
	cloned.Messages[0].Role = "assistant"

	fetchedAgain, err := sm.GetSession(sess.ID)
	if err != nil {
		t.Fatalf("GetSession() second err = %v", err)
	}

	if fetchedAgain.LastResponseID != "resp_1" {
		t.Fatalf("LastResponseID = %s, want resp_1", fetchedAgain.LastResponseID)
	}
	if fetchedAgain.Messages[0].Role != "user" {
		t.Fatalf("Messages[0].Role = %s, want user", fetchedAgain.Messages[0].Role)
	}
}
