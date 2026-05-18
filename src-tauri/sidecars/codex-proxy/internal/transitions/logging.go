package transitions

import "github.com/jlcodes99/cockpit-tools/codex-proxy/internal/statelog"

func LogStateTransition(component, scope, subject, from, to, cause string, extras ...string) {
	statelog.LogStateTransition(component, scope, subject, from, to, cause, extras...)
}
