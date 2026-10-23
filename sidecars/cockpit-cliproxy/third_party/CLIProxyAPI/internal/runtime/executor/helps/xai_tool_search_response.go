package helps

import (
	"fmt"
	"strings"

	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"
)

const xaiSearchMaxArguments = 4 * 1024 * 1024
const xaiSearchMaxPending = 256
const xaiSearchMaxTotalArguments = 8 * 1024 * 1024

type xaiSearchStreamCall struct {
	id        string
	callID    string
	index     int64
	hasIndex  bool
	arguments strings.Builder
}

// XAIClientToolSearchRestorer owns bounded buffering for one upstream response.
// Codex consumes the completed tool_search_call item, not function argument events.
type XAIClientToolSearchRestorer struct {
	mapping  *XAIClientToolSearch
	calls    map[string]*xaiSearchStreamCall
	byIndex  map[int64]*xaiSearchStreamCall
	buffered int
}

func NewXAIClientToolSearchRestorer(mapping *XAIClientToolSearch) *XAIClientToolSearchRestorer {
	return &XAIClientToolSearchRestorer{mapping: mapping, calls: make(map[string]*xaiSearchStreamCall), byIndex: make(map[int64]*xaiSearchStreamCall)}
}

func xaiSearchClientID(id string) string {
	if id == "" || strings.HasPrefix(id, "tsc_") {
		return id
	}
	return "tsc_" + strings.TrimPrefix(id, "fc_")
}

func (r *XAIClientToolSearchRestorer) find(data []byte) *xaiSearchStreamCall {
	for _, key := range []string{"item_id", "call_id", "item.id", "item.call_id"} {
		if call := r.calls[gjson.GetBytes(data, key).String()]; call != nil {
			return call
		}
	}
	if index := gjson.GetBytes(data, "output_index"); index.Exists() {
		return r.byIndex[index.Int()]
	}
	return nil
}

func (r *XAIClientToolSearchRestorer) record(data []byte) (*xaiSearchStreamCall, error) {
	item := gjson.GetBytes(data, "item")
	if item.Get("type").String() != "function_call" || item.Get("name").String() != r.mapping.Alias {
		return nil, nil
	}
	call := r.find(data)
	if call == nil {
		if len(r.calls) >= xaiSearchMaxPending*2 || len(r.byIndex) >= xaiSearchMaxPending {
			return nil, fmt.Errorf("too many pending tool_search calls")
		}
		call = &xaiSearchStreamCall{id: item.Get("id").String(), callID: item.Get("call_id").String()}
		if call.id != "" {
			r.calls[call.id] = call
		}
		if call.callID != "" {
			r.calls[call.callID] = call
		}
		if index := gjson.GetBytes(data, "output_index"); index.Exists() {
			call.index, call.hasIndex = index.Int(), true
			r.byIndex[call.index] = call
		}
	}
	if args := item.Get("arguments"); args.Exists() && args.String() != "" {
		if len(args.String()) > xaiSearchMaxArguments || r.buffered-call.arguments.Len()+len(args.String()) > xaiSearchMaxTotalArguments {
			return nil, fmt.Errorf("tool_search arguments exceed buffer limit")
		}
		r.buffered += len(args.String()) - call.arguments.Len()
		call.arguments.Reset()
		call.arguments.WriteString(args.String())
	}
	return call, nil
}

func (r *XAIClientToolSearchRestorer) clear(call *xaiSearchStreamCall) {
	r.buffered -= call.arguments.Len()
	delete(r.calls, call.id)
	delete(r.calls, call.callID)
	if call.hasIndex {
		delete(r.byIndex, call.index)
	}
}

// Apply restores SSE data or WS JSON using the same item protocol. nil absorbs
// the proxy function's argument events without inventing client event types.
func (r *XAIClientToolSearchRestorer) Apply(data []byte) ([]byte, error) {
	if r == nil || r.mapping == nil || !gjson.ValidBytes(data) {
		return data, nil
	}
	switch gjson.GetBytes(data, "type").String() {
	case "response.output_item.added", "response.output_item.done":
		call, err := r.record(data)
		if err != nil || call == nil {
			return data, err
		}
		args := call.arguments.String()
		if gjson.GetBytes(data, "type").String() == "response.output_item.added" {
			args = "{}"
		} else {
			defer r.clear(call)
		}
		return r.mapping.restoreItem(data, "item", args)
	case "response.function_call_arguments.delta", "response.function_call_arguments.done":
		call := r.find(data)
		if call == nil {
			return data, nil
		}
		if gjson.GetBytes(data, "type").String() == "response.function_call_arguments.done" {
			if args := gjson.GetBytes(data, "arguments"); args.Exists() {
				if len(args.String()) > xaiSearchMaxArguments || r.buffered-call.arguments.Len()+len(args.String()) > xaiSearchMaxTotalArguments {
					return nil, fmt.Errorf("tool_search arguments exceed buffer limit")
				}
				r.buffered += len(args.String()) - call.arguments.Len()
				call.arguments.Reset()
				call.arguments.WriteString(args.String())
			}
		} else {
			delta := gjson.GetBytes(data, "delta").String()
			if call.arguments.Len()+len(delta) > xaiSearchMaxArguments || r.buffered+len(delta) > xaiSearchMaxTotalArguments {
				return nil, fmt.Errorf("tool_search arguments exceed buffer limit")
			}
			r.buffered += len(delta)
			call.arguments.WriteString(delta)
		}
		return nil, nil
	case "response.completed", "response.done", "response.incomplete", "response.failed", "response.cancelled", "response.canceled":
		defer func() { r.buffered = 0 }()
		defer clear(r.calls)
		defer clear(r.byIndex)
		for index, item := range gjson.GetBytes(data, "response.output").Array() {
			if item.Get("type").String() != "function_call" || item.Get("name").String() != r.mapping.Alias || item.Get("arguments").String() != "" {
				continue
			}
			if call := r.calls[item.Get("id").String()]; call != nil && call.arguments.Len() > 0 {
				data, _ = sjson.SetBytes(data, fmt.Sprintf("response.output.%d.arguments", index), call.arguments.String())
			}
		}
		return r.mapping.RestorePayload(data)
	}
	return data, nil
}

// RestorePayload accepts both Responses objects and terminal event envelopes.
func (m *XAIClientToolSearch) RestorePayload(data []byte) ([]byte, error) {
	if m == nil || !gjson.ValidBytes(data) {
		return data, nil
	}
	for _, path := range []string{"item", "output", "response.output"} {
		value := gjson.GetBytes(data, path)
		if value.IsArray() {
			for index, item := range value.Array() {
				if item.Get("type").String() == "function_call" && item.Get("name").String() == m.Alias {
					var err error
					data, err = m.restoreItem(data, fmt.Sprintf("%s.%d", path, index), item.Get("arguments").String())
					if err != nil {
						return nil, err
					}
				}
			}
		} else if value.Get("type").String() == "function_call" && value.Get("name").String() == m.Alias {
			var err error
			data, err = m.restoreItem(data, path, value.Get("arguments").String())
			if err != nil {
				return nil, err
			}
		}
	}
	return data, nil
}

func (m *XAIClientToolSearch) restoreItem(data []byte, path, args string) ([]byte, error) {
	if !gjson.Valid(args) || !gjson.Parse(args).IsObject() {
		return nil, fmt.Errorf("upstream tool_search arguments must be a JSON object")
	}
	var err error
	data, err = sjson.SetBytes(data, path+".type", "tool_search_call")
	if err != nil {
		return nil, err
	}
	data, _ = sjson.SetBytes(data, path+".execution", "client")
	data, _ = sjson.SetRawBytes(data, path+".arguments", []byte(args))
	if id := gjson.GetBytes(data, path+".id").String(); id != "" {
		data, _ = sjson.SetBytes(data, path+".id", xaiSearchClientID(id))
	}
	for _, field := range []string{"name", "namespace"} {
		data, _ = sjson.DeleteBytes(data, path+"."+field)
	}
	return data, nil
}
