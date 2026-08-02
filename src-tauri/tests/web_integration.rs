#![cfg(feature = "web")]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::Duration;

use llm_wiki_lib::web::{run_server, BackendConfig};

fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral");
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

fn wait_for_server(addr: &str, timeout: Duration) {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if let Ok(mut stream) = TcpStream::connect(addr) {
            let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
            let _ = stream.set_write_timeout(Some(Duration::from_millis(200)));
            let _ = stream.write_all(b"GET /health HTTP/1.0\r\nHost: localhost\r\n\r\n");
            let mut buf = [0u8; 1024];
            if stream.read(&mut buf).is_ok() {
                return;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("Server did not start in {timeout:?}");
}

fn temp_data_dir(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("llm-wiki-web-test-{label}-{nanos}"));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn http_get(addr: &str, path: &str, token: Option<&str>) -> String {
    let mut stream = TcpStream::connect(addr).expect("connect");
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let token_header = token
        .map(|t| format!("X-LLM-Wiki-Token: {t}\r\n"))
        .unwrap_or_default();
    let request = format!(
        "GET {path} HTTP/1.0\r\nHost: localhost\r\n{token_header}Connection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).unwrap();
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).unwrap();
    String::from_utf8_lossy(&buf).into_owned()
}

fn http_post_json(addr: &str, path: &str, body: &str, token: Option<&str>) -> String {
    let mut stream = TcpStream::connect(addr).expect("connect");
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let token_header = token
        .map(|t| format!("X-LLM-Wiki-Token: {t}\r\n"))
        .unwrap_or_default();
    let request = format!(
        "POST {path} HTTP/1.0\r\nHost: localhost\r\n{token_header}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).unwrap();
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).unwrap();
    String::from_utf8_lossy(&buf).into_owned()
}

fn http_post_multipart(
    addr: &str,
    path: &str,
    files: &[(&str, &[u8])],
    token: Option<&str>,
) -> String {
    let boundary = "----testboundary";
    let mut body = Vec::new();
    for (name, data) in files {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"files\"; filename=\"{name}\"\r\n").as_bytes(),
        );
        body.extend_from_slice(b"Content-Type: application/octet-stream\r\n\r\n");
        body.extend_from_slice(data);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    let mut stream = TcpStream::connect(addr).expect("connect");
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let token_header = token
        .map(|t| format!("X-LLM-Wiki-Token: {t}\r\n"))
        .unwrap_or_default();
    let request = format!(
        "POST {path} HTTP/1.0\r\nHost: localhost\r\n{token_header}Content-Type: multipart/form-data; boundary={boundary}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let mut head = request.into_bytes();
    head.extend_from_slice(&body);
    stream.write_all(&head).unwrap();
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).unwrap();
    String::from_utf8_lossy(&buf).into_owned()
}

#[test]
fn health_endpoint_returns_running() {
    let port = free_port();
    let data_dir = temp_data_dir("health");
    let dist_dir = data_dir.join("dist");
    std::fs::create_dir_all(&dist_dir).unwrap();
    std::fs::write(dist_dir.join("index.html"), "<html></html>").unwrap();
    let config = BackendConfig {
        bind_addr: ([127, 0, 0, 1], port).into(),
        data_dir: data_dir.clone(),
        dist_dir,
        api_token: None,
        allow_unauthenticated: true,
        allow_lan_access: false,
        max_upload_bytes: 1024 * 1024,
        max_body_bytes: 1024 * 1024,
    };
    let handle = run_server(config).expect("server start");
    let addr = format!("127.0.0.1:{port}");
    wait_for_server(&addr, Duration::from_secs(2));
    let response = http_get(&addr, "/api/v1/health", None);
    assert!(response.starts_with("HTTP/1.0 200"), "status: {response}");
    assert!(response.contains("\"status\":\"running\""));
    drop(handle);
    std::fs::remove_dir_all(data_dir).ok();
}

#[test]
fn projects_endpoint_lists_default_project() {
    let port = free_port();
    let data_dir = temp_data_dir("projects");
    let dist_dir = data_dir.join("dist");
    std::fs::create_dir_all(&dist_dir).unwrap();
    let config = BackendConfig {
        bind_addr: ([127, 0, 0, 1], port).into(),
        data_dir: data_dir.clone(),
        dist_dir,
        api_token: None,
        allow_unauthenticated: true,
        allow_lan_access: false,
        max_upload_bytes: 1024 * 1024,
        max_body_bytes: 1024 * 1024,
    };
    let handle = run_server(config).expect("server start");
    let addr = format!("127.0.0.1:{port}");
    wait_for_server(&addr, Duration::from_secs(2));
    let response = http_get(&addr, "/api/v1/projects", None);
    assert!(response.starts_with("HTTP/1.0 200"));
    assert!(response.contains("\"projects\""));
    assert!(response.contains("\"name\":\"default\""));
    drop(handle);
    std::fs::remove_dir_all(data_dir).ok();
}

#[test]
fn token_auth_blocks_unauthorized_access() {
    let port = free_port();
    let data_dir = temp_data_dir("auth");
    let dist_dir = data_dir.join("dist");
    std::fs::create_dir_all(&dist_dir).unwrap();
    let config = BackendConfig {
        bind_addr: ([127, 0, 0, 1], port).into(),
        data_dir: data_dir.clone(),
        dist_dir,
        api_token: Some("secret-token".to_string()),
        allow_unauthenticated: false,
        allow_lan_access: false,
        max_upload_bytes: 1024 * 1024,
        max_body_bytes: 1024 * 1024,
    };
    let handle = run_server(config).expect("server start");
    let addr = format!("127.0.0.1:{port}");
    wait_for_server(&addr, Duration::from_secs(2));
    let unauthorized = http_get(&addr, "/api/v1/projects", None);
    assert!(unauthorized.starts_with("HTTP/1.0 401"));
    let authorized = http_get(&addr, "/api/v1/projects", Some("secret-token"));
    assert!(authorized.starts_with("HTTP/1.0 200"));
    drop(handle);
    std::fs::remove_dir_all(data_dir).ok();
}

#[test]
fn file_upload_persists_into_project() {
    let port = free_port();
    let data_dir = temp_data_dir("upload");
    let dist_dir = data_dir.join("dist");
    std::fs::create_dir_all(&dist_dir).unwrap();
    let config = BackendConfig {
        bind_addr: ([127, 0, 0, 1], port).into(),
        data_dir: data_dir.clone(),
        dist_dir,
        api_token: None,
        allow_unauthenticated: true,
        allow_lan_access: false,
        max_upload_bytes: 1024 * 1024,
        max_body_bytes: 1024 * 1024,
    };
    let handle = run_server(config).expect("server start");
    let addr = format!("127.0.0.1:{port}");
    wait_for_server(&addr, Duration::from_secs(2));
    let project_id = "default";
    let response = http_post_multipart(
        &addr,
        &format!("/api/v1/projects/{project_id}/uploads"),
        &[("hello.txt", b"hello world from a test")],
        None,
    );
    assert!(response.starts_with("HTTP/1.0 200"), "{response}");
    assert!(response.contains("\"saved\""));
    assert!(response.contains("hello.txt"));
    let saved_path = data_dir.join("projects/default/raw/sources/hello.txt");
    let saved = std::fs::read(&saved_path).expect("file was saved");
    assert_eq!(saved, b"hello world from a test");
    drop(handle);
    std::fs::remove_dir_all(data_dir).ok();
}

#[test]
fn background_task_survives_after_enqueue() {
    let port = free_port();
    let data_dir = temp_data_dir("tasks");
    let dist_dir = data_dir.join("dist");
    std::fs::create_dir_all(&dist_dir).unwrap();
    let config = BackendConfig {
        bind_addr: ([127, 0, 0, 1], port).into(),
        data_dir: data_dir.clone(),
        dist_dir,
        api_token: None,
        allow_unauthenticated: true,
        allow_lan_access: false,
        max_upload_bytes: 1024 * 1024,
        max_body_bytes: 1024 * 1024,
    };
    let handle = run_server(config).expect("server start");
    let addr = format!("127.0.0.1:{port}");
    wait_for_server(&addr, Duration::from_secs(2));
    let enqueue = http_post_json(
        &addr,
        "/api/v1/tasks",
        r#"{"projectId":"default","kind":"rescan","target":"raw/sources"}"#,
        None,
    );
    assert!(enqueue.starts_with("HTTP/1.0 200"), "{enqueue}");
    assert!(enqueue.contains("\"task\""));
    assert!(enqueue.contains("\"kind\":\"rescan\""));
    let list = http_get(&addr, "/api/v1/tasks", None);
    assert!(list.starts_with("HTTP/1.0 200"));
    assert!(list.contains("\"kind\":\"rescan\""));
    drop(handle);
    std::fs::remove_dir_all(data_dir).ok();
}

#[test]
fn health_reports_token_state() {
    let port = free_port();
    let data_dir = temp_data_dir("token-state");
    let dist_dir = data_dir.join("dist");
    std::fs::create_dir_all(&dist_dir).unwrap();
    let config = BackendConfig {
        bind_addr: ([127, 0, 0, 1], port).into(),
        data_dir: data_dir.clone(),
        dist_dir,
        api_token: Some("xyz".to_string()),
        allow_unauthenticated: false,
        allow_lan_access: false,
        max_upload_bytes: 1024 * 1024,
        max_body_bytes: 1024 * 1024,
    };
    let handle = run_server(config).expect("server start");
    let addr = format!("127.0.0.1:{port}");
    wait_for_server(&addr, Duration::from_secs(2));
    let response = http_get(&addr, "/api/v1/health", None);
    assert!(response.contains("\"authRequired\":true"));
    assert!(response.contains("\"authConfigured\":true"));
    drop(handle);
    std::fs::remove_dir_all(data_dir).ok();
}

#[test]
fn post_projects_scaffolds_a_new_wiki_project() {
    let port = free_port();
    let data_dir = temp_data_dir("create-project");
    let dist_dir = data_dir.join("dist");
    std::fs::create_dir_all(&dist_dir).unwrap();
    let config = BackendConfig {
        bind_addr: ([127, 0, 0, 1], port).into(),
        data_dir: data_dir.clone(),
        dist_dir,
        api_token: None,
        allow_unauthenticated: true,
        allow_lan_access: false,
        max_upload_bytes: 1024 * 1024,
        max_body_bytes: 1024 * 1024,
    };
    let handle = run_server(config).expect("server start");
    let addr = format!("127.0.0.1:{port}");
    wait_for_server(&addr, Duration::from_secs(2));
    let response = http_post_json(
        &addr,
        "/api/v1/projects",
        r#"{"name":"My Research"}"#,
        None,
    );
    assert!(response.starts_with("HTTP/1.0 200"), "{response}");
    assert!(response.contains("\"name\":\"My Research\""));
    let scaffolded = data_dir.join("projects/my research");
    assert!(scaffolded.join("wiki/index.md").is_file(), "wiki index scaffolded");
    assert!(scaffolded.join("purpose.md").is_file(), "purpose scaffolded");
    assert!(scaffolded.join("schema.md").is_file(), "schema scaffolded");
    let listed = http_get(&addr, "/api/v1/projects", None);
    assert!(listed.contains("My Research"));
    drop(handle);
    std::fs::remove_dir_all(data_dir).ok();
}

#[test]
fn post_projects_rejects_duplicate_name() {
    let port = free_port();
    let data_dir = temp_data_dir("create-dup");
    let dist_dir = data_dir.join("dist");
    std::fs::create_dir_all(&dist_dir).unwrap();
    let config = BackendConfig {
        bind_addr: ([127, 0, 0, 1], port).into(),
        data_dir: data_dir.clone(),
        dist_dir,
        api_token: None,
        allow_unauthenticated: true,
        allow_lan_access: false,
        max_upload_bytes: 1024 * 1024,
        max_body_bytes: 1024 * 1024,
    };
    let handle = run_server(config).expect("server start");
    let addr = format!("127.0.0.1:{port}");
    wait_for_server(&addr, Duration::from_secs(2));
    let first = http_post_json(&addr, "/api/v1/projects", r#"{"name":"dup"}"#, None);
    assert!(first.starts_with("HTTP/1.0 200"));
    let second = http_post_json(&addr, "/api/v1/projects", r#"{"name":"dup"}"#, None);
    assert!(second.starts_with("HTTP/1.0 409"), "{second}");
    drop(handle);
    std::fs::remove_dir_all(data_dir).ok();
}

#[test]
fn post_projects_rejects_empty_name() {
    let port = free_port();
    let data_dir = temp_data_dir("create-empty");
    let dist_dir = data_dir.join("dist");
    std::fs::create_dir_all(&dist_dir).unwrap();
    let config = BackendConfig {
        bind_addr: ([127, 0, 0, 1], port).into(),
        data_dir: data_dir.clone(),
        dist_dir,
        api_token: None,
        allow_unauthenticated: true,
        allow_lan_access: false,
        max_upload_bytes: 1024 * 1024,
        max_body_bytes: 1024 * 1024,
    };
    let handle = run_server(config).expect("server start");
    let addr = format!("127.0.0.1:{port}");
    wait_for_server(&addr, Duration::from_secs(2));
    let resp = http_post_json(&addr, "/api/v1/projects", r#"{"name":"   "}"#, None);
    assert!(resp.starts_with("HTTP/1.0 400"), "{resp}");
    drop(handle);
    std::fs::remove_dir_all(data_dir).ok();
}

#[test]
fn open_project_by_path_round_trips_after_create() {
    let port = free_port();
    let data_dir = temp_data_dir("open-by-path");
    let dist_dir = data_dir.join("dist");
    std::fs::create_dir_all(&dist_dir).unwrap();
    let config = BackendConfig {
        bind_addr: ([127, 0, 0, 1], port).into(),
        data_dir: data_dir.clone(),
        dist_dir,
        api_token: None,
        allow_unauthenticated: true,
        allow_lan_access: false,
        max_upload_bytes: 1024 * 1024,
        max_body_bytes: 1024 * 1024,
    };
    let handle = run_server(config).expect("server start");
    let addr = format!("127.0.0.1:{port}");
    wait_for_server(&addr, Duration::from_secs(2));
    let created = http_post_json(
        &addr,
        "/api/v1/projects",
        r#"{"name":"Round Trip"}"#,
        None,
    );
    assert!(created.starts_with("HTTP/1.0 200"));
    // The actual on-disk path includes spaces; the API percent-decodes
    // the URL segment, so this exercises the round-trip the web client
    // uses after a `getLastProject` hydration.
    let path = data_dir.join("projects/round trip");
    let normalized = path.to_string_lossy().replace('\\', "/");
    let url = format!(
        "/api/v1/projects/by-path/{}",
        url_encode(&normalized)
    );
    let resolved = http_get(&addr, &url, None);
    assert!(resolved.starts_with("HTTP/1.0 200"), "{resolved}");
    assert!(resolved.contains("\"name\":\"Round Trip\""));
    let missing = http_get(
        &addr,
        "/api/v1/projects/by-path/this%2Fpath%2Fdoes%2Fnot%2Fexist",
        None,
    );
    assert!(missing.starts_with("HTTP/1.0 404"), "{missing}");
    drop(handle);
    std::fs::remove_dir_all(data_dir).ok();
}

#[test]
fn enqueue_task_returns_real_rescan_record() {
    let port = free_port();
    let data_dir = temp_data_dir("enqueue-rescan");
    let dist_dir = data_dir.join("dist");
    std::fs::create_dir_all(&dist_dir).unwrap();
    let config = BackendConfig {
        bind_addr: ([127, 0, 0, 1], port).into(),
        data_dir: data_dir.clone(),
        dist_dir,
        api_token: None,
        allow_unauthenticated: true,
        allow_lan_access: false,
        max_upload_bytes: 1024 * 1024,
        max_body_bytes: 1024 * 1024,
    };
    let handle = run_server(config).expect("server start");
    let addr = format!("127.0.0.1:{port}");
    wait_for_server(&addr, Duration::from_secs(2));
    let enqueue = http_post_json(
        &addr,
        "/api/v1/tasks",
        r#"{"projectId":"default","kind":"rescan","target":"raw/sources"}"#,
        None,
    );
    assert!(enqueue.starts_with("HTTP/1.0 200"));
    assert!(enqueue.contains("\"kind\":\"rescan\""));
    let list = http_get(&addr, "/api/v1/tasks", None);
    assert!(list.starts_with("HTTP/1.0 200"));
    assert!(list.contains("\"kind\":\"rescan\""));
    // The rescan worker should finish quickly because there are no
    // source files in the empty default project. Poll for it once.
    std::thread::sleep(Duration::from_millis(250));
    let listed = http_get(&addr, "/api/v1/tasks?status=done", None);
    assert!(listed.starts_with("HTTP/1.0 200"));
    assert!(listed.contains("\"kind\":\"rescan\""));
    drop(handle);
    std::fs::remove_dir_all(data_dir).ok();
}

#[test]
fn enqueue_enrich_returns_unsupported_failure_status() {
    let port = free_port();
    let data_dir = temp_data_dir("enrich");
    let dist_dir = data_dir.join("dist");
    std::fs::create_dir_all(&dist_dir).unwrap();
    let config = BackendConfig {
        bind_addr: ([127, 0, 0, 1], port).into(),
        data_dir: data_dir.clone(),
        dist_dir,
        api_token: None,
        allow_unauthenticated: true,
        allow_lan_access: false,
        max_upload_bytes: 1024 * 1024,
        max_body_bytes: 1024 * 1024,
    };
    let handle = run_server(config).expect("server start");
    let addr = format!("127.0.0.1:{port}");
    wait_for_server(&addr, Duration::from_secs(2));
    let enqueue = http_post_json(
        &addr,
        "/api/v1/tasks",
        r#"{"projectId":"default","kind":"enrich","target":"all"}"#,
        None,
    );
    assert!(enqueue.starts_with("HTTP/1.0 200"), "{enqueue}");
    std::thread::sleep(Duration::from_millis(250));
    let listed = http_get(&addr, "/api/v1/tasks?status=failed", None);
    assert!(listed.starts_with("HTTP/1.0 200"));
    assert!(
        listed.contains("not supported"),
        "enrich task should record the unsupported reason, got: {listed}"
    );
    drop(handle);
    std::fs::remove_dir_all(data_dir).ok();
}

#[test]
fn reject_request_with_invalid_token() {
    let port = free_port();
    let data_dir = temp_data_dir("bad-token");
    let dist_dir = data_dir.join("dist");
    std::fs::create_dir_all(&dist_dir).unwrap();
    let config = BackendConfig {
        bind_addr: ([127, 0, 0, 1], port).into(),
        data_dir: data_dir.clone(),
        dist_dir,
        api_token: Some("good-token".to_string()),
        allow_unauthenticated: false,
        allow_lan_access: false,
        max_upload_bytes: 1024 * 1024,
        max_body_bytes: 1024 * 1024,
    };
    let handle = run_server(config).expect("server start");
    let addr = format!("127.0.0.1:{port}");
    wait_for_server(&addr, Duration::from_secs(2));
    let denied = http_get(&addr, "/api/v1/projects", Some("wrong-token"));
    assert!(denied.starts_with("HTTP/1.0 401"));
    drop(handle);
    std::fs::remove_dir_all(data_dir).ok();
}

fn url_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}
