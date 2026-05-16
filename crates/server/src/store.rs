use std::collections::HashMap;
#[cfg(feature = "sqlite")]
use std::path::Path;
use std::sync::RwLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, anyhow};
use protocol::chat::ChatMessage;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

// ── Data types ───────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ResponseState {
    pub id: String,
    pub conversation_id: Option<String>,
    pub model: String,
    pub created_at: i64,
    pub status: String,
    pub response_json: Value,
    pub previous_response_id: Option<String>,
    pub canonical_messages: Vec<ChatMessage>,
}

#[derive(Debug, Clone)]
pub struct DebugArtifact {
    pub id: String,
    pub conversation_id: Option<String>,
    pub model: String,
    pub created_at: i64,
    pub status: String,
    pub request_json: Value,
    pub mapped_request_json: Value,
    pub upstream_error: Option<Value>,
    pub debug_annotations: Vec<String>,
    pub upstream_sse_events: Vec<Value>,
    pub response_sse_events: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebugArtifactView {
    pub id: String,
    pub model: String,
    pub created_at: i64,
    pub status: String,
    pub request_json: Value,
    pub mapped_request_json: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_error: Option<Value>,
    #[serde(default)]
    pub debug_annotations: Vec<String>,
    #[serde(default)]
    pub upstream_sse_events: Vec<Value>,
    #[serde(default)]
    pub response_sse_events: Vec<Value>,
    #[serde(default)]
    pub conversation_id: Option<String>,
    pub artifact_expires_at: i64,
}

#[derive(Debug, Clone)]
struct ResponseRecord {
    response_json: Value,
    expires_at: i64,
}

#[derive(Debug, Clone)]
struct DebugArtifactRecord {
    artifact: DebugArtifact,
    expires_at: i64,
}

#[derive(Debug, Clone)]
struct ReasoningEntry {
    response_id: String,
    reasoning: String,
}

#[derive(Debug, Clone)]
struct MessageBlob {
    message: ChatMessage,
}

fn default_conversation_id(conversation_id: Option<&str>) -> String {
    conversation_id.unwrap_or("default").to_string()
}

fn now_unix_seconds() -> anyhow::Result<i64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before Unix epoch")?
        .as_secs() as i64)
}

fn message_hash(message: &ChatMessage) -> anyhow::Result<(String, String)> {
    let json = serde_json::to_string(message).context("failed to serialize canonical message")?;
    let digest = Sha256::digest(json.as_bytes());
    let mut hash = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut hash, "{byte:02x}").expect("writing to String cannot fail");
    }
    Ok((hash, json))
}

fn debug_view(record: DebugArtifactRecord) -> DebugArtifactView {
    DebugArtifactView {
        id: record.artifact.id,
        model: record.artifact.model,
        created_at: record.artifact.created_at,
        status: record.artifact.status,
        request_json: record.artifact.request_json,
        mapped_request_json: record.artifact.mapped_request_json,
        upstream_error: record.artifact.upstream_error,
        debug_annotations: record.artifact.debug_annotations,
        upstream_sse_events: record.artifact.upstream_sse_events,
        response_sse_events: record.artifact.response_sse_events,
        conversation_id: record.artifact.conversation_id,
        artifact_expires_at: record.expires_at,
    }
}

fn reasoning_map_from_messages(messages: &[ChatMessage]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for msg in messages {
        if let protocol::chat::ChatMessage::Assistant {
            reasoning_content: Some(rc),
            tool_calls: Some(tool_calls),
            ..
        } = msg
            && !rc.is_empty()
        {
            for tc in tool_calls {
                map.insert(tc.id.clone(), rc.clone());
            }
        }
    }
    map
}

// ── Backend trait ────────────────────────────────────────────────

pub trait ResponseStoreBackend: Send + Sync {
    fn put_response_state(&self, state: ResponseState) -> anyhow::Result<()>;
    fn get_response_json(&self, id: &str) -> anyhow::Result<Option<Value>>;
    fn get_canonical_messages(&self, id: &str) -> anyhow::Result<Option<Vec<ChatMessage>>>;
    fn put_debug_artifact(&self, artifact: DebugArtifact) -> anyhow::Result<()>;
    fn get_debug_artifact(&self, id: &str) -> anyhow::Result<Option<DebugArtifactView>>;
    fn list_debug_artifacts(&self, limit: usize) -> anyhow::Result<Vec<DebugArtifactView>>;
    fn delete(&self, id: &str) -> anyhow::Result<bool>;
    fn clear(&self) -> anyhow::Result<()>;
    fn cleanup_expired(&self) -> anyhow::Result<usize>;
    fn len(&self) -> anyhow::Result<usize>;
    fn find_reasoning_for_call_ids(
        &self,
        conv_id: &str,
        call_ids: &[String],
    ) -> anyhow::Result<HashMap<String, String>>;
}

// ── Memory store ─────────────────────────────────────────────────

pub struct MemoryStore {
    inner: RwLock<MemoryInner>,
    ttl: Duration,
    debug_ttl: Duration,
}

#[derive(Default)]
struct MemoryInner {
    responses: HashMap<String, ResponseRecord>,
    refs: HashMap<String, Vec<String>>,
    blobs: HashMap<String, MessageBlob>,
    debug_artifacts: HashMap<String, DebugArtifactRecord>,
    reasoning: HashMap<(String, String), ReasoningEntry>,
}

impl MemoryStore {
    pub fn new(ttl_seconds: u64, debug_artifact_ttl_seconds: u64) -> Self {
        Self {
            inner: RwLock::new(MemoryInner::default()),
            ttl: Duration::from_secs(ttl_seconds),
            debug_ttl: Duration::from_secs(debug_artifact_ttl_seconds),
        }
    }
}

impl MemoryInner {
    fn insert_response(&mut self, state: ResponseState, ttl: Duration) -> anyhow::Result<()> {
        let conv_id = default_conversation_id(state.conversation_id.as_deref());
        let mut refs = Vec::with_capacity(state.canonical_messages.len());
        for message in &state.canonical_messages {
            let (hash, _json) = message_hash(message)?;
            self.blobs
                .entry(hash.clone())
                .or_insert_with(|| MessageBlob {
                    message: message.clone(),
                });
            refs.push(hash);
        }

        self.refs.insert(state.id.clone(), refs);
        self.reasoning
            .retain(|_, entry| entry.response_id != state.id);
        for (call_id, reasoning) in reasoning_map_from_messages(&state.canonical_messages) {
            self.reasoning.insert(
                (conv_id.clone(), call_id),
                ReasoningEntry {
                    response_id: state.id.clone(),
                    reasoning,
                },
            );
        }

        self.responses.insert(
            state.id.clone(),
            ResponseRecord {
                response_json: state.response_json,
                expires_at: state.created_at + ttl.as_secs() as i64,
            },
        );
        self.cleanup_unreferenced_blobs();
        Ok(())
    }

    fn canonical_messages(&self, id: &str) -> anyhow::Result<Option<Vec<ChatMessage>>> {
        let Some(refs) = self.refs.get(id) else {
            if self.responses.contains_key(id) {
                return Err(anyhow!("response {id} has no canonical message refs"));
            }
            return Ok(None);
        };
        let mut messages = Vec::with_capacity(refs.len());
        for hash in refs {
            let blob = self.blobs.get(hash).ok_or_else(|| {
                anyhow!("canonical message blob {hash} missing for response {id}")
            })?;
            messages.push(blob.message.clone());
        }
        Ok(Some(messages))
    }

    fn delete_response(&mut self, id: &str) -> bool {
        let existed = self.responses.remove(id).is_some();
        self.refs.remove(id);
        self.debug_artifacts.remove(id);
        self.reasoning
            .retain(|_, entry| entry.response_id.as_str() != id);
        self.cleanup_unreferenced_blobs();
        existed
    }

    fn cleanup_unreferenced_blobs(&mut self) {
        let mut ref_counts: HashMap<&str, usize> = HashMap::new();
        for refs in self.refs.values() {
            for hash in refs {
                *ref_counts.entry(hash.as_str()).or_default() += 1;
            }
        }
        self.blobs
            .retain(|hash, _| ref_counts.contains_key(hash.as_str()));
    }
}

impl ResponseStoreBackend for MemoryStore {
    fn put_response_state(&self, state: ResponseState) -> anyhow::Result<()> {
        self.inner.write().unwrap().insert_response(state, self.ttl)
    }

    fn get_response_json(&self, id: &str) -> anyhow::Result<Option<Value>> {
        Ok(self
            .inner
            .read()
            .unwrap()
            .responses
            .get(id)
            .map(|record| record.response_json.clone()))
    }

    fn get_canonical_messages(&self, id: &str) -> anyhow::Result<Option<Vec<ChatMessage>>> {
        self.inner.read().unwrap().canonical_messages(id)
    }

    fn put_debug_artifact(&self, artifact: DebugArtifact) -> anyhow::Result<()> {
        let expires_at = now_unix_seconds()? + self.debug_ttl.as_secs() as i64;
        self.inner.write().unwrap().debug_artifacts.insert(
            artifact.id.clone(),
            DebugArtifactRecord {
                artifact,
                expires_at,
            },
        );
        Ok(())
    }

    fn get_debug_artifact(&self, id: &str) -> anyhow::Result<Option<DebugArtifactView>> {
        let now = now_unix_seconds()?;
        let mut inner = self.inner.write().unwrap();
        let Some(record) = inner.debug_artifacts.get(id) else {
            return Ok(None);
        };
        if record.expires_at <= now {
            inner.debug_artifacts.remove(id);
            return Ok(None);
        }
        Ok(inner.debug_artifacts.get(id).cloned().map(debug_view))
    }

    fn list_debug_artifacts(&self, limit: usize) -> anyhow::Result<Vec<DebugArtifactView>> {
        let now = now_unix_seconds()?;
        let mut inner = self.inner.write().unwrap();
        inner
            .debug_artifacts
            .retain(|_, record| record.expires_at > now);
        let mut artifacts: Vec<_> = inner
            .debug_artifacts
            .values()
            .cloned()
            .map(debug_view)
            .collect();
        artifacts.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| b.id.cmp(&a.id))
        });
        artifacts.truncate(limit);
        Ok(artifacts)
    }

    fn delete(&self, id: &str) -> anyhow::Result<bool> {
        Ok(self.inner.write().unwrap().delete_response(id))
    }

    fn clear(&self) -> anyhow::Result<()> {
        *self.inner.write().unwrap() = MemoryInner::default();
        Ok(())
    }

    fn cleanup_expired(&self) -> anyhow::Result<usize> {
        let now = now_unix_seconds()?;
        let mut inner = self.inner.write().unwrap();
        let expired_ids = inner
            .responses
            .iter()
            .filter_map(|(id, record)| (record.expires_at <= now).then_some(id.clone()))
            .collect::<Vec<_>>();
        for id in &expired_ids {
            inner.delete_response(id);
        }
        let before_debug = inner.debug_artifacts.len();
        inner
            .debug_artifacts
            .retain(|_, record| record.expires_at > now);
        Ok(expired_ids.len() + before_debug - inner.debug_artifacts.len())
    }

    fn len(&self) -> anyhow::Result<usize> {
        Ok(self.inner.read().unwrap().responses.len())
    }

    fn find_reasoning_for_call_ids(
        &self,
        conv_id: &str,
        call_ids: &[String],
    ) -> anyhow::Result<HashMap<String, String>> {
        let inner = self.inner.read().unwrap();
        let mut map = HashMap::new();
        for call_id in call_ids {
            if let Some(entry) = inner.reasoning.get(&(conv_id.to_string(), call_id.clone())) {
                map.insert(call_id.clone(), entry.reasoning.clone());
            }
        }
        Ok(map)
    }
}

// ── SQLite store ─────────────────────────────────────────────────

#[cfg(feature = "sqlite")]
pub struct SqliteStore {
    conn: std::sync::Mutex<rusqlite::Connection>,
    ttl: Duration,
    debug_ttl: Duration,
}

#[cfg(feature = "sqlite")]
impl SqliteStore {
    pub fn new(
        path: impl AsRef<Path>,
        ttl_seconds: u64,
        debug_artifact_ttl_seconds: u64,
    ) -> anyhow::Result<Self> {
        let mut conn = rusqlite::Connection::open(path)?;
        conn.pragma_update(None, "auto_vacuum", "INCREMENTAL")
            .context("failed to enable SQLite incremental vacuum")?;
        if table_exists(&conn, "responses")?
            && table_has_column(&conn, "responses", "request_json")?
        {
            migrate_legacy_schema(&mut conn, ttl_seconds, debug_artifact_ttl_seconds)?;
        } else {
            create_v2_schema(&conn)?;
        }
        Ok(Self {
            conn: std::sync::Mutex::new(conn),
            ttl: Duration::from_secs(ttl_seconds),
            debug_ttl: Duration::from_secs(debug_artifact_ttl_seconds),
        })
    }

    fn insert_response_with_conn(
        conn: &rusqlite::Connection,
        state: ResponseState,
        ttl: Duration,
    ) -> anyhow::Result<()> {
        let conv_id = default_conversation_id(state.conversation_id.as_deref());
        let expires_at = state.created_at + ttl.as_secs() as i64;
        let response_json = serde_json::to_string(&state.response_json)
            .context("failed to serialize response_json")?;
        conn.execute(
            "INSERT OR REPLACE INTO responses (id, conversation_id, model, created_at, status, response_json, previous_response_id, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                &state.id,
                &conv_id,
                &state.model,
                state.created_at,
                &state.status,
                response_json,
                &state.previous_response_id,
                expires_at,
            ],
        )
        .with_context(|| format!("failed to write response state {}", state.id))?;
        conn.execute(
            "DELETE FROM response_canonical_refs WHERE response_id = ?1",
            rusqlite::params![&state.id],
        )
        .with_context(|| format!("failed to replace canonical refs for {}", state.id))?;
        conn.execute(
            "DELETE FROM tool_reasoning_refs WHERE response_id = ?1",
            rusqlite::params![&state.id],
        )
        .with_context(|| format!("failed to replace reasoning refs for {}", state.id))?;

        for (ordinal, message) in state.canonical_messages.iter().enumerate() {
            let (hash, json) = message_hash(message)?;
            conn.execute(
                "INSERT OR IGNORE INTO canonical_message_blobs (hash, message_json)
                 VALUES (?1, ?2)",
                rusqlite::params![&hash, json],
            )
            .with_context(|| format!("failed to intern canonical message {hash}"))?;
            conn.execute(
                "INSERT INTO response_canonical_refs (response_id, ordinal, message_hash)
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![&state.id, ordinal as i64, &hash],
            )
            .with_context(|| format!("failed to write canonical ref {ordinal} for {}", state.id))?;
        }

        let now = now_unix_seconds()?;
        for (call_id, reasoning) in reasoning_map_from_messages(&state.canonical_messages) {
            conn.execute(
                "INSERT OR REPLACE INTO tool_reasoning_refs (conversation_id, tool_call_id, response_id, reasoning_content, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![&conv_id, call_id, &state.id, reasoning, now],
            )
            .with_context(|| format!("failed to write reasoning ref for response {}", state.id))?;
        }
        cleanup_unreferenced_blobs(conn)?;
        Ok(())
    }
}

#[cfg(feature = "sqlite")]
fn table_exists(conn: &rusqlite::Connection, table: &str) -> anyhow::Result<bool> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            rusqlite::params![table],
            |row| row.get(0),
        )
        .with_context(|| format!("failed to inspect SQLite table {table}"))?;
    Ok(count > 0)
}

#[cfg(feature = "sqlite")]
fn table_has_column(
    conn: &rusqlite::Connection,
    table: &str,
    column: &str,
) -> anyhow::Result<bool> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .with_context(|| format!("failed to inspect SQLite table {table}"))?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .with_context(|| format!("failed to read SQLite table info for {table}"))?;
    for row in rows {
        if row? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(feature = "sqlite")]
fn create_v2_schema(conn: &rusqlite::Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS responses (
            id TEXT PRIMARY KEY,
            conversation_id TEXT NOT NULL,
            model TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            status TEXT NOT NULL,
            response_json TEXT NOT NULL,
            previous_response_id TEXT,
            expires_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS canonical_message_blobs (
            hash TEXT PRIMARY KEY,
            message_json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS response_canonical_refs (
            response_id TEXT NOT NULL,
            ordinal INTEGER NOT NULL,
            message_hash TEXT NOT NULL,
            PRIMARY KEY (response_id, ordinal)
        );
        CREATE TABLE IF NOT EXISTS tool_reasoning_refs (
            conversation_id TEXT NOT NULL,
            tool_call_id TEXT NOT NULL,
            response_id TEXT NOT NULL,
            reasoning_content TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            PRIMARY KEY (conversation_id, tool_call_id)
        );
        CREATE TABLE IF NOT EXISTS debug_artifacts (
            id TEXT PRIMARY KEY,
            conversation_id TEXT NOT NULL,
            model TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            status TEXT NOT NULL,
            request_json TEXT NOT NULL,
            mapped_request_json TEXT NOT NULL,
            upstream_error_json TEXT,
            debug_annotations_json TEXT NOT NULL,
            upstream_sse_events_json TEXT NOT NULL,
            response_sse_events_json TEXT NOT NULL,
            expires_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_response_refs_response
            ON response_canonical_refs(response_id, ordinal);
        CREATE INDEX IF NOT EXISTS idx_response_refs_hash
            ON response_canonical_refs(message_hash);
        CREATE INDEX IF NOT EXISTS idx_reasoning_refs_call
            ON tool_reasoning_refs(conversation_id, tool_call_id);
        CREATE INDEX IF NOT EXISTS idx_debug_artifacts_expires
            ON debug_artifacts(expires_at);
        CREATE INDEX IF NOT EXISTS idx_responses_expires
            ON responses(expires_at);",
    )
    .context("failed to create SQLite state schema v2")
}

#[cfg(feature = "sqlite")]
fn migrate_legacy_schema(
    conn: &mut rusqlite::Connection,
    ttl_seconds: u64,
    debug_artifact_ttl_seconds: u64,
) -> anyhow::Result<()> {
    let required = [
        "id",
        "conversation_id",
        "model",
        "created_at",
        "status",
        "request_json",
        "mapped_request_json",
        "response_json",
        "upstream_error_json",
        "debug_annotations_json",
        "canonical_messages_json",
        "upstream_sse_events_json",
        "response_sse_events_json",
    ];
    for column in required {
        if !table_has_column(conn, "responses", column)? {
            return Err(anyhow!(
                "legacy SQLite responses table is missing required column {column}"
            ));
        }
    }

    let legacy_name = format!("responses_legacy_{}", now_unix_seconds()?);
    conn.execute(
        &format!("ALTER TABLE responses RENAME TO {legacy_name}"),
        [],
    )
    .context("failed to rename legacy responses table")?;
    create_v2_schema(conn)?;

    let tx = conn
        .transaction()
        .context("failed to begin SQLite migration")?;
    {
        let mut stmt = tx
            .prepare(&format!(
                "SELECT id, conversation_id, model, created_at, status, request_json, mapped_request_json, response_json, upstream_error_json, debug_annotations_json, canonical_messages_json, upstream_sse_events_json, response_sse_events_json FROM {legacy_name}"
            ))
            .context("failed to read legacy responses")?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, String>(11)?,
                    row.get::<_, String>(12)?,
                ))
            })
            .context("failed to iterate legacy responses")?;

        let debug_expires_at = now_unix_seconds()? + debug_artifact_ttl_seconds as i64;
        for row in rows {
            let (
                id,
                conversation_id,
                model,
                created_at,
                status,
                request_json,
                mapped_request_json,
                response_json,
                upstream_error_json,
                debug_annotations_json,
                canonical_messages_json,
                upstream_sse_events_json,
                response_sse_events_json,
            ) = row.context("failed to read legacy response row")?;
            let response_value: Value = serde_json::from_str(&response_json)
                .with_context(|| format!("failed to parse response_json for {id}"))?;
            let canonical_messages: Vec<ChatMessage> =
                serde_json::from_str(&canonical_messages_json)
                    .with_context(|| format!("failed to parse canonical_messages_json for {id}"))?;
            SqliteStore::insert_response_with_conn(
                &tx,
                ResponseState {
                    id: id.clone(),
                    conversation_id: conversation_id.clone(),
                    model: model.clone(),
                    created_at,
                    status: status.clone(),
                    response_json: response_value,
                    previous_response_id: None,
                    canonical_messages,
                },
                Duration::from_secs(ttl_seconds),
            )?;

            tx.execute(
                "INSERT OR REPLACE INTO debug_artifacts (id, conversation_id, model, created_at, status, request_json, mapped_request_json, upstream_error_json, debug_annotations_json, upstream_sse_events_json, response_sse_events_json, expires_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                rusqlite::params![
                    &id,
                    default_conversation_id(conversation_id.as_deref()),
                    &model,
                    created_at,
                    &status,
                    request_json,
                    mapped_request_json,
                    upstream_error_json,
                    debug_annotations_json,
                    upstream_sse_events_json,
                    response_sse_events_json,
                    debug_expires_at,
                ],
            )
            .with_context(|| format!("failed to migrate debug artifact for {id}"))?;
        }
    }
    tx.commit().context("failed to commit SQLite migration")?;
    conn.execute(&format!("DROP TABLE {legacy_name}"), [])
        .context("failed to drop legacy responses table")?;
    conn.execute("DROP TABLE IF EXISTS tool_reasoning", [])
        .context("failed to drop legacy tool_reasoning table")?;
    conn.execute_batch("VACUUM;")
        .context("failed to vacuum SQLite database after migration")?;
    Ok(())
}

#[cfg(feature = "sqlite")]
fn cleanup_unreferenced_blobs(conn: &rusqlite::Connection) -> anyhow::Result<()> {
    conn.execute(
        "DELETE FROM canonical_message_blobs
         WHERE hash NOT IN (SELECT DISTINCT message_hash FROM response_canonical_refs)",
        [],
    )
    .context("failed to clean up unreferenced canonical message blobs")?;
    Ok(())
}

#[cfg(feature = "sqlite")]
fn delete_response_with_conn(conn: &rusqlite::Connection, id: &str) -> anyhow::Result<bool> {
    conn.execute(
        "DELETE FROM response_canonical_refs WHERE response_id = ?1",
        rusqlite::params![id],
    )
    .with_context(|| format!("failed to delete canonical refs for {id}"))?;
    conn.execute(
        "DELETE FROM tool_reasoning_refs WHERE response_id = ?1",
        rusqlite::params![id],
    )
    .with_context(|| format!("failed to delete reasoning refs for {id}"))?;
    conn.execute(
        "DELETE FROM debug_artifacts WHERE id = ?1",
        rusqlite::params![id],
    )
    .with_context(|| format!("failed to delete debug artifact for {id}"))?;
    let deleted = conn
        .execute("DELETE FROM responses WHERE id = ?1", rusqlite::params![id])
        .with_context(|| format!("failed to delete response {id}"))?;
    cleanup_unreferenced_blobs(conn)?;
    Ok(deleted > 0)
}

#[cfg(feature = "sqlite")]
impl ResponseStoreBackend for SqliteStore {
    fn put_response_state(&self, state: ResponseState) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        Self::insert_response_with_conn(&conn, state, self.ttl)
    }

    fn get_response_json(&self, id: &str) -> anyhow::Result<Option<Value>> {
        let conn = self.conn.lock().unwrap();
        let result = conn.query_row(
            "SELECT response_json FROM responses WHERE id = ?1",
            rusqlite::params![id],
            |row| row.get::<_, String>(0),
        );
        match result {
            Ok(json) => {
                Ok(Some(serde_json::from_str(&json).with_context(|| {
                    format!("failed to parse response_json for {id}")
                })?))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e).with_context(|| format!("failed to read response_json for {id}")),
        }
    }

    fn get_canonical_messages(&self, id: &str) -> anyhow::Result<Option<Vec<ChatMessage>>> {
        let conn = self.conn.lock().unwrap();
        let exists = conn
            .query_row(
                "SELECT 1 FROM responses WHERE id = ?1",
                rusqlite::params![id],
                |_| Ok(()),
            )
            .optional()
            .with_context(|| format!("failed to check response existence for {id}"))?
            .is_some();
        if !exists {
            return Ok(None);
        }

        let mut stmt = conn
            .prepare(
                "SELECT b.message_json
                 FROM response_canonical_refs r
                 JOIN canonical_message_blobs b ON b.hash = r.message_hash
                 WHERE r.response_id = ?1
                 ORDER BY r.ordinal ASC",
            )
            .with_context(|| format!("failed to prepare canonical refs read for {id}"))?;
        let rows = stmt
            .query_map(rusqlite::params![id], |row| row.get::<_, String>(0))
            .with_context(|| format!("failed to read canonical refs for {id}"))?;
        let mut messages = Vec::new();
        for row in rows {
            let json = row.with_context(|| format!("failed to read canonical blob for {id}"))?;
            messages.push(
                serde_json::from_str::<ChatMessage>(&json)
                    .with_context(|| format!("failed to parse canonical message for {id}"))?,
            );
        }
        Ok(Some(messages))
    }

    fn put_debug_artifact(&self, artifact: DebugArtifact) -> anyhow::Result<()> {
        let conv_id = default_conversation_id(artifact.conversation_id.as_deref());
        let expires_at = now_unix_seconds()? + self.debug_ttl.as_secs() as i64;
        let request_json = serde_json::to_string(&artifact.request_json)
            .context("failed to serialize request_json")?;
        let mapped_request_json = serde_json::to_string(&artifact.mapped_request_json)
            .context("failed to serialize mapped_request_json")?;
        let upstream_error_json = artifact
            .upstream_error
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .context("failed to serialize upstream_error")?;
        let debug_annotations_json = serde_json::to_string(&artifact.debug_annotations)
            .context("failed to serialize debug_annotations")?;
        let upstream_sse_events_json = serde_json::to_string(&artifact.upstream_sse_events)
            .context("failed to serialize upstream_sse_events")?;
        let response_sse_events_json = serde_json::to_string(&artifact.response_sse_events)
            .context("failed to serialize response_sse_events")?;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO debug_artifacts (id, conversation_id, model, created_at, status, request_json, mapped_request_json, upstream_error_json, debug_annotations_json, upstream_sse_events_json, response_sse_events_json, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                &artifact.id,
                &conv_id,
                &artifact.model,
                artifact.created_at,
                &artifact.status,
                request_json,
                mapped_request_json,
                upstream_error_json,
                debug_annotations_json,
                upstream_sse_events_json,
                response_sse_events_json,
                expires_at,
            ],
        )
        .with_context(|| format!("failed to write debug artifact {}", artifact.id))?;
        Ok(())
    }

    fn get_debug_artifact(&self, id: &str) -> anyhow::Result<Option<DebugArtifactView>> {
        let conn = self.conn.lock().unwrap();
        let now = now_unix_seconds()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, conversation_id, model, created_at, status, request_json, mapped_request_json, upstream_error_json, debug_annotations_json, upstream_sse_events_json, response_sse_events_json, expires_at
                 FROM debug_artifacts
                 WHERE id = ?1 AND expires_at > ?2",
            )
            .with_context(|| format!("failed to prepare debug artifact read for {id}"))?;
        let result = stmt.query_row(rusqlite::params![id, now], sqlite_debug_artifact_view);
        match result {
            Ok(view) => Ok(Some(view)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e).with_context(|| format!("failed to read debug artifact {id}")),
        }
    }

    fn list_debug_artifacts(&self, limit: usize) -> anyhow::Result<Vec<DebugArtifactView>> {
        let conn = self.conn.lock().unwrap();
        let now = now_unix_seconds()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, conversation_id, model, created_at, status, request_json, mapped_request_json, upstream_error_json, debug_annotations_json, upstream_sse_events_json, response_sse_events_json, expires_at
                 FROM debug_artifacts
                 WHERE expires_at > ?1
                 ORDER BY created_at DESC, id DESC
                 LIMIT ?2",
            )
            .context("failed to prepare debug artifact list")?;
        let rows = stmt
            .query_map(
                rusqlite::params![now, limit as i64],
                sqlite_debug_artifact_view,
            )
            .context("failed to list debug artifacts")?;
        let mut artifacts = Vec::new();
        for row in rows {
            artifacts.push(row.context("failed to read debug artifact row")?);
        }
        Ok(artifacts)
    }

    fn delete(&self, id: &str) -> anyhow::Result<bool> {
        let conn = self.conn.lock().unwrap();
        delete_response_with_conn(&conn, id)
    }

    fn clear(&self) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM response_canonical_refs", [])
            .context("failed to clear canonical refs")?;
        conn.execute("DELETE FROM canonical_message_blobs", [])
            .context("failed to clear canonical blobs")?;
        conn.execute("DELETE FROM tool_reasoning_refs", [])
            .context("failed to clear reasoning refs")?;
        conn.execute("DELETE FROM debug_artifacts", [])
            .context("failed to clear debug artifacts")?;
        conn.execute("DELETE FROM responses", [])
            .context("failed to clear responses")?;
        Ok(())
    }

    fn cleanup_expired(&self) -> anyhow::Result<usize> {
        let conn = self.conn.lock().unwrap();
        let now = now_unix_seconds()?;
        let mut stmt = conn
            .prepare("SELECT id FROM responses WHERE expires_at <= ?1")
            .context("failed to prepare expired response lookup")?;
        let expired_ids = stmt
            .query_map(rusqlite::params![now], |row| row.get::<_, String>(0))
            .context("failed to read expired responses")?
            .collect::<Result<Vec<_>, _>>()
            .context("failed to collect expired responses")?;
        drop(stmt);
        for id in &expired_ids {
            delete_response_with_conn(&conn, id)?;
        }
        let debug_removed = conn
            .execute(
                "DELETE FROM debug_artifacts WHERE expires_at <= ?1",
                rusqlite::params![now],
            )
            .context("failed to delete expired debug artifacts")?;
        cleanup_unreferenced_blobs(&conn)?;
        conn.execute("PRAGMA incremental_vacuum", [])
            .context("failed to run SQLite incremental vacuum")?;
        Ok(expired_ids.len() + debug_removed)
    }

    fn len(&self) -> anyhow::Result<usize> {
        let conn = self.conn.lock().unwrap();
        conn.query_row("SELECT COUNT(*) FROM responses", [], |r| {
            r.get::<_, usize>(0)
        })
        .context("failed to count responses")
    }

    fn find_reasoning_for_call_ids(
        &self,
        conv_id: &str,
        call_ids: &[String],
    ) -> anyhow::Result<HashMap<String, String>> {
        let conn = self.conn.lock().unwrap();
        let mut map = HashMap::new();
        let mut stmt = conn
            .prepare(
                "SELECT reasoning_content FROM tool_reasoning_refs
                 WHERE conversation_id = ?1 AND tool_call_id = ?2",
            )
            .context("failed to prepare reasoning lookup")?;
        for call_id in call_ids {
            let result = stmt.query_row(rusqlite::params![conv_id, call_id], |row| {
                row.get::<_, String>(0)
            });
            match result {
                Ok(reasoning) => {
                    map.insert(call_id.clone(), reasoning);
                }
                Err(rusqlite::Error::QueryReturnedNoRows) => {}
                Err(e) => {
                    return Err(e)
                        .with_context(|| format!("failed to read reasoning for call {call_id}"));
                }
            }
        }
        Ok(map)
    }
}

#[cfg(feature = "sqlite")]
fn sqlite_debug_artifact_view(row: &rusqlite::Row<'_>) -> rusqlite::Result<DebugArtifactView> {
    let request_json: String = row.get(5)?;
    let mapped_request_json: String = row.get(6)?;
    let upstream_error_json: Option<String> = row.get(7)?;
    let debug_annotations_json: String = row.get(8)?;
    let upstream_sse_events_json: String = row.get(9)?;
    let response_sse_events_json: String = row.get(10)?;
    Ok(DebugArtifactView {
        id: row.get(0)?,
        conversation_id: Some(row.get::<_, String>(1)?),
        model: row.get(2)?,
        created_at: row.get(3)?,
        status: row.get(4)?,
        request_json: serde_json::from_str(&request_json).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(e))
        })?,
        mapped_request_json: serde_json::from_str(&mapped_request_json).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(6, rusqlite::types::Type::Text, Box::new(e))
        })?,
        upstream_error: upstream_error_json
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    7,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?,
        debug_annotations: serde_json::from_str(&debug_annotations_json).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(8, rusqlite::types::Type::Text, Box::new(e))
        })?,
        upstream_sse_events: serde_json::from_str(&upstream_sse_events_json).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(9, rusqlite::types::Type::Text, Box::new(e))
        })?,
        response_sse_events: serde_json::from_str(&response_sse_events_json).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(10, rusqlite::types::Type::Text, Box::new(e))
        })?,
        artifact_expires_at: row.get(11)?,
    })
}

#[cfg(feature = "sqlite")]
use rusqlite::OptionalExtension;

// ── Response store (delegates to backend) ─────────────────────────

pub struct ResponseStore {
    backend: Box<dyn ResponseStoreBackend>,
}

impl ResponseStore {
    pub fn new(backend: Box<dyn ResponseStoreBackend>) -> Self {
        Self { backend }
    }

    pub fn put_response_state(&self, state: ResponseState) -> anyhow::Result<()> {
        self.backend.put_response_state(state)
    }

    pub fn get_response_json(&self, id: &str) -> anyhow::Result<Option<Value>> {
        self.backend.get_response_json(id)
    }

    pub fn get_canonical_messages(&self, id: &str) -> anyhow::Result<Option<Vec<ChatMessage>>> {
        self.backend.get_canonical_messages(id)
    }

    pub fn put_debug_artifact(&self, artifact: DebugArtifact) -> anyhow::Result<()> {
        self.backend.put_debug_artifact(artifact)
    }

    pub fn get_debug_artifact(&self, id: &str) -> anyhow::Result<Option<DebugArtifactView>> {
        self.backend.get_debug_artifact(id)
    }

    pub fn list_debug_artifacts(&self, limit: usize) -> anyhow::Result<Vec<DebugArtifactView>> {
        self.backend.list_debug_artifacts(limit)
    }

    pub fn delete(&self, id: &str) -> anyhow::Result<bool> {
        self.backend.delete(id)
    }

    pub fn clear(&self) -> anyhow::Result<()> {
        self.backend.clear()
    }

    pub fn cleanup_expired(&self) -> anyhow::Result<usize> {
        self.backend.cleanup_expired()
    }

    pub fn len(&self) -> anyhow::Result<usize> {
        self.backend.len()
    }

    pub fn find_reasoning_for_call_ids(
        &self,
        conv_id: &str,
        call_ids: &[String],
    ) -> anyhow::Result<HashMap<String, String>> {
        self.backend.find_reasoning_for_call_ids(conv_id, call_ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assistant_with_reasoning() -> ChatMessage {
        ChatMessage::Assistant {
            content: None,
            name: None,
            tool_calls: Some(vec![
                protocol::chat::ChatToolCall {
                    id: "call-a".into(),
                    call_type: "function".into(),
                    function: protocol::chat::ChatFunctionCall {
                        name: Some("exec_command".into()),
                        arguments: "{}".into(),
                    },
                },
                protocol::chat::ChatToolCall {
                    id: "call-b".into(),
                    call_type: "function".into(),
                    function: protocol::chat::ChatFunctionCall {
                        name: Some("exec_command".into()),
                        arguments: "{\"cmd\":\"pwd\"}".into(),
                    },
                },
            ]),
            reasoning_content: Some("real thinking".into()),
        }
    }

    fn response_state(id: &str, created_at: i64, messages: Vec<ChatMessage>) -> ResponseState {
        ResponseState {
            id: id.into(),
            model: "m".into(),
            created_at,
            status: "completed".into(),
            response_json: serde_json::json!({"id": id, "status": "completed"}),
            previous_response_id: None,
            canonical_messages: messages,
            conversation_id: None,
        }
    }

    fn debug_artifact(id: &str, created_at: i64) -> DebugArtifact {
        DebugArtifact {
            id: id.into(),
            model: "m".into(),
            created_at,
            status: "completed".into(),
            request_json: serde_json::json!({"input": "hello"}),
            mapped_request_json: serde_json::json!({"messages": []}),
            upstream_error: None,
            debug_annotations: vec!["note".into()],
            upstream_sse_events: vec![serde_json::json!({"id": "chunk-1"})],
            response_sse_events: vec![serde_json::json!({"type": "response.completed"})],
            conversation_id: None,
        }
    }

    #[test]
    fn memory_store_compact_state_round_trips_canonical_messages() {
        let store = MemoryStore::new(3600, 600);
        let messages = vec![
            ChatMessage::System {
                content: protocol::chat::ChatContent::Text("instructions".into()),
                name: None,
            },
            ChatMessage::User {
                content: protocol::chat::ChatContent::Text("hi".into()),
                name: None,
            },
            assistant_with_reasoning(),
            ChatMessage::Tool {
                content: protocol::chat::ChatContent::Text("tool output".into()),
                tool_call_id: "call-a".into(),
            },
        ];

        store
            .put_response_state(response_state("resp-1", 1000, messages.clone()))
            .unwrap();

        assert_eq!(
            store.get_response_json("resp-1").unwrap(),
            Some(serde_json::json!({"id": "resp-1", "status": "completed"}))
        );
        assert_eq!(
            store.get_canonical_messages("resp-1").unwrap(),
            Some(messages)
        );
        assert_eq!(store.len().unwrap(), 1);
    }

    #[test]
    fn memory_store_debug_artifact_expires_without_deleting_state() {
        let store = MemoryStore::new(3600, 1);
        store
            .put_response_state(response_state(
                "resp-1",
                now_unix_seconds().unwrap(),
                vec![],
            ))
            .unwrap();
        store
            .put_debug_artifact(debug_artifact("resp-1", now_unix_seconds().unwrap()))
            .unwrap();
        assert!(store.get_debug_artifact("resp-1").unwrap().is_some());

        std::thread::sleep(Duration::from_secs(2));
        let removed = store.cleanup_expired().unwrap();
        assert_eq!(removed, 1);
        assert!(store.get_debug_artifact("resp-1").unwrap().is_none());
        assert!(store.get_response_json("resp-1").unwrap().is_some());
        assert_eq!(store.len().unwrap(), 1);
    }

    #[test]
    fn memory_store_reasoning_lookup_is_targeted() {
        let store = MemoryStore::new(3600, 600);
        store
            .put_response_state(response_state(
                "resp-1",
                1000,
                vec![assistant_with_reasoning()],
            ))
            .unwrap();

        let map = store
            .find_reasoning_for_call_ids("default", &["call-a".into(), "missing".into()])
            .unwrap();
        assert_eq!(map.get("call-a"), Some(&"real thinking".to_string()));
        assert!(!map.contains_key("missing"));
    }

    #[test]
    fn memory_store_lists_recent_debug_artifacts_only() {
        let store = MemoryStore::new(3600, 600);
        for (id, created_at) in [("old", 1), ("new", 3), ("mid", 2)] {
            store
                .put_debug_artifact(debug_artifact(id, created_at))
                .unwrap();
        }
        let ids = store
            .list_debug_artifacts(2)
            .unwrap()
            .into_iter()
            .map(|artifact| artifact.id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["new", "mid"]);
    }

    #[test]
    fn memory_store_delete_removes_state_debug_and_reasoning() {
        let store = MemoryStore::new(3600, 600);
        store
            .put_response_state(response_state(
                "resp-1",
                1000,
                vec![assistant_with_reasoning()],
            ))
            .unwrap();
        store
            .put_debug_artifact(debug_artifact("resp-1", 1000))
            .unwrap();

        assert!(store.delete("resp-1").unwrap());
        assert!(store.get_response_json("resp-1").unwrap().is_none());
        assert!(store.get_canonical_messages("resp-1").unwrap().is_none());
        assert!(store.get_debug_artifact("resp-1").unwrap().is_none());
        assert!(
            store
                .find_reasoning_for_call_ids("default", &["call-a".into()])
                .unwrap()
                .is_empty()
        );
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn sqlite_store_compact_state_round_trips_canonical_messages() {
        let path =
            std::env::temp_dir().join(format!("codex-shim-store-{}.db", uuid::Uuid::new_v4()));
        let store = SqliteStore::new(&path, 3600, 600).expect("sqlite store");
        let messages = vec![
            ChatMessage::User {
                content: protocol::chat::ChatContent::Text("hi".into()),
                name: None,
            },
            assistant_with_reasoning(),
        ];

        store
            .put_response_state(response_state("sqlite-1", 1000, messages.clone()))
            .unwrap();
        assert_eq!(
            store.get_canonical_messages("sqlite-1").unwrap(),
            Some(messages)
        );
        assert_eq!(
            store
                .find_reasoning_for_call_ids("default", &["call-b".into()])
                .unwrap()
                .get("call-b"),
            Some(&"real thinking".to_string())
        );

        drop(store);
        let _ = std::fs::remove_file(path);
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn sqlite_store_round_trips_debug_artifacts_separately() {
        let path =
            std::env::temp_dir().join(format!("codex-shim-debug-{}.db", uuid::Uuid::new_v4()));
        let store = SqliteStore::new(&path, 3600, 600).expect("sqlite store");
        store
            .put_response_state(response_state("sqlite-debug-1", 1000, vec![]))
            .unwrap();
        store
            .put_debug_artifact(debug_artifact("sqlite-debug-1", 1000))
            .unwrap();

        let artifact = store
            .get_debug_artifact("sqlite-debug-1")
            .unwrap()
            .expect("debug artifact");
        assert_eq!(
            artifact.upstream_sse_events,
            vec![serde_json::json!({"id": "chunk-1"})]
        );
        assert_eq!(
            artifact.response_sse_events,
            vec![serde_json::json!({"type": "response.completed"})]
        );
        assert_eq!(store.len().unwrap(), 1);

        drop(store);
        let _ = std::fs::remove_file(path);
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn sqlite_store_migrates_legacy_full_snapshot_schema() {
        let path =
            std::env::temp_dir().join(format!("codex-shim-legacy-{}.db", uuid::Uuid::new_v4()));
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE responses (
                    id TEXT PRIMARY KEY,
                    conversation_id TEXT,
                    model TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    status TEXT NOT NULL,
                    request_json TEXT NOT NULL,
                    mapped_request_json TEXT NOT NULL DEFAULT 'null',
                    response_json TEXT NOT NULL,
                    upstream_error_json TEXT,
                    debug_annotations_json TEXT NOT NULL DEFAULT '[]',
                    canonical_messages_json TEXT NOT NULL,
                    upstream_sse_events_json TEXT NOT NULL DEFAULT '[]',
                    response_sse_events_json TEXT NOT NULL DEFAULT '[]',
                    expires_at INTEGER
                );
                CREATE TABLE tool_reasoning (
                    conversation_id TEXT NOT NULL,
                    response_id TEXT NOT NULL,
                    tool_call_id TEXT NOT NULL,
                    reasoning_content TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    PRIMARY KEY (conversation_id, tool_call_id)
                );",
            )
            .unwrap();
            let messages = serde_json::to_string(&vec![assistant_with_reasoning()]).unwrap();
            conn.execute(
                "INSERT INTO responses (id, conversation_id, model, created_at, status, request_json, mapped_request_json, response_json, canonical_messages_json, upstream_sse_events_json, response_sse_events_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                rusqlite::params![
                    "legacy-1",
                    "default",
                    "m",
                    1000,
                    "completed",
                    "{\"input\":\"hello\"}",
                    "{\"messages\":[]}",
                    "{\"id\":\"legacy-1\"}",
                    messages,
                    "[{\"id\":\"chunk-1\"}]",
                    "[{\"type\":\"response.completed\"}]",
                ],
            )
            .unwrap();
        }

        let store = SqliteStore::new(&path, 3600, 600).expect("migrated sqlite store");
        assert!(store.get_response_json("legacy-1").unwrap().is_some());
        assert!(store.get_canonical_messages("legacy-1").unwrap().is_some());
        assert!(store.get_debug_artifact("legacy-1").unwrap().is_some());
        assert_eq!(
            store
                .find_reasoning_for_call_ids("default", &["call-a".into()])
                .unwrap()
                .get("call-a"),
            Some(&"real thinking".to_string())
        );

        drop(store);
        let _ = std::fs::remove_file(path);
    }
}
