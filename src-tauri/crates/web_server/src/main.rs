use std::env;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

mod app_state;
mod cors;
mod http;
mod multipart;
mod search;
mod server;
mod tasks;
mod watcher;

use server::{run_server, BackendConfig};

#[derive(Debug, Clone, Deserialize)]
struct RawEnvConfig {
    #[serde(default)]
    bind_host: Option<String>,
    #[serde(default)]
    port: Option<u16>,
    #[serde(default)]
    data_dir: Option<String>,
    #[serde(default)]
    dist_dir: Option<String>,
    #[serde(default)]
    api_token: Option<String>,
    #[serde(default)]
    allow_unauthenticated: Option<bool>,
    #[serde(default)]
    allow_lan_access: Option<bool>,
    #[serde(default)]
    log_level: Option<String>,
    #[serde(default)]
    max_upload_bytes: Option<u64>,
    #[serde(default)]
    max_body_bytes: Option<usize>,
}

fn empty_env_config() -> RawEnvConfig {
    RawEnvConfig {
        bind_host: None,
        port: None,
        data_dir: None,
        dist_dir: None,
        api_token: None,
        allow_unauthenticated: None,
        allow_lan_access: None,
        log_level: None,
        max_upload_bytes: None,
        max_body_bytes: None,
    }
}

fn parse_env_file(path: &Path) -> RawEnvConfig {
    match std::fs::read_to_string(path) {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_else(|err| {
            eprintln!(
                "[llm-wiki-web] WARN: failed to parse config {}: {err}",
                path.display(),
            );
            empty_env_config()
        }),
        Err(_) => empty_env_config(),
    }
}

fn env_or(value: impl Into<String>, fallback: Option<String>) -> Option<String> {
    let key = value.into();
    match env::var(&key) {
        Ok(v) if !v.trim().is_empty() => Some(v),
        _ => fallback,
    }
}

fn env_bool(key: &str, fallback: bool) -> bool {
    env::var(key)
        .ok()
        .map(|v| matches_yes(&v))
        .unwrap_or(fallback)
}

fn matches_yes(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on" | "y" | "t"
    )
}

fn main() -> ExitCode {
    let raw = std::fs::read_to_string("app-state.json")
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
    let env_cfg = env::var("LLM_WIKI_CONFIG_FILE")
        .ok()
        .map(|p| parse_env_file(Path::new(&p)))
        .unwrap_or_else(empty_env_config);

    let lan_from_state = raw
        .as_ref()
        .and_then(|v| v.get("apiConfig"))
        .and_then(|c| c.get("allowLanAccess"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let lan_from_flag = env_bool("LLM_WIKI_ALLOW_LAN", env_cfg.allow_lan_access.unwrap_or(false));
    let lan = lan_from_state || lan_from_flag;

    let bind_host = env_or(
        "LLM_WIKI_BIND_HOST",
        env_cfg.bind_host.clone().or_else(|| {
            if lan {
                Some("0.0.0.0".to_string())
            } else {
                Some("127.0.0.1".to_string())
            }
        }),
    )
    .unwrap_or_else(|| "127.0.0.1".to_string());

    let port: u16 = env::var("LLM_WIKI_PORT")
        .ok()
        .or_else(|| env_cfg.port.map(|p| p.to_string()))
        .and_then(|s| s.parse().ok())
        .unwrap_or(8080);

    let data_dir: PathBuf = env_or(
        "LLM_WIKI_DATA_DIR",
        env_cfg.data_dir.clone(),
    )
    .map(PathBuf::from)
    .unwrap_or_else(|| PathBuf::from("data"));

    let dist_dir: PathBuf = env_or(
        "LLM_WIKI_DIST_DIR",
        env_cfg.dist_dir.clone(),
    )
    .map(PathBuf::from)
    .unwrap_or_else(|| PathBuf::from("dist"));

    let api_token = env_or("LLM_WIKI_API_TOKEN", env_cfg.api_token.clone());
    let allow_unauthenticated = env_bool(
        "LLM_WIKI_ALLOW_UNAUTHENTICATED",
        env_cfg.allow_unauthenticated.unwrap_or(false),
    );
    let log_level = env_or("LLM_WIKI_LOG", env_cfg.log_level.clone())
        .unwrap_or_else(|| "info".to_string());
    let max_upload_bytes = env::var("LLM_WIKI_MAX_UPLOAD_BYTES")
        .ok()
        .and_then(|s| s.parse().ok())
        .or(env_cfg.max_upload_bytes)
        .unwrap_or(1024 * 1024 * 1024);
    let max_body_bytes = env::var("LLM_WIKI_MAX_BODY_BYTES")
        .ok()
        .and_then(|s| s.parse().ok())
        .or(env_cfg.max_body_bytes)
        .unwrap_or(40 * 1024 * 1024);

    if let Err(err) = std::fs::create_dir_all(&data_dir) {
        eprintln!(
            "[llm-wiki-web] failed to prepare data dir {}: {err}",
            data_dir.display(),
        );
        return ExitCode::from(2);
    }

    eprintln!(
        "[llm-wiki-web] bind={bind_host}:{port} data_dir={} dist_dir={} log_level={log_level} token_configured={} allow_unauthenticated={allow_unauthenticated}",
        data_dir.display(),
        dist_dir.display(),
        api_token.is_some(),
    );

    let bind_addr: SocketAddr = match format!("{bind_host}:{port}").parse() {
        Ok(addr) => addr,
        Err(err) => {
            eprintln!("[llm-wiki-web] invalid bind address {bind_host}:{port}: {err}");
            return ExitCode::from(1);
        }
    };

    let config = BackendConfig {
        bind_addr,
        data_dir,
        dist_dir,
        api_token,
        allow_unauthenticated,
        allow_lan_access: lan,
        max_upload_bytes,
        max_body_bytes,
    };

    let handle = match run_server(config) {
        Ok(handle) => handle,
        Err(err) => {
            eprintln!("[llm-wiki-web] failed to start: {err}");
            return ExitCode::from(1);
        }
    };

    let running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    install_signal_handler(running.clone());
    while running.load(std::sync::atomic::Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(250));
    }
    drop(handle);
    ExitCode::SUCCESS
}

fn install_signal_handler(running: std::sync::Arc<std::sync::atomic::AtomicBool>) {
    let mut signals = match signal_hook::iterator::Signals::new([
        signal_hook::consts::SIGINT,
        signal_hook::consts::SIGTERM,
    ]) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("[llm-wiki-web] WARN: failed to register signal handlers: {err}");
            return;
        }
    };
    std::thread::Builder::new()
        .name("llm-wiki-web-signal".to_string())
        .spawn(move || {
            for sig in signals.forever() {
                eprintln!("[llm-wiki-web] received signal {}, shutting down", sig);
                running.store(false, std::sync::atomic::Ordering::SeqCst);
                break;
            }
        })
        .ok();
}

