use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::{Component, Path, PathBuf};

use serde::Serialize;
use serde_json::{json, Value};
use walkdir::WalkDir;
use zip::write::SimpleFileOptions;

use crate::web::app_state::ProjectEntry;
use crate::web::server::{AppContext, MAX_FILE_CONTENT_BYTES};
use crate::web::multipart;

#[derive(Debug, Clone)]
pub enum ApiBody {
    Json(Value),
    Raw {
        content_type: String,
        data: Vec<u8>,
        extra_headers: Vec<(String, String)>,
    },
}

#[derive(Debug, Clone)]
pub struct ApiResponse {
    pub status: u16,
    pub body: ApiBody,
}

pub fn ok_json(body: Value) -> ApiResponse {
    ApiResponse {
        status: 200,
        body: ApiBody::Json(body),
    }
}

pub fn err_json(status: u16, message: impl Into<String>) -> ApiResponse {
    ApiResponse {
        status,
        body: ApiBody::Json(json!({ "ok": false, "error": message.into() })),
    }
}

pub fn json_response(status: u16, body: Value) -> ApiResponse {
    ApiResponse {
        status,
        body: ApiBody::Json(body),
    }
}

pub fn raw_response(status: u16, content_type: impl Into<String>, data: Vec<u8>) -> ApiResponse {
    ApiResponse {
        status,
        body: ApiBody::Raw {
            content_type: content_type.into(),
            data,
            extra_headers: Vec::new(),
        },
    }
}

pub fn raw_response_with_filename(
    status: u16,
    content_type: impl Into<String>,
    data: Vec<u8>,
    disposition: String,
) -> ApiResponse {
    ApiResponse {
        status,
        body: ApiBody::Raw {
            content_type: content_type.into(),
            data,
            extra_headers: vec![("Content-Disposition".to_string(), disposition)],
        },
    }
}

#[derive(Debug, Clone)]
pub struct FileOpError {
    pub status: u16,
    pub message: String,
}

pub fn read_body(request: &mut tiny_http::Request, max_bytes: usize) -> Result<String, String> {
    let bytes = read_body_bytes(request, max_bytes)?;
    String::from_utf8(bytes).map_err(|_| "Request body must be UTF-8".to_string())
}

pub fn read_body_bytes(request: &mut tiny_http::Request, max_bytes: usize) -> Result<Vec<u8>, String> {
    let mut limited = request.as_reader().take(max_bytes as u64 + 1);
    let mut bytes = Vec::new();
    limited
        .read_to_end(&mut bytes)
        .map_err(|e| format!("Failed to read body: {e}"))?;
    if bytes.len() > max_bytes {
        return Err("Request body too large".to_string());
    }
    Ok(bytes)
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
        out.insert(crate::web::server::percent_decode(k), crate::web::server::percent_decode(v));
    }
    out
}

/// Resolve a project-relative path against the data root and return the
/// canonical absolute path, or an error if the resolved path escapes
/// the project directory.
pub fn safe_join(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let rel = rel.trim_start_matches('/');
    let rel_path = Path::new(rel);
    if rel_path.is_absolute() {
        return Err("Absolute paths are not allowed".to_string());
    }
    for component in rel_path.components() {
        if matches!(
            component,
            Component::ParentDir | Component::Prefix(_) | Component::RootDir
        ) {
            return Err("Path traversal is not allowed".to_string());
        }
    }
    let joined = root.join(rel_path);
    if joined.exists() {
        let joined_canon = joined
            .canonicalize()
            .map_err(|e| format!("Failed to resolve path: {e}"))?;
        let root_canon = root
            .canonicalize()
            .map_err(|e| format!("Failed to resolve project root: {e}"))?;
        if !joined_canon.starts_with(&root_canon) {
            return Err("Resolved path escapes the project directory".to_string());
        }
        return Ok(joined_canon);
    }
    let parent = joined
        .parent()
        .ok_or_else(|| "Path has no parent directory".to_string())?;
    if parent.exists() {
        let parent_canon = parent
            .canonicalize()
            .map_err(|e| format!("Failed to resolve parent path: {e}"))?;
        let root_canon = root
            .canonicalize()
            .map_err(|e| format!("Failed to resolve project root: {e}"))?;
        if !parent_canon.starts_with(&root_canon) {
            return Err("Resolved parent escapes the project directory".to_string());
        }
    }
    Ok(joined)
}

pub fn is_public_project_rel(rel: &str) -> bool {
    let rel = rel.replace('\\', "/").trim_start_matches('/').to_string();
    if rel
        .split('/')
        .any(|part| part.is_empty() || part.starts_with('.'))
    {
        return false;
    }
    let lower = rel.to_lowercase();
    lower == "purpose.md"
        || lower == "schema.md"
        || lower.starts_with("wiki/")
        || lower.starts_with("raw/sources/")
}

pub fn is_text_content_rel(rel: &str) -> bool {
    let rel = rel.to_lowercase();
    let ext = Path::new(&rel)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    matches!(
        ext,
        "md" | "mdx" | "txt" | "csv" | "json" | "yaml" | "yml" | "xml" | "html" | "htm" | "rtf" | "log"
    )
}

pub fn read_text_file(
    ctx: &AppContext,
    project: &ProjectEntry,
    rel: &str,
) -> Result<String, FileOpError> {
    if !is_public_project_rel(rel) {
        return Err(FileOpError {
            status: 403,
            message: "Path is not exposed by the local API".to_string(),
        });
    }
    if !is_text_content_rel(rel) {
        return Err(FileOpError {
            status: 415,
            message: "Only text-like project files can be read via this endpoint".to_string(),
        });
    }
    let project_root = PathBuf::from(&project.path);
    let path = match safe_join(&project_root, rel) {
        Ok(p) => p,
        Err(err) => {
            return Err(FileOpError {
                status: 400,
                message: err,
            })
        }
    };
    let meta = match fs::metadata(&path) {
        Ok(m) => m,
        Err(err) => {
            return Err(FileOpError {
                status: 404,
                message: format!("File not found: {err}"),
            })
        }
    };
    if meta.len() > MAX_FILE_CONTENT_BYTES {
        return Err(FileOpError {
            status: 413,
            message: "File is too large to return via API".to_string(),
        });
    }
    match fs::read_to_string(&path) {
        Ok(content) => Ok(content),
        Err(_) => Err(FileOpError {
            status: 415,
            message: "File is not valid UTF-8 text".to_string(),
        }),
    }
}

pub fn write_text_file(
    ctx: &AppContext,
    project: &ProjectEntry,
    rel: &str,
    contents: &str,
) -> Result<(), FileOpError> {
    if !is_public_project_rel(rel) {
        return Err(FileOpError {
            status: 403,
            message: "Path is not exposed by the local API".to_string(),
        });
    }
    let project_root = PathBuf::from(&project.path);
    let path = match safe_join(&project_root, rel) {
        Ok(p) => p,
        Err(err) => {
            return Err(FileOpError {
                status: 400,
                message: err,
            })
        }
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let tmp = path.with_extension("tmp-write");
    fs::write(&tmp, contents).map_err(|err| FileOpError {
        status: 500,
        message: format!("Failed to write file: {err}"),
    })?;
    if path.exists() {
        let _ = fs::remove_file(&path);
    }
    fs::rename(&tmp, &path).map_err(|err| FileOpError {
        status: 500,
        message: format!("Failed to rename temp file: {err}"),
    })?;
    let _ = ctx;
    Ok(())
}

pub fn delete_text_file(
    ctx: &AppContext,
    project: &ProjectEntry,
    rel: &str,
) -> Result<(), FileOpError> {
    if !is_public_project_rel(rel) {
        return Err(FileOpError {
            status: 403,
            message: "Path is not exposed by the local API".to_string(),
        });
    }
    let project_root = PathBuf::from(&project.path);
    let path = match safe_join(&project_root, rel) {
        Ok(p) => p,
        Err(err) => {
            return Err(FileOpError {
                status: 400,
                message: err,
            })
        }
    };
    if !path.exists() {
        return Err(FileOpError {
            status: 404,
            message: "File does not exist".to_string(),
        });
    }
    if path.is_dir() {
        return Err(FileOpError {
            status: 400,
            message: "Directory deletion is not supported via the local API".to_string(),
        });
    }
    fs::remove_file(&path).map_err(|err| FileOpError {
        status: 500,
        message: format!("Failed to delete file: {err}"),
    })?;
    let _ = ctx;
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiFileNode {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: Option<u64>,
    pub children: Option<Vec<ApiFileNode>>,
}

pub fn list_project_files(
    ctx: &AppContext,
    project: &ProjectEntry,
    root: &str,
    recursive: bool,
    max_files: usize,
) -> Result<Value, String> {
    let project_path = &project.path;
    let project_root = PathBuf::from(project_path);
    let rel = match root {
        "wiki" => "wiki",
        "sources" | "raw" | "raw/sources" => "raw/sources",
        "all" | "" => "",
        _ => return Err("root must be wiki, sources, or all".to_string()),
    };
    if rel.is_empty() {
        let mut count = 0usize;
        let mut roots = Vec::new();
        for prefix in ["purpose.md", "schema.md", "wiki", "raw/sources"] {
            let path = safe_join(&project_root, prefix)?;
            if !path.exists() {
                continue;
            }
            push_file_node(
                &project_root,
                &path,
                recursive,
                max_files,
                &mut count,
                &mut roots,
            )?;
        }
        let _ = ctx;
        return Ok(json!({
            "ok": true,
            "projectId": project.id,
            "root": "all",
            "files": roots,
            "truncated": false,
        }));
    }
    let dir = safe_join(&project_root, rel)?;
    let mut count = 0usize;
    let files = list_tree(&project_root, &dir, recursive, max_files, &mut count)?;
    Ok(json!({
        "ok": true,
        "projectId": project.id,
        "root": rel,
        "files": files,
        "truncated": false,
    }))
}

fn list_tree(
    project_root: &Path,
    path: &Path,
    recursive: bool,
    max_files: usize,
    count: &mut usize,
) -> Result<Vec<ApiFileNode>, String> {
    let mut out = Vec::new();
    for entry in fs::read_dir(path).map_err(|e| format!("Failed to list directory: {e}"))? {
        let entry = entry.map_err(|e| format!("Failed to read directory entry: {e}"))?;
        push_file_node(
            project_root,
            &entry.path(),
            recursive,
            max_files,
            count,
            &mut out,
        )?;
    }
    out.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
    Ok(out)
}

fn push_file_node(
    project_root: &Path,
    path: &Path,
    recursive: bool,
    max_files: usize,
    count: &mut usize,
    out: &mut Vec<ApiFileNode>,
) -> Result<(), String> {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    if name.starts_with('.') {
        return Ok(());
    }
    let meta = fs::symlink_metadata(path)
        .map_err(|e| format!("Failed to read metadata: {e}"))?;
    if meta.file_type().is_symlink() {
        return Ok(());
    }
    *count += 1;
    if *count > max_files {
        return Err(format!("File listing exceeds maxFiles limit ({max_files})"));
    }
    let is_dir = meta.file_type().is_dir();
    let children = if recursive && is_dir {
        Some(list_tree(project_root, path, recursive, max_files, count)?)
    } else {
        None
    };
    out.push(ApiFileNode {
        name,
        path: relative_to_project(project_root, path),
        is_dir,
        size: if is_dir { None } else { Some(meta.len()) },
        children,
    });
    Ok(())
}

fn relative_to_project(project_root: &Path, path: &Path) -> String {
    path.strip_prefix(project_root)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| path.to_string_lossy().replace('\\', "/"))
}

pub fn search_project(
    ctx: &AppContext,
    project: &ProjectEntry,
    query: &str,
    top_k: usize,
    include_content: bool,
) -> Result<Value, String> {
    let project_path = project.path.clone();
    let query = query.to_string();
    let embedding_config = ctx
        .app_state
        .read_app_state()
        .and_then(|value| value.get("embeddingConfig").cloned())
        .and_then(|value| {
            serde_json::from_value::<crate::commands::search::SearchEmbeddingConfig>(value).ok()
        })
        .filter(|cfg| cfg.enabled);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build();
    let result = match rt {
        Ok(rt) => {
            let project_path_clone = project_path.clone();
            let query_clone = query.clone();
            rt.block_on(async move {
                let query_embedding = if let Some(cfg) = embedding_config.clone() {
                    crate::commands::search::resolve_query_embedding(
                        &query_clone,
                        None,
                        Some(cfg),
                    )
                    .await
                    .ok()
                } else {
                    None
                };
                crate::commands::search::search_project_inner(
                    project_path,
                    query,
                    top_k,
                    include_content,
                    query_embedding,
                )
                .await
            })
        }
        Err(err) => return Err(format!("Failed to start async runtime: {err}")),
    };
    match result {
        Ok(search) => Ok(json!({
            "ok": true,
            "projectId": project.id,
            "mode": search.mode,
            "tokenHits": search.token_hits,
            "vectorHits": search.vector_hits,
            "graphHits": search.graph_hits,
            "results": search.results,
        })),
        Err(err) => Err(err),
    }
}

pub fn build_graph(
    _ctx: &AppContext,
    project: &ProjectEntry,
    q: Option<String>,
    node_type: Option<String>,
    limit: usize,
) -> Result<Value, String> {
    let project_path = &project.path;
    let wiki_root = Path::new(project_path).join("wiki");
    if !wiki_root.exists() {
        return Ok(json!({ "ok": true, "projectId": project.id, "nodes": [], "edges": [] }));
    }
    let mut raw: BTreeMap<String, (String, String, String, Vec<String>)> = BTreeMap::new();
    for entry in WalkDir::new(&wiki_root)
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file()
            || entry.path().extension().and_then(|s| s.to_str()) != Some("md")
        {
            continue;
        }
        let content = match fs::read_to_string(entry.path()) {
            Ok(content) => content,
            Err(_) => continue,
        };
        let id = entry
            .path()
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        if id.is_empty() {
            continue;
        }
        let title =
            crate::commands::search::extract_title(&content, entry.file_name().to_string_lossy().as_ref());
        let node_type = extract_type(&content);
        let path = relative_to_project(Path::new(project_path), entry.path());
        let links = extract_wikilinks(&content);
        raw.insert(id, (title, node_type, path, links));
    }
    let ids: std::collections::BTreeSet<String> = raw.keys().cloned().collect();
    let mut link_count: BTreeMap<String, usize> =
        raw.keys().map(|id| (id.clone(), 0)).collect();
    let mut seen = std::collections::BTreeSet::new();
    let mut edges = Vec::new();
    for (source, (_, _, _, links)) in &raw {
        for link in links {
            let Some(target) = resolve_link(link, &ids) else {
                continue;
            };
            if &target == source {
                continue;
            }
            let key = if source < &target {
                format!("{source}::{target}")
            } else {
                format!("{target}::{source}")
            };
            if seen.insert(key) {
                *link_count.entry(source.clone()).or_default() += 1;
                *link_count.entry(target.clone()).or_default() += 1;
                edges.push(json!({ "source": source, "target": target, "weight": 1.0 }));
            }
        }
    }
    let mut nodes: Vec<Value> = raw
        .into_iter()
        .filter(|(_, (_, nt, _, _))| nt != "query")
        .map(|(id, (label, nt, path, _))| {
            json!({
                "id": id,
                "label": label,
                "nodeType": nt,
                "path": path,
                "linkCount": *link_count.get(&id).unwrap_or(&0)
            })
        })
        .collect();
    if let Some(q) = &q {
        nodes.retain(|n| {
            let id = n.get("id").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
            let label = n.get("label").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
            id.contains(q) || label.contains(q)
        });
    }
    if let Some(node_type) = &node_type {
        nodes.retain(|n| n.get("nodeType").and_then(|v| v.as_str()) == Some(node_type.as_str()));
    }
    nodes.truncate(limit);
    let ids: std::collections::BTreeSet<String> = nodes
        .iter()
        .filter_map(|n| n.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();
    let edges: Vec<Value> = edges
        .into_iter()
        .filter(|e| {
            let s = e.get("source").and_then(|v| v.as_str()).unwrap_or("");
            let t = e.get("target").and_then(|v| v.as_str()).unwrap_or("");
            ids.contains(s) && ids.contains(t)
        })
        .collect();
    Ok(json!({
        "ok": true,
        "projectId": project.id,
        "nodes": nodes,
        "edges": edges
    }))
}

fn extract_type(content: &str) -> String {
    for line in content.lines() {
        if let Some(value) = line.trim().strip_prefix("type:") {
            return value
                .trim()
                .trim_matches('"')
                .trim_matches('\'')
                .to_lowercase();
        }
    }
    "other".to_string()
}

fn extract_wikilinks(content: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = content;
    while let Some(start) = rest.find("[[") {
        rest = &rest[start + 2..];
        let Some(end) = rest.find("]]") else { break };
        let inner = &rest[..end];
        let target = inner.split('|').next().unwrap_or("").trim();
        if !target.is_empty() {
            out.push(target.to_string());
        }
        rest = &rest[end + 2..];
    }
    out
}

fn resolve_link(raw: &str, ids: &std::collections::BTreeSet<String>) -> Option<String> {
    if ids.contains(raw) {
        return Some(raw.to_string());
    }
    let normalized = raw.to_lowercase().replace(' ', "-");
    ids.iter()
        .find(|id| id.to_lowercase() == normalized || id.to_lowercase() == raw.to_lowercase())
        .cloned()
}

pub fn serve_static(ctx: &AppContext, path: &str) -> Option<ApiResponse> {
    if path == "/" || path.is_empty() {
        return Some(serve_index(ctx));
    }
    if path.contains("..") {
        return Some(err_json(400, "Path traversal is not allowed"));
    }
    let dist = &ctx.config.dist_dir;
    if !dist.exists() {
        return Some(err_json(503, "Frontend assets are not built"));
    }
    let stripped = path.trim_start_matches('/');
    let candidate = dist.join(stripped);
    if candidate.is_file() {
        return Some(static_file_response(&candidate));
    }
    if path.starts_with("/assets/") || path.starts_with("/static/") {
        return Some(err_json(404, "Asset not found"));
    }
    Some(serve_index(ctx))
}

fn static_file_response(path: &Path) -> ApiResponse {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) => {
            return err_json(500, format!("Failed to read asset: {err}"));
        }
    };
    let mime = mime_from_path(path);
    raw_response(200, mime, bytes)
}

fn mime_from_path(path: &Path) -> String {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "svg" => "image/svg+xml",
        "json" => "application/json",
        "map" => "application/json",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
    .to_string()
}

fn serve_index(ctx: &AppContext) -> ApiResponse {
    let index = ctx.config.dist_dir.join("index.html");
    if !index.exists() {
        return err_json(503, "Frontend index.html not found; run `npm run build` first");
    }
    static_file_response(&index)
}

pub fn handle_upload(
    ctx: &AppContext,
    project_id: &str,
    body: &[u8],
    content_type: Option<&str>,
) -> ApiResponse {
    let project = match crate::web::server::resolve_project(ctx, project_id) {
        Ok(p) => p,
        Err(e) => return err_json(404, e),
    };
    let parsed = match multipart::parse_multipart_with_content_type(body, content_type) {
        Ok(parsed) => parsed,
        Err(err) => {
            return err_json(400, format!("Failed to parse multipart payload: {err}"))
        }
    };
    let subdir = parsed
        .fields
        .get("subdir")
        .and_then(|f| std::str::from_utf8(&f.data).ok().map(|s| s.to_string()))
        .unwrap_or_default();
    let mut saved = Vec::new();
    let mut skipped = Vec::new();
    let project_root = PathBuf::from(&project.path);
    for file in parsed.files {
        let name = file.filename.clone();
        if name.is_empty() {
            skipped.push(SkippedUpload {
                name: String::new(),
                reason: "Empty filename".to_string(),
            });
            continue;
        }
        if name.contains('/') || name.contains('\\') || name.starts_with('.') {
            skipped.push(SkippedUpload {
                name: name.clone(),
                reason: "Invalid filename".to_string(),
            });
            continue;
        }
        if file.data.len() as u64 > ctx.config.max_upload_bytes {
            skipped.push(SkippedUpload {
                name: name.clone(),
                reason: format!(
                    "File exceeds max upload size ({} bytes)",
                    ctx.config.max_upload_bytes
                ),
            });
            continue;
        }
        let rel_target = if subdir.is_empty() {
            format!("raw/sources/{name}")
        } else {
            format!("raw/sources/{}/{}", subdir.trim_matches('/'), name)
        };
        let target = match safe_join(&project_root, &rel_target) {
            Ok(p) => p,
            Err(err) => {
                skipped.push(SkippedUpload {
                    name: name.clone(),
                    reason: err,
                });
                continue;
            }
        };
        if let Some(parent) = target.parent() {
            if let Err(err) = fs::create_dir_all(parent) {
                skipped.push(SkippedUpload {
                    name: name.clone(),
                    reason: format!("Failed to create parent: {err}"),
                });
                continue;
            }
        }
        let tmp = target.with_extension("upload.tmp");
        if let Err(err) = fs::write(&tmp, &file.data) {
            skipped.push(SkippedUpload {
                name: name.clone(),
                reason: format!("Failed to write: {err}"),
            });
            continue;
        }
        if target.exists() {
            let _ = fs::remove_file(&target);
        }
        if let Err(err) = fs::rename(&tmp, &target) {
            skipped.push(SkippedUpload {
                name: name.clone(),
                reason: format!("Failed to move: {err}"),
            });
            continue;
        }
        saved.push(SavedUpload {
            name,
            path: rel_target,
            size: file.data.len() as u64,
        });
    }
    ctx.invalidate_app_state();
    ok_json(json!({
        "ok": true,
        "projectId": project.id,
        "saved": saved,
        "skipped": skipped,
    }))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SavedUpload {
    name: String,
    path: String,
    size: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SkippedUpload {
    name: String,
    reason: String,
}

pub fn export_project_zip(project: &ProjectEntry) -> Result<Vec<u8>, FileOpError> {
    let project_root = PathBuf::from(&project.path);
    if !project_root.is_dir() {
        return Err(FileOpError {
            status: 404,
            message: format!(
                "Project path '{}' is not a directory",
                project_root.display()
            ),
        });
    }
    let cursor = Cursor::new(Vec::<u8>::new());
    let mut writer = zip::ZipWriter::new(cursor);
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o644);
    let dir_options = options.clone().unix_permissions(0o755);
    let mut buffer = Vec::new();
    for entry in WalkDir::new(&project_root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| !is_skipped_archive_path(e.path(), &project_root))
        .filter_map(Result::ok)
    {
        let path = entry.path();
        let rel = match path.strip_prefix(&project_root) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        if rel_str.is_empty() {
            continue;
        }
        if entry.file_type().is_dir() {
            writer
                .add_directory(rel_str, dir_options)
                .map_err(|err| FileOpError {
                    status: 500,
                    message: format!("Failed to add archive directory: {err}"),
                })?;
            continue;
        }
        if !entry.file_type().is_file() {
            continue;
        }
        writer.start_file(rel_str, options).map_err(|err| FileOpError {
            status: 500,
            message: format!("Failed to start archive entry: {err}"),
        })?;
        buffer.clear();
        let mut file = match fs::File::open(path) {
            Ok(file) => file,
            Err(err) => {
                return Err(FileOpError {
                    status: 500,
                    message: format!("Failed to open '{}': {err}", path.display()),
                })
            }
        };
        if let Err(err) = file.read_to_end(&mut buffer) {
            return Err(FileOpError {
                status: 500,
                message: format!("Failed to read '{}': {err}", path.display()),
            });
        }
        if let Err(err) = writer.write_all(&buffer) {
            return Err(FileOpError {
                status: 500,
                message: format!("Failed to write archive entry: {err}"),
            });
        }
    }
    let cursor = writer
        .finish()
        .map_err(|err| FileOpError {
            status: 500,
            message: format!("Failed to finalize archive: {err}"),
        })?;
    Ok(cursor.into_inner())
}

fn is_skipped_archive_path(path: &Path, root: &Path) -> bool {
    let rel = match path.strip_prefix(root) {
        Ok(r) => r,
        Err(_) => return true,
    };
    rel.components().any(|component| {
        matches!(component, Component::Normal(part) if part == ".llm-wiki" || part == "lancedb")
    })
}

pub fn import_project_zip(
    ctx: &AppContext,
    project_name: &str,
    archive: &[u8],
) -> Result<ProjectEntry, FileOpError> {
    let trimmed = project_name.trim();
    if trimmed.is_empty() {
        return Err(FileOpError {
            status: 400,
            message: "Project name is required".to_string(),
        });
    }
    let cursor = Cursor::new(archive);
    let mut reader = match zip::ZipArchive::new(cursor) {
        Ok(reader) => reader,
        Err(err) => {
            return Err(FileOpError {
                status: 400,
                message: format!("Invalid zip archive: {err}"),
            })
        }
    };
    let safe_name = crate::web::app_state::sanitize_dirname_public(trimmed);
    let target_dir = ctx.app_state.projects_dir().join(&safe_name);
    if target_dir.exists() {
        return Err(FileOpError {
            status: 409,
            message: format!(
                "Project directory '{}' already exists",
                target_dir.display()
            ),
        });
    }
    fs::create_dir_all(&target_dir).map_err(|err| FileOpError {
        status: 500,
        message: format!("Failed to create project dir: {err}"),
    })?;
    let root_canon = match target_dir.canonicalize() {
        Ok(value) => value,
        Err(_) => target_dir.clone(),
    };
    for index in 0..reader.len() {
        let mut file = reader.by_index(index).map_err(|err| FileOpError {
            status: 400,
            message: format!("Failed to read zip entry: {err}"),
        })?;
        let entry_path = match file.enclosed_name() {
            Some(value) => value.to_path_buf(),
            None => continue,
        };
        let joined = match safe_join(&target_dir, &entry_path.to_string_lossy()) {
            Ok(value) => value,
            Err(err) => {
                return Err(FileOpError {
                    status: 400,
                    message: err,
                })
            }
        };
        let canonical = match joined.canonicalize() {
            Ok(value) => value,
            Err(_) => joined.clone(),
        };
        if !canonical.starts_with(&root_canon) {
            return Err(FileOpError {
                status: 400,
                message: format!("Entry escapes project root: {}", entry_path.display()),
            });
        }
        if file.is_dir() {
            fs::create_dir_all(&canonical).map_err(|err| FileOpError {
                status: 500,
                message: format!("Failed to create directory: {err}"),
            })?;
            continue;
        }
        if let Some(parent) = canonical.parent() {
            fs::create_dir_all(parent).map_err(|err| FileOpError {
                status: 500,
                message: format!("Failed to create directory: {err}"),
            })?;
        }
        let mut output = fs::File::create(&canonical).map_err(|err| FileOpError {
            status: 500,
            message: format!("Failed to create file: {err}"),
        })?;
        std::io::copy(&mut file, &mut output).map_err(|err| FileOpError {
            status: 500,
            message: format!("Failed to write file: {err}"),
        })?;
    }
    let project_id = crate::web::app_state::generate_id_public(&target_dir);
    ctx.app_state
        .register_project_public(&project_id, trimmed, &target_dir.to_string_lossy(), true)
        .map_err(|err| FileOpError {
            status: 500,
            message: format!("Failed to register project: {err}"),
        })
}
