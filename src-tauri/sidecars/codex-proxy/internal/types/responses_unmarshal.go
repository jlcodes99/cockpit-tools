package types

import "encoding/json"

type responsesRequestAlias ResponsesRequest

func (r *ResponsesRequest) UnmarshalJSON(data []byte) error {
	type rawRequest responsesRequestAlias
	var req rawRequest
	if err := json.Unmarshal(data, &req); err != nil {
		return err
	}
	*r = ResponsesRequest(req)
	r.Tools = nil
	for _, raw := range r.RawTools {
		if tool, ok := raw.(map[string]interface{}); ok {
			r.Tools = append(r.Tools, tool)
		}
	}
	return nil
}

func (r ResponsesRequest) MarshalJSON() ([]byte, error) {
	type rawRequest responsesRequestAlias
	req := rawRequest(r)
	if len(req.RawTools) == 0 && len(r.Tools) > 0 {
		req.RawTools = make([]interface{}, 0, len(r.Tools))
		for _, tool := range r.Tools {
			req.RawTools = append(req.RawTools, tool)
		}
	}
	return json.Marshal(req)
}
