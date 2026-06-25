# msfind requests for fod-indexer

This file collects functionality that `msfind` wants from `fod-indexer` so `msfind` can stay thin and reuse the shared FOD indexing core instead of building a separate indexing engine.

Add entries as short bullets:
- what is missing
- why `msfind` needs it
- expected behavior or constraints
- links to the related `msfind` work, if useful

When adding new requests, prefer capabilities that can be implemented once in `fod-indexer` and reused by both tools. Do not use this file to track duplicate scan/hash/materialize logic for `msfind`; that work belongs in the shared core.

At the current stage, `fod-indexer` is already useful for `msfind` as the single indexing engine. The remaining asks are about a stable integration surface, not a second scan/hash pipeline.

## Requests

- [ ] Provide a machine-readable output mode for `source list`, `scan`, `hash`, `report duplicates`, `plan-import`, `clean`, `materialize`, and `cleanup-failed`.
  - `msfind` should not need to parse ad-hoc human-readable CLI text.
  - Keep the current text output for interactive use.
- [ ] Expose a read-only source-inspection view that returns source kind, policy, capability metadata, and browsable roots in one structured response.
  - `msfind` should reuse the same discovery logic for `local`, `smb`, `qnap`, `adb`, and `github` instead of duplicating it.
- [ ] Add plan and duplicate snapshot export by id, so an existing import plan or duplicate set can be inspected later without rerunning the pipeline.
  - That keeps `msfind` on the consumer side of the contract.
- [ ] Document and preserve the idempotent retry boundary for replay-safe commands.
  - `msfind` may call into `fod-indexer` across transient disconnects and needs the contract to stay explicit.
- [ ] Keep non-indexing work out of the shared core.
  - Text extraction, classification, embeddings, and other AI steps should stay in `msfind`; `fod-indexer` should only own source indexing and materialization.
