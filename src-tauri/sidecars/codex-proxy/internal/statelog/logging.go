package statelog

import "log"

func LogStateTransition(component, scope, subject, from, to, cause string, extras ...string) {
	msg := "[" + component + "] transition scope=" + scope + " subject=" + subject + " from=" + from + " to=" + to + " cause=" + cause
	for _, extra := range extras {
		if extra == "" {
			continue
		}
		msg += " " + extra
	}
	log.Print(msg)
}
