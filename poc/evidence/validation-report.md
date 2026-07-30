# POC Validation Report — 2026-07-27

## Verdict

The architecture baseline is implementable and the bounded Retrieval Index POC works. The repository is ready to enter MVP development, not production. The decisive remaining correctness gap is projection mutation idempotency: Mem0 creates duplicates when the same canonical `operation_id` is submitted twice.

## Fixed inputs

- Rust `1.97.1` with locked dependencies in `Cargo.lock`.
- PostgreSQL `postgres:17-alpine` for the canonical Ledger spike.
- Mem0 source commit `b357a5a1b03c299ec8229c268e63cfac0f7c6566`.
- Locally built Mem0 image `sha256:b1b429b5cf9d636beceaa35866b6b01b839cb8b0de0d1145f3d0901e4a5adb31`.
- Live OpenAPI raw SHA-256 `6a320c62367ce6d56562ebaba7fec4f83b914a7043839673a834cae285ddaa25`.
- Archived normalized OpenAPI: `poc/evidence/mem0-openapi.json`, SHA-256 `0b199615bf940edcd0cd77b95ae5e28ca8764d3fe567623c9fd6125a14a81577`.

## Passed evidence

| Check | Result |
|---|---|
| Rust formatting | `cargo fmt --all -- --check` passed |
| Rust lint | `cargo clippy --workspace --all-targets -- -D warnings` passed |
| Rust tests | 11 passed: 7 domain invariants + 4 Mem0 Adapter HTTP/timeout contracts; 0 failed |
| PostgreSQL Migration | `0001` and `0002` applied to PostgreSQL 17 |
| SQL invariants | typed Scope, RLS cross-tenant denial, revision step guard and Forgotten no-resurrection passed |
| Mem0 liveness | Rust `poc-cli health` passed against the official server |
| Live projection cycle | `infer=false` upsert, scoped search, canonical mapping validation, namespace/generation isolation, delete and post-delete absence passed |
| Restart persistence | fixed marker survived a Mem0 service restart, remained searchable and was then deleted |
| Log boundary | successful POC used a local hash-only Embedder and disabled Mem0 telemetry |

The local POC Embedder returns a constant non-zero vector. It intentionally proves filter and lifecycle behavior only; it does not prove semantic quality, ranking or multilingual recall.

## Negative evidence and production blockers

1. Repeating exactly the same `operation_id`, record ID and version ID returned two different Mem0 object IDs and two searchable projections.
2. A stalled mutation is classified as `UnknownResult`, but Mem0 exposes no operation lookup or idempotency-key contract. Blind retry is unsafe.
3. With the default OpenAI Embedder, even `infer=false` sends accepted Version text to the embedding endpoint. Provider routing and data classification must therefore be enforced before projection.
4. Candidate Extractor, Projection Operation repository, Outbox worker, canonical revalidation, Context Pack/Receipt, generation rebuild and semantic evaluation are not implemented.
5. The live contract POC used `AUTH_DISABLED=true`; production-like security verification still requires Auth, least-privilege credentials and network policy.

## Required next implementation slice

1. Implement `ProjectionOperationRepository` with a unique stable `operation_id`, lease/fencing, provider mapping and terminal receipt.
2. Make the worker reserve the operation before calling Mem0 and reconcile unknown results without creating a new logical operation.
3. Implement canonical Repository transactions for Candidate → Decision → Version/Event/Head → Outbox.
4. Implement the read authority intersection: Provider hit IDs → current Head/Version → Scope/ACL/Consent/Purpose/Tombstone checks → Context Pack.
5. Add a deterministic rebuild command that creates a new generation exclusively from canonical active Heads.
6. Replace the POC Embedder with the approved deployment route and run the fixed Chinese/keyword/update/conflict/deletion evaluation set.
