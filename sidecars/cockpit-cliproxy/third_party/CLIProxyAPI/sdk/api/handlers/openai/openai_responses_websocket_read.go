package openai

import (
	"context"
	"errors"
	"sync/atomic"
	"time"

	"github.com/gorilla/websocket"
)

const (
	responsesWebsocketInboundQueueSize = 8
	responsesWebsocketInboundMaxBytes  = 32 << 20
)

type responsesWebsocketInboundMessage struct {
	kind    int
	payload []byte
}

// Read independently of bootstrap and output forwarding, so a real downstream
// disconnect cancels an upstream that has not produced any frame yet. Messages
// still execute serially. Excess pipelining terminates the connection instead of
// occupying unbounded memory or preventing the reader from observing closure.
type responsesWebsocketReadPump struct {
	ctx      context.Context
	cancel   context.CancelCauseFunc
	conn     *websocket.Conn
	writer   *responsesWebsocketWriter
	messages chan responsesWebsocketInboundMessage
	done     chan struct{}
	bytes    atomic.Int64
}

func newResponsesWebsocketReadPump(parent context.Context, conn *websocket.Conn, writers ...*responsesWebsocketWriter) *responsesWebsocketReadPump {
	ctx, cancel := context.WithCancelCause(parent)
	writer := newResponsesWebsocketWriter(conn)
	if len(writers) > 0 && writers[0] != nil {
		writer = writers[0]
	}
	pump := &responsesWebsocketReadPump{ctx: ctx, cancel: cancel, conn: conn,
		writer: writer, messages: make(chan responsesWebsocketInboundMessage, responsesWebsocketInboundQueueSize), done: make(chan struct{})}
	go pump.read()
	return pump
}

func (p *responsesWebsocketReadPump) read() {
	defer close(p.done)
	for {
		kind, payload, errRead := p.conn.ReadMessage()
		if errRead != nil {
			p.cancel(errRead)
			return
		}
		queuedBytes := p.bytes.Add(int64(len(payload)))
		// Preserve the existing single-frame acceptance. The budget limits
		// additional pipelined messages, not a lone valid large request.
		if queuedBytes > responsesWebsocketInboundMaxBytes && queuedBytes != int64(len(payload)) {
			p.failQueue("responses websocket inbound queue byte limit exceeded")
			return
		}
		select {
		case p.messages <- responsesWebsocketInboundMessage{kind: kind, payload: payload}:
		case <-p.ctx.Done():
			return
		default:
			p.failQueue("responses websocket inbound queue full")
			return
		}
	}
}

func (p *responsesWebsocketReadPump) next() (int, []byte, error) {
	if p.ctx.Err() != nil {
		return 0, nil, context.Cause(p.ctx)
	}
	select {
	case <-p.ctx.Done():
		return 0, nil, context.Cause(p.ctx)
	case message := <-p.messages:
		p.bytes.Add(-int64(len(message.payload)))
		if p.ctx.Err() != nil {
			return 0, nil, context.Cause(p.ctx)
		}
		return message.kind, message.payload, nil
	}
}

func (p *responsesWebsocketReadPump) failQueue(reason string) {
	// Cancel first so bootstrap cannot wait on a downstream writer. A close
	// frame is best effort, using the writer's existing non-waiting close policy.
	p.cancel(errors.New(reason))
	w := p.writer
	if w.closing.CompareAndSwap(false, true) && w.writeMu.TryLock() {
		_ = w.conn.WriteControl(websocket.CloseMessage, websocket.FormatCloseMessage(websocket.ClosePolicyViolation, reason), time.Now().Add(100*time.Millisecond))
		w.writeMu.Unlock()
	}
	_ = p.conn.Close()
}

func (p *responsesWebsocketReadPump) stop() {
	p.cancel(context.Canceled)
	_ = p.conn.Close()
	<-p.done
	for {
		select {
		case <-p.messages:
		default:
			p.bytes.Store(0)
			return
		}
	}
}
