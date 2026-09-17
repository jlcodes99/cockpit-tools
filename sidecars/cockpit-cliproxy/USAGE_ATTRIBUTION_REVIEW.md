# Request Usage Attribution

## Scope

The HTTP/SSE request tracker chooses a usage callback to finalize a downstream
request. That callback's tokens and account identity must travel together.
Previously the last selected account unconditionally overwrote that identity;
when no selection entry existed, the identity was cleared.

This patch preserves the chosen usage callback's account. The selector remains
a fallback for requests without usage callbacks, or for a missing account mapping
when the callback AuthID matches the selected AuthID. Unmapped callbacks from a
different auth remain unattributed instead of being charged to an unrelated account.

The patch does not change account routing, retries, model pricing, quota querying,
WebSocket execution IDs, or the existing choice of the last successful callback.
It does not change already persisted request logs.

## Verification

Run from this directory:

```sh
go test . -run 'TestRequestUsageTracker|TestUsageAttribution|TestUnmappedUsage|TestMatchingSelectedAuth|TestWebsocketUsage' -count=1
go test -race . -run 'TestRequestUsageTracker|TestUsageAttribution|TestUnmappedUsage|TestMatchingSelectedAuth|TestWebsocketUsage' -count=1
```

All regression data is synthetic. Tests reproduce a later selection on another
account, missing selection state, unmapped AuthID disagreement, and matching
AuthID fallback. No credentials or model calls are required.

## Limitations

This is a code-level reproduction, not proof that every difference between plan
quota percentage and estimated API dollars was caused by this defect. Percent
quota and API-equivalent cost are different measures.

Restoring historical attribution requires original per-attempt evidence. A
persisted row that already replaced its account identity cannot be reliably
reassigned from amount, email, quota percentage, or timing alone.
