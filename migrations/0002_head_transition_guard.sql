BEGIN;

CREATE OR REPLACE FUNCTION memory_core.guard_memory_head_update()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $function$
BEGIN
    IF NEW.tenant_id <> OLD.tenant_id OR NEW.memory_record_id <> OLD.memory_record_id THEN
        RAISE EXCEPTION 'memory head identity is immutable'
            USING ERRCODE = '23514';
    END IF;

    IF NEW.revision <> OLD.revision + 1 THEN
        RAISE EXCEPTION 'memory head revision must advance by exactly one'
            USING ERRCODE = '23514';
    END IF;

    IF OLD.state = 'erased' THEN
        RAISE EXCEPTION 'erased memory head is terminal'
            USING ERRCODE = '23514';
    END IF;

    IF OLD.state = 'forgotten' AND NEW.state <> 'erased' THEN
        RAISE EXCEPTION 'forgotten memory head may only transition to erased'
            USING ERRCODE = '23514';
    END IF;

    IF OLD.state = 'expired' AND NEW.state NOT IN ('expired', 'forgotten') THEN
        RAISE EXCEPTION 'expired memory head cannot become active or contested'
            USING ERRCODE = '23514';
    END IF;

    IF NEW.state IN ('active', 'contested', 'expired') AND NEW.current_version_id IS NULL THEN
        RAISE EXCEPTION 'readable or reviewable head requires a current version'
            USING ERRCODE = '23514';
    END IF;

    NEW.updated_at := now();
    RETURN NEW;
END
$function$;

CREATE TRIGGER trg_guard_memory_head_update
BEFORE UPDATE ON memory_core.memory_heads
FOR EACH ROW
EXECUTE FUNCTION memory_core.guard_memory_head_update();

COMMIT;
