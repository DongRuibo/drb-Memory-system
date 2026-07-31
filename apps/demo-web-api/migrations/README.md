# Demo Chat Migrations

This directory is the migration source embedded by `demo-web-api`. It must stay independent from
`memory_core` and `memory_search` so the Demo can move to a separate repository and database.

During the monorepo transition, migration `0030` is also present in the repository root with the
same bytes and checksum. This lets a legacy shared database recognize the same applied version.
Run `scripts/check-demo-boundary.sh` after changing either copy.
