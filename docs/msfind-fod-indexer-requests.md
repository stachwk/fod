# msfind requests for fod-indexer

This file collects functionality that `msfind` wants from `fod-indexer` so `msfind` can stay thin and reuse the shared FOD indexing core instead of building a separate indexing engine.

Add entries as short bullets:
- what is missing
- why `msfind` needs it
- expected behavior or constraints
- links to the related `msfind` work, if useful

When adding new requests, prefer capabilities that can be implemented once in `fod-indexer` and reused by both tools. Do not use this file to track duplicate scan/hash/materialize logic for `msfind`; that work belongs in the shared core.

## Requests
