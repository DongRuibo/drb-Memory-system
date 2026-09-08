BEGIN;

CREATE TABLE memory_search.retrieval_terms (
    tenant_id TEXT NOT NULL,
    memory_record_id UUID NOT NULL,
    memory_version_id UUID NOT NULL,
    generation TEXT NOT NULL,
    term TEXT NOT NULL,
    term_frequency INTEGER NOT NULL CHECK (term_frequency > 0),
    document_length INTEGER NOT NULL CHECK (document_length > 0),
    tokenizer_version TEXT NOT NULL,
    PRIMARY KEY (tenant_id, memory_version_id, generation, term),
    FOREIGN KEY (tenant_id, memory_version_id, generation)
        REFERENCES memory_search.retrieval_documents (
            tenant_id, memory_version_id, generation
        )
        ON DELETE CASCADE
);

CREATE INDEX idx_retrieval_terms_lookup
    ON memory_search.retrieval_terms (tenant_id, generation, term, memory_version_id);

ALTER TABLE memory_search.retrieval_terms ENABLE ROW LEVEL SECURITY;
ALTER TABLE memory_search.retrieval_terms FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON memory_search.retrieval_terms
    USING (tenant_id = nullif(current_setting('app.tenant_id', true), ''))
    WITH CHECK (tenant_id = nullif(current_setting('app.tenant_id', true), ''));

COMMENT ON TABLE memory_search.retrieval_terms IS
    'Rebuildable Jieba term projection used by the local BM25 candidate channel.';

COMMIT;
