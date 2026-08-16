//! Search core for the headless web server.
//!
//! The web binary is intentionally a single crate without the
//! Tauri command surface, so we don't import anything from
//! `commands::search`. Instead the small subset of helpers the
//! web backend actually uses (keyword walkdir + extract_title) is
//! inlined here.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchEmbeddingConfig {
    pub enabled: bool,
    pub endpoint: String,
    pub api_key: String,
    pub model: String,
    pub batch_size: Option<usize>,
    pub max_chars: Option<usize>,
    pub provider: Option<String>,
    pub extra_headers: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchImageRef {
    pub url: String,
    pub alt: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSearchResult {
    pub path: String,
    pub title: String,
    pub snippet: String,
    pub title_match: bool,
    pub score: f32,
    pub vector_score: Option<f32>,
    pub images: Vec<SearchImageRef>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSearchResponse {
    pub ok: bool,
    pub mode: String,
    pub results: Vec<ProjectSearchResult>,
    pub token_hits: usize,
    pub vector_hits: usize,
    pub graph_hits: Option<usize>,
}

pub fn extract_title(content: &str, file_name: &str) -> String {
    for line in content.lines() {
        let trimmed = line.trim_start_matches('\u{feff}').trim();
        if let Some(rest) = trimmed.strip_prefix("# ") {
            return rest.trim().to_string();
        }
        if let Some(rest) = trimmed.strip_prefix("#\t") {
            return rest.trim().to_string();
        }
    }
    file_name.trim_end_matches(".md").to_string()
}

pub async fn resolve_query_embedding(
    _query: &str,
    _explicit_embedding: Option<Vec<f32>>,
    _embedding_config: Option<SearchEmbeddingConfig>,
) -> Result<Option<Vec<f32>>, String> {
    Ok(None)
}

pub async fn search_project_inner(
    project_path: String,
    query: String,
    top_k: usize,
    include_content: bool,
    _query_embedding: Option<Vec<f32>>,
) -> Result<ProjectSearchResponse, String> {
    if query.trim().is_empty() {
        return Err("query is required".to_string());
    }
    let wiki_root = PathBuf::from(&project_path).join("wiki");
    let mut results: Vec<ProjectSearchResult> = Vec::new();
    let mut token_hits = 0usize;
    if !wiki_root.exists() {
        return Ok(ProjectSearchResponse {
            ok: true,
            mode: "keyword".to_string(),
            results,
            token_hits,
            vector_hits: 0,
            graph_hits: Some(0),
        });
    }
    let tokens: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|token| !token.is_empty() && token.len() >= 2)
        .map(|token| token.to_lowercase())
        .collect();
    if tokens.is_empty() {
        return Ok(ProjectSearchResponse {
            ok: true,
            mode: "keyword".to_string(),
            results,
            token_hits,
            vector_hits: 0,
            graph_hits: Some(0),
        });
    }
    for entry in WalkDir::new(&wiki_root)
        .max_depth(8)
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let ext = path.extension().and_then(|value| value.to_str());
        if !matches!(ext, Some("md") | Some("markdown")) {
            continue;
        }
        let content = match std::fs::read_to_string(path) {
            Ok(content) => content,
            Err(_) => continue,
        };
        let lower = content.to_lowercase();
        let mut score = 0usize;
        let mut title_match = false;
        let title_line = content.lines().find(|line| line.starts_with("# "));
        if let Some(title_line) = title_line {
            if title_line.to_lowercase().contains(&query.to_lowercase()) {
                title_match = true;
            }
        }
        for token in &tokens {
            score += count_occurrences(&lower, token);
        }
        if score == 0 && !title_match {
            continue;
        }
        token_hits += 1;
        let snippet = build_snippet(&content, &query);
        let rel = path
            .strip_prefix(&wiki_root)
            .map(|value| value.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| path.to_string_lossy().to_string());
        let title = extract_title(&content, path.file_name().unwrap_or_default().to_string_lossy().as_ref());
        results.push(ProjectSearchResult {
            path: rel,
            title,
            snippet: if include_content { content.chars().take(2000).collect() } else { snippet },
            title_match,
            score: score as f32,
            vector_score: None,
            images: Vec::new(),
        });
        if results.len() >= top_k {
            break;
        }
    }
    Ok(ProjectSearchResponse {
        ok: true,
        mode: "keyword".to_string(),
        results,
        token_hits,
        vector_hits: 0,
        graph_hits: Some(0),
    })
}

fn build_snippet(content: &str, query: &str) -> String {
    let lower_content = content.to_lowercase();
    let lower_query = query.to_lowercase();
    if let Some(idx) = lower_content.find(&lower_query) {
        let start = idx.saturating_sub(40);
        let end = (idx + lower_query.len() + 80).min(content.len());
        let mut snippet = content[start..end].to_string();
        if start > 0 {
            snippet = format!("\u{2026}{snippet}");
        }
        if end < content.len() {
            snippet.push('\u{2026}');
        }
        return snippet;
    }
    content.lines().take(3).collect::<Vec<_>>().join("\n")
}

fn count_occurrences(haystack: &str, needle: &str) -> usize {
    if needle.is_empty() {
        return 0;
    }
    haystack
        .as_bytes()
        .windows(needle.len())
        .filter(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
        .count()
}

#[allow(dead_code)]
fn _typecheck(_value: &Value, _path: &Path) {}