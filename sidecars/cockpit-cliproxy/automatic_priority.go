package main

import (
	"context"
	"sort"
	"strings"

	"github.com/gin-gonic/gin"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
	"github.com/tidwall/gjson"
)

type automaticPriorityRank struct {
	usage, keyPriority, priority int
}

// Partition both transports by the same account policy. Native-first is only
// a tie breaker inside a tier; it must not outrank a preferred provider route.
func (s *relayServer) customAutomaticTiers(spec *apiKeySpec, model string) []*apiKeySpec {
	if s.manifest == nil || spec == nil || spec.ModelRouting == nil ||
		!strings.EqualFold(s.manifest.RoutingStrategy, "custom") {
		return nil
	}
	providerIDs := map[string]bool{}
	for _, route := range spec.ModelRouting.Routes {
		providerIDs[route.ProviderAccountID] = true
	}
	routableProviders := map[string]bool{}
	for _, candidate := range s.automaticCandidates(spec, model) {
		routableProviders[candidate.route.ProviderAccountID] = true
	}
	rules := map[string]customRoutingRule{}
	for _, rule := range s.manifest.CustomRoutingRules {
		rules[rule.AccountID] = rule
	}
	keyPriorities := map[string]int{}
	keyOrder := s.priorities.priorityAccountIDs(spec.ID)
	for index, id := range keyOrder {
		keyPriorities[id] = len(keyOrder) - index
	}
	groups := map[automaticPriorityRank][]string{}
	ids := spec.AccountIDs
	if len(ids) == 0 {
		for _, account := range s.manifest.Accounts {
			ids = append(ids, account.ID)
		}
	}
	seen := map[string]bool{}
	for _, id := range ids {
		if seen[id] || s.manifest.accountByID[id] == nil ||
			(providerIDs[id] && !routableProviders[id]) ||
			(!providerIDs[id] && !automaticNativeModel(spec, model)) {
			continue
		}
		seen[id] = true
		rule := rules[id]
		rank := automaticPriorityRank{priority: rule.Priority, keyPriority: keyPriorities[id]}
		if rule.IsPreferred {
			rank.usage = 1
		} else if rule.IsBackup {
			rank.usage = -1
		}
		groups[rank] = append(groups[rank], id)
	}
	if len(groups) <= 1 {
		return nil
	}
	ranks := make([]automaticPriorityRank, 0, len(groups))
	for rank := range groups {
		ranks = append(ranks, rank)
	}
	sort.Slice(ranks, func(i, j int) bool {
		if ranks[i].usage != ranks[j].usage {
			return ranks[i].usage > ranks[j].usage
		}
		if ranks[i].keyPriority != ranks[j].keyPriority {
			return ranks[i].keyPriority > ranks[j].keyPriority
		}
		return ranks[i].priority > ranks[j].priority
	})
	var tiers []*apiKeySpec
	for _, rank := range ranks {
		tier := *spec
		tier.AccountIDs = groups[rank]
		routing := *spec.ModelRouting
		routing.NativeModels, routing.RoutableModels, routing.Routes = nil, nil, nil
		for _, id := range tier.AccountIDs {
			if !providerIDs[id] {
				routing.NativeModels = spec.ModelRouting.NativeModels
				routing.RoutableModels = spec.ModelRouting.RoutableModels
			}
			for _, route := range spec.ModelRouting.Routes {
				if route.ProviderAccountID == id {
					routing.Routes = append(routing.Routes, route)
				}
			}
		}
		tier.ModelRouting = &routing
		tiers = append(tiers, &tier)
	}
	return tiers
}

func (s *relayServer) handleCustomAutomaticPriority(c *gin.Context, spec *apiKeySpec, body []byte, model string, format sdktranslator.Format, alt string) bool {
	// Opaque/incremental continuations must first resolve their protected owner.
	// Their existing continuity path can rebuild portable history before routing.
	if alt == "responses/compact" || gjson.GetBytes(body, "previous_response_id").String() != "" ||
		strings.Contains(string(body), `"encrypted_content"`) || c.Request.Header.Get("X-Codex-Turn-State") != "" {
		return false
	}
	tiers := s.customAutomaticTiers(spec, model)
	if len(tiers) == 0 {
		return false
	}
	originalRequest, originalWriter := c.Request, c.Writer
	defer func() { c.Request, c.Writer = originalRequest, originalWriter }()
	for index, tier := range tiers {
		attempt := newAutomaticAttemptWriter(originalWriter)
		c.Writer = attempt
		c.Errors = nil
		ctx := context.WithValue(originalRequest.Context(), clientAPIKeyContextKey, tier)
		c.Request = originalRequest.WithContext(ctx)
		s.handleAutomaticModelRequestTier(c, tier, body, model, format, alt)
		if originalWriter.Written() || !retryableAutomaticStatus(attempt.Status()) ||
			originalRequest.Context().Err() != nil || index == len(tiers)-1 {
			attempt.commit()
			break
		}
		s.releaseAutomaticAccountSlots(c)
	}
	return true
}
