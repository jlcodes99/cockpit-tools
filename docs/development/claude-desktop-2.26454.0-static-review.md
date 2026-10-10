# Claude Desktop 2.26454.0 local record contract

Observed 2026-10-07 after the installed application updated while the previous
candidate was reopening Claude. ASAR SHA-256:
`9564e806afde6391f5666646827f14ea0ccfabc244ed6fad7dae4329cba9cb17`.
Reproduce with `node scripts/inspect_claude_continuity_contract.mjs`.

The persisted projection contains 169 fields, eleven more than 2.16120.0.
Account/organization namespaces, Desktop row IDs, separate CLI transcript IDs,
loader map keys, and native fork account ownership checks remain present. The
projection is in `index.chunk-BQ6X9i-D.js`; the loader is in
`index.chunk-AgchhrPF.js`. Minified export names changed, so the inspector now
locates the unique persisted record projection by its structural fields.

The adapter preserves `errorRows` and absolute `gitAnchorsFolderRealpath` with
the latest local conversation snapshot. Native `contextRecovery` counters are
limited to two attempts/failures, with failures not exceeding attempts.
Compacting requires a remaining attempt; restart restages it to waiting or
spent. Waiting at two attempts becomes spent. Compacted and spent are retained.
The adapter validates whichever source or destination recovery will actually
be retained before restaging. Unknown fields and active running descriptors
are blocked rather than copied as executable work. Newer source absence clears
older destination recovery state.

Other newly persisted execution/remote fields are not covered by this local
adapter when meaningful. They produce an explicit unresolved row rather than
being silently omitted. Native waiting recovery can schedule automatic compact;
it is not a passive display field.

An independent reader executed 15 native recovery cases and three native
loader/path scenarios successfully, including two Desktop row IDs referencing
one CLI head. The full normalizer, filesystem, lease/resume and scheduler were
mocked in that probe. These results establish a bounded contract, not a real
authenticated continuation. Installation and actual GUI acceptance evidence
are recorded separately in the dated continuity review.
