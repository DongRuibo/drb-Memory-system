# Interactive Memory Agent V2

Status: implementation contract (2026-07-29)

## Product outcome

The demo is a normal streaming chat application. A product or management audience can see four
observable promises without understanding the storage implementation:

1. Stable user information can be remembered and used in a later answer.
2. Repeating the same fact does not create duplicate active memories; provenance is merged.
3. A correction creates a new version and is effective in the same turn.
4. Forget blocks future use immediately, while physical Erase remains separately auditable.

The default screen has a conversation list, a central chat, and a right-hand "what it remembers"
panel. It offers a guided story and unrestricted free chat. Technical IDs and status transitions are
available in a collapsed evidence drawer.

For the high-value response-style preferences used by the guided demo, the extractor is constrained
to a small semantic-key taxonomy and the service applies conservative deterministic normalization
before canonical ingestion. This prevents model-generated aliases from bypassing PostgreSQL exact
deduplication. Unknown concepts remain model-proposed keys and are not claimed to have general
semantic deduplication.

## Authority and persistence boundaries

- `memory_core` in PostgreSQL is the canonical memory authority.
- Mem0 is a rebuildable semantic-search projection. A provider hit is never trusted without a
  canonical authorization check.
- `demo_chat` in PostgreSQL owns sessions, messages, turns, and reconnectable public turn events.
  It does not become a second memory authority.
- Browser state is disposable. Reloading the page reconstructs the UI from the chat API.
- The hosted Web application owns no durable product data; `.openai/hosting.json` therefore keeps
  D1 and R2 disabled.

## Turn lifecycle

`accepted -> processing -> streaming -> completed`

Terminal alternatives are `failed` and `cancelled`. Only one turn may be active per session. A
client-supplied idempotency key binds the request body to one persisted turn. Regeneration creates a
new Attempt, while automatic extraction uses the user-message identity so a retry cannot create a
second memory.

Each turn persists ordered events. `GET /api/chat/turns/{turn_id}/events?after=N` replays events
after `N` and then waits for more, so reconnecting does not lose answer text or memory effects.

## Current-turn memory contract

Automatic extraction runs before context compilation. Newly accepted/corrected memory-version IDs
are passed as pinned IDs to Context Pack compilation. The Memory Service resolves those IDs against
the canonical Head, scope, purpose, retention, consent, and tombstone rules inside the transaction;
the caller cannot inject arbitrary memory text. Pinned canonical memories are merged ahead of
semantic candidates and remain subject to the token budget.

This write-through path means a memory can affect the current answer even while its Mem0 projection
operation is still pending. When an authorized low-risk memory explicitly supersedes the same
semantic identity, the Web runtime automatically accepts the conflict through the deterministic
Review API, pins the resulting new version, and emits an `automatic_supersede` result notice. The
browser does not ask the user to choose between versions.

## Public API

- `GET|POST /api/chat/sessions`
- `GET /api/chat/sessions/{session_id}`
- `POST /api/chat/turns`
- `GET /api/chat/turns/{turn_id}/events?after=N`
- `POST /api/chat/turns/{turn_id}/cancel`
- `GET /api/memories` and `GET /api/memories/{memory_record_id}`
- `POST /api/memories/{memory_record_id}/correct`
- `POST /api/memories/{memory_record_id}/forget`
- `POST /api/memories/{memory_record_id}/erase`
- `POST /api/demo/reset`

## Failure semantics

- A normal chat turn may continue without newly extracted memory if extraction fails. The event
  stream exposes a warning and the assistant must not claim that new memory was saved.
- Explicit correction, Forget, and Erase never report success until the canonical operation has
  been accepted.
- Stop generation records `cancelled`; memory already committed before cancellation remains visible
  with a separate undo/Forget action.
- Model errors create a terminal failed turn and, when a Context Pack was consumed, a failed Outcome
  Receipt.

## Demo script

1. Say a stable response-style preference. Show the auto-save chip and answer footer.
2. Repeat the preference. Show one active memory with two source references.
3. Correct the preference. Show version 1 -> 2 and the answer using version 2 immediately.
4. Forget it. Ask the same kind of question and show `0` used memories.
5. Open the evidence drawer only if technical viewers ask for IDs, Context Pack, projection, or
   Outcome Receipt proof.
