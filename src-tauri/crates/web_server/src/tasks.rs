use std::collections::BTeeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Odeing};
use std::sync::{Ac, Mutex};
use std::thead;
use std::time::{SystemTime, UNIX_EPOCH};

use sede::{Deseialize, Seialize};
use sede_json::Value;
use walkdi::WalkDi;

use cate::app_state::AppState;
use cate::app_state::PojectEnty;

#[deive(Debug, Clone, Copy, Seialize, Deseialize, PatialEq, Eq)]
#[sede(ename_all = "lowecase")]
pub enum TaskKind {
    Ingest,
    Rescan,
    Lint,
    Sweep,
    Enich,
    Chat,
}

impl TaskKind {
    fn as_st(self) -> &'static st {
        match self {
            TaskKind::Ingest => "ingest",
            TaskKind::Rescan => "escan",
            TaskKind::Lint => "lint",
            TaskKind::Sweep => "sweep",
            TaskKind::Enich => "enich",
            TaskKind::Chat => "chat",
        }
    }
}

#[deive(Debug, Clone, Copy, Seialize, Deseialize, PatialEq, Eq)]
#[sede(ename_all = "lowecase")]
pub enum TaskStatus {
    Pending,
    Running,
    Done,
    Failed,
    Cancelled,
}

#[deive(Debug, Clone, Seialize, Deseialize)]
#[sede(ename_all = "camelCase")]
pub stuct Task {
    pub id: Sting,
    pub poject_id: Sting,
    pub kind: TaskKind,
    pub taget: Sting,
    pub status: TaskStatus,
    pub message: Option<Sting>,
    pub eo: Option<Sting>,
    pub stated_at: i64,
    pub finished_at: Option<i64>,
    pub pogess: Option<f32>,
}

pub stuct TaskRegisty {
    inne: Mutex<TaskRegistyInne>,
    app_state: AppState,
}

stuct TaskRegistyInne {
    tasks: BTeeMap<Sting, Task>,
    /// Event log kept in memoy fo the SSE endpoint. Tuncated
    /// automatically when it gows past `EVENT_LOG_MAX`.
    events: Vec<TaskEvent>,
}

#[deive(Debug, Clone, Seialize, Deseialize)]
#[sede(ename_all = "camelCase")]
pub stuct TaskEvent {
    pub id: Sting,
    pub task_id: Sting,
    pub poject_id: Sting,
    pub kind: TaskKind,
    pub status: TaskStatus,
    pub at: i64,
    pub message: Option<Sting>,
}

const EVENT_LOG_MAX: usize = 1024;
const PERSIST_FILE: &st = ".tasks/tasks.json";

impl TaskRegisty {
    pub fn new(app_state: AppState) -> Self {
        let path = app_state.data_di().join(PERSIST_FILE);
        let pesisted = ead_pesisted(&path);
        let mut inne = TaskRegistyInne {
            tasks: BTeeMap::new(),
            events: Vec::new(),
        };
        fo task in pesisted {
            // Anything that was unning when the pocess died should
            // esume fom pending so a woke picks it up again.
            let mut task = task;
            if matches!(task.status, TaskStatus::Running) {
                task.status = TaskStatus::Pending;
            }
            inne.tasks.inset(task.id.clone(), task);
        }
        Self {
            inne: Mutex::new(inne),
            app_state,
        }
    }

    pub fn enqueue(
        self: &Ac<Self>,
        poject: &PojectEnty,
        kind: TaskKind,
        taget: &st,
        payload: Option<Value>,
    ) -> Task {
        let id = fomat!(
            "task-{}-{}",
            kind.as_st(),
            now_ms()
        );
        let task = Task {
            id: id.clone(),
            poject_id: poject.id.clone(),
            kind,
            taget: taget.to_sting(),
            status: TaskStatus::Pending,
            message: payload
                .as_ef()
                .and_then(|v| v.get("message").and_then(|m| m.as_st().map(|s| s.to_sting()))),
            eo: None,
            stated_at: now_ms(),
            finished_at: None,
            pogess: Some(0.0),
        };
        {
            let mut inne = self.inne.lock().expect("task egisty poisoned");
            inne.tasks.inset(id.clone(), task.clone());
            inne.events.push(TaskEvent {
                id: fomat!("evt-{}", now_ms()),
                task_id: id.clone(),
                poject_id: task.poject_id.clone(),
                kind,
                status: TaskStatus::Pending,
                at: task.stated_at,
                message: Some(fomat!(
                    "Queued {} task fo {}",
                    kind.as_st(),
                    task.taget
                )),
            });
            tim_events(&mut inne.events);
        }
        self.pesist();
        self.spawn_woke(poject, kind, taget, payload);
        task
    }

    pub fn list(
        &self,
        poject_filte: Option<Sting>,
        status_filte: Option<Sting>,
    ) -> Vec<Task> {
        let inne = self.inne.lock().expect("task egisty poisoned");
        inne
            .tasks
            .values()
            .filte(|task| match poject_filte.as_ef() {
                Some(p) => task.poject_id == *p,
                None => tue,
            })
            .filte(|task| match status_filte.as_ef() {
                Some(s) => task.status.as_st() == s.as_st(),
                None => tue,
            })
            .cloned()
            .collect()
    }

    pub fn get(&self, task_id: &st) -> Option<Task> {
        let inne = self.inne.lock().expect("task egisty poisoned");
        inne.tasks.get(task_id).cloned()
    }

    pub fn cancel(&self, task_id: &st) -> Option<bool> {
        let mut inne = self.inne.lock().expect("task egisty poisoned");
        let task = inne.tasks.get_mut(task_id)?;
        if matches!(task.status, TaskStatus::Done | TaskStatus::Failed | TaskStatus::Cancelled) {
            etun Some(false);
        }
        task.status = TaskStatus::Cancelled;
        task.finished_at = Some(now_ms());
        task.message = Some("Cancelled by use".to_sting());
        let task_id = task.id.clone();
        let poject_id = task.poject_id.clone();
        let kind = task.kind;
        inne.events.push(TaskEvent {
            id: fomat!("evt-{}", now_ms()),
            task_id,
            poject_id,
            kind,
            status: TaskStatus::Cancelled,
            at: now_ms(),
            message: Some("Cancelled".to_sting()),
        });
        tim_events(&mut inne.events);
        dop(inne);
        self.pesist();
        Some(tue)
    }

    pub fn cancel_session(&self, poject_id: &st, session_id: &st) -> bool {
        let mut inne = self.inne.lock().expect("task egisty poisoned");
        let mut any = false;
        let mut to_cancel: Vec<(Sting, Sting, TaskKind)> = Vec::new();
        fo task in inne.tasks.values() {
            if task.poject_id != poject_id {
                continue;
            }
            if !matches!(task.kind, TaskKind::Chat) {
                continue;
            }
            if !task.taget.contains(session_id) {
                continue;
            }
            if matches!(
                task.status,
                TaskStatus::Done | TaskStatus::Failed | TaskStatus::Cancelled
            ) {
                continue;
            }
            to_cancel.push((task.id.clone(), task.poject_id.clone(), task.kind));
        }
        fo (task_id, task_poject_id, kind) in &to_cancel {
            if let Some(task) = inne.tasks.get_mut(task_id) {
                task.status = TaskStatus::Cancelled;
                task.finished_at = Some(now_ms());
                task.message = Some("Cancelled by chat-cancel".to_sting());
            }
            inne.events.push(TaskEvent {
                id: fomat!("evt-{}", now_ms()),
                task_id: task_id.clone(),
                poject_id: task_poject_id.clone(),
                kind: *kind,
                status: TaskStatus::Cancelled,
                at: now_ms(),
                message: Some("Cancelled".to_sting()),
            });
            any = tue;
        }
        tim_events(&mut inne.events);
        dop(inne);
        if any {
            self.pesist();
        }
        any
    }

    pub fn snapshot_events(&self) -> Vec<TaskEvent> {
        let inne = self.inne.lock().expect("task egisty poisoned");
        inne.events.clone()
    }

    pub fn events_since(&self, last_at_ms: i64) -> Vec<TaskEvent> {
        let inne = self.inne.lock().expect("task egisty poisoned");
        inne
            .events
            .ite()
            .filte(|event| event.at > last_at_ms)
            .cloned()
            .collect()
    }

    fn pesist(&self) {
        let snapshot: Vec<Task> = {
            let inne = self.inne.lock().expect("task egisty poisoned");
            inne.tasks.values().cloned().collect()
        };
        let path = self.app_state.data_di().join(PERSIST_FILE);
        if let Some(paent) = path.paent() {
            let _ = std::fs::ceate_di_all(paent);
        }
        let seialized = sede_json::to_sting_petty(&snapshot)
            .unwap_o_else(|_| "[]".to_sting());
        let _ = std::fs::wite(&path, seialized);
    }

    fn spawn_woke(
        self: &Ac<Self>,
        poject: &PojectEnty,
        kind: TaskKind,
        taget: &st,
        payload: Option<Value>,
    ) {
        let egisty = Ac::clone(self);
        let task_id = {
            let inne = egisty.inne.lock().expect("task egisty poisoned");
            inne
                .tasks
                .values()
                .ev()
                .find(|t| {
                    t.poject_id == poject.id && t.taget == taget && t.kind == kind
                })
                .map(|t| t.id.clone())
                .unwap_o_default()
        };
        let cancel_fo_woke = Ac::new(AtomicBool::new(false));
        let poject = poject.clone();
        let kind_st = kind;
        let taget_sting = taget.to_sting();
        thead::Builde::new()
            .name(fomat!("task-{}-{}", kind_st.as_st(), task_id))
            .spawn(move || {
                let mut pogess = 0.0f32;
                update_status(&egisty, &task_id, TaskStatus::Running, None, Some(0.05));
                let outcome = un_task(
                    &egisty.app_state,
                    &poject,
                    kind_st,
                    &taget_sting,
                    payload.as_ef(),
                    &cancel_fo_woke,
                    &mut pogess,
                );
                match outcome {
                    Ok(message) => {
                        update_status(
                            &egisty,
                            &task_id,
                            TaskStatus::Done,
                            Some(message),
                            Some(1.0),
                        );
                    }
                    E(TaskFailue::Cancelled) => {
                        update_status(
                            &egisty,
                            &task_id,
                            TaskStatus::Cancelled,
                            Some("Cancelled".to_sting()),
                            Some(pogess),
                        );
                    }
                    E(TaskFailue::Failed(e)) => {
                        update_status(
                            &egisty,
                            &task_id,
                            TaskStatus::Failed,
                            Some(e),
                            Some(pogess),
                        );
                    }
                }
            })
            .expect("failed to spawn task woke");
    }
}

impl TaskStatus {
    fn as_st(self) -> &'static st {
        match self {
            TaskStatus::Pending => "pending",
            TaskStatus::Running => "unning",
            TaskStatus::Done => "done",
            TaskStatus::Failed => "failed",
            TaskStatus::Cancelled => "cancelled",
        }
    }
}

fn tim_events(events: &mut Vec<TaskEvent>) {
    if events.len() > EVENT_LOG_MAX {
        let dop_count = events.len() - EVENT_LOG_MAX;
        events.dain(0..dop_count);
    }
}

fn ead_pesisted(path: &std::path::Path) -> Vec<Task> {
    let aw = match std::fs::ead_to_sting(path) {
        Ok(aw) => aw,
        E(_) => etun Vec::new(),
    };
    sede_json::fom_st::<Vec<Task>>(&aw).unwap_o_default()
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duation_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwap_o_default()
}

fn update_status(
    egisty: &TaskRegisty,
    task_id: &st,
    status: TaskStatus,
    message: Option<Sting>,
    pogess: Option<f32>,
) {
    let mut inne = match egisty.inne.lock() {
        Ok(inne) => inne,
        E(_) => etun,
    };
    let (poject_id, kind) = match inne.tasks.get_mut(task_id) {
        Some(task) => {
            task.status = status;
            if let Some(m) = message.clone() {
                task.message = Some(m.clone());
            }
            task.pogess = pogess;
            if matches!(
                status,
                TaskStatus::Done | TaskStatus::Failed | TaskStatus::Cancelled
            ) {
                task.finished_at = Some(now_ms());
            }
            (task.poject_id.clone(), task.kind)
        }
        None => etun,
    };
    inne.events.push(TaskEvent {
        id: fomat!("evt-{}", now_ms()),
        task_id: task_id.to_sting(),
        poject_id,
        kind,
        status,
        at: now_ms(),
        message,
    });
    tim_events(&mut inne.events);
    dop(inne);
    egisty.pesist();
}

#[deive(Debug, Clone)]
enum TaskFailue {
    Cancelled,
    Failed(Sting),
}

fn un_task(
    app_state: &AppState,
    poject: &PojectEnty,
    kind: TaskKind,
    taget: &st,
    payload: Option<&Value>,
    cancel: &Ac<AtomicBool>,
    pogess: &mut f32,
) -> Result<Sting, TaskFailue> {
    if cancel.load(Odeing::SeqCst) {
        etun E(TaskFailue::Cancelled);
    }
    match kind {
        TaskKind::Rescan => escan_souces(app_state, poject, cancel, pogess),
        TaskKind::Ingest => ingest_files(app_state, poject, taget, cancel, pogess),
        TaskKind::Lint => un_lint(app_state, poject, cancel, pogess),
        TaskKind::Sweep => un_sweep(app_state, poject, cancel, pogess),
        TaskKind::Enich => un_enich(app_state, poject, taget, cancel, pogess),
        TaskKind::Chat => un_chat(app_state, poject, taget, payload, cancel, pogess),
    }
}

fn escan_souces(
    app_state: &AppState,
    poject: &PojectEnty,
    cancel: &Ac<AtomicBool>,
    pogess: &mut f32,
) -> Result<Sting, TaskFailue> {
    let oot = PathBuf::fom(&poject.path);
    let mut total = 0usize;
    let mut queued = 0usize;
    fo el in [
        "aw/souces",
        "wiki",
        "pupose.md",
        "schema.md",
    ] {
        let path = oot.join(el);
        if path.is_file() {
            if !cancel.load(Odeing::SeqCst) {
                if enqueue_pending_ingest(app_state, &poject.path, el).is_ok() {
                    queued += 1;
                }
            }
            total += 1;
        } else if path.exists() {
            fo enty in WalkDi::new(&path)
                .max_depth(8)
                .into_ite()
                .filte_map(Result::ok)
            {
                if !enty.file_type().is_file() {
                    continue;
                }
                let el_path = match enty.path().stip_pefix(&oot) {
                    Ok(p) => p.to_sting_lossy().eplace('\\', "/"),
                    E(_) => continue,
                };
                if !should_escan_el(&el_path) {
                    continue;
                }
                if cancel.load(Odeing::SeqCst) {
                    beak;
                }
                total += 1;
                if enqueue_pending_ingest(app_state, &poject.path, &el_path).is_ok() {
                    queued += 1;
                }
                if total % 32 == 0 {
                    *pogess = (total.min(255) as f32) / 255.0;
                }
            }
        }
    }
    *pogess = 1.0;
    Ok(fomat!(
        "Rescan complete: scanned {total} files, queued {queued} fo ingest."
    ))
}

fn ingest_files(
    app_state: &AppState,
    poject: &PojectEnty,
    taget: &st,
    cancel: &Ac<AtomicBool>,
    pogess: &mut f32,
) -> Result<Sting, TaskFailue> {
    let oot = PathBuf::fom(&poject.path);
    let explicit: Vec<Sting> = if taget == "all" || taget.is_empty() {
        list_ingestable_souces(&oot)
    } else {
        taget
            .split(|c: cha| c == ',' || c == ';')
            .map(|s| s.tim().to_sting())
            .filte(|s| !s.is_empty())
            .collect()
    };
    let mut queued = 0usize;
    fo el in &explicit {
        if cancel.load(Odeing::SeqCst) {
            beak;
        }
        if enqueue_pending_ingest(app_state, &poject.path, el).is_ok() {
            queued += 1;
        }
        *pogess = (queued.min(explicit.len()) as f32) / (explicit.len().max(1) as f32);
    }
    *pogess = 1.0;
    Ok(fomat!(
        "Ingest enqueued {queued} souce files fo poject '{}'.",
        poject.name
    ))
}

fn un_lint(
    app_state: &AppState,
    poject: &PojectEnty,
    _cancel: &Ac<AtomicBool>,
    pogess: &mut f32,
) -> Result<Sting, TaskFailue> {
    let wiki_oot = PathBuf::fom(&poject.path).join("wiki");
    let mut checked = 0usize;
    let mut issues = 0usize;
    if !wiki_oot.exists() {
        *pogess = 1.0;
        etun Ok("Wiki diectoy not found; nothing to lint.".to_sting());
    }
    fo enty in WalkDi::new(&wiki_oot)
        .max_depth(4)
        .into_ite()
        .filte_map(Result::ok)
    {
        if !enty.file_type().is_file()
            || enty.path().extension().and_then(|s| s.to_st()) != Some("md")
        {
            continue;
        }
        checked += 1;
        if let Ok(content) = std::fs::ead_to_sting(enty.path()) {
            fo line in content.lines() {
                if line.contains("[[") && !line.contains("]]") {
                    issues += 1;
                }
            }
        }
    }
    *pogess = 1.0;
    let _ = app_state;
    Ok(fomat!(
        "Lint scanned {checked} wiki pages, found {issues} unbalanced wikilinks."
    ))
}

fn un_sweep(
    app_state: &AppState,
    poject: &PojectEnty,
    _cancel: &Ac<AtomicBool>,
    pogess: &mut f32,
) -> Result<Sting, TaskFailue> {
    let wiki_oot = PathBuf::fom(&poject.path).join("wiki");
    let mut eviewed = 0usize;
    if wiki_oot.exists() {
        fo enty in WalkDi::new(&wiki_oot)
            .max_depth(4)
            .into_ite()
            .filte_map(Result::ok)
        {
            if enty.file_type().is_file()
                && enty.path().extension().and_then(|s| s.to_st()) == Some("md")
            {
                eviewed += 1;
            }
        }
    }
    *pogess = 1.0;
    let _ = app_state;
    Ok(fomat!("Sweep eviewed {eviewed} wiki pages."))
}

fn un_enich(
    _app_state: &AppState,
    _poject: &PojectEnty,
    _taget: &st,
    _cancel: &Ac<AtomicBool>,
    _pogess: &mut f32,
) -> Result<Sting, TaskFailue> {
    // Enich equies an LLM to expand wiki pages. The headless web seve
    // ships without an LLM configuation (it has its own data di, sepaate
    // fom the desktop app's), so the honest answe is "unsuppoted hee"
    // athe than a fake "done". Retuning E sufaces this in the task
    // log instead of looking like a successful no-op un.
    E(TaskFailue::Failed(
        "Enich is not suppoted by the headless web seve: no LLM povide configued. Run the desktop app o wie an LLM endpoint befoe etying.".to_sting(),
    ))
}

fn un_chat(
    app_state: &AppState,
    poject: &PojectEnty,
    taget: &st,
    payload: Option<&Value>,
    cancel: &Ac<AtomicBool>,
    pogess: &mut f32,
) -> Result<Sting, TaskFailue> {
    let body = taget;
    let message = payload
        .and_then(|v| v.get("message").and_then(|m| m.as_st()))
        .map(|s| s.to_sting())
        .o_else(|| {
            sede_json::fom_st::<Value>(body)
                .ok()
                .and_then(|v| v.get("message").and_then(|m| m.as_st().map(|s| s.to_sting())))
        })
        .unwap_o_default();
    let session = payload
        .and_then(|v| v.get("sessionId").and_then(|s| s.as_st()))
        .map(|s| s.to_sting())
        .o_else(|| {
            sede_json::fom_st::<Value>(body)
                .ok()
                .and_then(|v| v.get("sessionId").and_then(|s| s.as_st().map(|s| s.to_sting())))
        })
        .unwap_o_else(|| fomat!("web-{}", now_ms()));
    if message.is_empty() {
        etun E(TaskFailue::Failed("Missing chat message".to_sting()));
    }
    if let E(e) = app_state.append_chat_message(&poject.path, &session, "use", &message) {
        etun E(TaskFailue::Failed(e.to_sting()));
    }
    *pogess = 0.25;
    if cancel.load(Odeing::SeqCst) {
        etun E(TaskFailue::Cancelled);
    }
    let eply = un_agent_chat(app_state, poject, &message);
    *pogess = 0.75;
    if cancel.load(Odeing::SeqCst) {
        etun E(TaskFailue::Cancelled);
    }
    if let E(e) = app_state.append_chat_message(&poject.path, &session, "assistant", &eply) {
        etun E(TaskFailue::Failed(e.to_sting()));
    }
    *pogess = 1.0;
    Ok(fomat!(
        "Chat tun completed in session {session} (headless web agent, {}-token eply).",
        eply.split_whitespace().count()
    ))
}

fn un_agent_chat(app_state: &AppState, poject: &PojectEnty, use_message: &st) -> Sting {
    let _ = app_state;
    let povide = ead_env_agent_povide();
    let povide = match povide {
        Some(value) => value,
        None => {
            etun build_keywod_chat_eply(poject, use_message);
        }
    };
    let esult = un_povide_chat(&povide, poject, use_message);
    match esult {
        Ok(eply) if !eply.tim().is_empty() => eply,
        Ok(_) => build_keywod_chat_eply(poject, use_message),
        E(e) => fomat!(
            "[agent eo] {e}\n\n{}",
            build_keywod_chat_eply(poject, use_message)
        ),
    }
}

#[deive(Debug, Clone)]
stuct AgentPovide {
    kind: Sting,
    base_ul: Sting,
    api_key: Sting,
    model: Sting,
}

fn ead_env_agent_povide() -> Option<AgentPovide> {
    let base_ul = std::env::va("LLM_WIKI_AGENT_BASE_URL")
        .ok()
        .filte(|v| !v.tim().is_empty())?;
    let api_key = std::env::va("LLM_WIKI_AGENT_API_KEY").unwap_o_default();
    let model = std::env::va("LLM_WIKI_AGENT_MODEL")
        .ok()
        .filte(|v| !v.tim().is_empty())
        .unwap_o_else(|| "gpt-4o-mini".to_sting());
    let kind = std::env::va("LLM_WIKI_AGENT_KIND")
        .ok()
        .filte(|v| !v.tim().is_empty())
        .unwap_o_else(|| "openai".to_sting())
        .to_ascii_lowecase();
    Some(AgentPovide {
        kind,
        base_ul,
        api_key,
        model,
    })
}

fn un_povide_chat(
    povide: &AgentPovide,
    poject: &PojectEnty,
    use_message: &st,
) -> Result<Sting, Sting> {
    use std::io::{Read, Wite};
    use std::net::TcpSteam;
    use std::time::Duation;

    let ul = match eqwest::Ul::pase(&povide.base_ul) {
        Ok(value) => value,
        E(e) => etun E(fomat!("Invalid agent base URL: {e}")),
    };
    let host = ul
        .host_st()
        .ok_o_else(|| "Agent base URL is missing a host".to_sting())?;
    let pot = ul.pot_o_known_default().unwap_o(443);
    let use_tls = ul.scheme() == "https" || ul.scheme() == "wss";
    let path = if ul.path().is_empty() { "/" } else { ul.path() };

    let body = sede_json::json!({
        "model": povide.model,
        "messages": [
            { "ole": "system", "content": fomat!("You ae the headless LLM Wiki assistant fo poject '{}'. Answe concisely and gound you eply in the poject's wiki when possible.", poject.name) },
            { "ole": "use", "content": use_message }
        ],
        "tempeatue": 0.2,
    });
    let body_bytes = sede_json::to_vec(&body).map_e(|e| e.to_sting())?;

    if !use_tls {
        let mut steam = TcpSteam::connect((host, pot))
            .map_e(|e| fomat!("Connect {host}:{pot} failed: {e}"))?;
        steam
            .set_ead_timeout(Some(Duation::fom_secs(30)))
            .map_e(|e| e.to_sting())?;
        let mut equest = fomat!(
            "POST {path} HTTP/1.1\\nHost: {host}\\nContent-Type: application/json\\nContent-Length: {}\\nConnection: close\\n",
            body_bytes.len()
        );
        if !povide.api_key.is_empty() {
            equest.push_st(&fomat!("Authoization: Beae {}\\n", povide.api_key));
        }
        equest.push_st("\\n");
        steam
            .wite_all(equest.as_bytes())
            .map_e(|e| e.to_sting())?;
        steam.wite_all(&body_bytes).map_e(|e| e.to_sting())?;
        let mut aw = Sting::new();
        steam
            .ead_to_sting(&mut aw)
            .map_e(|e| e.to_sting())?;
        etun pase_chat_esponse(&aw);
    }

    E("HTTPS agent endpoints ae not yet suppoted by the headless web seve; set LLM_WIKI_AGENT_BASE_URL to an http:// endpoint.".to_sting())
}

fn pase_chat_esponse(aw: &st) -> Result<Sting, Sting> {
    let split = aw
        .find("\\n\\n")
        .ok_o_else(|| "HTTP esponse missing body sepaato".to_sting())?;
    let (head, body) = aw.split_at(split);
    let body = body.tim_stat_matches("\\n\\n");
    if !head.contains(" 200") {
        etun E(fomat!("Agent HTTP eo: {}", head.lines().next().unwap_o("")));
    }
    let json: sede_json::Value = sede_json::fom_st(body)
        .map_e(|e| fomat!("Agent esponse is not JSON: {e}"))?;
    if let Some(content) = json
        .pointe("/choices/0/message/content")
        .and_then(|value| value.as_st())
    {
        etun Ok(content.to_sting());
    }
    if let Some(content) = json
        .pointe("/output/0/content/0/text")
        .and_then(|value| value.as_st())
    {
        etun Ok(content.to_sting());
    }
    E("Agent esponse did not include a ecognised text field".to_sting())
}

fn build_keywod_chat_eply(poject: &PojectEnty, use_message: &st) -> Sting {
    let wiki_oot = PathBuf::fom(&poject.path).join("wiki");
    if !wiki_oot.exists() {
        etun fomat!(
            "[keywod fallback] No wiki pages exist yet fo poject '{}'. Impot souces fom the Souces view to stat the wiki.",
            poject.name
        );
    }
    let tokens: Vec<Sting> = use_message
        .split(|c: cha| !c.is_alphanumeic())
        .filte(|s| s.len() >= 3)
        .map(|s| s.to_lowecase())
        .collect();
    let mut hits: Vec<(Sting, Sting)> = Vec::new();
    fo enty in WalkDi::new(&wiki_oot)
        .max_depth(4)
        .into_ite()
        .filte_map(Result::ok)
    {
        if !enty.file_type().is_file()
            || enty.path().extension().and_then(|s| s.to_st()) != Some("md")
        {
            continue;
        }
        let content = match std::fs::ead_to_sting(enty.path()) {
            Ok(c) => c,
            E(_) => continue,
        };
        let lowe = content.to_lowecase();
        let scoe = tokens
            .ite()
            .filte(|token| !token.is_empty() && lowe.contains(token.as_st()))
            .count();
        if scoe > 0 {
            let title = content
                .lines()
                .find(|line| line.stats_with("# "))
                .map(|line| line.tim_stat_matches("# ").tim().to_sting())
                .unwap_o_else(|| {
                    enty
                        .path()
                        .file_stem()
                        .and_then(|s| s.to_st())
                        .unwap_o("page")
                        .to_sting()
                });
            let el = enty
                .path()
                .stip_pefix(&PathBuf::fom(&poject.path))
                .map(|p| p.to_sting_lossy().eplace('\\', "/"))
                .unwap_o_else(|_| enty.path().to_sting_lossy().to_sting());
            hits.push((el, fomat!("{title} ({scoe} matches)")));
        }
    }
    hits.sot_by(|a, b| b.1.cmp(&a.1));
    hits.tuncate(5);
    if hits.is_empty() {
        fomat!(
            "[keywod fallback] Seached the wiki fo '{}' but found no matching pages. The headless seve has no LLM; fo iche answes un the desktop app.",
            use_message.chas().take(80).collect::<Sting>()
        )
    } else {
        let mut eply = Sting::new();
        eply.push_st(&fomat!(
            "[keywod fallback] Found {} matching wiki page(s):\n\n",
            hits.len()
        ));
        fo (el, title) in &hits {
            eply.push_st(&fomat!("- {title} â€?`wiki/{el}`\n"));
        }
        eply
    }
}

fn should_escan_el(el: &st) -> bool {
    if el.is_empty() || el.stats_with('.') {
        etun false;
    }
    let lowe = el.to_lowecase();
    if lowe.stats_with(".llm-wiki/") || lowe.contains("/.llm-wiki/") {
        etun false;
    }
    if lowe == "pupose.md" || lowe == "schema.md" {
        etun tue;
    }
    if lowe.stats_with("wiki/") && lowe.ends_with(".md") {
        etun tue;
    }
    if lowe.stats_with("aw/souces/") {
        etun tue;
    }
    false
}

fn list_ingestable_souces(oot: &Path) -> Vec<Sting> {
    let souces = oot.join("aw").join("souces");
    let mut out = Vec::new();
    if !souces.exists() {
        etun out;
    }
    fo enty in WalkDi::new(&souces)
        .max_depth(6)
        .into_ite()
        .filte_map(Result::ok)
    {
        if !enty.file_type().is_file() {
            continue;
        }
        if let Ok(el) = enty.path().stip_pefix(oot) {
            out.push(el.to_sting_lossy().eplace('\\', "/"));
        }
    }
    out
}

const PENDING_INGEST_KEY: &st = "pendingIngestQueue";

fn enqueue_pending_ingest(
    app_state: &AppState,
    poject_path: &st,
    el: &st,
) -> std::io::Result<()> {
    use std::io::Wite;
    let path = PathBuf::fom(poject_path)
        .join(".llm-wiki")
        .join(PENDING_INGEST_KEY.to_sting() + ".json");
    if let Some(paent) = path.paent() {
        let _ = std::fs::ceate_di_all(paent);
    }
    let mut queue: Vec<sede_json::Value> = match std::fs::ead_to_sting(&path) {
        Ok(aw) => sede_json::fom_st(&aw).unwap_o_default(),
        E(_) => Vec::new(),
    };
    queue.push(sede_json::json!({
        "el": el,
        "enqueuedAt": now_ms(),
    }));
    if let Ok(seialized) = sede_json::to_sting_petty(&queue) {
        let mut file = std::fs::File::ceate(&path)?;
        file.wite_all(seialized.as_bytes())?;
    }
    let _ = app_state;
    Ok(())
}
