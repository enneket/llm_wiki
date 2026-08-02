/**
 * File watcher for the headless web server. Watches each project's
 * `raw/sources` and `wiki` directories; on changes it enqueues a
 * `Rescan` task through the registry so the same work runs whether
 * the user clicked "rescan" in the UI or dropped a file in via a
 * bind mount.
 *
 * This replaces the Tauri `commands::file_sync` watcher in web mode
 * so ingest auto-runs even after the browser tab closes.
 */

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::web::app_state::{AppState, ProjectEntry};
use crate::web::tasks::{TaskKind, TaskRegistry};

pub struct ProjectWatcher {
    _watcher: RecommendedWatcher,
}

impl ProjectWatcher {
    pub fn start(
        registry: Arc<TaskRegistry>,
        app_state: AppState,
    ) -> notify::Result<Self> {
        let callback_state = app_state.clone();
        let callback_registry = registry.clone();
        let debounce = Arc::new(std::sync::Mutex::new(HashMap::<
            PathBuf,
            std::time::Instant,
        >::new()));
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
            let event = match res {
                Ok(event) => event,
                Err(_) => return,
            };
            if !matches!(
                event.kind,
                EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
            ) {
                return;
            }
            for path in event.paths {
                if !should_handle(&path) {
                    continue;
                }
                let project = match find_project_for_path(&callback_state, &path) {
                    Some(project) => project,
                    None => continue,
                };
                if should_debounce(&debounce, &path) {
                    continue;
                }
                callback_registry.enqueue(
                    &project,
                    TaskKind::Rescan,
                    "all",
                    Some(serde_json::json!({
                        "triggerIngest": true,
                        "reason": "file_watcher",
                        "path": path.to_string_lossy(),
                    })),
                );
            }
        })?;
        for project in app_state.list_projects() {
            watch_project(&mut watcher, &project.path);
        }
        Ok(Self { _watcher: watcher })
    }
}

fn watch_project(watcher: &mut RecommendedWatcher, project_path: &str) {
    let root = PathBuf::from(project_path);
    for rel in ["raw/sources", "wiki"] {
        let path = root.join(rel);
        if path.exists() {
            let _ = watcher.watch(&path, RecursiveMode::Recursive);
        }
    }
}

fn should_handle(path: &Path) -> bool {
    let lower = path.to_string_lossy().to_lowercase();
    if lower.contains(".llm-wiki") || lower.contains("lancedb") {
        return false;
    }
    let ext = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_lowercase();
    matches!(
        ext.as_str(),
        "md" | "markdown" | "txt" | "pdf" | "docx" | "doc" | "pptx" | "xlsx" | "html" | "htm" | "epub"
    )
}

fn find_project_for_path(app_state: &AppState, path: &Path) -> Option<ProjectEntry> {
    let normalized = path
        .to_string_lossy()
        .trim_end_matches('/')
        .to_lowercase();
    for project in app_state.list_projects() {
        let root = project.path.to_lowercase();
        if normalized.starts_with(&root) {
            return Some(project);
        }
    }
    None
}

fn should_debounce(
    debounce: &Arc<std::sync::Mutex<HashMap<PathBuf, std::time::Instant>>>,
    path: &Path,
) -> bool {
    let now = std::time::Instant::now();
    let mut guard = match debounce.lock() {
        Ok(value) => value,
        Err(_) => return false,
    };
    if let Some(last) = guard.get(path) {
        if now.duration_since(*last) < Duration::from_millis(750) {
            return true;
        }
    }
    guard.insert(path.to_path_buf(), now);
    false
}
