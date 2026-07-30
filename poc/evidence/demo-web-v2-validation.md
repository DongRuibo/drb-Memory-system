# Interactive Memory Agent V2 validation

Validated locally on 2026-07-29 with the isolated `semantic-demo` user.

## Runtime under test

- Kimi Code OpenAI-compatible API, model `k3`, for extraction and streamed answers;
- local Qwen3 Embedding 0.6B and Qwen3 Reranker 0.6B gateway;
- PostgreSQL `memory_core` as authority and `demo_chat` for chat persistence;
- Mem0 as the rebuildable retrieval projection;
- Memory Service on `127.0.0.1:8080` and Demo Web API on `127.0.0.1:8090`.

No API key or bearer credential is recorded in this evidence.

## Observed business chain

### Current-turn write-through

The first message requested a stable preference and asked a technical question. Turn
`ce3819c0-1350-45fe-9c1a-bd7d1e849b77` completed with:

- memory record `6479d1f7-ef4f-5677-9508-3704f3351585`;
- version `489cdc65-f1a1-5905-bb15-05a9bc83338f`;
- canonical content `用户偏好回答技术问题时先给一句结论。`;
- `projection_status=pending` at write time;
- the same record already present in `used_memory_ids`;
- Context Pack `8d6bdee4-5d9e-5baa-a5ca-c5fbc2911442`;
- Outcome Receipt `8c66ed55-ebcf-5541-8f3e-8a7f73d10f3f`;
- real model `k3` produced an answer beginning with `结论：`.

This proves same-turn use does not wait for eventual Mem0 projection convergence.

### Semantic identity and provenance merge

The second message repeated the preference with different conversational wording. Turn
`28752b2a-51e3-47c7-ac9e-b1ecda92dd22` emitted `source_merged`. PostgreSQL still contained one
active record with semantic key `user.response_style.technical.conclusion_first`, while its source
count increased to two.

The extractor now uses controlled-taxonomy prompting plus conservative deterministic normalization
before PostgreSQL exact deduplication. Regression coverage includes both semantic-key aliases seen in
the original failure.

### Correction and immutable versions

Turn `1becef2f-3837-4e2d-bd57-43b71e0edba5` corrected the same record to
`用户偏好回答技术问题时先给一句结论，再给三个要点。`. The Head moved to revision 1 and
version `440810a0-4d23-573c-b0d8-b2ab63f9e11e`; both v1 and v2 remained inspectable. The corrected
version was pinned into the current Context Pack, and the streamed answer contained exactly three
points.

### Forget and zero injection

Turn `a12b5d79-3ac7-41b5-8939-ae13cb4f3ada` executed Forget. The Head moved to `forgotten`,
revision 2, and appended `memory_forgotten`. The turn completed with `used_memory_ids=[]`, Context
Pack `93536006-e473-5f7d-8fc9-98c4428df0b9`, and Outcome Receipt
`201e280d-13cc-5a79-a585-5b8b70b56bdf` while projection deletion was still pending.

## Result

The meeting-critical chain is implemented and runtime-verified: real chat persistence and streaming,
current-turn authorized memory use, semantic deduplication with provenance merge, immutable
Correction, immediate Forget, and auditable Context Pack/Outcome Receipt evidence.

This evidence is a local demo gate, not a Production gate. External identity authority, HA, disaster
recovery, production-scale evaluation, and compliance operations remain separate work.
