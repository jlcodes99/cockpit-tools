package openai

import (
	"bytes"
	"context"
	"crypto/sha256"
	"errors"
	"fmt"
	"net/http"
	"strings"
	"sync"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/interfaces"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/runtime/executor/helps"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	"github.com/tidwall/gjson"
)

// responsesLocalInterrupt cancels an in-flight HTTP turn. A websocket upstream
// does not use it; that interrupt is written to the existing socket instead.
type responsesLocalInterrupt struct {
	mu          sync.Mutex
	active      bool
	responseID  string
	turn        context.Context
	turnCancel  context.CancelFunc
	frames      chan []byte
	terminalIDs helps.TerminalResponseIDs
}

func newResponsesLocalInterrupt() *responsesLocalInterrupt {
	return &responsesLocalInterrupt{frames: make(chan []byte, 1)}
}

func (s *responsesLocalInterrupt) begin(parent context.Context) {
	if s == nil {
		return
	}
	s.mu.Lock()
	s.active = true
	s.responseID = ""
	s.turn, s.turnCancel = context.WithCancel(parent)
	s.mu.Unlock()
}

func (s *responsesLocalInterrupt) end() {
	if s == nil {
		return
	}
	s.mu.Lock()
	s.active = false
	s.responseID = ""
	if s.turnCancel != nil {
		s.turnCancel()
		s.turnCancel = nil
	}
	select {
	case <-s.frames:
	default:
	}
	s.mu.Unlock()
}

func (s *responsesLocalInterrupt) deliver(turn context.Context, payload []byte) bool {
	if s == nil {
		return false
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	responseID := gjson.GetBytes(payload, "response_id")
	if !s.active || s.turn != turn || s.responseID == "" || responseID.Type != gjson.String || responseID.String() != s.responseID {
		return false
	}
	select {
	case s.frames <- bytes.Clone(payload):
	default:
	}
	return true
}

func (s *responsesLocalInterrupt) framesChan() <-chan []byte {
	if s == nil {
		return nil
	}
	return s.frames
}

// observeResponseID binds local cancellation to the actual current response.
func (s *responsesLocalInterrupt) observeResponseID(payload []byte) {
	if s == nil {
		return
	}
	id := gjson.GetBytes(payload, "response.id")
	if id.Type != gjson.String || id.String() == "" {
		return
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	if !s.active {
		return
	}
	if gjson.GetBytes(payload, "type").String() == "response.created" {
		s.responseID = id.String()
	} else if terminalID := helps.TerminalResponseID(payload); terminalID != "" {
		s.terminalIDs.Add(terminalID)
	}

}

// Completed IDs belong only to this downstream connection and never cancel another turn.
func (s *responsesLocalInterrupt) alreadyTerminal(payload []byte) bool {
	if s == nil {
		return false
	}
	id := gjson.GetBytes(payload, "response_id")
	if id.Type != gjson.String {
		return false
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	return (!s.active || id.String() != s.responseID) && s.terminalIDs.Contains(id.String())
}

func (s *responsesLocalInterrupt) interruptTurn() (context.Context, bool) {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.turn, s.active
}

type responsesWebsocketInterruptKey struct {
	turn        context.Context
	payloadHash [32]byte
}

type responsesWebsocketInterruptJob struct {
	payload []byte
	turn    context.Context
	idle    bool
	key     responsesWebsocketInterruptKey
}

// One worker and one waiting frame keep control writes off the downstream reader.
// Jobs retain the turn context observed at receipt, never a later turn's context.
func (p *responsesWebsocketReadPump) queueInterrupt(payload []byte) bool {
	if len(payload) > responsesWebsocketInboundMaxBytes {
		p.failQueue("responses websocket interrupt byte limit exceeded")
		return false
	}
	job := responsesWebsocketInterruptJob{payload: payload, turn: p.ctx, key: responsesWebsocketInterruptKey{payloadHash: sha256.Sum256(payload)}}
	if p.local != nil {
		var active bool
		job.turn, active = p.local.interruptTurn()
		job.idle = !active
	}
	job.key.turn = job.turn
	p.interruptMu.Lock()
	if _, found := p.interruptPending[job.key]; found || (job.idle && p.lastInterruptSent && p.lastInterruptKey == job.key) {
		p.interruptMu.Unlock()
		return true
	}
	if p.interruptBytes+int64(len(payload)) > responsesWebsocketInboundMaxBytes {
		p.interruptMu.Unlock()
		p.failQueue("responses websocket interrupt byte limit exceeded")
		return false
	}
	p.interruptPending[job.key] = struct{}{}
	p.interruptBytes += int64(len(payload))
	p.interruptMu.Unlock()
	select {
	case p.interruptJobs <- job:
		return true
	case <-p.ctx.Done():
		p.finishInterrupt(job)
		return false
	default:
		p.finishInterrupt(job)
		p.failQueue("responses websocket interrupt queue full")
		return false
	}
}

func (p *responsesWebsocketReadPump) runInterrupts() {
	defer close(p.interruptDone)
	for {
		select {
		case <-p.ctx.Done():
			return
		case job := <-p.interruptJobs:
			p.handleInterrupt(job)
			p.finishInterrupt(job)
		}
	}
}

func (p *responsesWebsocketReadPump) finishInterrupt(job responsesWebsocketInterruptJob) {
	p.interruptMu.Lock()
	if _, found := p.interruptPending[job.key]; found || (job.idle && p.lastInterruptSent && p.lastInterruptKey == job.key) {
		delete(p.interruptPending, job.key)
		p.interruptBytes -= int64(len(job.payload))
	}
	p.interruptMu.Unlock()
}

func (p *responsesWebsocketReadPump) handleInterrupt(job responsesWebsocketInterruptJob) {
	if p.ctx.Err() != nil {
		return
	}
	responseID := gjson.GetBytes(job.payload, "response_id")
	validID := responseID.Type == gjson.String && strings.TrimSpace(responseID.String()) != ""
	if validID && p.local.alreadyTerminal(job.payload) {
		p.interruptMu.Lock()
		p.lastInterruptKey, p.lastInterruptSent = job.key, true
		p.interruptMu.Unlock()
		return
	}
	errInterrupt := cliproxyexecutor.ErrNoActiveUpstreamWebsocket
	if !validID {
		errInterrupt = fmt.Errorf("response.interrupt requires response_id")
	} else if !job.idle && job.turn.Err() == nil {
		ctx, cancel := context.WithCancel(job.turn)
		stop := context.AfterFunc(p.ctx, cancel)
		errInterrupt = p.interrupt(ctx, job.payload)
		stop()
		cancel()
	}
	if p.ctx.Err() != nil {
		return
	}
	if errInterrupt == nil || p.local.alreadyTerminal(job.payload) {
		p.interruptMu.Lock()
		p.lastInterruptKey, p.lastInterruptSent = job.key, true
		p.interruptMu.Unlock()
		return
	}
	if errors.Is(errInterrupt, cliproxyexecutor.ErrNoActiveUpstreamWebsocket) && !job.idle && p.local.deliver(job.turn, job.payload) {
		return
	}
	_, errWrite := writeResponsesWebsocketError(p.writer, nil, &interfaces.ErrorMessage{StatusCode: http.StatusBadRequest, Error: errInterrupt})
	if errWrite != nil {
		p.cancel(errWrite)
		_ = p.conn.Close()
	}
}
