BEGIN;

ALTER TABLE memory_core.memory_candidates
    ADD COLUMN event_time_start TIMESTAMPTZ,
    ADD COLUMN event_time_end TIMESTAMPTZ,
    ADD COLUMN event_timezone TEXT,
    ADD COLUMN event_time_precision TEXT;

ALTER TABLE memory_core.memory_candidates
    ADD CONSTRAINT memory_candidates_event_time_check CHECK (
        (
            event_time_start IS NULL
            AND event_time_end IS NULL
            AND event_timezone IS NULL
            AND event_time_precision IS NULL
        )
        OR (
            memory_type = 'episodic_summary'
            AND event_time_start IS NOT NULL
            AND event_time_end IS NOT NULL
            AND event_time_start < event_time_end
            AND char_length(event_timezone) BETWEEN 1 AND 64
            AND event_time_precision IN (
                'instant', 'minute', 'hour', 'part_of_day', 'day',
                'week', 'month', 'quarter', 'year'
            )
        )
    );

ALTER TABLE memory_core.memory_versions
    ADD COLUMN event_time_start TIMESTAMPTZ,
    ADD COLUMN event_time_end TIMESTAMPTZ,
    ADD COLUMN event_timezone TEXT,
    ADD COLUMN event_time_precision TEXT;

ALTER TABLE memory_core.memory_versions
    ADD CONSTRAINT memory_versions_event_time_check CHECK (
        (
            event_time_start IS NULL
            AND event_time_end IS NULL
            AND event_timezone IS NULL
            AND event_time_precision IS NULL
        )
        OR (
            event_time_start IS NOT NULL
            AND event_time_end IS NOT NULL
            AND event_time_start < event_time_end
            AND char_length(event_timezone) BETWEEN 1 AND 64
            AND event_time_precision IN (
                'instant', 'minute', 'hour', 'part_of_day', 'day',
                'week', 'month', 'quarter', 'year'
            )
        )
    );

CREATE INDEX idx_memory_versions_event_time
    ON memory_core.memory_versions (
        tenant_id,
        event_time_start,
        event_time_end,
        memory_record_id
    )
    WHERE event_time_start IS NOT NULL;

COMMENT ON COLUMN memory_core.memory_versions.event_time_start IS
    'Inclusive UTC start of the source-supported real-world event interval.';
COMMENT ON COLUMN memory_core.memory_versions.event_time_end IS
    'Exclusive UTC end of the source-supported real-world event interval.';
COMMENT ON COLUMN memory_core.memory_versions.event_timezone IS
    'Bounded IANA timezone used to interpret the original event expression.';
COMMENT ON COLUMN memory_core.memory_versions.event_time_precision IS
    'Controlled precision of the event interval; distinct from query-window precision.';

COMMIT;
