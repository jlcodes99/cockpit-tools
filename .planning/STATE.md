# STATE

## Active
- **Milestone:** Provider Expansion — Integrating OpenCode, Groq, and OpenRouter
- **Current Phase:** 2 (Groq Integration)
- **Branches:** phase-1-opencode

## Completed Phases
- **Phase 1:** OpenCode Go & Zen Provider Integration — completed 2026-05-12

## Decisions
- Each provider gets its own `PlatformId` (opencode, groq, openrouter) 
- OpenCode Go and Zen share the same `opencode` provider but have sub-account types (go vs zen)
- All three use standard Bearer token auth — reuse existing token-based account patterns
- Groq and OpenRouter have OpenAI-compatible APIs — can use standard chat completions
- OpenRouter requires management key for credit checks — separate from inference key
