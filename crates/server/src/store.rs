use std::collections::HashMap;
#[cfg(feature = "sqlite")]
use std::path::Path;
use std::sync::RwLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use protocol::chat::ChatMessage;
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── Data types ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredResponse {
    pub id: String,
    pub model: String,
    pub created_at: i64,
    pub status: String,
    pub request_json: Value,
    pub response_json: Value,
    pub canonical_messages: Vec<ChatMessage>,
    /// Raw upstream SSE event payloads captured for streamed responses.
    #[serde(default)]
    pub upstream_sse_events: Vec<Value>,
    /// Responses SSE event payloads emitted by the shim.
    #[serde(default)]
    pub response_sse_events: Vec<Value>,
    /// Optional conversation grouping (populated by store v2 backends).
    #[serde(default)]
    pub conversation_id: Option<String>,
}

// ── Backend trait ────────────────────────────────────────────────

pub trait ResponseStoreBackend: Send + Sync {
    fn put(&self, record: StoredResponse);
    fn get(&self, id: &str) -> Option<StoredResponse>;
    fn delete(&self, id: &str) -> bool;
    fn clear(&self);
    fn cleanup_expired(&self) -> usize;
    fn len(&self) -> usize;

    /// Reasoning recovery helpers.
    fn save_reasoning(&self, conv_id: &str, response_id: &str, call_id: &str, reasoning: &str);
    fn find_reasoning_by_call_id(&self, conv_id: &str, call_id: &str) -> Option<String>;
    fn build_reasoning_map(&self, conv_id: &str) -> HashMap<String, String>;
}

// ── Memory store ─────────────────────────────────────────────────

pub struct MemoryStore {
    inner: RwLock<HashMap<String, StoredResponse>>,
    ttl: Duration,
}

impl MemoryStore {
    pub fn new(ttl_seconds: u64) -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
            ttl: Duration::from_secs(ttl_seconds),
        }
    }
}

impl ResponseStoreBackend for MemoryStore {
    fn put(&self, record: StoredResponse) {
        let mut inner = self.inner.write().unwrap();
        inner.insert(record.id.clone(), record);
    }

    fn get(&self, id: &str) -> Option<StoredResponse> {
        self.inner.read().unwrap().get(id).cloned()
    }

    fn delete(&self, id: &str) -> bool {
        self.inner.write().unwrap().remove(id).is_some()
    }

    fn clear(&self) {
        self.inner.write().unwrap().clear();
    }

    fn cleanup_expired(&self) -> usize {
        let mut inner = self.inner.write().unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let cutoff = now - self.ttl.as_secs() as i64;
        let before = inner.len();
        inner.retain(|_, v| v.created_at > cutoff);
        before - inner.len()
    }

    fn len(&self) -> usize {
        self.inner.read().unwrap().len()
    }

    fn save_reasoning(&self, _conv_id: &str, _response_id: &str, _call_id: &str, _reasoning: &str) {
        // Memory store: reasoning is already embedded in canonical_messages.
        // No separate storage needed — build_reasoning_map scans canonical_messages.
    }

    fn find_reasoning_by_call_id(&self, conv_id: &str, call_id: &str) -> Option<String> {
        self.build_reasoning_map(conv_id).get(call_id).cloned()
    }

    fn build_reasoning_map(&self, _conv_id: &str) -> HashMap<String, String> {
        let inner = self.inner.read().unwrap();
        let mut map = HashMap::new();
        for (_id, resp) in inner.iter() {
            for msg in &resp.canonical_messages {
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
        }
        map
    }
}

// ── SQLite store ─────────────────────────────────────────────────

#[cfg(feature = "sqlite")]
pub struct SqliteStore {
    conn: std::sync::Mutex<rusqlite::Connection>,
    ttl: Duration,
}

#[cfg(feature = "sqlite")]
impl SqliteStore {
    pub fn new(path: impl AsRef<Path>, ttl_seconds: u64) -> anyhow::Result<Self> {
        let conn = rusqlite::Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS responses (
                id TEXT PRIMARY KEY,
                conversation_id TEXT,
                model TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                status TEXT NOT NULL,
                request_json TEXT NOT NULL,
                response_json TEXT NOT NULL,
                canonical_messages_json TEXT NOT NULL,
                upstream_sse_events_json TEXT NOT NULL DEFAULT '[]',
                response_sse_events_json TEXT NOT NULL DEFAULT '[]',
                expires_at INTEGER
            );
            CREATE TABLE IF NOT EXISTS tool_reasoning (
                conversation_id TEXT NOT NULL,
                response_id TEXT NOT NULL,
                tool_call_id TEXT NOT NULL,
                reasoning_content TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                PRIMARY KEY (conversation_id, tool_call_id)
            );
            CREATE INDEX IF NOT EXISTS idx_reasoning_call_id
                ON tool_reasoning(conversation_id, tool_call_id);
            CREATE INDEX IF NOT EXISTS idx_responses_conv
                ON responses(conversation_id);
            ",
        )?;
        let _ = conn.execute(
            "ALTER TABLE responses ADD COLUMN upstream_sse_events_json TEXT NOT NULL DEFAULT '[]'",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE responses ADD COLUMN response_sse_events_json TEXT NOT NULL DEFAULT '[]'",
            [],
        );
        Ok(Self {
            conn: std::sync::Mutex::new(conn),
            ttl: Duration::from_secs(ttl_seconds),
        })
    }
}

#[cfg(feature = "sqlite")]
impl ResponseStoreBackend for SqliteStore {
    fn put(&self, record: StoredResponse) {
        let conv_id = record
            .conversation_id
            .clone()
            .unwrap_or_else(|| "default".into());
        let msgs_json = serde_json::to_string(&record.canonical_messages).unwrap_or_default();
        let req_json = serde_json::to_string(&record.request_json).unwrap_or_default();
        let resp_json = serde_json::to_string(&record.response_json).unwrap_or_default();
        let upstream_sse_json =
            serde_json::to_string(&record.upstream_sse_events).unwrap_or_default();
        let response_sse_json =
            serde_json::to_string(&record.response_sse_events).unwrap_or_default();
        let expires_at = record.created_at + self.ttl.as_secs() as i64;

        let conn = self.conn.lock().unwrap();
        let _ = conn.execute(
            "INSERT OR REPLACE INTO responses (id, conversation_id, model, created_at, status, request_json, response_json, canonical_messages_json, upstream_sse_events_json, response_sse_events_json, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![
                record.id, conv_id, record.model, record.created_at,
                record.status, req_json, resp_json, msgs_json, upstream_sse_json,
                response_sse_json, expires_at
            ],
        );
    }

    fn get(&self, id: &str) -> Option<StoredResponse> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT id, conversation_id, model, created_at, status, request_json, response_json, canonical_messages_json, upstream_sse_events_json, response_sse_events_json FROM responses WHERE id = ?1")
            .ok()?;
        let row = stmt
            .query_row(rusqlite::params![id], |row| {
                let msgs_json: String = row.get(7)?;
                let msgs: Vec<ChatMessage> = serde_json::from_str(&msgs_json).unwrap_or_default();
                let upstream_sse_json: String = row.get(8)?;
                let response_sse_json: String = row.get(9)?;
                Ok(StoredResponse {
                    id: row.get(0)?,
                    conversation_id: row.get(1)?,
                    model: row.get(2)?,
                    created_at: row.get(3)?,
                    status: row.get(4)?,
                    request_json: serde_json::from_str(&row.get::<_, String>(5)?)
                        .unwrap_or_default(),
                    response_json: serde_json::from_str(&row.get::<_, String>(6)?)
                        .unwrap_or_default(),
                    canonical_messages: msgs,
                    upstream_sse_events: serde_json::from_str(&upstream_sse_json)
                        .unwrap_or_default(),
                    response_sse_events: serde_json::from_str(&response_sse_json)
                        .unwrap_or_default(),
                })
            })
            .ok()?;
        Some(row)
    }

    fn delete(&self, id: &str) -> bool {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM responses WHERE id = ?1", rusqlite::params![id])
            .map(|n| n > 0)
            .unwrap_or(false)
    }

    fn clear(&self) {
        let conn = self.conn.lock().unwrap();
        let _ = conn.execute("DELETE FROM responses", []);
        let _ = conn.execute("DELETE FROM tool_reasoning", []);
    }

    fn cleanup_expired(&self) -> usize {
        let conn = self.conn.lock().unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let count = conn
            .execute(
                "DELETE FROM responses WHERE expires_at IS NOT NULL AND expires_at < ?1",
                rusqlite::params![now],
            )
            .unwrap_or(0);
        let _ = conn.execute(
            "DELETE FROM tool_reasoning WHERE conversation_id NOT IN (SELECT DISTINCT conversation_id FROM responses)",
            [],
        );
        count
    }

    fn len(&self) -> usize {
        let conn = self.conn.lock().unwrap();
        conn.query_row("SELECT COUNT(*) FROM responses", [], |r| {
            r.get::<_, usize>(0)
        })
        .unwrap_or(0)
    }

    fn save_reasoning(&self, conv_id: &str, response_id: &str, call_id: &str, reasoning: &str) {
        let conn = self.conn.lock().unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let _ = conn.execute(
            "INSERT OR REPLACE INTO tool_reasoning (conversation_id, response_id, tool_call_id, reasoning_content, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![conv_id, response_id, call_id, reasoning, now],
        );
    }

    fn find_reasoning_by_call_id(&self, conv_id: &str, call_id: &str) -> Option<String> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT reasoning_content FROM tool_reasoning WHERE conversation_id = ?1 AND tool_call_id = ?2",
            rusqlite::params![conv_id, call_id],
            |row| row.get(0),
        )
        .ok()
    }

    fn build_reasoning_map(&self, conv_id: &str) -> HashMap<String, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT tool_call_id, reasoning_content FROM tool_reasoning WHERE conversation_id = ?1")
            .ok();
        let mut map = HashMap::new();
        if let Some(ref mut stmt) = stmt {
            if let Ok(rows) = stmt.query_map(rusqlite::params![conv_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            }) {
                for row in rows.flatten() {
                    map.insert(row.0, row.1);
                }
            }
        }
        map
    }
}

// ── Response store (delegates to backend) ─────────────────────────

pub struct ResponseStore {
    backend: Box<dyn ResponseStoreBackend>,
    _ttl: Duration,
}

impl ResponseStore {
    pub fn new(backend: Box<dyn ResponseStoreBackend>, ttl_seconds: u64) -> Self {
        Self {
            backend,
            _ttl: Duration::from_secs(ttl_seconds),
        }
    }

    pub fn put(&self, record: StoredResponse) {
        self.backend.put(record)
    }

    pub fn get(&self, id: &str) -> Option<StoredResponse> {
        self.backend.get(id)
    }

    pub fn delete(&self, id: &str) -> bool {
        self.backend.delete(id)
    }

    pub fn clear(&self) {
        self.backend.clear()
    }

    pub fn cleanup_expired(&self) -> usize {
        self.backend.cleanup_expired()
    }

    pub fn len(&self) -> usize {
        self.backend.len()
    }

    pub fn save_reasoning(&self, conv_id: &str, response_id: &str, call_id: &str, reasoning: &str) {
        self.backend
            .save_reasoning(conv_id, response_id, call_id, reasoning)
    }

    pub fn find_reasoning_by_call_id(&self, conv_id: &str, call_id: &str) -> Option<String> {
        self.backend.find_reasoning_by_call_id(conv_id, call_id)
    }

    pub fn build_reasoning_map(&self, conv_id: &str) -> HashMap<String, String> {
        self.backend.build_reasoning_map(conv_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_put_get_delete() {
        let store = MemoryStore::new(3600);
        let record = StoredResponse {
            id: "test-1".into(),
            model: "test".into(),
            created_at: 1000,
            status: "completed".into(),
            request_json: serde_json::json!({"input": "hello"}),
            response_json: serde_json::json!({"output": "world"}),
            canonical_messages: vec![],
            upstream_sse_events: vec![],
            response_sse_events: vec![],
            conversation_id: None,
        };

        store.put(record.clone());
        assert_eq!(store.len(), 1);

        let got = store.get("test-1").unwrap();
        assert_eq!(got.id, "test-1");
        assert_eq!(got.status, "completed");

        assert!(store.delete("test-1"));
        assert_eq!(store.len(), 0);
        assert!(store.get("test-1").is_none());
    }

    #[test]
    fn memory_store_ttl_expiry() {
        let store = MemoryStore::new(1); // 1 second TTL
        let record = StoredResponse {
            id: "expire-1".into(),
            model: "test".into(),
            created_at: 1, // Unix epoch + 1 second
            status: "completed".into(),
            request_json: serde_json::json!({}),
            response_json: serde_json::json!({}),
            canonical_messages: vec![],
            upstream_sse_events: vec![],
            response_sse_events: vec![],
            conversation_id: None,
        };
        store.put(record);
        assert_eq!(store.len(), 1);

        let removed = store.cleanup_expired();
        assert_eq!(removed, 1);
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn reasoning_map_from_canonical_messages() {
        let store = MemoryStore::new(3600);
        let msg = protocol::chat::ChatMessage::Assistant {
            content: None,
            name: None,
            tool_calls: Some(vec![protocol::chat::ChatToolCall {
                id: "call-1".into(),
                call_type: "function".into(),
                function: protocol::chat::ChatFunctionCall {
                    name: Some("test_fn".into()),
                    arguments: "{}".into(),
                },
            }]),
            reasoning_content: Some("let me think...".into()),
        };

        let record = StoredResponse {
            id: "reasoning-1".into(),
            model: "test".into(),
            created_at: 1000,
            status: "completed".into(),
            request_json: serde_json::json!({}),
            response_json: serde_json::json!({}),
            canonical_messages: vec![msg],
            upstream_sse_events: vec![],
            response_sse_events: vec![],
            conversation_id: Some("conv-1".into()),
        };
        store.put(record);

        let map = store.find_reasoning_by_call_id("conv-1", "call-1");
        assert_eq!(map, Some("let me think...".into()));

        let map = store.find_reasoning_by_call_id("conv-1", "nonexistent");
        assert_eq!(map, None);
    }

    #[test]
    fn response_store_delegates_to_backend() {
        let backend = MemoryStore::new(3600);
        let store = ResponseStore::new(Box::new(backend), 3600);

        store.put(StoredResponse {
            id: "rs-1".into(),
            model: "m".into(),
            created_at: 1,
            status: "ok".into(),
            request_json: serde_json::json!({}),
            response_json: serde_json::json!({}),
            canonical_messages: vec![],
            upstream_sse_events: vec![],
            response_sse_events: vec![],
            conversation_id: None,
        });

        assert_eq!(store.len(), 1);
        let got = store.get("rs-1").unwrap();
        assert_eq!(got.id, "rs-1");
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn sqlite_store_round_trips_sse_events() {
        let path =
            std::env::temp_dir().join(format!("codex-shim-store-{}.db", uuid::Uuid::new_v4()));
        let store = SqliteStore::new(&path, 3600).expect("sqlite store");

        store.put(StoredResponse {
            id: "sqlite-sse-1".into(),
            model: "m".into(),
            created_at: 1,
            status: "completed".into(),
            request_json: serde_json::json!({"input": "hello"}),
            response_json: serde_json::json!({"id": "sqlite-sse-1"}),
            canonical_messages: vec![],
            upstream_sse_events: vec![serde_json::json!({"id": "chunk-1"})],
            response_sse_events: vec![serde_json::json!({"type": "response.completed"})],
            conversation_id: None,
        });

        assert!(path.exists());
        let got = store.get("sqlite-sse-1").expect("stored response");
        assert_eq!(
            got.upstream_sse_events,
            vec![serde_json::json!({"id": "chunk-1"})]
        );
        assert_eq!(
            got.response_sse_events,
            vec![serde_json::json!({"type": "response.completed"})]
        );

        drop(store);
        let _ = std::fs::remove_file(path);
    }
}
