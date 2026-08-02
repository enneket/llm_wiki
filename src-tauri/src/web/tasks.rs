use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use walkdir::WalkDir;

use crate::web::app_state::AppState;
use crate::web::app_state::ProjectEntry;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TaskKind {
    Ingest,
    Rescan,
    Lint,
    Sweep,
    Enrich,
    Chat,
}

impl TaskKind {
    fn as_str(self) -> &'static str {
        match self {
            TaskKind::Ingest => "ingest",
            TaskKind::Rescan => "rescan",
            TaskKind::Lint => "lint",
            TaskKind::Sweep => "sweep",
            TaskKind::Enrich => "enrich",
            TaskKind::Chat => "chat",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Pending,
    Running,
    Done,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub project_id: String,
    pub kind: TaskKind,
    pub target: String,
    pub status: TaskStatus,
    pub message: Option<String>,
    pub error: Option<String>,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub progress: Option<f32>,
}

pub struct TaskRegistry {
    inner: Mutex<TaskRegistryInner>,
    app_state: AppState,
}

struct TaskRegistryInner {
    tasks: BTreeMap<String, Task>,
    /// Event log kept in memory for the SSE endpoint. Truncated
    /// automatically when it grows past `EVENT_LOG_MAX`.
    events: Vec<TaskEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskEvent {
    pub id: String,
    pub task_id: String,
    pub project_id: String,
    pub kind: TaskKind,
    pub status: TaskStatus,
    pub at: i64,
    pub message: Option<String>,
}

const EVENT_LOG_MAX: usize = 1024;
const PERSIST_FILE: &str = ".tasks/tasks.json";

impl TaskRegistry {
    pub fn new(app_state: AppState) -> Self {
        let path = app_state.data_dir().join(PERSIST_FILE);
        let persisted = read_persisted(&path);
        let mut inner = TaskRegistryInner {
            tasks: BTreeMap::new(),
            events: Vec::new(),
        };
        for task in persisted {
            // Anything that was running when the process died should
            // resume from pending so a worker picks it up again.
            let mut task = task;
            if matches!(task.status, TaskStatus::Running) {
                task.status = TaskStatus::Pending;
            }
            inner.tasks.insert(task.id.clone(), task);
        }
        Self {
            inner: Mutex::new(inner),
            app_state,
        }
    }

    pub fn enqueue(
        self: &Arc<Self>,
        project: &ProjectEntry,
        kind: TaskKind,
        target: &str,
        payload: Option<Value>,
    ) -> Task {
        let id = format!(
            "task-{}-{}",
            kind.as_str(),
            now_ms()
        );
        let task = Task {
            id: id.clone(),
            project_id: project.id.clone(),
            kind,
            target: target.to_string(),
            status: TaskStatus::Pending,
            message: payload
                .as_ref()
                .and_then(|v| v.get("message").and_then(|m| m.as_str().map(|s| s.to_string()))),
            error: None,
            started_at: now_ms(),
            finished_at: None,
            progress: Some(0.0),
        };
        {
            let mut inner = self.inner.lock().expect("task registry poisoned");
            inner.tasks.insert(id.clone(), task.clone());
            inner.events.push(TaskEvent {
                id: format!("evt-{}", now_ms()),
                task_id: id.clone(),
                project_id: task.project_id.clone(),
                kind,
                status: TaskStatus::Pending,
                at: task.started_at,
                message: Some(format!(
                    "Queued {} task for {}",
                    kind.as_str(),
                    task.target
                )),
            });
            trim_events(&mut inner.events);
        }
        self.persist();
        self.spawn_worker(project, kind, target, payload);
        task
    }

    pub fn list(
        &self,
        project_filter: Option<String>,
        status_filter: Option<String>,
    ) -> Vec<Task> {
        let inner = self.inner.lock().expect("task registry poisoned");
        inner
            .tasks
            .values()
            .filter(|task| match project_filter.as_ref() {
                Some(p) => task.project_id == *p,
                None => true,
            })
            .filter(|task| match status_filter.as_ref() {
                Some(s) => task.status.as_str() == s.as_str(),
                None => true,
            })
            .cloned()
            .collect()
    }

    pub fn get(&self, task_id: &str) -> Option<Task> {
        let inner = self.inner.lock().expect("task registry poisoned");
        inner.tasks.get(task_id).cloned()
    }

    pub fn cancel(&self, task_id: &str) -> Option<bool> {
        let mut inner = self.inner.lock().expect("task registry poisoned");
        let task = inner.tasks.get_mut(task_id)?;
        if matches!(task.status, TaskStatus::Done | TaskStatus::Failed | TaskStatus::Cancelled) {
            return Some(false);
        }
        task.status = TaskStatus::Cancelled;
        task.finished_at = Some(now_ms());
        task.message = Some("Cancelled by user".to_string());
        let task_id = task.id.clone();
        let project_id = task.project_id.clone();
        let kind = task.kind;
        inner.events.push(TaskEvent {
            id: format!("evt-{}", now_ms()),
            task_id,
            project_id,
            kind,
            status: TaskStatus::Cancelled,
            at: now_ms(),
            message: Some("Cancelled".to_string()),
        });
        trim_events(&mut inner.events);
        drop(inner);
        self.persist();
        Some(true)
    }

    pub fn cancel_session(&self, project_id: &str, session_id: &str) -> bool {
        let mut inner = self.inner.lock().expect("task registry poisoned");
        let mut any = false;
        let mut to_cancel: Vec<(String, String, TaskKind)> = Vec::new();
        for task in inner.tasks.values() {
            if task.project_id != project_id {
                continue;
            }
            if !matches!(task.kind, TaskKind::Chat) {
                continue;
            }
            if !task.target.contains(session_id) {
                continue;
            }
            if matches!(
                task.status,
                TaskStatus::Done | TaskStatus::Failed | TaskStatus::Cancelled
            ) {
                continue;
            }
            to_cancel.push((task.id.clone(), task.project_id.clone(), task.kind));
        }
        for (task_id, task_project_id, kind) in &to_cancel {
            if let Some(task) = inner.tasks.get_mut(task_id) {
                task.status = TaskStatus::Cancelled;
                task.finished_at = Some(now_ms());
                task.message = Some("Cancelled by chat-cancel".to_string());
            }
            inner.events.push(TaskEvent {
                id: format!("evt-{}", now_ms()),
                task_id: task_id.clone(),
                project_id: task_project_id.clone(),
                kind: *kind,
                status: TaskStatus::Cancelled,
                at: now_ms(),
                message: Some("Cancelled".to_string()),
            });
            any = true;
        }
        trim_events(&mut inner.events);
        drop(inner);
        if any {
            self.persist();
        }
        any
    }

    pub fn snapshot_events(&self) -> Vec<TaskEvent> {
        let inner = self.inner.lock().expect("task registry poisoned");
        inner.events.clone()
    }

    fn persist(&self) {
        let snapshot: Vec<Task> = {
            let inner = self.inner.lock().expect("task registry poisoned");
            inner.tasks.values().cloned().collect()
        };
        let path = self.app_state.data_dir().join(PERSIST_FILE);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let serialized = serde_json::to_string_pretty(&snapshot)
            .unwrap_or_else(|_| "[]".to_string());
        let _ = std::fs::write(&path, serialized);
    }

    fn spawn_worker(
        self: &Arc<Self>,
        project: &ProjectEntry,
        kind: TaskKind,
        target: &str,
        payload: Option<Value>,
    ) {
        let registry = Arc::clone(self);
        let task_id = {
            let inner = registry.inner.lock().expect("task registry poisoned");
            inner
                .tasks
                .values()
                .rev()
                .find(|t| {
                    t.project_id == project.id && t.target == target && t.kind == kind
                })
                .map(|t| t.id.clone())
                .unwrap_or_default()
        };
        let cancel_for_worker = Arc::new(AtomicBool::new(false));
        let project = project.clone();
        let kind_str = kind;
        let target_string = target.to_string();
        thread::Builder::new()
            .name(format!("task-{}-{}", kind_str.as_str(), task_id))
            .spawn(move || {
                let mut progress = 0.0f32;
                update_status(&registry, &task_id, TaskStatus::Running, None, Some(0.05));
                let outcome = run_task(
                    &registry.app_state,
                    &project,
                    kind_str,
                    &target_string,
                    payload.as_ref(),
                    &cancel_for_worker,
                    &mut progress,
                );
                match outcome {
                    Ok(message) => {
                        update_status(
                            &registry,
                            &task_id,
                            TaskStatus::Done,
                            Some(message),
                            Some(1.0),
                        );
                    }
                    Err(TaskFailure::Cancelled) => {
                        update_status(
                            &registry,
                            &task_id,
                            TaskStatus::Cancelled,
                            Some("Cancelled".to_string()),
                            Some(progress),
                        );
                    }
                    Err(TaskFailure::Failed(err)) => {
                        update_status(
                            &registry,
                            &task_id,
                            TaskStatus::Failed,
                            Some(err),
                            Some(progress),
                        );
                    }
                }
            })
            .expect("failed to spawn task worker");
    }
}

impl TaskStatus {
    fn as_str(self) -> &'static str {
        match self {
            TaskStatus::Pending => "pending",
            TaskStatus::Running => "running",
            TaskStatus::Done => "done",
            TaskStatus::Failed => "failed",
            TaskStatus::Cancelled => "cancelled",
        }
    }
}

fn trim_events(events: &mut Vec<TaskEvent>) {
    if events.len() > EVENT_LOG_MAX {
        let drop_count = events.len() - EVENT_LOG_MAX;
        events.drain(0..drop_count);
    }
}

fn read_persisted(path: &std::path::Path) -> Vec<Task> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(_) => return Vec::new(),
    };
    serde_json::from_str::<Vec<Task>>(&raw).unwrap_or_default()
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

fn update_status(
    registry: &TaskRegistry,
    task_id: &str,
    status: TaskStatus,
    message: Option<String>,
    progress: Option<f32>,
) {
    let mut inner = match registry.inner.lock() {
        Ok(inner) => inner,
        Err(_) => return,
    };
    let (project_id, kind) = match inner.tasks.get_mut(task_id) {
        Some(task) => {
            task.status = status;
            if let Some(m) = message.clone() {
                task.message = Some(m.clone());
            }
            task.progress = progress;
            if matches!(
                status,
                TaskStatus::Done | TaskStatus::Failed | TaskStatus::Cancelled
            ) {
                task.finished_at = Some(now_ms());
            }
            (task.project_id.clone(), task.kind)
        }
        None => return,
    };
    inner.events.push(TaskEvent {
        id: format!("evt-{}", now_ms()),
        task_id: task_id.to_string(),
        project_id,
        kind,
        status,
        at: now_ms(),
        message,
    });
    trim_events(&mut inner.events);
    drop(inner);
    registry.persist();
}

#[derive(Debug, Clone)]
enum TaskFailure {
    Cancelled,
    Failed(String),
}

fn run_task(
    app_state: &AppState,
    project: &ProjectEntry,
    kind: TaskKind,
    target: &str,
    payload: Option<&Value>,
    cancel: &Arc<AtomicBool>,
    progress: &mut f32,
) -> Result<String, TaskFailure> {
    if cancel.load(Ordering::SeqCst) {
        return Err(TaskFailure::Cancelled);
    }
    match kind {
        TaskKind::Rescan => rescan_sources(app_state, project, cancel, progress),
        TaskKind::Ingest => ingest_files(app_state, project, target, cancel, progress),
        TaskKind::Lint => run_lint(app_state, project, cancel, progress),
        TaskKind::Sweep => run_sweep(app_state, project, cancel, progress),
        TaskKind::Enrich => run_enrich(app_state, project, target, cancel, progress),
        TaskKind::Chat => run_chat(app_state, project, target, payload, cancel, progress),
    }
}

fn rescan_sources(
    app_state: &AppState,
    project: &ProjectEntry,
    cancel: &Arc<AtomicBool>,
    progress: &mut f32,
) -> Result<String, TaskFailure> {
    let root = PathBuf::from(&project.path);
    let mut total = 0usize;
    let mut queued = 0usize;
    for rel in [
        "raw/sources",
        "wiki",
        "purpose.md",
        "schema.md",
    ] {
        let path = root.join(rel);
        if path.is_file() {
            if !cancel.load(Ordering::SeqCst) {
                if enqueue_pending_ingest(app_state, &project.path, rel).is_ok() {
                    queued += 1;
                }
            }
            total += 1;
        } else if path.exists() {
            for entry in WalkDir::new(&path)
                .max_depth(8)
                .into_iter()
                .filter_map(Result::ok)
            {
                if !entry.file_type().is_file() {
                    continue;
                }
                let rel_path = match entry.path().strip_prefix(&root) {
                    Ok(p) => p.to_string_lossy().replace('\\', "/"),
                    Err(_) => continue,
                };
                if !should_rescan_rel(&rel_path) {
                    continue;
                }
                if cancel.load(Ordering::SeqCst) {
                    break;
                }
                total += 1;
                if enqueue_pending_ingest(app_state, &project.path, &rel_path).is_ok() {
                    queued += 1;
                }
                if total % 32 == 0 {
                    *progress = (total.min(255) as f32) / 255.0;
                }
            }
        }
    }
    *progress = 1.0;
    Ok(format!(
        "Rescan complete: scanned {total} files, queued {queued} for ingest."
    ))
}

fn ingest_files(
    app_state: &AppState,
    project: &ProjectEntry,
    target: &str,
    cancel: &Arc<AtomicBool>,
    progress: &mut f32,
) -> Result<String, TaskFailure> {
    let root = PathBuf::from(&project.path);
    let explicit: Vec<String> = if target == "all" || target.is_empty() {
        list_ingestable_sources(&root)
    } else {
        target
            .split(|c: char| c == ',' || c == ';')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    };
    let mut queued = 0usize;
    for rel in &explicit {
        if cancel.load(Ordering::SeqCst) {
            break;
        }
        if enqueue_pending_ingest(app_state, &project.path, rel).is_ok() {
            queued += 1;
        }
        *progress = (queued.min(explicit.len()) as f32) / (explicit.len().max(1) as f32);
    }
    *progress = 1.0;
    Ok(format!(
        "Ingest enqueued {queued} source files for project '{}'.",
        project.name
    ))
}

fn run_lint(
    app_state: &AppState,
    project: &ProjectEntry,
    _cancel: &Arc<AtomicBool>,
    progress: &mut f32,
) -> Result<String, TaskFailure> {
    let wiki_root = PathBuf::from(&project.path).join("wiki");
    let mut checked = 0usize;
    let mut issues = 0usize;
    if !wiki_root.exists() {
        *progress = 1.0;
        return Ok("Wiki directory not found; nothing to lint.".to_string());
    }
    for entry in WalkDir::new(&wiki_root)
        .max_depth(4)
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file()
            || entry.path().extension().and_then(|s| s.to_str()) != Some("md")
        {
            continue;
        }
        checked += 1;
        if let Ok(content) = std::fs::read_to_string(entry.path()) {
            for line in content.lines() {
                if line.contains("[[") && !line.contains("]]") {
                    issues += 1;
                }
            }
        }
    }
    *progress = 1.0;
    let _ = app_state;
    Ok(format!(
        "Lint scanned {checked} wiki pages, found {issues} unbalanced wikilinks."
    ))
}

fn run_sweep(
    app_state: &AppState,
    project: &ProjectEntry,
    _cancel: &Arc<AtomicBool>,
    progress: &mut f32,
) -> Result<String, TaskFailure> {
    let wiki_root = PathBuf::from(&project.path).join("wiki");
    let mut reviewed = 0usize;
    if wiki_root.exists() {
        for entry in WalkDir::new(&wiki_root)
            .max_depth(4)
            .into_iter()
            .filter_map(Result::ok)
        {
            if entry.file_type().is_file()
                && entry.path().extension().and_then(|s| s.to_str()) == Some("md")
            {
                reviewed += 1;
            }
        }
    }
    *progress = 1.0;
    let _ = app_state;
    Ok(format!("Sweep reviewed {reviewed} wiki pages."))
}

fn run_enrich(
    _app_state: &AppState,
    _project: &ProjectEntry,
    _target: &str,
    _cancel: &Arc<AtomicBool>,
    _progress: &mut f32,
) -> Result<String, TaskFailure> {
    // Enrich requires an LLM to expand wiki pages. The headless web server
    // ships without an LLM configuration (it has its own data dir, separate
    // from the desktop app's), so the honest answer is "unsupported here"
    // rather than a fake "done". Returning Err surfaces this in the task
    // log instead of looking like a successful no-op run.
    Err(TaskFailure::Failed(
        "Enrich is not supported by the headless web server: no LLM provider configured. Run the desktop app or wire an LLM endpoint before retrying.".to_string(),
    ))
}

fn run_chat(
    app_state: &AppState,
    project: &ProjectEntry,
    target: &str,
    payload: Option<&Value>,
    cancel: &Arc<AtomicBool>,
    progress: &mut f32,
) -> Result<String, TaskFailure> {
    let body = target;
    let message = payload
        .and_then(|v| v.get("message").and_then(|m| m.as_str()))
        .map(|s| s.to_string())
        .or_else(|| {
            serde_json::from_str::<Value>(body)
                .ok()
                .and_then(|v| v.get("message").and_then(|m| m.as_str().map(|s| s.to_string())))
        })
        .unwrap_or_default();
    let session = payload
        .and_then(|v| v.get("sessionId").and_then(|s| s.as_str()))
        .map(|s| s.to_string())
        .or_else(|| {
            serde_json::from_str::<Value>(body)
                .ok()
                .and_then(|v| v.get("sessionId").and_then(|s| s.as_str().map(|s| s.to_string())))
        })
        .unwrap_or_else(|| format!("web-{}", now_ms()));
    if message.is_empty() {
        return Err(TaskFailure::Failed("Missing chat message".to_string()));
    }
    if let Err(err) = app_state.append_chat_message(&project.path, &session, "user", &message) {
        return Err(TaskFailure::Failed(err.to_string()));
    }
    *progress = 0.25;
    if cancel.load(Ordering::SeqCst) {
        return Err(TaskFailure::Cancelled);
    }
    let reply = run_agent_chat(app_state, project, &message);
    *progress = 0.75;
    if cancel.load(Ordering::SeqCst) {
        return Err(TaskFailure::Cancelled);
    }
    if let Err(err) = app_state.append_chat_message(&project.path, &session, "assistant", &reply) {
        return Err(TaskFailure::Failed(err.to_string()));
    }
    *progress = 1.0;
    Ok(format!(
        "Chat turn completed in session {session} (headless web agent, {}-token reply).",
        reply.split_whitespace().count()
    ))
}

fn run_agent_chat(app_state: &AppState, project: &ProjectEntry, user_message: &str) -> String {
    let _ = app_state;
    let provider = read_env_agent_provider();
    let provider = match provider {
        Some(value) => value,
        None => {
            return build_keyword_chat_reply(project, user_message);
        }
    };
    let result = run_provider_chat(&provider, project, user_message);
    match result {
        Ok(reply) if !reply.trim().is_empty() => reply,
        Ok(_) => build_keyword_chat_reply(project, user_message),
        Err(err) => format!(
            "[agent error] {err}\n\n{}",
            build_keyword_chat_reply(project, user_message)
        ),
    }
}

#[derive(Debug, Clone)]
struct AgentProvider {
    kind: String,
    base_url: String,
    api_key: String,
    model: String,
}

fn read_env_agent_provider() -> Option<AgentProvider> {
    let base_url = std::env::var("LLM_WIKI_AGENT_BASE_URL")
        .ok()
        .filter(|v| !v.trim().is_empty())?;
    let api_key = std::env::var("LLM_WIKI_AGENT_API_KEY").unwrap_or_default();
    let model = std::env::var("LLM_WIKI_AGENT_MODEL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "gpt-4o-mini".to_string());
    let kind = std::env::var("LLM_WIKI_AGENT_KIND")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "openai".to_string())
        .to_ascii_lowercase();
    Some(AgentProvider {
        kind,
        base_url,
        api_key,
        model,
    })
}

fn run_provider_chat(
    provider: &AgentProvider,
    project: &ProjectEntry,
    user_message: &str,
) -> Result<String, String> {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::Duration;

    let url = match reqwest::Url::parse(&provider.base_url) {
        Ok(value) => value,
        Err(err) => return Err(format!("Invalid agent base URL: {err}")),
    };
    let host = url
        .host_str()
        .ok_or_else(|| "Agent base URL is missing a host".to_string())?;
    let port = url.port_or_known_default().unwrap_or(443);
    let use_tls = url.scheme() == "https" || url.scheme() == "wss";
    let path = if url.path().is_empty() { "/" } else { url.path() };

    let body = serde_json::json!({
        "model": provider.model,
        "messages": [
            { "role": "system", "content": format!("You are the headless LLM Wiki assistant for project '{}'. Answer concisely and ground your reply in the project's wiki when possible.", project.name) },
            { "role": "user", "content": user_message }
        ],
        "temperature": 0.2,
    });
    let body_bytes = serde_json::to_vec(&body).map_err(|err| err.to_string())?;

    if !use_tls {
        let mut stream = TcpStream::connect((host, port))
            .map_err(|err| format!("Connect {host}:{port} failed: {err}"))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .map_err(|err| err.to_string())?;
        let mut request = format!(
            "POST {path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
            body_bytes.len()
        );
        if !provider.api_key.is_empty() {
            request.push_str(&format!("Authorization: Bearer {}\r\n", provider.api_key));
        }
        request.push_str("\r\n");
        stream
            .write_all(request.as_bytes())
            .map_err(|err| err.to_string())?;
        stream.write_all(&body_bytes).map_err(|err| err.to_string())?;
        let mut raw = String::new();
        stream
            .read_to_string(&mut raw)
            .map_err(|err| err.to_string())?;
        return parse_chat_response(&raw);
    }

    Err("HTTPS agent endpoints are not yet supported by the headless web server; set LLM_WIKI_AGENT_BASE_URL to an http:// endpoint.".to_string())
}

fn parse_chat_response(raw: &str) -> Result<String, String> {
    let split = raw
        .find("\r\n\r\n")
        .ok_or_else(|| "HTTP response missing body separator".to_string())?;
    let (head, body) = raw.split_at(split);
    let body = body.trim_start_matches("\r\n\r\n");
    if !head.contains(" 200") {
        return Err(format!("Agent HTTP error: {}", head.lines().next().unwrap_or("")));
    }
    let json: serde_json::Value = serde_json::from_str(body)
        .map_err(|err| format!("Agent response is not JSON: {err}"))?;
    if let Some(content) = json
        .pointer("/choices/0/message/content")
        .and_then(|value| value.as_str())
    {
        return Ok(content.to_string());
    }
    if let Some(content) = json
        .pointer("/output/0/content/0/text")
        .and_then(|value| value.as_str())
    {
        return Ok(content.to_string());
    }
    Err("Agent response did not include a recognised text field".to_string())
}

fn build_keyword_chat_reply(project: &ProjectEntry, user_message: &str) -> String {
    let wiki_root = PathBuf::from(&project.path).join("wiki");
    if !wiki_root.exists() {
        return format!(
            "[keyword fallback] No wiki pages exist yet for project '{}'. Import sources from the Sources view to start the wiki.",
            project.name
        );
    }
    let tokens: Vec<String> = user_message
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| s.len() >= 3)
        .map(|s| s.to_lowercase())
        .collect();
    let mut hits: Vec<(String, String)> = Vec::new();
    for entry in WalkDir::new(&wiki_root)
        .max_depth(4)
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file()
            || entry.path().extension().and_then(|s| s.to_str()) != Some("md")
        {
            continue;
        }
        let content = match std::fs::read_to_string(entry.path()) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let lower = content.to_lowercase();
        let score = tokens
            .iter()
            .filter(|token| !token.is_empty() && lower.contains(token.as_str()))
            .count();
        if score > 0 {
            let title = content
                .lines()
                .find(|line| line.starts_with("# "))
                .map(|line| line.trim_start_matches("# ").trim().to_string())
                .unwrap_or_else(|| {
                    entry
                        .path()
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("page")
                        .to_string()
                });
            let rel = entry
                .path()
                .strip_prefix(&PathBuf::from(&project.path))
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| entry.path().to_string_lossy().to_string());
            hits.push((rel, format!("{title} ({score} matches)")));
        }
    }
    hits.sort_by(|a, b| b.1.cmp(&a.1));
    hits.truncate(5);
    if hits.is_empty() {
        format!(
            "[keyword fallback] Searched the wiki for '{}' but found no matching pages. The headless server has no LLM; for richer answers run the desktop app.",
            user_message.chars().take(80).collect::<String>()
        )
    } else {
        let mut reply = String::new();
        reply.push_str(&format!(
            "[keyword fallback] Found {} matching wiki page(s):\n\n",
            hits.len()
        ));
        for (rel, title) in &hits {
            reply.push_str(&format!("- {title} — `wiki/{rel}`\n"));
        }
        reply
    }
}

fn should_rescan_rel(rel: &str) -> bool {
    if rel.is_empty() || rel.starts_with('.') {
        return false;
    }
    let lower = rel.to_lowercase();
    if lower.starts_with(".llm-wiki/") || lower.contains("/.llm-wiki/") {
        return false;
    }
    if lower == "purpose.md" || lower == "schema.md" {
        return true;
    }
    if lower.starts_with("wiki/") && lower.ends_with(".md") {
        return true;
    }
    if lower.starts_with("raw/sources/") {
        return true;
    }
    false
}

fn list_ingestable_sources(root: &Path) -> Vec<String> {
    let sources = root.join("raw").join("sources");
    let mut out = Vec::new();
    if !sources.exists() {
        return out;
    }
    for entry in WalkDir::new(&sources)
        .max_depth(6)
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }
        if let Ok(rel) = entry.path().strip_prefix(root) {
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
    out
}

const PENDING_INGEST_KEY: &str = "pendingIngestQueue";

fn enqueue_pending_ingest(
    app_state: &AppState,
    project_path: &str,
    rel: &str,
) -> std::io::Result<()> {
    use std::io::Write;
    let path = PathBuf::from(project_path)
        .join(".llm-wiki")
        .join(PENDING_INGEST_KEY.to_string() + ".json");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut queue: Vec<serde_json::Value> = match std::fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
        Err(_) => Vec::new(),
    };
    queue.push(serde_json::json!({
        "rel": rel,
        "enqueuedAt": now_ms(),
    }));
    if let Ok(serialized) = serde_json::to_string_pretty(&queue) {
        let mut file = std::fs::File::create(&path)?;
        file.write_all(serialized.as_bytes())?;
    }
    let _ = app_state;
    Ok(())
}
