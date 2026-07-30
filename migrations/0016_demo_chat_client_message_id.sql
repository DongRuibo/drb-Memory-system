BEGIN;

ALTER TABLE demo_chat.messages
    DROP CONSTRAINT messages_tenant_id_session_id_client_message_id_key;

CREATE UNIQUE INDEX uq_demo_chat_client_message_id
    ON demo_chat.messages (tenant_id, session_id, client_message_id)
    WHERE client_message_id IS NOT NULL;

COMMIT;
