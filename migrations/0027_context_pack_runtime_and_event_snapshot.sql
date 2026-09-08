BEGIN;

ALTER TABLE memory_core.context_packs
    ADD COLUMN reference_time TIMESTAMPTZ,
    ADD COLUMN timezone TEXT;

UPDATE memory_core.context_packs
SET reference_time = created_at,
    timezone = 'Asia/Shanghai';

ALTER TABLE memory_core.context_packs
    ALTER COLUMN reference_time SET NOT NULL,
    ALTER COLUMN timezone SET NOT NULL,
    ADD CONSTRAINT context_packs_timezone_bounded
        CHECK (char_length(btrim(timezone)) BETWEEN 1 AND 64);

ALTER TABLE memory_core.context_pack_items
    ADD COLUMN event_time_start TIMESTAMPTZ,
    ADD COLUMN event_time_end TIMESTAMPTZ,
    ADD COLUMN event_timezone TEXT,
    ADD COLUMN event_time_precision TEXT;

UPDATE memory_core.context_pack_items AS item
SET event_time_start = version.event_time_start,
    event_time_end = version.event_time_end,
    event_timezone = version.event_timezone,
    event_time_precision = version.event_time_precision
FROM memory_core.memory_versions AS version
WHERE version.tenant_id = item.tenant_id
  AND version.id = item.memory_version_id;

ALTER TABLE memory_core.context_pack_items
    ADD CONSTRAINT context_pack_items_event_time_complete
    CHECK (
        (
            event_time_start IS NULL
            AND event_time_end IS NULL
            AND event_timezone IS NULL
            AND event_time_precision IS NULL
        )
        OR
        (
            event_time_start IS NOT NULL
            AND event_time_end IS NOT NULL
            AND event_time_start < event_time_end
            AND char_length(btrim(event_timezone)) BETWEEN 1 AND 64
            AND event_time_precision IN (
                'instant', 'minute', 'hour', 'part_of_day', 'day',
                'week', 'month', 'quarter', 'year'
            )
        )
    );

CREATE FUNCTION demo_chat.prompt_context_for_turn(
    requested_tenant_id TEXT,
    requested_context_pack_id UUID
) RETURNS JSONB
LANGUAGE SQL
STABLE
AS $$
    SELECT jsonb_build_object(
        'runtime_context', jsonb_build_object(
            'current_time_utc', pack.reference_time,
            'timezone', pack.timezone,
            'local_date', (pack.reference_time AT TIME ZONE pack.timezone)::date::text
        ),
        'memory_context', COALESCE(
            (
                SELECT jsonb_agg(
                    jsonb_build_object(
                        'memory_record_id', item.memory_record_id,
                        'memory_version_id', item.memory_version_id,
                        'section_kind', item.section_kind,
                        'memory_data', item.content_snapshot,
                        'source_refs', item.source_refs,
                        'event_time', CASE
                            WHEN item.event_time_start IS NULL THEN NULL
                            ELSE jsonb_build_object(
                                'start', item.event_time_start,
                                'end', item.event_time_end,
                                'timezone', item.event_timezone,
                                'precision', item.event_time_precision
                            )
                        END,
                        'inclusion_reason', item.inclusion_reason,
                        'trust', 'untrusted_memory_data',
                        'allowed_effect', CASE item.section_kind
                            WHEN 'user_preference' THEN 'response_style_only'
                            WHEN 'current_fact' THEN 'factual_personalization_only'
                            WHEN 'relevant_experience' THEN 'contextual_example_only'
                        END
                    )
                    ORDER BY item.section_first_position, item.position
                )
                FROM (
                    SELECT context_item.*,
                           min(context_item.position) OVER (
                               PARTITION BY context_item.section_kind
                           ) AS section_first_position
                    FROM memory_core.context_pack_items AS context_item
                    WHERE context_item.tenant_id = pack.tenant_id
                      AND context_item.context_pack_id = pack.id
                ) AS item
            ),
            '[]'::jsonb
        )
    )
    FROM memory_core.context_packs AS pack
    WHERE pack.tenant_id = requested_tenant_id
      AND pack.id = requested_context_pack_id
$$;

COMMIT;
