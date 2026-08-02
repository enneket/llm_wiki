use std::collections::{BTreeMap, VecDeque};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use tiny_http::{Header, Method, Response, Server, StatusCode};

use crate::cors::{local_cors_headers, request_origin};
use crate::web::app_state::{AppState, ProjectEntry};
use crate::web::http::{self, ApiResponse};
use crate::web::multipart;
use crate::web::tasks::{TaskKind, TaskRegistry};

pub const DEFAULT_BIND_HOST: &str = "127.0.0.1";
pub const DEFAULT_PORT: u16 = 8080;
pub const API_PREFIX: &str = "/api/v1";
pub const MAX_FILE_CONTENT_BYTES: u64 = 2 * 1024 * 1024;
pub const DEFAULT_MAX_FILES: usize = 2_000;
pub const HARD_MAX_FILES: usize = 10_000;
pub const DEFAULT_MAX_REVIEWS: usize = 200;
pub const HARD_MAX_REVIEWS: usize = 1_000;
pub const BIND_RETRY_DELAY_SECS: u64 = 2;
pub const MAX_BIND_RETRIES: u32 = 3;
pub const APP_STATE_CACHE_TTL: Duration = Duration::from_secs(2);
pub const RATE_LIMIT_WINDOW: Duration = Duration::from_secs(1);
pub const RATE_LIMIT_MAX_REQUESTS: usize = 240;
pub const MAX_IN_FLIGHT_REQUESTS: usize = 128;

#[derive(Debug, Clone)]
pub struct BackendConfig {
    pub bind_addr: SocketAddr,
    pub data_dir: PathBuf,
    pub dist_dir: PathBuf,
    pub api_token: Option<String>,
    pub allow_unauthenticated: bool,
    pub allow_lan_access: bool,
    pub max_upload_bytes: u64,
    pub max_body_bytes: usize,
}

pub struct BackendHandle {
    shutdown: Arc<AtomicBool>,
    server_thread: Option<thread::JoinHandle<()>>,
    ctx: Arc<AppContext>,
    _watcher: Option<crate::web::watcher::ProjectWatcher>,
}

impl Drop for BackendHandle {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(handle) = self.server_thread.take() {
            let _ = handle.join();
        }
    }
}

pub struct AppContext {
    pub config: BackendConfig,
    pub app_state: AppState,
    pub tasks: Arc<TaskRegistry>,
    app_state_cache: Mutex<Option<CachedAppState>>,
    rate_limit: Mutex<VecDeque<std::time::Instant>>,
    in_flight: Arc<AtomicUsize>,
    bind_status: Arc<AtomicU8>,
}

#[derive(Clone)]
struct CachedAppState {
    loaded_at: std::time::Instant,
    value: Option<serde_json::Value>,
}

impl AppContext {
    pub fn new(config: BackendConfig) -> std::io::Result<Arc<Self>> {
        let app_state = AppState::open(&config.data_dir)?;
        let tasks = Arc::new(TaskRegistry::new(app_state.clone()));
        Ok(Arc::new(Self {
            config,
            app_state,
            tasks,
            app_state_cache: Mutex::new(None),
            rate_limit: Mutex::new(VecDeque::new()),
            in_flight: Arc::new(AtomicUsize::new(0)),
            bind_status: Arc::new(AtomicU8::new(0)),
        }))
    }

    pub fn data_root(&self) -> &Path {
        &self.config.data_dir
    }

    pub fn is_authorized(&self, query: &str, headers: &[(String, String)]) -> bool {
        if self.config.allow_unauthenticated {
            return true;
        }
        let Some(token) = self.config.api_token.as_ref().filter(|s| !s.is_empty()) else {
            return false;
        };
        if let Some(value) = parse_query(query).get("token") {
            if constant_time_eq(value.as_bytes(), token.as_bytes()) {
                return true;
            }
        }
        for (k, v) in headers {
            let key = k.to_ascii_lowercase();
            if key == "x-llm-wiki-token" {
                if constant_time_eq(v.as_bytes(), token.as_bytes()) {
                    return true;
                }
            } else if key == "authorization" {
                if let Some(value) = v.strip_prefix("Bearer ") {
                    if constant_time_eq(value.as_bytes(), token.as_bytes()) {
                        return true;
                    }
                }
            }
        }
        false
    }

    pub fn auth_required(&self) -> bool {
        !self.config.allow_unauthenticated
    }

    pub fn auth_configured(&self) -> bool {
        self.config
            .api_token
            .as_ref()
            .map(|s| !s.is_empty())
            .unwrap_or(false)
    }

    pub fn allow_request(&self) -> bool {
        let now = std::time::Instant::now();
        let mut guard = match self.rate_limit.lock() {
            Ok(g) => g,
            Err(_) => return false,
        };
        let cutoff = now - RATE_LIMIT_WINDOW;
        while guard
            .front()
            .map(|t| *t < cutoff)
            .unwrap_or(false)
        {
            guard.pop_front();
        }
        if guard.len() >= RATE_LIMIT_MAX_REQUESTS {
            return false;
        }
        guard.push_back(now);
        true
    }

    pub fn try_acquire_request_slot(&self) -> Option<RequestSlot> {
        let mut current = self.in_flight.load(Ordering::Relaxed);
        loop {
            if current >= MAX_IN_FLIGHT_REQUESTS {
                return None;
            }
            match self.in_flight.compare_exchange_weak(
                current,
                current + 1,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return Some(RequestSlot {
                    counter: self.in_flight.clone(),
                }),
                Err(next) => current = next,
            }
        }
    }

    pub fn load_app_state_value(&self) -> Option<serde_json::Value> {
        let now = std::time::Instant::now();
        if let Ok(guard) = self.app_state_cache.lock() {
            if let Some(cached) = guard.as_ref() {
                if now.duration_since(cached.loaded_at) < APP_STATE_CACHE_TTL {
                    return cached.value.clone();
                }
            }
        }
        let fresh = self.app_state.read_app_state();
        if let Ok(mut guard) = self.app_state_cache.lock() {
            *guard = Some(CachedAppState {
                loaded_at: now,
                value: fresh.clone(),
            });
        }
        fresh
    }

    pub fn invalidate_app_state(&self) {
        if let Ok(mut guard) = self.app_state_cache.lock() {
            *guard = None;
        }
    }
}

pub struct RequestSlot {
    counter: Arc<AtomicUsize>,
}

impl Drop for RequestSlot {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::Relaxed);
    }
}

pub fn run_server(config: BackendConfig) -> std::io::Result<BackendHandle> {
    let ctx = AppContext::new(config.clone())?;
    let bind_status = ctx.bind_status.clone();
    let shutdown = Arc::new(AtomicBool::new(false));
    let server_shutdown = shutdown.clone();
    let ctx_for_thread = ctx.clone();
    let server_thread = thread::Builder::new()
        .name("llm-wiki-web".to_string())
        .spawn(move || {
            serve_until(ctx_for_thread, server_shutdown, bind_status);
        })?;
    let watcher = match crate::web::watcher::ProjectWatcher::start(
        ctx.tasks.clone(),
        ctx.app_state.clone(),
    ) {
        Ok(watcher) => Some(watcher),
        Err(err) => {
            eprintln!("[web] WARN: file watcher failed to start: {err}");
            None
        }
    };
    Ok(BackendHandle {
        shutdown,
        server_thread: Some(server_thread),
        ctx,
        _watcher: watcher,
    })
}

fn serve_until(ctx: Arc<AppContext>, shutdown: Arc<AtomicBool>, bind_status: Arc<AtomicU8>) {
    let mut last_err: Option<String> = None;
    for attempt in 1..=MAX_BIND_RETRIES {
        if shutdown.load(Ordering::SeqCst) {
            return;
        }
        match Server::http(ctx.config.bind_addr) {
            Ok(server) => {
                bind_status.store(1, Ordering::Relaxed);
                eprintln!(
                    "[web] Listening on http://{}{}",
                    ctx.config.bind_addr, API_PREFIX
                );
                serve_loop(server, ctx, shutdown);
                return;
            }
            Err(err) => {
                last_err = Some(err.to_string());
                eprintln!(
                    "[web] bind attempt {attempt}/{MAX_BIND_RETRIES} failed: {err}"
                );
                thread::sleep(Duration::from_secs(BIND_RETRY_DELAY_SECS));
            }
        }
    }
    bind_status.store(2, Ordering::Relaxed);
    eprintln!(
        "[web] failed to bind after {MAX_BIND_RETRIES} attempts: {}",
        last_err.unwrap_or_default()
    );
}

fn serve_loop(server: Server, ctx: Arc<AppContext>, shutdown: Arc<AtomicBool>) {
    for request in server.incoming_requests() {
        if shutdown.load(Ordering::SeqCst) {
            break;
        }
        let method = request.method().clone();
        let url = request.url().to_string();
        let origin = request_origin(&request);
        if should_rate_limit(&method, &url) && !ctx.allow_request() {
            respond_error(request, 429, "Too many requests", origin.as_deref());
            continue;
        }
        let slot = match ctx.try_acquire_request_slot() {
            Some(slot) => slot,
            None => {
                respond_error(request, 503, "Server is busy", origin.as_deref());
                continue;
            }
        };
        let ctx_clone = ctx.clone();
        thread::spawn(move || {
            let _slot = slot;
            process_request(ctx_clone, request);
        });
    }
}

fn should_rate_limit(method: &Method, url: &str) -> bool {
    if method == &Method::Options {
        return false;
    }
    let (path, _) = split_url(url);
    if path == "/health" || path == "/api/v1/health" {
        return false;
    }
    !path.starts_with("/assets/") && !path.starts_with("/static/")
}

fn process_request(ctx: Arc<AppContext>, mut request: tiny_http::Request) {
    let method = request.method().clone();
    let url = request.url().to_string();
    let origin = request_origin(&request);
    if method == Method::Options {
        respond_options(request, origin.as_deref());
        return;
    }
    let headers: Vec<(String, String)> = request
        .headers()
        .iter()
        .map(|h| {
            (
                h.field.as_str().to_string().to_ascii_lowercase(),
                h.value.as_str().to_string(),
            )
        })
        .collect();
    let body_limit = body_limit_for(&method, &url, ctx.config.max_body_bytes);
    let body = if is_upload_request(&method, &url) || is_binary_request(&method, &url) {
        let mut reader = request.as_reader();
        let mut buf = Vec::new();
        let limit = ctx.config.max_upload_bytes.min(usize::MAX as u64) as usize;
        if std::io::Read::read_to_end(&mut reader, &mut buf).is_err() {
            respond_error(request, 400, "Failed to read upload body", origin.as_deref());
            return;
        }
        if buf.len() > limit {
            respond_error(
                request,
                413,
                "Upload too large",
                origin.as_deref(),
            );
            return;
        }
        buf
    } else {
        match http::read_body(&mut request, body_limit) {
            Ok(body) => body.into_bytes(),
            Err(err) => {
                respond_error(request, 400, &err, origin.as_deref());
                return;
            }
        }
    };
    let body_str = match std::str::from_utf8(&body) {
        Ok(s) => s.to_string(),
        Err(_) => {
            let (path, _) = split_url(&url);
            let path_parts: Vec<&str> = path
                .trim_start_matches('/')
                .split('/')
                .filter(|p| !p.is_empty())
                .collect();
            let is_binary = path_parts.last() == Some(&"uploads")
                || path.ends_with("/import-archive");
            if is_binary {
                String::new()
            } else {
                respond_error(request, 400, "Request body must be UTF-8", origin.as_deref());
                return;
            }
        }
    };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        handle_request(&ctx, &method, &url, &body, &body_str, &headers)
    }));
    let response: ApiResponse = match result {
        Ok(value) => value,
        Err(_) => http::err_json(500, "Internal server error"),
    };
    match response.body {
        crate::web::http::ApiBody::Json(value) => {
            let serialized = serde_json::to_string(&value)
                .unwrap_or_else(|_| "{\"ok\":false,\"error\":\"serialization failed\"}".to_string());
            respond(request, response.status, &serialized, origin.as_deref(), "application/json");
        }
        crate::web::http::ApiBody::Raw {
            content_type,
            data,
            extra_headers,
        } => {
            respond_bytes(
                request,
                response.status,
                content_type,
                data,
                extra_headers,
                origin.as_deref(),
            );
        }
    }
}

fn is_upload_request(method: &Method, url: &str) -> bool {
    if method != &Method::Post {
        return false;
    }
    let (path, _) = split_url(url);
    path.ends_with("/uploads")
}

fn is_binary_request(method: &Method, url: &str) -> bool {
    if method != &Method::Post {
        return false;
    }
    let (path, _) = split_url(url);
    path.ends_with("/import-archive")
}

fn body_limit_for(method: &Method, url: &str, fallback: usize) -> usize {
    if method == &Method::Post {
        let (path, _) = split_url(url);
        if path.contains("/chat") {
            return fallback.max(40 * 1024 * 1024);
        }
    }
    fallback.min(1024 * 1024)
}

fn handle_request(
    ctx: &AppContext,
    method: &Method,
    url: &str,
    body: &[u8],
    body_str: &str,
    headers: &[(String, String)],
) -> ApiResponse {
    let (path, query) = split_url(url);
    if method == &Method::Post && path.ends_with("/uploads") {
        let parts: Vec<&str> = path
            .trim_start_matches(API_PREFIX)
            .trim_start_matches('/')
            .split('/')
            .filter(|p| !p.is_empty())
            .collect();
        if let ["projects", project_id, "uploads"] = parts.as_slice() {
            if !ctx.is_authorized(query, headers) {
                return http::err_json(401, "Unauthorized");
            }
            let content_type = headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
                .map(|(_, value)| value.as_str());
            return http::handle_upload(ctx, project_id, body, content_type);
        }
        return http::err_json(404, "Not found");
    }
    if path == "/health" || path == "/api/v1/health" {
        return http::ok_json(serde_json::json!({
            "ok": true,
            "status": "running",
            "version": env!("CARGO_PKG_VERSION"),
            "authRequired": ctx.auth_required(),
            "authConfigured": ctx.auth_configured(),
            "tokenSource": if ctx.auth_configured() { "env" } else { "none" },
            "enabled": true,
            "mcpEnabled": false,
            "allowUnauthenticated": ctx.config.allow_unauthenticated,
            "allowLanAccess": ctx.config.allow_lan_access,
            "agent": { "chat": true, "streaming": true }
        }));
    }
    if !path.starts_with(API_PREFIX) {
        if let Some(response) = http::serve_static(ctx, &path) {
            return response;
        }
        return http::err_json(404, "Not found");
    }
    if !ctx.is_authorized(query, headers) {
        return http::err_json(401, "Unauthorized");
    }

    let parts: Vec<&str> = path
        .trim_start_matches(API_PREFIX)
        .trim_start_matches('/')
        .split('/')
        .filter(|p| !p.is_empty())
        .collect();
    if method == &Method::Get
        && parts.len() == 4
        && parts[0] == "projects"
        && parts[2] == "files"
        && parts[3] == "content"
    {
        let q = parse_query(query);
        if q.get("export").map(|v| v.as_str()) == Some("1") {
            return handle_export_project(ctx, &parts[1]);
        }
    }
    if method == &Method::Post && parts.as_slice() == ["projects", "import-archive"] {
        let content_type = headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
            .map(|(_, value)| value.as_str());
        return handle_import_archive(ctx, body, content_type);
    }
    match (method, parts.as_slice()) {
        (&Method::Get, ["projects"]) => handle_projects(ctx),
        (&Method::Post, ["projects"]) => handle_create_project(ctx, body_str),
        (&Method::Get, ["projects", "by-path", project_path @ ..]) => {
            handle_open_project_by_path(ctx, project_path.join("/"))
        }
        (&Method::Get, ["projects", project_id, "files"]) => handle_files(ctx, project_id, query),
        (&Method::Get, ["projects", project_id, "files", "content"]) => {
            handle_file_content(ctx, project_id, query)
        }
        (&Method::Post, ["projects", project_id, "files", "content"]) => {
            handle_write_file(ctx, project_id, body_str)
        }
        (&Method::Delete, ["projects", project_id, "files", "content"]) => {
            handle_delete_file(ctx, project_id, body_str)
        }
        (&Method::Get, ["projects", project_id, "reviews"]) => {
            handle_reviews(ctx, project_id, query)
        }
        (&Method::Post, ["projects", project_id, "reviews", "resolve"]) => {
            handle_bulk_resolve_reviews(ctx, project_id, body_str)
        }
        (&Method::Patch, ["projects", project_id, "reviews", review_id]) => {
            handle_patch_review(ctx, project_id, review_id, body_str)
        }
        (&Method::Post, ["projects", project_id, "search"]) => handle_search(ctx, project_id, body_str),
        (&Method::Get, ["projects", project_id, "graph"]) => handle_graph(ctx, project_id, query),
        (&Method::Post, ["projects", project_id, "sources", "rescan"]) => {
            handle_rescan(ctx, project_id, body_str)
        }
        (&Method::Post, ["projects", project_id, "chat"]) => handle_chat(ctx, project_id, body_str),
        (&Method::Post, ["projects", project_id, "chat", session_id, "cancel"]) => {
            handle_cancel_chat(ctx, project_id, session_id)
        }
        (&Method::Get, ["projects", project_id, "chat", session_id]) => {
            handle_get_chat(ctx, project_id, session_id)
        }
        (&Method::Get, ["tasks"]) => handle_tasks(ctx, query),
        (&Method::Get, ["events"]) => handle_events(ctx, query),
        (&Method::Get, ["tasks", task_id]) => handle_task(ctx, task_id),
        (&Method::Post, ["tasks", task_id, "cancel"]) => handle_task_cancel(ctx, task_id),
        (&Method::Post, ["tasks"]) => handle_task_enqueue(ctx, body_str),
        _ => http::err_json(404, "Not found"),
    }
}

fn handle_projects(ctx: &AppContext) -> ApiResponse {
    let projects = ctx.app_state.list_projects();
    let current = projects.iter().find(|p| p.current).cloned();
    http::ok_json(serde_json::json!({
        "ok": true,
        "projects": projects,
        "currentProject": current,
    }))
}

fn handle_create_project(ctx: &AppContext, body: &str) -> ApiResponse {
    let req: serde_json::Value = match serde_json::from_str(body) {
        Ok(value) => value,
        Err(err) => return http::err_json(400, format!("Invalid JSON: {err}")),
    };
    let name = match req.get("name").and_then(|v| v.as_str()) {
        Some(name) => name,
        None => return http::err_json(400, "Missing 'name'"),
    };
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return http::err_json(400, "Project name must not be empty");
    }
    let project_id = req
        .get("id")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    match ctx.app_state.create_project(trimmed, project_id.as_deref()) {
        Ok(entry) => http::ok_json(serde_json::json!({
            "ok": true,
            "project": entry,
        })),
        Err(err) => http::err_json(409, format!("{err}")),
    }
}

fn handle_open_project_by_path(ctx: &AppContext, encoded_path: String) -> ApiResponse {
    let decoded = percent_decode(&encoded_path);
    match ctx.app_state.find_project_by_path(&decoded) {
        Some(entry) => http::ok_json(serde_json::json!({
            "ok": true,
            "project": entry,
        })),
        None => http::err_json(404, format!("Unknown project: {decoded}")),
    }
}

fn handle_files(ctx: &AppContext, project_id: &str, query: &str) -> ApiResponse {
    let project = match resolve_project(ctx, project_id) {
        Ok(p) => p,
        Err(e) => return http::err_json(404, e),
    };
    let params = parse_query(query);
    let root = params.get("root").map(|s| s.as_str()).unwrap_or("wiki");
    let recursive = params
        .get("recursive")
        .map(|v| v != "false")
        .unwrap_or(true);
    let max_files = params
        .get("maxFiles")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(DEFAULT_MAX_FILES)
        .clamp(1, HARD_MAX_FILES);
    match http::list_project_files(ctx, &project, root, recursive, max_files) {
        Ok(payload) => http::ok_json(payload),
        Err(e) => http::err_json(if e.contains("exceeds") { 413 } else { 500 }, e),
    }
}

fn handle_file_content(ctx: &AppContext, project_id: &str, query: &str) -> ApiResponse {
    let project = match resolve_project(ctx, project_id) {
        Ok(p) => p,
        Err(e) => return http::err_json(404, e),
    };
    let params = parse_query(query);
    let rel = match params.get("path") {
        Some(p) => p,
        None => return http::err_json(400, "Missing path query parameter"),
    };
    match http::read_text_file(ctx, &project, rel) {
        Ok(content) => http::ok_json(serde_json::json!({
            "ok": true,
            "projectId": project.id,
            "path": rel,
            "content": content,
        })),
        Err(e) => http::err_json(e.status, e.message),
    }
}

fn handle_write_file(ctx: &AppContext, project_id: &str, body: &str) -> ApiResponse {
    let project = match resolve_project(ctx, project_id) {
        Ok(p) => p,
        Err(e) => return http::err_json(404, e),
    };
    let req: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => return http::err_json(400, format!("Invalid JSON: {e}")),
    };
    let rel = match req.get("path").and_then(|v| v.as_str()) {
        Some(p) => p,
        None => return http::err_json(400, "Missing path"),
    };
    let contents = match req.get("contents").and_then(|v| v.as_str()) {
        Some(c) => c,
        None => return http::err_json(400, "Missing contents"),
    };
    match http::write_text_file(ctx, &project, rel, contents) {
        Ok(()) => http::ok_json(serde_json::json!({
            "ok": true,
            "projectId": project.id,
            "path": rel,
        })),
        Err(e) => http::err_json(e.status, e.message),
    }
}

fn handle_delete_file(ctx: &AppContext, project_id: &str, body: &str) -> ApiResponse {
    let project = match resolve_project(ctx, project_id) {
        Ok(p) => p,
        Err(e) => return http::err_json(404, e),
    };
    let req: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => return http::err_json(400, format!("Invalid JSON: {e}")),
    };
    let rel = match req.get("path").and_then(|v| v.as_str()) {
        Some(p) => p,
        None => return http::err_json(400, "Missing path"),
    };
    match http::delete_text_file(ctx, &project, rel) {
        Ok(()) => http::ok_json(serde_json::json!({
            "ok": true,
            "projectId": project.id,
            "path": rel,
        })),
        Err(e) => http::err_json(e.status, e.message),
    }
}

fn handle_search(ctx: &AppContext, project_id: &str, body: &str) -> ApiResponse {
    let project = match resolve_project(ctx, project_id) {
        Ok(p) => p,
        Err(e) => return http::err_json(404, e),
    };
    let req: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => return http::err_json(400, format!("Invalid JSON: {e}")),
    };
    let query = match req.get("query").and_then(|v| v.as_str()) {
        Some(q) => q,
        None => return http::err_json(400, "Missing query"),
    };
    let top_k = req
        .get("topK")
        .and_then(|v| v.as_u64())
        .unwrap_or(10)
        .clamp(1, 50) as usize;
    let include_content = req
        .get("includeContent")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    match http::search_project(ctx, &project, query, top_k, include_content) {
        Ok(value) => http::ok_json(value),
        Err(e) => http::err_json(500, e),
    }
}

fn handle_graph(ctx: &AppContext, project_id: &str, query: &str) -> ApiResponse {
    let project = match resolve_project(ctx, project_id) {
        Ok(p) => p,
        Err(e) => return http::err_json(404, e),
    };
    let params = parse_query(query);
    let q = params.get("q").map(|s| s.to_lowercase());
    let node_type = params.get("nodeType").map(|s| s.to_lowercase());
    let limit = params
        .get("limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(200)
        .clamp(1, 1000);
    match http::build_graph(ctx, &project, q, node_type, limit) {
        Ok(value) => http::ok_json(value),
        Err(err) => http::err_json(500, err),
    }
}

fn handle_events(ctx: &AppContext, query: &str) -> ApiResponse {
    let params = parse_query(query);
    let since = params
        .get("since")
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0);
    let wait_ms = params
        .get("wait")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    let events = if wait_ms == 0 {
        ctx.tasks.events_since(since)
    } else {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(wait_ms.min(25_000));
        let mut last = ctx.tasks.events_since(since);
        while last.is_empty() {
            if std::time::Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
            last = ctx.tasks.events_since(since);
        }
        last
    };
    let max_at = events.iter().map(|event| event.at).max().unwrap_or(since);
    http::ok_json(serde_json::json!({
        "ok": true,
        "events": events,
        "maxAt": max_at,
    }))
}
}

fn handle_rescan(ctx: &AppContext, project_id: &str, body: &str) -> ApiResponse {
    let project = match resolve_project(ctx, project_id) {
        Ok(p) => p,
        Err(e) => return http::err_json(404, e),
    };
    let trigger_ingest = if body.trim().is_empty() {
        false
    } else {
        serde_json::from_str::<serde_json::Value>(body)
            .ok()
            .and_then(|v| v.get("triggerIngest").and_then(|x| x.as_bool()))
            .unwrap_or(false)
    };
    let _ = trigger_ingest;
    let task = ctx.tasks.enqueue(
        &project,
        TaskKind::Rescan,
        "all",
        Some(serde_json::json!({ "triggerIngest": trigger_ingest })),
    );
    http::ok_json(serde_json::json!({
        "ok": true,
        "projectId": project.id,
        "task": task,
    }))
}

fn handle_chat(ctx: &AppContext, project_id: &str, body: &str) -> ApiResponse {
    let project = match resolve_project(ctx, project_id) {
        Ok(p) => p,
        Err(e) => return http::err_json(404, e),
    };
    let task = ctx.tasks.enqueue(
        &project,
        TaskKind::Chat,
        body,
        Some(serde_json::json!({ "stream": false })),
    );
    http::ok_json(serde_json::json!({
        "ok": true,
        "projectId": project.id,
        "taskId": task.id,
    }))
}

fn handle_cancel_chat(ctx: &AppContext, project_id: &str, session_id: &str) -> ApiResponse {
    let project = match resolve_project(ctx, project_id) {
        Ok(p) => p,
        Err(e) => return http::err_json(404, e),
    };
    let cancelled = ctx.tasks.cancel_session(&project.id, session_id);
    http::ok_json(serde_json::json!({
        "ok": true,
        "cancelled": cancelled,
        "sessionId": session_id,
    }))
}

fn handle_get_chat(ctx: &AppContext, project_id: &str, session_id: &str) -> ApiResponse {
    let project = match resolve_project(ctx, project_id) {
        Ok(p) => p,
        Err(e) => return http::err_json(404, e),
    };
    match ctx
        .app_state
        .read_chat_session(&project.path, session_id)
    {
        Ok(messages) => http::ok_json(serde_json::json!({
            "ok": true,
            "sessionId": session_id,
            "messages": messages,
        })),
        Err(e) => http::err_json(500, e),
    }
}

fn handle_tasks(ctx: &AppContext, query: &str) -> ApiResponse {
    let params = parse_query(query);
    let project_filter = params.get("projectId").cloned();
    let status_filter = params.get("status").cloned();
    let tasks = ctx.tasks.list(project_filter, status_filter);
    http::ok_json(serde_json::json!({ "ok": true, "tasks": tasks }))
}

fn handle_task(ctx: &AppContext, task_id: &str) -> ApiResponse {
    match ctx.tasks.get(task_id) {
        Some(task) => http::ok_json(serde_json::json!({ "ok": true, "task": task })),
        None => http::err_json(404, "Task not found"),
    }
}

fn handle_task_cancel(ctx: &AppContext, task_id: &str) -> ApiResponse {
    match ctx.tasks.cancel(task_id) {
        Some(cancelled) => http::ok_json(serde_json::json!({ "ok": true, "cancelled": cancelled })),
        None => http::err_json(404, "Task not found"),
    }
}

fn handle_task_enqueue(ctx: &AppContext, body: &str) -> ApiResponse {
    let req: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => return http::err_json(400, format!("Invalid JSON: {e}")),
    };
    let project_id = match req.get("projectId").and_then(|v| v.as_str()) {
        Some(p) => p,
        None => return http::err_json(400, "Missing projectId"),
    };
    let project = match resolve_project(ctx, project_id) {
        Ok(p) => p,
        Err(e) => return http::err_json(404, e),
    };
    let kind_str = req.get("kind").and_then(|v| v.as_str()).unwrap_or("ingest");
    let kind = match kind_str {
        "ingest" => TaskKind::Ingest,
        "rescan" => TaskKind::Rescan,
        "lint" => TaskKind::Lint,
        "sweep" => TaskKind::Sweep,
        "enrich" => TaskKind::Enrich,
        other => return http::err_json(400, format!("Unsupported task kind: {other}")),
    };
    let target = req
        .get("target")
        .and_then(|v| v.as_str())
        .unwrap_or("all")
        .to_string();
    let payload = req.get("payload").cloned();
    let task = ctx.tasks.enqueue(&project, kind, &target, payload);
    http::ok_json(serde_json::json!({ "ok": true, "task": task }))
}

fn handle_reviews(ctx: &AppContext, project_id: &str, query: &str) -> ApiResponse {
    let project = match resolve_project(ctx, project_id) {
        Ok(p) => p,
        Err(e) => return http::err_json(404, e),
    };
    let params = parse_query(query);
    let status = params
        .get("status")
        .map(|s| s.as_str())
        .unwrap_or("unresolved");
    let item_type = params.get("type").cloned();
    let limit = params
        .get("limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(DEFAULT_MAX_REVIEWS)
        .clamp(1, HARD_MAX_REVIEWS);
    match ctx
        .app_state
        .read_reviews(&project.path, status, item_type.as_deref(), limit)
    {
        Ok(reviews) => http::ok_json(serde_json::json!({
            "ok": true,
            "projectId": project.id,
            "status": status,
            "count": reviews.len(),
            "reviews": reviews
        })),
        Err(e) => http::err_json(500, e),
    }
}

fn handle_bulk_resolve_reviews(ctx: &AppContext, project_id: &str, body: &str) -> ApiResponse {
    let project = match resolve_project(ctx, project_id) {
        Ok(p) => p,
        Err(e) => return http::err_json(404, e),
    };
    let req: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => return http::err_json(400, format!("Invalid JSON: {e}")),
    };
    let ids = match req.get("ids").and_then(|v| v.as_array()) {
        Some(arr) => arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect::<Vec<_>>(),
        None => return http::err_json(400, "ids must be a non-empty array"),
    };
    if ids.is_empty() {
        return http::err_json(400, "ids must be a non-empty array");
    }
    let action = req
        .get("action")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    match ctx
        .app_state
        .resolve_review_items(&project.path, &ids, action.as_deref())
    {
        Ok((resolved, not_found)) => http::ok_json(serde_json::json!({
            "ok": true,
            "projectId": project.id,
            "resolved": resolved,
            "notFound": not_found,
            "count": resolved.len(),
        })),
        Err(e) => http::err_json(500, e),
    }
}

fn handle_patch_review(
    ctx: &AppContext,
    project_id: &str,
    review_id: &str,
    body: &str,
) -> ApiResponse {
    let project = match resolve_project(ctx, project_id) {
        Ok(p) => p,
        Err(e) => return http::err_json(404, e),
    };
    let mut resolved = true;
    let mut action: Option<String> = None;
    if !body.trim().is_empty() {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(body) {
            if let Some(v) = value.get("resolved").and_then(|x| x.as_bool()) {
                resolved = v;
            }
            if let Some(v) = value.get("action").and_then(|x| x.as_str()) {
                action = Some(v.to_string());
            }
        }
    }
    match ctx
        .app_state
        .patch_review_item(&project.path, review_id, resolved, action.as_deref())
    {
        Ok(true) => http::ok_json(serde_json::json!({
            "ok": true,
            "projectId": project.id,
            "reviewId": review_id,
            "resolved": resolved,
        })),
        Ok(false) => http::err_json(404, format!("Review item '{review_id}' not found")),
        Err(e) => http::err_json(500, e),
    }
}

pub(crate) fn resolve_project(ctx: &AppContext, project_id: &str) -> Result<ProjectEntry, String> {
    let decoded = percent_decode(project_id);
    let wants_current = decoded.eq_ignore_ascii_case("current");
    ctx.app_state
        .list_projects()
        .into_iter()
        .find(|p| {
            p.id == decoded
                || path_matches(&p.path, &decoded)
                || (wants_current && p.current)
        })
        .ok_or_else(|| format!("Unknown project: {decoded}"))
}

pub fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let max_len = left.len().max(right.len());
    let mut diff = left.len() ^ right.len();
    for i in 0..max_len {
        let a = left.get(i).copied().unwrap_or(0);
        let b = right.get(i).copied().unwrap_or(0);
        diff |= (a ^ b) as usize;
    }
    diff == 0
}

pub fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

pub fn path_matches(a: &str, b: &str) -> bool {
    let a = a.replace('\\', "/").trim_end_matches('/').to_string();
    let b = b.replace('\\', "/").trim_end_matches('/').to_string();
    if cfg!(windows) {
        a.eq_ignore_ascii_case(&b)
    } else {
        a == b
    }
}

pub fn split_url(url: &str) -> (String, &str) {
    match url.split_once('?') {
        Some((path, query)) => (path.to_string(), query),
        None => (url.to_string(), ""),
    }
}

pub fn parse_query(query: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for pair in query.split('&').filter(|s| !s.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        out.insert(percent_decode(k), percent_decode(v));
    }
    out
}

pub fn respond_error(
    request: tiny_http::Request,
    status: u16,
    message: &str,
    origin: Option<&str>,
) {
    let body = serde_json::json!({ "ok": false, "error": message }).to_string();
    respond(request, status, &body, origin, "application/json");
}

pub fn respond_options(request: tiny_http::Request, origin: Option<&str>) {
    let mut response = Response::empty(StatusCode(204));
    for header in cors_headers(origin, "Content-Type, Authorization, X-LLM-Wiki-Token") {
        response.add_header(header);
    }
    response.add_header(Header::from_bytes("Access-Control-Max-Age", "600").unwrap());
    let _ = request.respond(response);
}

pub fn respond(
    request: tiny_http::Request,
    status: u16,
    body: &str,
    origin: Option<&str>,
    content_type: &str,
) {
    let mut response = Response::from_string(body.to_string()).with_status_code(StatusCode(status));
    for header in cors_headers(origin, "Content-Type, Authorization, X-LLM-Wiki-Token") {
        response.add_header(header);
    }
    response.add_header(Header::from_bytes("Content-Type", content_type).unwrap());
    let _ = request.respond(response);
}

pub fn respond_bytes(
    request: tiny_http::Request,
    status: u16,
    content_type: String,
    data: Vec<u8>,
    extra_headers: Vec<(String, String)>,
    origin: Option<&str>,
) {
    let cursor = std::io::Cursor::new(data);
    let mut response = Response::new(StatusCode(status), Vec::new(), cursor, None, None);
    for header in cors_headers(origin, "Content-Type, Authorization, X-LLM-Wiki-Token") {
        response.add_header(header);
    }
    response.add_header(Header::from_bytes("Content-Type", content_type).unwrap());
    for (name, value) in extra_headers {
        if let Ok(header) = Header::from_bytes(name, value) {
            response.add_header(header);
        }
    }
    let _ = request.respond(response);
}

fn cors_headers(origin: Option<&str>, allow_headers: &str) -> Vec<Header> {
    local_cors_headers(origin, allow_headers)
}

fn handle_export_project(ctx: &AppContext, project_id: &str) -> ApiResponse {
    let project = match resolve_project(ctx, project_id) {
        Ok(p) => p,
        Err(e) => return http::err_json(404, e),
    };
    match http::export_project_zip(&project) {
        Ok(bytes) => {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let filename = format!(
                "Content-Disposition: attachment; filename=\"{}-{}.llmwiki.zip\"",
                sanitize_attachment(&project.name),
                timestamp
            );
            http::raw_response_with_filename(200, "application/zip", bytes, filename)
        }
        Err(err) => http::err_json(err.status, err.message),
    }
}

fn handle_import_archive(ctx: &AppContext, body: &[u8], content_type: Option<&str>) -> ApiResponse {
    let parsed = match multipart::parse_multipart_with_content_type(body, content_type) {
        Ok(parts) => parts,
        Err(err) => return http::err_json(400, format!("Invalid multipart body: {err}")),
    };
    let archive = match parsed.fields.get("archive") {
        Some(field) => field,
        None => return http::err_json(400, "Missing 'archive' field"),
    };
    let name = parsed
        .fields
        .get("name")
        .and_then(|f| std::str::from_utf8(&f.data).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| archive_filename_stem(&archive.filename));
    match http::import_project_zip(ctx, &name, &archive.data) {
        Ok(entry) => http::ok_json(serde_json::json!({
            "ok": true,
            "name": entry.name,
            "project": entry,
        })),
        Err(err) => http::err_json(err.status, err.message),
    }
}

fn sanitize_attachment(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn archive_filename_stem(filename: &str) -> String {
    let stem = Path::new(filename)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("imported-project");
    let trimmed = stem.trim();
    if trimmed.is_empty() {
        "imported-project".to_string()
    } else {
        trimmed.to_string()
    }
}
