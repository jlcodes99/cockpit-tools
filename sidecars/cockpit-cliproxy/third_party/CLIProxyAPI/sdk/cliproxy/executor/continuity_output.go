package executor

import (
	"bytes"
	"encoding/json"
	"reflect"
	"strconv"
	"strings"

	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"
)

// continuityOutput belongs to one attempt and is accessed only under its mutex.
// A terminal can omit output already delivered as complete upstream done items.
// Normalize only that omission for the durable observer, whose digest checks still
// verify every item. This never changes upstream bytes or client-facing repairs.
type continuityOutput struct {
	items   map[uint64]json.RawMessage
	invalid bool
}

func (o *continuityOutput) discard() {
	o.items = nil
	o.invalid = true
}

func (o *continuityOutput) observe(event []byte) []byte {
	value := gjson.ParseBytes(event)
	kind := value.Get("type").String()
	response := value.Get("response")
	status := response.Get("status").String()
	if continuityHasError(value) || continuityHasError(response) ||
		kind == "error" || kind == "response.error" || kind == "response.failed" || kind == "response.incomplete" || kind == "response.cancelled" ||
		status == "failed" || status == "incomplete" || status == "cancelled" {
		o.discard()
		return event
	}
	if kind == "response.output_item.done" {
		o.collect(value)
		return event
	}
	if kind != "response.completed" && kind != "response.done" {
		return event
	}
	defer o.discard()
	id := response.Get("id")
	output := response.Get("output")
	if o.invalid || len(o.items) == 0 || !response.IsObject() || status != "completed" ||
		id.Type != gjson.String || strings.TrimSpace(id.String()) == "" ||
		output.Exists() && (!output.IsArray() || len(output.Array()) != 0) {
		return event
	}
	var buffer bytes.Buffer
	buffer.WriteByte('[')
	for index := uint64(0); index < uint64(len(o.items)); index++ {
		item, ok := o.items[index]
		if !ok {
			return event
		}
		if index > 0 {
			buffer.WriteByte(',')
		}
		buffer.Write(item)
	}
	buffer.WriteByte(']')
	normalized, err := sjson.SetRawBytes(event, "response.output", buffer.Bytes())
	if err != nil {
		return event
	}
	return normalized
}

func continuityHasError(value gjson.Result) bool {
	err := value.Get("error")
	return err.Exists() && err.Type != gjson.Null
}

func (o *continuityOutput) collect(event gjson.Result) {
	if o.invalid {
		return
	}
	index := event.Get("output_index")
	number, err := strconv.ParseUint(index.Raw, 10, 64)
	item := event.Get("item")
	if index.Type != gjson.Number || err != nil || number >= 4096 || !continuityCompleteOutputItem(item) {
		o.discard()
		return
	}
	if previous, exists := o.items[number]; exists {
		if !continuitySameJSON(previous, []byte(item.Raw)) {
			o.discard()
		}
		return
	}
	if o.items == nil {
		o.items = make(map[uint64]json.RawMessage)
	}
	o.items[number] = append(json.RawMessage(nil), item.Raw...)
}

func continuitySameJSON(first, second []byte) bool {
	if bytes.Equal(first, second) {
		return true
	}
	decode := func(raw []byte) (any, error) {
		var value any
		decoder := json.NewDecoder(bytes.NewReader(raw))
		decoder.UseNumber()
		err := decoder.Decode(&value)
		return value, err
	}
	a, errA := decode(first)
	b, errB := decode(second)
	return errA == nil && errB == nil && reflect.DeepEqual(a, b)
}

// Reject incomplete done items before synthesizing a transcript. Unknown fields
// remain byte-preserved; the durable hook retains its full replay validation.
func continuityCompleteOutputItem(item gjson.Result) bool {
	id, kind, status := item.Get("id"), item.Get("type"), item.Get("status")
	if !item.IsObject() || id.Type != gjson.String || strings.TrimSpace(id.String()) == "" ||
		kind.Type != gjson.String || status.Exists() && status.String() != "completed" {
		return false
	}
	stringField := func(name string) bool { return item.Get(name).Type == gjson.String }
	jsonArguments := func() bool {
		arguments := item.Get("arguments")
		return arguments.Type == gjson.String && json.Valid([]byte(arguments.String()))
	}
	switch kind.String() {
	case "message":
		role := item.Get("role").String()
		return (role == "assistant" || role == "user" || role == "developer" || role == "system") && continuityCompleteContent(item.Get("content"))
	case "agent_message":
		if !stringField("author") || !stringField("recipient") || !item.Get("content").IsArray() || len(item.Get("content").Array()) == 0 {
			return false
		}
		for _, part := range item.Get("content").Array() {
			switch part.Get("type").String() {
			case "input_text":
				if part.Get("text").Type != gjson.String {
					return false
				}
			case "encrypted_content":
				if part.Get("encrypted_content").Type != gjson.String || part.Get("encrypted_content").String() == "" {
					return false
				}
			default:
				return false
			}
		}
		return true
	case "reasoning":
		return stringField("encrypted_content") && item.Get("encrypted_content").String() != "" ||
			item.Get("summary").IsArray() || item.Get("content").IsArray()
	case "function_call":
		return stringField("call_id") && stringField("name") && item.Get("name").String() != "" && jsonArguments()
	case "custom_tool_call":
		return stringField("call_id") && stringField("name") && item.Get("name").String() != "" && stringField("input")
	case "function_call_output", "custom_tool_call_output":
		return continuityCompleteContent(item.Get("output"))
	case "additional_tools", "tool_search_output":
		tools := item.Get("tools")
		if !tools.IsArray() {
			return false
		}
		for _, tool := range tools.Array() {
			if !tool.IsObject() {
				return false
			}
		}
		return true
	case "compaction", "compaction_summary", "context_compaction":
		return stringField("encrypted_content") && item.Get("encrypted_content").String() != ""
	case "web_search_call", "local_shell_call":
		return item.Get("action").IsObject()
	case "image_generation_call":
		return stringField("result")
	case "tool_search_call":
		return item.Get("arguments").Exists()
	case "configuration_update":
		// The protocol defines this as an input control without an ID. Retain
		// replay support only if an upstream supplies a complete identified item.
		return item.Get("reasoning").IsObject()
	default:
		return false
	}
}

func continuityCompleteContent(content gjson.Result) bool {
	if content.Type == gjson.String {
		return true
	}
	if !content.IsArray() || len(content.Array()) == 0 {
		return false
	}
	for _, part := range content.Array() {
		field := "text"
		switch part.Get("type").String() {
		case "input_text", "output_text", "text":
		case "refusal":
			field = "refusal"
		default:
			return false
		}
		if part.Get(field).Type != gjson.String {
			return false
		}
	}
	return true
}
