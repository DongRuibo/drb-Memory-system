# LazyTT Demo Web API

`demo-web-api` is a reference Agent host for the optional Web demo. It is not part of the
reusable Memory Service product boundary.

It owns:

- demo viewer sessions and the Alice/Bob/Carol switcher;
- chat sessions, turns and server-sent events;
- answer-model orchestration;
- short-lived Memory Service JWT issuance;
- the immutable `prompt_context_snapshot` shown by the evidence panel.

It does not own canonical memory. The only supported Memory boundary is the versioned HTTP API
implemented by `memory-service` and wrapped by `memory-client`.

## Database

Set `DEMO_DATABASE_URL` for Demo chat persistence. `DATABASE_URL` is accepted only as a transition
fallback while the monorepo is being split.

The embedded migrations under `migrations/` are self-contained: a clean PostgreSQL database does
not need `memory_core` or `memory_search`. If the Demo and Memory Service temporarily share a
database, the Demo migrator ignores unrelated already-applied backend migration versions.

## Local configuration

Copy `.env.example` values into the process environment. The JWT issuer, audience, secret and
tenant must match the Memory Service configuration. The browser never receives that JWT.

The default listener remains loopback-only. A container may set
`DEMO_WEB_API_CONTAINER_MODE=true` and bind `0.0.0.0:8090`; its Compose file must still publish the
port only on `127.0.0.1` for local testing.
