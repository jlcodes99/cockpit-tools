package handlers

import "context"

type gatewayDrainKey struct{}

// WithGatewayDrain lets an embedding server retire a connection between turns
// without canceling the response currently being generated.
func WithGatewayDrain(ctx context.Context, signal <-chan struct{}) context.Context {
	return context.WithValue(ctx, gatewayDrainKey{}, signal)
}

func GatewayDrain(ctx context.Context) <-chan struct{} {
	signal, _ := ctx.Value(gatewayDrainKey{}).(<-chan struct{})
	return signal
}
