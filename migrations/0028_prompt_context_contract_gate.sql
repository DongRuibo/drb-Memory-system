BEGIN;

-- Only Context Packs compiled with the prompt contract that actually injects runtime_context and
-- item event_time may be presented as exact model-input evidence. Older packs were backfilled for
-- data integrity by 0027, but their historical model calls did not receive those new fields.
CREATE OR REPLACE FUNCTION demo_chat.prompt_context_for_turn(
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
      AND pack.compiler_version = 'demo-context-compiler-v2'
$$;

COMMIT;
