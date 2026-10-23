package helps

import "github.com/tidwall/gjson"

// TerminalResponseIDs is a bounded, connection-local ledger. The owner serializes access.
// Limits also bound memory retained from upstream-controlled identifiers.
type TerminalResponseIDs struct {
	ids   map[string]struct{}
	order []string
}

const TerminalResponseIDLimit = 128
const terminalResponseIDMaxBytes = 256

func (s *TerminalResponseIDs) Add(id string) {
	if id == "" || len(id) > terminalResponseIDMaxBytes || s.Contains(id) {
		return
	}
	if s.ids == nil {
		s.ids = make(map[string]struct{})
	}
	if len(s.order) == TerminalResponseIDLimit {
		delete(s.ids, s.order[0])
		copy(s.order, s.order[1:])
		s.order = s.order[:len(s.order)-1]
	}
	s.ids[id] = struct{}{}
	s.order = append(s.order, id)
}

func (s *TerminalResponseIDs) Contains(id string) bool {
	_, found := s.ids[id]
	return found
}

func (s *TerminalResponseIDs) Reset() { *s = TerminalResponseIDs{} }

// TerminalResponseID accepts only an actual terminal response, never a transport error.
func TerminalResponseID(payload []byte) string {
	switch gjson.GetBytes(payload, "type").String() {
	case "response.completed", "response.failed", "response.incomplete":
	case "response.done":
		switch gjson.GetBytes(payload, "response.status").String() {
		case "completed", "failed", "incomplete":
		default:
			return ""
		}
	default:
		return ""
	}
	id := gjson.GetBytes(payload, "response.id")
	if id.Type != gjson.String || len(id.String()) > terminalResponseIDMaxBytes {
		return ""
	}
	return id.String()
}
