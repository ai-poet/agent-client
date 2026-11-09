# agent-core-engine

Sailry's Agent execution core, vendored at `cfc0a06ceaa14fad16f1b1b1ee6957bd53b0610e`.

The directory intentionally keeps Sailry's boundaries:

- `agent/engine`: state machine, inference projection, tool scheduling, compaction, cancellation, and steering.
- `agent/model`: model/stream types and request-scoped recovery.
- `agent/canonical`: durable history types.
- `agent/output`: bounded tool-output projection.
- `store`: the embedding host's in-memory effect owner.

The port does not include Sailry's Node admission queue, Turso connection, GPUI code, Rig transport, plugin host, or remote child supervisor. Existing CheapRouter providers, tools, permissions, history persistence, browser, search, and Computer Use remain owned by `agent-core-query` and `agent-bridge`.

The `node-tests` feature marks tests that require Sailry's complete Node and Turso fixtures; it is not a supported standalone configuration. Host-neutral tests and the copied model/output/algorithm tests run without it.

Sailry and the retained Goose-derived code are Apache-2.0. rmcp is MIT. See the repository's `third_party_licenses` directory and `NOTICE.md`.
