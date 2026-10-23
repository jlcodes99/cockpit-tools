package main

import (
	"errors"
	"strings"
)

const officialAccountNativeRequired = "CHATGPT_PLAN_NATIVE_REQUIRED"

// Official grants remain native-only. Account IDs now name the original Codex
// members, so their ordinary OAuth credentials cannot identify this route by
// themselves. Reject before registering credentials or starting any listeners.
func validateSidecarOfficialBindings(m *manifest) error {
	if m == nil {
		return nil
	}
	reject := func() error {
		return errors.New(officialAccountNativeRequired + ": official ChatGPT access requires the native gateway")
	}
	if len(m.OfficialAccountBindings) > 0 {
		// Pausing keeps the selected connection mode; it never authorizes a
		// silent switch to the source account's local-gateway credentials.
		return reject()
	}
	legacy := func(id string) bool {
		return strings.HasPrefix(strings.TrimSpace(id), "chatgpt_plan_")
	}
	for _, account := range m.Accounts {
		if legacy(account.ID) {
			return reject()
		}
	}
	for _, id := range m.PausedAccountIDs {
		if legacy(id) {
			return reject()
		}
	}
	for _, key := range m.APIKeys {
		for _, ids := range [][]string{key.AccountIDs, key.ImageGenerationAccountIDs} {
			for _, id := range ids {
				if legacy(id) {
					return reject()
				}
			}
		}
		if key.ModelRouting != nil {
			for _, route := range key.ModelRouting.Routes {
				if legacy(route.ProviderAccountID) {
					return reject()
				}
			}
		}
	}
	return nil
}
