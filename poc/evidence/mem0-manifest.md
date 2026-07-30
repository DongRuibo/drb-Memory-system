# Mem0 POC Evidence Manifest

## Status

`CONTRACT_POC_COMPLETE__PRODUCTION_HOLD`

## Verified from official documentation/source

- Self-hosted REST endpoints do not use `/v1`; create/search/delete are `/memories`, `/search`, `/memories/{id}`.
- Auth is enabled by default in the current self-hosted server.
- `infer=true` performs extraction/conflict resolution and durable storage together.
- `infer=false` stores curated input without inference or dedupe and is the selected canonical projection path.

## Local contract evidence

- Rust Adapter sends `infer=false` for ProjectionUpsert.
- Provider receives an opaque namespace key, canonical record/version IDs, revision, generation, operation ID and content hash.
- Search results are reduced to canonical mapping fields; Provider memory text is not returned by the Adapter.
- Unit/HTTP contract tests cover upsert, search, delete and mutation timeout classification.

## Runtime manifest

| Field | Value |
|---|---|
| Mem0 repository commit/tag | `b357a5a1b03c299ec8229c268e63cfac0f7c6566` (2026-07-25, shallow source inspection) |
| Mem0 container image digest | local build `sha256:b1b429b5cf9d636beceaa35866b6b01b839cb8b0de0d1145f3d0901e4a5adb31` |
| REST OpenAPI hash | live raw SHA-256 `6a320c62367ce6d56562ebaba7fec4f83b914a7043839673a834cae285ddaa25`; pretty archived artifact SHA-256 `0b199615bf940edcd0cd77b95ae5e28ca8764d3fe567623c9fd6125a14a81577` |
| LLM provider/model | OpenAI / `gpt-4.1-nano-2025-04-14`, configured but not invoked by the `infer=false` path |
| Embedder provider/model | local deterministic OpenAI-compatible POC endpoint / configured model name `text-embedding-3-small`; no semantic-quality claim |
| Vector store | Mem0-managed PostgreSQL 17 + pgvector, collection `memories` |
| Auth mode | `AUTH_DISABLED=true`, permitted only for isolated local synthetic POC |
| Telemetry | `MEM0_TELEMETRY=false` |
| Last service start | `2026-07-27T14:28:40.8045263Z` |

## Remaining live checks

- [x] Start Docker Desktop and the official Mem0 server stack.
- [x] Export and archive `/openapi.json` as `poc/evidence/mem0-openapi.json`.
- [x] Run `poc-cli health`.
- [x] Run synthetic `poc-cli cycle`; verify canonical mapping, namespace/generation isolation, delete, and post-delete absence.
- [x] Repeat identical operation behavior: the same `operation_id` created two different Provider objects; Mem0 does not provide operation-level idempotency.
- [ ] Complete timeout/unknown-result recovery. A stalled mutation is already mapped to `UnknownResult`, but safe recovery remains unimplemented because the Provider has no operation lookup/idempotency API.
- [x] Restart Mem0 and verify search/delete persistence with a fixed marker.
- [ ] Rebuild an empty projection from canonical rows.
- [x] Verify metadata filters for generation and opaque namespace in mock contract tests and the live POC.
- [x] Inspect Mem0 logs and local Embedder logs. The successful path used no external endpoint and hash-only Embedder logging; default OpenAI Embedder configuration necessarily sends indexed text to the embedding Provider.

## Decision

ADR-0001 remains `proposed`. Mem0 is technically viable as a replaceable Retrieval Index for accepted versions, but the Adapter cannot safely retry an unknown mutation result until the system implements a canonical Projection Operation/Mapping idempotency barrier. Candidate extraction, rebuild, semantic evaluation, production Auth and security review are also incomplete. Mem0 is not approved as a production Candidate Extractor.

## Source inspection notes

- `server/main.py::MemoryCreate` exposes `infer: Optional[bool]` and `POST /memories` passes it to `Memory.add`.
- `mem0/memory/main.py::Memory.add` documents that `infer=false` stores raw memories directly; `infer=true` performs LLM extraction plus add/update/delete decisions.
- Current `server/main.py::SearchRequest` uses `filters` and `top_k`; top-level user/run/agent IDs are deprecated.
- The server exposes no request idempotency-key contract for `POST /memories`; timeout recovery and duplicate prevention remain unproven POC blockers.
- `infer=false` still calls the configured Embedder because durable vector storage requires an embedding; it bypasses the LLM extraction/conflict path, not all model-provider traffic.
- The live duplicate probe confirms that `operation_id` stored in metadata is not an idempotency mechanism.
