BEGIN;

ALTER TABLE memory_core.retrieval_candidate_receipts
    DROP CONSTRAINT retrieval_candidate_receipts_channel_check,
    ADD CONSTRAINT retrieval_candidate_receipts_channel_check
        CHECK (channel IN ('provider', 'bm25', 'lexical', 'temporal', 'global_preference'));

COMMIT;
