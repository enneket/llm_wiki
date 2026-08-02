use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const APP_STATE_FILE: &str = "app-state.json";
const PROJECTS_DIR: &str = "projects";
const DEFAULT_PROJECT_NAME: &str = "default";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectEntry {
    pub id: String,
    pub name: String,
    pub path: String,
    pub current: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AppStateFile {
    #[serde(default)]
    pub project_registry: BTreeMap<String, ProjectEntry>,
    #[serde(default)]
    pub recent_projects: Vec<ProjectEntry>,
    #[serde(default)]
    pub current_project: String,
    #[serde(default)]
    pub llm_config: Option<serde_json::Value>,
    #[serde(default)]
    pub embedding_config: Option<serde_json::Value>,
    #[serde(default)]
    pub search_api_config: Option<serde_json::Value>,
    #[serde(default)]
    pub api_config: Option<serde_json::Value>,
    #[serde(default)]
    pub custom_llm_presets: Option<serde_json::Value>,
    #[serde(default)]
    pub provider_configs: Option<serde_json::Value>,
    #[serde(default)]
    pub project_llm_overrides: Option<serde_json::Value>,
}

#[derive(Clone)]
pub struct AppState {
    data_dir: PathBuf,
    state_path: PathBuf,
    lock: std::sync::Arc<std::sync::Mutex<()>>,
}

impl AppState {
    pub fn open(data_dir: &Path) -> std::io::Result<Self> {
        fs::create_dir_all(data_dir)?;
        fs::create_dir_all(data_dir.join(PROJECTS_DIR))?;
        let state_path = data_dir.join(APP_STATE_FILE);
        let state = Self {
            data_dir: data_dir.to_path_buf(),
            state_path,
            lock: std::sync::Arc::new(std::sync::Mutex::new(())),
        };
        if !state.state_path.exists() {
            state.bootstrap_default_project()?;
            state.write_state(&AppStateFile::default())?;
        }
        Ok(state)
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn projects_dir(&self) -> PathBuf {
        self.data_dir.join(PROJECTS_DIR)
    }

    pub fn read_app_state(&self) -> Option<serde_json::Value> {
        let raw = match fs::read_to_string(&self.state_path) {
            Ok(raw) => raw,
            Err(_) => return None,
        };
        serde_json::from_str(&raw).ok()
    }

    pub fn load_state(&self) -> AppStateFile {
        let raw = match fs::read_to_string(&self.state_path) {
            Ok(raw) => raw,
            Err(_) => return AppStateFile::default(),
        };
        serde_json::from_str(&raw).unwrap_or_default()
    }

    pub fn write_state(&self, state: &AppStateFile) -> std::io::Result<()> {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = self.state_path.with_extension("json.tmp");
        let serialized = serde_json::to_string_pretty(state)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let mut file = fs::File::create(&tmp)?;
        file.write_all(serialized.as_bytes())?;
        file.sync_all()?;
        if self.state_path.exists() {
            fs::remove_file(&self.state_path)?;
        }
        fs::rename(&tmp, &self.state_path)?;
        Ok(())
    }

    pub fn list_projects(&self) -> Vec<ProjectEntry> {
        let state = self.load_state();
        let mut by_path: BTreeMap<String, ProjectEntry> = BTreeMap::new();
        for (_, project) in state.project_registry {
            by_path.insert(project.path.clone(), project);
        }
        for project in state.recent_projects {
            by_path.entry(project.path.clone()).or_insert(project);
        }
        let current = state.current_project;
        by_path.values_mut().for_each(|p| {
            p.current = !current.is_empty() && p.id == current;
        });
        by_path.into_values().collect()
    }

    pub fn register_project(
        &self,
        id: &str,
        name: &str,
        path: &str,
        make_current: bool,
    ) -> std::io::Result<ProjectEntry> {
        let mut state = self.load_state();
        let entry = ProjectEntry {
            id: id.to_string(),
            name: name.to_string(),
            path: path.to_string(),
            current: make_current,
        };
        state
            .project_registry
            .insert(id.to_string(), entry.clone());
        state
            .recent_projects
            .retain(|p| p.id != id && p.path != path);
        state.recent_projects.insert(0, entry.clone());
        if make_current {
            state.current_project = id.to_string();
        }
        self.write_state(&state)?;
        Ok(entry)
    }

    pub fn register_project_public(
        &self,
        id: &str,
        name: &str,
        path: &str,
        make_current: bool,
    ) -> std::io::Result<ProjectEntry> {
        self.register_project(id, name, path, make_current)
    }

    pub fn set_current_project(&self, id: &str) -> std::io::Result<()> {
        let mut state = self.load_state();
        state.current_project = id.to_string();
        self.write_state(&state)
    }

    pub fn create_project(
        &self,
        name: &str,
        project_id: Option<&str>,
    ) -> std::io::Result<ProjectEntry> {
        let safe_name = sanitize_dirname(name);
        let project_dir = self.projects_dir().join(&safe_name);
        if project_dir.exists() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!(
                    "Project directory '{}' already exists",
                    project_dir.display()
                ),
            ));
        }
        fs::create_dir_all(&project_dir)?;
        scaffold_wiki_project(&project_dir, name)?;
        let id = project_id
            .map(|s| s.to_string())
            .unwrap_or_else(|| generate_id(&project_dir));
        self.register_project(&id, name, &project_dir.to_string_lossy(), true)
    }

    pub fn find_project_by_path(&self, path: &str) -> Option<ProjectEntry> {
        let normalized = normalize_project_path(path);
        self.list_projects().into_iter().find(|entry| {
            normalize_project_path(&entry.path) == normalized
        })
    }

    pub fn read_reviews(
        &self,
        project_path: &str,
        status: &str,
        item_type: Option<&str>,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>, String> {
        let path = Path::new(project_path).join(".llm-wiki/review.json");
        let raw = match fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(format!("Failed to read review state: {err}")),
        };
        let parsed: serde_json::Value = serde_json::from_str(&raw)
            .map_err(|err| format!("Invalid review state JSON: {err}"))?;
        let items = parsed
            .as_array()
            .ok_or_else(|| "Invalid review state JSON: expected an array".to_string())?;
        let mut reviews = Vec::new();
        for item in items {
            let resolved = item
                .get("resolved")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let include = match status {
                "unresolved" => !resolved,
                "resolved" => resolved,
                "all" => true,
                _ => true,
            };
            if !include {
                continue;
            }
            if let Some(t) = item_type {
                if item.get("type").and_then(|v| v.as_str()) != Some(t) {
                    continue;
                }
            }
            reviews.push(item.clone());
            if reviews.len() >= limit {
                break;
            }
        }
        Ok(reviews)
    }

    pub fn patch_review_item(
        &self,
        project_path: &str,
        review_id: &str,
        resolved: bool,
        action: Option<&str>,
    ) -> Result<bool, String> {
        let path = Path::new(project_path).join(".llm-wiki/review.json");
        let mut parsed = match read_json_array(&path)? {
            Some(value) => value,
            None => return Ok(false),
        };
        let items = parsed
            .as_array_mut()
            .ok_or_else(|| "Invalid review state JSON: expected an array".to_string())?;
        let mut found = false;
        for item in items.iter_mut() {
            let id_matches = item
                .get("id")
                .and_then(|v| v.as_str())
                .map(|s| s == review_id)
                .unwrap_or(false);
            if !id_matches {
                continue;
            }
            if let Some(obj) = item.as_object_mut() {
                obj.insert("resolved".to_string(), serde_json::Value::Bool(resolved));
                if resolved {
                    if let Some(action) = action {
                        obj.insert(
                            "resolvedAction".to_string(),
                            serde_json::Value::String(action.to_string()),
                        );
                    }
                } else {
                    obj.remove("resolvedAction");
                }
            }
            found = true;
        }
        if !found {
            return Ok(false);
        }
        write_json_array(&path, &parsed)?;
        Ok(true)
    }

    pub fn resolve_review_items(
        &self,
        project_path: &str,
        ids: &[String],
        action: Option<&str>,
    ) -> Result<(Vec<String>, Vec<String>), String> {
        let path = Path::new(project_path).join(".llm-wiki/review.json");
        let mut parsed = match read_json_array(&path)? {
            Some(value) => value,
            None => return Ok((Vec::new(), ids.to_vec())),
        };
        let items = parsed
            .as_array_mut()
            .ok_or_else(|| "Invalid review state JSON: expected an array".to_string())?;
        let mut found: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for item in items.iter_mut() {
            let id = item.get("id").and_then(|v| v.as_str()).map(|s| s.to_string());
            let Some(id) = id else { continue };
            if ids.iter().any(|want| want == &id) {
                if let Some(obj) = item.as_object_mut() {
                    obj.insert("resolved".to_string(), serde_json::Value::Bool(true));
                    if let Some(action) = action {
                        obj.insert(
                            "resolvedAction".to_string(),
                            serde_json::Value::String(action.to_string()),
                        );
                    }
                }
                found.insert(id);
            }
        }
        if !found.is_empty() {
            write_json_array(&path, &parsed)?;
        }
        let resolved: Vec<String> = ids
            .iter()
            .filter(|id| found.contains(*id))
            .cloned()
            .collect();
        let not_found: Vec<String> = ids
            .iter()
            .filter(|id| !found.contains(*id))
            .cloned()
            .collect();
        Ok((resolved, not_found))
    }

    pub fn read_chat_session(
        &self,
        project_path: &str,
        session_id: &str,
    ) -> Result<Vec<serde_json::Value>, String> {
        let path = Path::new(project_path)
            .join(".llm-wiki")
            .join("chats")
            .join(format!("{session_id}.json"));
        let raw = match fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(format!("Failed to read chat session: {err}")),
        };
        let parsed: serde_json::Value = serde_json::from_str(&raw)
            .map_err(|err| format!("Invalid chat session JSON: {err}"))?;
        let messages = parsed
            .as_array()
            .cloned()
            .or_else(|| {
                parsed
                    .get("messages")
                    .and_then(|m| m.as_array())
                    .cloned()
            })
            .unwrap_or_default();
        Ok(messages)
    }

    pub fn append_chat_message(
        &self,
        project_path: &str,
        session_id: &str,
        role: &str,
        content: &str,
    ) -> std::io::Result<()> {
        let dir = Path::new(project_path).join(".llm-wiki").join("chats");
        fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{session_id}.json"));
        let mut messages = match fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        {
            Some(value) => match value {
                serde_json::Value::Array(arr) => arr,
                other => {
                    if let Some(arr) = other.get("messages").and_then(|m| m.as_array()) {
                        arr.clone()
                    } else {
                        Vec::new()
                    }
                }
            },
            None => Vec::new(),
        };
        messages.push(serde_json::json!({
            "role": role,
            "content": content,
            "timestamp": chrono::Utc::now().timestamp_millis(),
        }));
        let serialized = serde_json::to_string_pretty(&messages)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, serialized)?;
        fs::rename(&tmp, &path)?;
        Ok(())
    }

    fn bootstrap_default_project(&self) -> std::io::Result<()> {
        let project_dir = self.projects_dir().join(DEFAULT_PROJECT_NAME);
        fs::create_dir_all(&project_dir)?;
        scaffold_wiki_project(&project_dir, DEFAULT_PROJECT_NAME)?;
        Ok(())
    }
}

fn read_json_array(path: &Path) -> Result<Option<serde_json::Value>, String> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(format!("Failed to read '{}': {err}", path.display())),
    };
    let parsed: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|err| format!("Invalid JSON in '{}': {err}", path.display()))?;
    Ok(Some(parsed))
}

fn write_json_array(path: &Path, value: &serde_json::Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create '{}': {err}", parent.display()))?;
    }
    let serialized = serde_json::to_string_pretty(value)
        .map_err(|err| format!("Failed to serialize: {err}"))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serialized)
        .map_err(|err| format!("Failed to write '{}': {err}", tmp.display()))?;
    fs::rename(&tmp, path)
        .map_err(|err| format!("Failed to rename tmp: {err}"))?;
    Ok(())
}

fn sanitize_dirname(name: &str) -> String {
    let cleaned = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim()
        .to_string();
    if cleaned.is_empty() {
        "untitled".to_string()
    } else {
        cleaned
    }
}

pub fn sanitize_dirname_public(name: &str) -> String {
    sanitize_dirname(name)
}

fn normalize_project_path(path: &str) -> String {
    path.replace('\\', "/")
        .trim_end_matches('/')
        .to_ascii_lowercase()
}

fn generate_id(dir: &Path) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut buf = [0u8; 16];
    if let Ok(mut file) = fs::File::open(dir) {
        let _ = file.read(&mut buf);
    }
    let hex = nanos
        .to_be_bytes()
        .iter()
        .chain(buf.iter())
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    format!("project-{hex}")
}

pub fn generate_id_public(dir: &Path) -> String {
    generate_id(dir)
}

fn scaffold_wiki_project(root: &Path, name: &str) -> std::io::Result<()> {
    let dirs = [
        "raw/sources",
        "raw/assets",
        "wiki/entities",
        "wiki/concepts",
        "wiki/sources",
        "wiki/queries",
        "wiki/comparisons",
        "wiki/synthesis",
    ];
    for d in dirs {
        fs::create_dir_all(root.join(d))?;
    }
    fs::write(
        root.join("schema.md"),
        WikiTemplates::schema_md(),
    )?;
    fs::write(
        root.join("purpose.md"),
        WikiTemplates::purpose_md(),
    )?;
    fs::write(
        root.join("wiki/index.md"),
        WikiTemplates::index_md(),
    )?;
    fs::write(
        root.join("wiki/log.md"),
        WikiTemplates::log_md(name),
    )?;
    fs::write(
        root.join("wiki/overview.md"),
        WikiTemplates::overview_md(),
    )?;
    fs::create_dir_all(root.join(".obsidian"))?;
    fs::write(
        root.join(".obsidian/app.json"),
        WikiTemplates::obsidian_app_json(),
    )?;
    fs::write(
        root.join(".obsidian/appearance.json"),
        WikiTemplates::obsidian_appearance_json(),
    )?;
    fs::write(
        root.join(".obsidian/core-plugins.json"),
        WikiTemplates::obsidian_core_plugins_json(),
    )?;
    Ok(())
}

struct WikiTemplates;

impl WikiTemplates {
    fn schema_md() -> String {
        include_str!("../../../src-tauri/src/web/templates/schema.md").to_string()
    }
    fn purpose_md() -> String {
        include_str!("../../../src-tauri/src/web/templates/purpose.md").to_string()
    }
    fn index_md() -> String {
        include_str!("../../../src-tauri/src/web/templates/wiki_index.md").to_string()
    }
    fn log_md(name: &str) -> String {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        format!("# Research Log\n\n## {today}\n\n- Project `{name}` created\n")
    }
    fn overview_md() -> String {
        include_str!("../../../src-tauri/src/web/templates/wiki_overview.md").to_string()
    }
    fn obsidian_app_json() -> String {
        include_str!("../../../src-tauri/src/web/templates/obsidian_app.json").to_string()
    }
    fn obsidian_appearance_json() -> String {
        include_str!("../../../src-tauri/src/web/templates/obsidian_appearance.json").to_string()
    }
    fn obsidian_core_plugins_json() -> String {
        include_str!("../../../src-tauri/src/web/templates/obsidian_core_plugins.json")
            .to_string()
    }
}
