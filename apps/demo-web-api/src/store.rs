use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Postgres, Row, Transaction};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error("chat resource was not found")]
    NotFound,
    #[error("chat request conflicts with current state: {0}")]
    Conflict(String),
}

#[derive(Clone)]
pub struct ChatStore {
    pool: PgPool,
    tenant_id: String,
    user_id: String,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct SessionSummary {
    pub id: Uuid,
    pub title: String,
    pub mode: String,
    pub guided_step: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct MessageView {
    pub id: Uuid,
    pub session_id: Uuid,
    pub turn_id: Option<Uuid>,
    pub role: String,
    pub content: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct TurnView {
    pub id: Uuid,
    pub session_id: Uuid,
    pub user_message_id: Uuid,
    pub assistant_message_id: Uuid,
    pub task_id: Uuid,
    pub attempt_id: Uuid,
    pub status: String,
    pub context_pack_id: Option<Uuid>,
    pub outcome_receipt_id: Option<Uuid>,
    pub used_memory_ids: Value,
    pub used_memory_version_ids: Value,
    pub memory_effects: Value,
    pub model_id: Option<String>,
    pub model_request_id_hash: Option<String>,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
    pub error_code: Option<String>,
    pub error_detail: Option<String>,
    pub cancel_requested_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionDetail {
    #[serde(flatten)]
    pub session: SessionSummary,
    pub messages: Vec<MessageView>,
    pub turns: Vec<TurnView>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct TurnEventView {
    pub sequence_no: i64,
    pub event_type: String,
    pub payload: Value,
    pub created_at: DateTime<Utc>,
}

pub struct CreateTurnCommand {
    pub session_id: Uuid,
    pub content: String,
    pub client_message_id: String,
    pub idempotency_key: String,
    pub request_hash: String,
}

pub struct CreatedTurn {
    pub turn: TurnView,
    pub created: bool,
}

pub struct CompletedTurn<'a> {
    pub context_pack_id: Option<Uuid>,
    pub outcome_receipt_id: Option<Uuid>,
    pub used_memory_ids: &'a [Uuid],
    pub used_memory_version_ids: &'a [Uuid],
    pub memory_effects: &'a Value,
    pub model_id: &'a str,
    pub model_request_id_hash: Option<&'a str>,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
}

impl ChatStore {
    pub async fn connect_and_migrate(
        database_url: &str,
        tenant_id: String,
        user_id: String,
    ) -> Result<Self, StoreError> {
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect(database_url)
            .await?;
        sqlx::migrate!("../../migrations").run(&pool).await?;
        Ok(Self {
            pool,
            tenant_id,
            user_id,
        })
    }

    pub fn user_id(&self) -> &str {
        &self.user_id
    }

    pub async fn create_session(
        &self,
        title: &str,
        mode: &str,
    ) -> Result<SessionSummary, StoreError> {
        let session_id = Uuid::new_v4();
        let mut tx = self.begin().await?;
        let session = sqlx::query_as::<_, SessionSummary>(
            "INSERT INTO demo_chat.sessions (tenant_id,id,user_id,title,mode) \
             VALUES ($1,$2,$3,$4,$5) \
             RETURNING id,title,mode,guided_step,created_at,updated_at",
        )
        .bind(&self.tenant_id)
        .bind(session_id)
        .bind(&self.user_id)
        .bind(title)
        .bind(mode)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(session)
    }

    pub async fn list_sessions(&self) -> Result<Vec<SessionSummary>, StoreError> {
        let mut tx = self.begin().await?;
        let sessions = sqlx::query_as::<_, SessionSummary>(
            "SELECT id,title,mode,guided_step,created_at,updated_at \
             FROM demo_chat.sessions \
             WHERE tenant_id=$1 AND user_id=$2 AND archived_at IS NULL \
             ORDER BY updated_at DESC,id",
        )
        .bind(&self.tenant_id)
        .bind(&self.user_id)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(sessions)
    }

    pub async fn session(&self, session_id: Uuid) -> Result<SessionDetail, StoreError> {
        let mut tx = self.begin().await?;
        let session = load_session_tx(&mut tx, &self.tenant_id, &self.user_id, session_id).await?;
        let messages = sqlx::query_as::<_, MessageView>(
            "SELECT id,session_id,turn_id,role,content,status,created_at,updated_at \
             FROM demo_chat.messages WHERE tenant_id=$1 AND session_id=$2 \
             ORDER BY created_at,\
                      CASE role WHEN 'user' THEN 0 WHEN 'assistant' THEN 1 ELSE 2 END,\
                      id",
        )
        .bind(&self.tenant_id)
        .bind(session_id)
        .fetch_all(&mut *tx)
        .await?;
        let turns = sqlx::query_as::<_, TurnView>(
            "SELECT id,session_id,user_message_id,assistant_message_id,task_id,attempt_id,status,\
                    context_pack_id,outcome_receipt_id,used_memory_ids,used_memory_version_ids,\
                    memory_effects,model_id,\
                    model_request_id_hash,prompt_tokens,completion_tokens,total_tokens,error_code,\
                    error_detail,cancel_requested_at,created_at,updated_at \
             FROM demo_chat.turns WHERE tenant_id=$1 AND session_id=$2 ORDER BY created_at,id",
        )
        .bind(&self.tenant_id)
        .bind(session_id)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(SessionDetail {
            session,
            messages,
            turns,
        })
    }

    pub async fn create_turn(&self, command: CreateTurnCommand) -> Result<CreatedTurn, StoreError> {
        let mut tx = self.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!(
                "demo-chat-turn:{}:{}:{}",
                self.tenant_id, command.session_id, command.idempotency_key
            ))
            .execute(&mut *tx)
            .await?;
        load_session_tx(&mut tx, &self.tenant_id, &self.user_id, command.session_id).await?;
        if let Some(row) = sqlx::query(
            "SELECT id,request_hash FROM demo_chat.turns \
             WHERE tenant_id=$1 AND idempotency_key=$2",
        )
        .bind(&self.tenant_id)
        .bind(&command.idempotency_key)
        .fetch_optional(&mut *tx)
        .await?
        {
            if row.try_get::<String, _>("request_hash")? != command.request_hash {
                return Err(StoreError::Conflict(
                    "Idempotency-Key was reused with a different message".into(),
                ));
            }
            let turn_id: Uuid = row.try_get("id")?;
            let turn = load_turn_tx(&mut tx, &self.tenant_id, turn_id).await?;
            tx.commit().await?;
            return Ok(CreatedTurn {
                turn,
                created: false,
            });
        }

        let active = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM demo_chat.turns \
             WHERE tenant_id=$1 AND session_id=$2 \
               AND status IN ('accepted','processing','streaming'))",
        )
        .bind(&self.tenant_id)
        .bind(command.session_id)
        .fetch_one(&mut *tx)
        .await?;
        if active {
            return Err(StoreError::Conflict(
                "this conversation already has an active turn".into(),
            ));
        }

        let turn_id = Uuid::new_v4();
        let user_message_id = Uuid::new_v4();
        let assistant_message_id = Uuid::new_v4();
        let task_id = Uuid::new_v4();
        let attempt_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO demo_chat.messages (\
                 tenant_id,id,session_id,client_message_id,role,content,status\
             ) VALUES ($1,$2,$3,$4,'user',$5,'completed'),\
                      ($1,$6,$3,NULL,'assistant','','pending')",
        )
        .bind(&self.tenant_id)
        .bind(user_message_id)
        .bind(command.session_id)
        .bind(&command.client_message_id)
        .bind(&command.content)
        .bind(assistant_message_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO demo_chat.turns (\
                 tenant_id,id,session_id,user_message_id,assistant_message_id,task_id,attempt_id,\
                 idempotency_key,request_hash\
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
        )
        .bind(&self.tenant_id)
        .bind(turn_id)
        .bind(command.session_id)
        .bind(user_message_id)
        .bind(assistant_message_id)
        .bind(task_id)
        .bind(attempt_id)
        .bind(&command.idempotency_key)
        .bind(&command.request_hash)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE demo_chat.messages SET turn_id=$3 \
             WHERE tenant_id=$1 AND id=ANY($2)",
        )
        .bind(&self.tenant_id)
        .bind(vec![user_message_id, assistant_message_id])
        .bind(turn_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE demo_chat.sessions SET \
                 title=CASE WHEN NOT EXISTS(\
                     SELECT 1 FROM demo_chat.turns t \
                     WHERE t.tenant_id=$1 AND t.session_id=$2 AND t.id<>$3\
                 ) THEN left($4,60) ELSE title END,\
                 updated_at=now() \
             WHERE tenant_id=$1 AND id=$2",
        )
        .bind(&self.tenant_id)
        .bind(command.session_id)
        .bind(turn_id)
        .bind(&command.content)
        .execute(&mut *tx)
        .await?;
        insert_event_tx(
            &mut tx,
            &self.tenant_id,
            turn_id,
            "turn.accepted",
            json!({"turn_id": turn_id, "task_id": task_id, "attempt_id": attempt_id}),
        )
        .await?;
        let turn = load_turn_tx(&mut tx, &self.tenant_id, turn_id).await?;
        tx.commit().await?;
        Ok(CreatedTurn {
            turn,
            created: true,
        })
    }

    pub async fn start_turn(&self, turn_id: Uuid) -> Result<(), StoreError> {
        let mut tx = self.begin().await?;
        let result = sqlx::query(
            "UPDATE demo_chat.turns SET status='processing',started_at=now(),updated_at=now() \
             WHERE tenant_id=$1 AND id=$2 AND status='accepted'",
        )
        .bind(&self.tenant_id)
        .bind(turn_id)
        .execute(&mut *tx)
        .await?;
        if result.rows_affected() != 1 {
            return Err(StoreError::Conflict("turn is no longer accepted".into()));
        }
        insert_event_tx(
            &mut tx,
            &self.tenant_id,
            turn_id,
            "turn.processing",
            json!({}),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn append_event(
        &self,
        turn_id: Uuid,
        event_type: &str,
        payload: Value,
    ) -> Result<i64, StoreError> {
        let mut tx = self.begin().await?;
        ensure_turn_tx(&mut tx, &self.tenant_id, turn_id).await?;
        let sequence =
            insert_event_tx(&mut tx, &self.tenant_id, turn_id, event_type, payload).await?;
        tx.commit().await?;
        Ok(sequence)
    }

    pub async fn begin_streaming(
        &self,
        turn_id: Uuid,
        context_pack_id: Option<Uuid>,
        used_memory_ids: &[Uuid],
        used_memory_version_ids: &[Uuid],
        memory_effects: &Value,
    ) -> Result<(), StoreError> {
        if used_memory_ids.len() != used_memory_version_ids.len() {
            return Err(StoreError::Conflict(
                "memory record and version evidence counts differ".into(),
            ));
        }
        let mut tx = self.begin().await?;
        let used = serde_json::to_value(used_memory_ids).expect("UUID JSON encoding cannot fail");
        let used_versions =
            serde_json::to_value(used_memory_version_ids).expect("UUID JSON encoding cannot fail");
        let result = sqlx::query(
            "UPDATE demo_chat.turns SET status='streaming',context_pack_id=$3,\
                 used_memory_ids=$4,used_memory_version_ids=$5,memory_effects=$6,updated_at=now() \
             WHERE tenant_id=$1 AND id=$2 AND status='processing' \
               AND cancel_requested_at IS NULL",
        )
        .bind(&self.tenant_id)
        .bind(turn_id)
        .bind(context_pack_id)
        .bind(&used)
        .bind(&used_versions)
        .bind(memory_effects)
        .execute(&mut *tx)
        .await?;
        if result.rows_affected() != 1 {
            return Err(StoreError::Conflict(
                "turn was cancelled before streaming".into(),
            ));
        }
        let assistant_id = sqlx::query_scalar::<_, Uuid>(
            "SELECT assistant_message_id FROM demo_chat.turns WHERE tenant_id=$1 AND id=$2",
        )
        .bind(&self.tenant_id)
        .bind(turn_id)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE demo_chat.messages SET status='streaming',updated_at=now() \
             WHERE tenant_id=$1 AND id=$2",
        )
        .bind(&self.tenant_id)
        .bind(assistant_id)
        .execute(&mut *tx)
        .await?;
        insert_event_tx(
            &mut tx,
            &self.tenant_id,
            turn_id,
            "context.ready",
            json!({
                "context_pack_id": context_pack_id,
                "used_memory_ids": used_memory_ids,
                "used_memory_version_ids": used_memory_version_ids,
                "memory_effects": memory_effects,
            }),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn append_answer_delta(&self, turn_id: Uuid, delta: &str) -> Result<(), StoreError> {
        let mut tx = self.begin().await?;
        let row = sqlx::query(
            "SELECT assistant_message_id,status,cancel_requested_at \
             FROM demo_chat.turns WHERE tenant_id=$1 AND id=$2 FOR UPDATE",
        )
        .bind(&self.tenant_id)
        .bind(turn_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(StoreError::NotFound)?;
        if row.try_get::<String, _>("status")? != "streaming"
            || row
                .try_get::<Option<DateTime<Utc>>, _>("cancel_requested_at")?
                .is_some()
        {
            return Err(StoreError::Conflict("generation was cancelled".into()));
        }
        let assistant_id: Uuid = row.try_get("assistant_message_id")?;
        sqlx::query(
            "UPDATE demo_chat.messages SET content=content || $3,updated_at=now() \
             WHERE tenant_id=$1 AND id=$2",
        )
        .bind(&self.tenant_id)
        .bind(assistant_id)
        .bind(delta)
        .execute(&mut *tx)
        .await?;
        insert_event_tx(
            &mut tx,
            &self.tenant_id,
            turn_id,
            "answer.delta",
            json!({"delta": delta}),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn complete_turn(
        &self,
        turn_id: Uuid,
        completed: CompletedTurn<'_>,
    ) -> Result<(), StoreError> {
        if completed.used_memory_ids.len() != completed.used_memory_version_ids.len() {
            return Err(StoreError::Conflict(
                "memory record and version evidence counts differ".into(),
            ));
        }
        let mut tx = self.begin().await?;
        let used = serde_json::to_value(completed.used_memory_ids)
            .expect("UUID JSON encoding cannot fail");
        let used_versions = serde_json::to_value(completed.used_memory_version_ids)
            .expect("UUID JSON encoding cannot fail");
        let assistant_id = sqlx::query_scalar::<_, Uuid>(
            "UPDATE demo_chat.turns SET status='completed',context_pack_id=$3,\
                 outcome_receipt_id=$4,used_memory_ids=$5,used_memory_version_ids=$6,\
                 memory_effects=$7,model_id=$8,model_request_id_hash=$9,prompt_tokens=$10,\
                 completion_tokens=$11,total_tokens=$12,\
                 completed_at=now(),updated_at=now() \
             WHERE tenant_id=$1 AND id=$2 AND status='streaming' \
             RETURNING assistant_message_id",
        )
        .bind(&self.tenant_id)
        .bind(turn_id)
        .bind(completed.context_pack_id)
        .bind(completed.outcome_receipt_id)
        .bind(&used)
        .bind(&used_versions)
        .bind(completed.memory_effects)
        .bind(completed.model_id)
        .bind(completed.model_request_id_hash)
        .bind(completed.prompt_tokens)
        .bind(completed.completion_tokens)
        .bind(completed.total_tokens)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| StoreError::Conflict("turn cannot be completed".into()))?;
        sqlx::query(
            "UPDATE demo_chat.messages SET status='completed',updated_at=now() \
             WHERE tenant_id=$1 AND id=$2",
        )
        .bind(&self.tenant_id)
        .bind(assistant_id)
        .execute(&mut *tx)
        .await?;
        insert_event_tx(
            &mut tx,
            &self.tenant_id,
            turn_id,
            "turn.completed",
            json!({
                "used_memory_count": completed.used_memory_ids.len(),
                "used_memory_ids": completed.used_memory_ids,
                "used_memory_version_ids": completed.used_memory_version_ids,
                "context_pack_id": completed.context_pack_id,
                "outcome_receipt_id": completed.outcome_receipt_id,
                "model_id": completed.model_id,
                "model_request_id_hash": completed.model_request_id_hash,
            }),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn fail_turn(
        &self,
        turn_id: Uuid,
        code: &str,
        detail: &str,
    ) -> Result<(), StoreError> {
        self.terminal_turn(turn_id, "failed", code, detail).await
    }

    pub async fn finish_cancelled(&self, turn_id: Uuid, detail: &str) -> Result<(), StoreError> {
        self.terminal_turn(turn_id, "cancelled", "TURN_CANCELLED", detail)
            .await
    }

    async fn terminal_turn(
        &self,
        turn_id: Uuid,
        status: &str,
        code: &str,
        detail: &str,
    ) -> Result<(), StoreError> {
        let mut tx = self.begin().await?;
        let assistant_id = sqlx::query_scalar::<_, Uuid>(
            "UPDATE demo_chat.turns SET status=$3,error_code=$4,error_detail=$5,\
                 completed_at=now(),updated_at=now() \
             WHERE tenant_id=$1 AND id=$2 \
               AND status IN ('accepted','processing','streaming') \
             RETURNING assistant_message_id",
        )
        .bind(&self.tenant_id)
        .bind(turn_id)
        .bind(status)
        .bind(code)
        .bind(detail)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| StoreError::Conflict("turn is already terminal".into()))?;
        sqlx::query(
            "UPDATE demo_chat.messages SET status=$3,updated_at=now() \
             WHERE tenant_id=$1 AND id=$2",
        )
        .bind(&self.tenant_id)
        .bind(assistant_id)
        .bind(status)
        .execute(&mut *tx)
        .await?;
        insert_event_tx(
            &mut tx,
            &self.tenant_id,
            turn_id,
            if status == "cancelled" {
                "turn.cancelled"
            } else {
                "turn.failed"
            },
            json!({"code": code, "detail": detail}),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn request_cancel(&self, turn_id: Uuid) -> Result<TurnView, StoreError> {
        let mut tx = self.begin().await?;
        let row = sqlx::query(
            "UPDATE demo_chat.turns SET cancel_requested_at=COALESCE(cancel_requested_at,now()),\
                 updated_at=now() WHERE tenant_id=$1 AND id=$2 \
                 AND status IN ('accepted','processing','streaming') RETURNING id",
        )
        .bind(&self.tenant_id)
        .bind(turn_id)
        .fetch_optional(&mut *tx)
        .await?;
        if row.is_none() {
            let existing = load_turn_tx(&mut tx, &self.tenant_id, turn_id).await?;
            tx.commit().await?;
            return Ok(existing);
        }
        insert_event_tx(
            &mut tx,
            &self.tenant_id,
            turn_id,
            "turn.cancel_requested",
            json!({}),
        )
        .await?;
        let turn = load_turn_tx(&mut tx, &self.tenant_id, turn_id).await?;
        tx.commit().await?;
        Ok(turn)
    }

    pub async fn cancel_requested(&self, turn_id: Uuid) -> Result<bool, StoreError> {
        let mut tx = self.begin().await?;
        let cancelled = sqlx::query_scalar::<_, bool>(
            "SELECT cancel_requested_at IS NOT NULL OR status='cancelled' \
             FROM demo_chat.turns WHERE tenant_id=$1 AND id=$2",
        )
        .bind(&self.tenant_id)
        .bind(turn_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(StoreError::NotFound)?;
        tx.commit().await?;
        Ok(cancelled)
    }

    pub async fn turn(&self, turn_id: Uuid) -> Result<TurnView, StoreError> {
        let mut tx = self.begin().await?;
        let turn = load_turn_tx(&mut tx, &self.tenant_id, turn_id).await?;
        let owns = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM demo_chat.sessions \
             WHERE tenant_id=$1 AND id=$2 AND user_id=$3)",
        )
        .bind(&self.tenant_id)
        .bind(turn.session_id)
        .bind(&self.user_id)
        .fetch_one(&mut *tx)
        .await?;
        if !owns {
            return Err(StoreError::NotFound);
        }
        tx.commit().await?;
        Ok(turn)
    }

    pub async fn events_after(
        &self,
        turn_id: Uuid,
        after: i64,
    ) -> Result<(Vec<TurnEventView>, String), StoreError> {
        let mut tx = self.begin().await?;
        let turn = load_turn_tx(&mut tx, &self.tenant_id, turn_id).await?;
        let events = sqlx::query_as::<_, TurnEventView>(
            "SELECT sequence_no,event_type,payload,created_at \
             FROM demo_chat.turn_events \
             WHERE tenant_id=$1 AND turn_id=$2 AND sequence_no>$3 \
             ORDER BY sequence_no LIMIT 200",
        )
        .bind(&self.tenant_id)
        .bind(turn_id)
        .bind(after.max(0))
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok((events, turn.status))
    }

    pub async fn reset_sessions(&self) -> Result<(), StoreError> {
        let mut tx = self.begin().await?;
        sqlx::query("DELETE FROM demo_chat.sessions WHERE tenant_id=$1 AND user_id=$2")
            .bind(&self.tenant_id)
            .bind(&self.user_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn begin(&self) -> Result<Transaction<'_, Postgres>, StoreError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
            .bind(&self.tenant_id)
            .execute(&mut *tx)
            .await?;
        Ok(tx)
    }
}

async fn load_session_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    user_id: &str,
    session_id: Uuid,
) -> Result<SessionSummary, StoreError> {
    sqlx::query_as::<_, SessionSummary>(
        "SELECT id,title,mode,guided_step,created_at,updated_at \
         FROM demo_chat.sessions \
         WHERE tenant_id=$1 AND id=$2 AND user_id=$3 AND archived_at IS NULL",
    )
    .bind(tenant_id)
    .bind(session_id)
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(StoreError::NotFound)
}

async fn load_turn_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    turn_id: Uuid,
) -> Result<TurnView, StoreError> {
    sqlx::query_as::<_, TurnView>(
        "SELECT id,session_id,user_message_id,assistant_message_id,task_id,attempt_id,status,\
                context_pack_id,outcome_receipt_id,used_memory_ids,used_memory_version_ids,\
                memory_effects,model_id,\
                model_request_id_hash,prompt_tokens,completion_tokens,total_tokens,error_code,\
                error_detail,cancel_requested_at,created_at,updated_at \
         FROM demo_chat.turns WHERE tenant_id=$1 AND id=$2",
    )
    .bind(tenant_id)
    .bind(turn_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(StoreError::NotFound)
}

async fn ensure_turn_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    turn_id: Uuid,
) -> Result<(), StoreError> {
    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM demo_chat.turns WHERE tenant_id=$1 AND id=$2)",
    )
    .bind(tenant_id)
    .bind(turn_id)
    .fetch_one(&mut **tx)
    .await?;
    if exists {
        Ok(())
    } else {
        Err(StoreError::NotFound)
    }
}

async fn insert_event_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    turn_id: Uuid,
    event_type: &str,
    payload: Value,
) -> Result<i64, StoreError> {
    sqlx::query("SELECT id FROM demo_chat.turns WHERE tenant_id=$1 AND id=$2 FOR UPDATE")
        .bind(tenant_id)
        .bind(turn_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(StoreError::NotFound)?;
    let sequence = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(max(sequence_no),0)+1 FROM demo_chat.turn_events \
         WHERE tenant_id=$1 AND turn_id=$2",
    )
    .bind(tenant_id)
    .bind(turn_id)
    .fetch_one(&mut **tx)
    .await?;
    sqlx::query(
        "INSERT INTO demo_chat.turn_events (tenant_id,turn_id,sequence_no,event_type,payload) \
         VALUES ($1,$2,$3,$4,$5)",
    )
    .bind(tenant_id)
    .bind(turn_id)
    .bind(sequence)
    .bind(event_type)
    .bind(payload)
    .execute(&mut **tx)
    .await?;
    Ok(sequence)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires a migrated local PostgreSQL configured by TEST_DATABASE_URL"]
    async fn persists_replays_and_resets_a_chat_turn() {
        let database_url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            "postgres://memory:memory-local-only@127.0.0.1:55432/memory".into()
        });
        let test_id = Uuid::new_v4();
        let store = ChatStore::connect_and_migrate(
            &database_url,
            format!("demo-chat-test-{test_id}"),
            "alice".into(),
        )
        .await
        .unwrap();
        let session = store.create_session("测试对话", "guided").await.unwrap();
        let created = store
            .create_turn(CreateTurnCommand {
                session_id: session.id,
                content: "请记住我偏好简洁回答".into(),
                client_message_id: format!("client-{test_id}"),
                idempotency_key: format!("turn-idempotency-{test_id}"),
                request_hash: format!("sha256:{test_id}"),
            })
            .await
            .unwrap();
        assert!(created.created);
        let replay = store
            .create_turn(CreateTurnCommand {
                session_id: session.id,
                content: "请记住我偏好简洁回答".into(),
                client_message_id: format!("client-{test_id}"),
                idempotency_key: format!("turn-idempotency-{test_id}"),
                request_hash: format!("sha256:{test_id}"),
            })
            .await
            .unwrap();
        assert!(!replay.created);
        assert_eq!(replay.turn.id, created.turn.id);
        let conflict = store
            .create_turn(CreateTurnCommand {
                session_id: session.id,
                content: "不同消息".into(),
                client_message_id: format!("client-conflict-{test_id}"),
                idempotency_key: format!("turn-idempotency-{test_id}"),
                request_hash: format!("sha256:different-{test_id}"),
            })
            .await;
        assert!(matches!(conflict, Err(StoreError::Conflict(_))));
        store.start_turn(created.turn.id).await.unwrap();
        let used_memory_id = Uuid::new_v4();
        let used_memory_version_id = Uuid::new_v4();
        store
            .begin_streaming(
                created.turn.id,
                None,
                &[used_memory_id],
                &[used_memory_version_id],
                &json!([]),
            )
            .await
            .unwrap();
        store
            .append_answer_delta(created.turn.id, "已记住。")
            .await
            .unwrap();
        store
            .complete_turn(
                created.turn.id,
                CompletedTurn {
                    context_pack_id: None,
                    outcome_receipt_id: None,
                    used_memory_ids: &[used_memory_id],
                    used_memory_version_ids: &[used_memory_version_id],
                    memory_effects: &json!([]),
                    model_id: "k3",
                    model_request_id_hash: None,
                    prompt_tokens: None,
                    completion_tokens: None,
                    total_tokens: None,
                },
            )
            .await
            .unwrap();
        let detail = store.session(session.id).await.unwrap();
        assert_eq!(detail.messages.len(), 2);
        assert_eq!(detail.messages[1].content, "已记住。");
        assert_eq!(detail.turns[0].used_memory_ids, json!([used_memory_id]));
        assert_eq!(
            detail.turns[0].used_memory_version_ids,
            json!([used_memory_version_id])
        );
        let (events, status) = store.events_after(created.turn.id, 0).await.unwrap();
        assert_eq!(status, "completed");
        assert!(
            events
                .iter()
                .any(|event| event.event_type == "answer.delta")
        );
        store.reset_sessions().await.unwrap();
        assert!(store.list_sessions().await.unwrap().is_empty());
    }
}
