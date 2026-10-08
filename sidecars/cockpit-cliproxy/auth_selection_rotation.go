package main

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"sort"
	"strings"

	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
)

const maxRoutingRotationCursors = 1024

// Namespace the cursor by the persistent authorization scope, not the transient
// eligible list. Cooldowns and quota guards must not reset another key's turn.
func routingRotationKey(ctx context.Context, provider, model string) [32]byte {
	keyID, target := "", ""
	if ctx != nil {
		if spec, _ := ctx.Value(clientAPIKeyContextKey).(*apiKeySpec); spec != nil {
			keyID = strings.TrimSpace(spec.ID)
		}
		target, _ = ctx.Value(targetAccountIDContextKey).(string)
	}
	scopeSet := make(map[string]struct{})
	for _, id := range apiKeyScopeAccountIDs(ctx) {
		if id = strings.TrimSpace(id); id != "" {
			scopeSet[id] = struct{}{}
		}
	}
	scope := make([]string, 0, len(scopeSet))
	for id := range scopeSet {
		scope = append(scope, id)
	}
	sort.Strings(scope)
	parts := []string{provider, resolveBaseModelKey(model), keyID, strings.TrimSpace(target)}
	// Explicit whitespace-only scope is denied, and differs from unrestricted.
	if len(apiKeyScopeAccountIDs(ctx)) > 0 {
		parts = append(parts, "scoped")
	} else {
		parts = append(parts, "unrestricted")
	}
	parts = append(parts, scope...)
	encoded, _ := json.Marshal(parts)
	return sha256.Sum256(encoded)
}

func (s *cockpitSelector) orderAuthsForRequest(ctx context.Context, provider, model string, auths []*coreauth.Auth, start int) []*coreauth.Auth {
	if s == nil || s.manifest == nil {
		return s.orderAuths(auths, start)
	}
	strategy := strings.ToLower(strings.TrimSpace(s.manifest.RoutingStrategy))
	if strategy == "random" || strategy == "custom" {
		return s.prioritizeAuthsForAPIKey(ctx, s.orderAuths(auths, start))
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	ordered := s.orderAuths(auths, 0)
	key := routingRotationKey(ctx, provider, model)
	lastID := s.rotationCursors[key]
	if len(ordered) > 1 && lastID != "" {
		// Only rotate the highest-ranked tie group; priority strategies still win.
		groupEnd := 1
		for groupEnd < len(ordered) && compareAccountSpecs(s.accountForAuth(ordered[0]), s.accountForAuth(ordered[groupEnd]), strategy) == 0 {
			groupEnd++
		}
		next, found := 0, false
		for index := 0; index < groupEnd; index++ {
			if ordered[index].ID == lastID {
				next, found = (index+1)%groupEnd, true
				break
			}
		}
		if !found {
			lastAccount := accountForAuthInManifest(s.manifest, &coreauth.Auth{ID: lastID})
			if lastAccount != nil {
				lastIndex, known := s.manifest.originalIndexByID[lastAccount.ID]
				if known {
					for index := 0; index < groupEnd; index++ {
						candidate := s.accountForAuth(ordered[index])
						if candidate != nil {
							candidateIndex, known := s.manifest.originalIndexByID[candidate.ID]
							if known && candidateIndex > lastIndex {
								next = index
								break
							}
						}
					}
				}
			}
		}
		if next > 0 {
			group := append([]*coreauth.Auth(nil), ordered[:groupEnd]...)
			copy(ordered, append(group[next:], group[:next]...))
		}
	}
	ordered = s.prioritizeAuthsForAPIKey(ctx, ordered)
	if len(ordered) > 0 {
		if s.rotationCursors == nil {
			s.rotationCursors = make(map[[32]byte]string)
		}
		if _, exists := s.rotationCursors[key]; !exists {
			if len(s.rotationCursorOrder) >= maxRoutingRotationCursors {
				delete(s.rotationCursors, s.rotationCursorOrder[0])
				s.rotationCursorOrder = s.rotationCursorOrder[1:]
			}
			s.rotationCursorOrder = append(s.rotationCursorOrder, key)
		}
		s.rotationCursors[key] = ordered[0].ID
	}
	return ordered
}
