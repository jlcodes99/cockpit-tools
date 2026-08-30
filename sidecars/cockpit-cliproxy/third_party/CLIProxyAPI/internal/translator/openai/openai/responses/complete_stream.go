package responses

import "context"

// CompleteOpenAIChatCompletionsResponseToOpenAIResponses emits the terminal
// response.completed event when an OpenAI-compatible stream closes after a
// response has started but before completion was emitted.
func CompleteOpenAIChatCompletionsResponseToOpenAIResponses(_ context.Context, requestRawJSON []byte, param *any) [][]byte {
	if param == nil || *param == nil {
		return nil
	}
	state, ok := (*param).(*oaiToResponsesState)
	if !ok || !state.Started || state.CompletedEmitted {
		return nil
	}
	state.CompletedEmitted = true
	return [][]byte{buildResponsesCompletedEvent(state, requestRawJSON, func() int {
		state.Seq++
		return state.Seq
	})}
}
