package helps

// CanFinalizeResponseStream asks the translator whether output is complete.
func CanFinalizeResponseStream(param any) bool {
	state, ok := param.(interface{ CanFinalizeResponseStream() bool })
	return ok && state.CanFinalizeResponseStream()
}
