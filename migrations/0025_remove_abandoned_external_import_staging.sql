BEGIN;

-- External-memory import was an abandoned experiment. LazyTT now creates all
-- new memories through its native ingestion path, so only the experimental
-- staging schema is removed here. Canonical ledger rows are intentionally not
-- touched by this migration.
DROP TABLE IF EXISTS memory_core.memory_import_batch_entries;
DROP TABLE IF EXISTS memory_core.memory_import_objects;
DROP TABLE IF EXISTS memory_core.memory_import_batches;

COMMIT;
