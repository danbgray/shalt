use axum::extract::{Path, Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{Html, IntoResponse, Json};
use axum::routing::{get, post};
use axum::Router;
use serde::{Deserialize, Serialize};
use shalt_core::board::Board;
use shalt_core::draft::Draft;
use shalt_core::jobs::{JobKind, JobQueue, JobStatus, LiveEvent};
use shalt_core::compose::{
    author_prompt, author_system_prompt, list_dirs, onboard_project, start_project, ComposeRequest,
};
use shalt_core::ledger::Ledger;

use shalt_core::org::Org;
use shalt_core::spec::{self, load_specs};
use std::collections::HashSet;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

const UI: &str = include_str!("ui.html");

#[derive(Clone)]
struct App {
    root: PathBuf,
    live: Arc<Mutex<HashSet<String>>>,
    port: u16,
    events: broadcast::Sender<LiveEvent>,
}

fn spawn_job(app: &App, job_id: String) {
    let live = app.live.clone();
    {
        let mut g = live.lock().unwrap();
        if !g.insert(job_id.clone()) {
            return;
        }
    }
    let app = app.clone();
    tokio::task::spawn_blocking(move || {
        let _ = shalt_core::execute_job(&job_id);
        live.lock().unwrap().remove(&job_id);
        // Play while the old HTTP call was still winding down: start a new
        // worker on whatever model the job has now.
        if JobQueue::load().wants_worker(&job_id) {
            spawn_job(&app, job_id);
            return;
        }
        chain_next(&app, &job_id);
    });
}

fn chain_next(app: &App, finished_id: &str) {
    let q = JobQueue::load();
    let Some(j) = q.get(finished_id).cloned() else {
        return;
    };
    if let Some(reason) = shalt_core::play_stop_reason(&j) {
        let mut org = Org::load();
        if org.pause(&j.project_id, true, Some(&reason)) {
            let _ = org.save();
        }
        return;
    }
    if shalt_core::play_chains_after(&j) {
        let org = Org::load();
        if !org.get(&j.project_id).map(|p| p.paused).unwrap_or(true) {
            loop {
                match shalt_core::continue_project(&j.project_id) {
                    Ok(Some(next)) => {
                        if next.kind == JobKind::Run && j.kind == JobKind::Run {
                            let mut q = JobQueue::load();
                            q.jobs.retain(|x| x.id != next.id);
                            let _ = q.save();
                            break;
                        }
                        spawn_job(app, next.id);
                    }
                    _ => break,
                }
            }
        }
    }
    fill_slots(app);
}

fn fill_slots(app: &App) {
    loop {
        let q = JobQueue::load();
        let org = Org::load();
        let cap = shalt_core::Capacity::load();
        let Some(id) = shalt_core::next_fillable(&q.jobs, &org, &cap) else {
            break;
        };
        if app.live.lock().unwrap().contains(&id) {
            break;
        }
        let mut org = Org::load();
        if let Some(j) = q.get(&id) {
            if org.get(&j.project_id).map(|p| p.paused).unwrap_or(false) {
                org.set_paused(&j.project_id, false);
                org.clear_notice(&j.project_id);
                let _ = org.save();
            }
        }
        let mut q = JobQueue::load();
        q.set_status(&id, JobStatus::Running);
        q.append(&id, "playing — a slot opened");
        let _ = q.save();
        spawn_job(app, id);
    }
}

pub async fn serve(root: PathBuf, port: u16, open: bool) -> Result<(), String> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::AddrInUse {
            format!("ADDR_IN_USE:{port}")
        } else {
            e.to_string()
        }
    })?;
    let bound = listener.local_addr().map_err(|e| e.to_string())?;
    let port = bound.port();
    if shalt_core::uis::current().is_none() {
        let mut q = JobQueue::load();
        q.interrupt_orphans();
        let _ = q.save();
    }
    let url = format!("http://127.0.0.1:{port}/");
    shalt_core::uis::record(shalt_core::uis::UiInstance {
        pid: std::process::id(),
        port,
        url: url.clone(),
        root: root.display().to_string(),
        started_at: chrono::Utc::now().to_rfc3339(),
    });
    let (events, _) = broadcast::channel::<LiveEvent>(256);
    {
        let tx = events.clone();
        shalt_core::jobs::set_live_hook(move |ev| {
            let _ = tx.send(ev.clone());
        });
    }
    {
        let tx = events.clone();
        tokio::spawn(async move {
            watch_jobs_file(tx).await;
        });
    }
    let app = Router::new()
        .route("/", get(index))
        .route("/api/health", get(api_health))
        .route("/api/events", get(api_events))
        .route("/api/org", get(api_org))
        .route("/api/models", get(api_models))
        .route("/api/keys", get(api_keys).post(api_keys_save))
        .route("/api/compose", post(api_compose))
        .route("/api/browse", get(api_browse))
        .route("/api/project/{id}", get(api_project).post(api_project_action))
        .route("/api/project/{id}/export", get(api_export))
        .route("/api/project/{id}/mockups/{*path}", get(api_mockup))
        .route("/api/project/{id}/markups/{rid}", get(api_markup))
        .route("/api/sketch.css", get(api_sketch_css))
        .route("/api/import", post(api_import))
        .route("/api/jobs", get(api_jobs).post(api_enqueue))
        .route("/api/jobs/{id}", get(api_job).post(api_job_action));
    let state = Arc::new(App {
        root,
        live: Arc::new(Mutex::new(HashSet::new())),
        port,
        events,
    });
    fill_slots(&state);
    let app = app.with_state(state);
    if open {
        let _ = std::process::Command::new("open").arg(&url).spawn();
    }
    eprintln!("shalt ui on {url}  pid {}", std::process::id());
    axum::serve(listener, app).await.map_err(|e| e.to_string())
}

async fn api_health(State(app): State<Arc<App>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "ok": true,
        "pid": std::process::id(),
        "port": app.port,
        "root": app.root.display().to_string(),
    }))
}

async fn api_events(
    State(app): State<Arc<App>>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let rx = app.events.subscribe();
    let hello = tokio_stream::once(Ok::<_, Infallible>(
        Event::default().event("hello").data("{}"),
    ));
    let body = BroadcastStream::new(rx).map(|item| {
        let ev = match item {
            Ok(ev) => {
                let name = if ev.tick { "tick" } else { "job" };
                Event::default()
                    .event(name)
                    .data(serde_json::to_string(&ev).unwrap_or_else(|_| "{}".into()))
            }
            Err(_) => Event::default().event("tick").data("{}"),
        };
        Ok::<_, Infallible>(ev)
    });
    Sse::new(hello.chain(body)).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(5))
            .text("ping"),
    )
}

async fn watch_jobs_file(tx: broadcast::Sender<LiveEvent>) {
    let path = JobQueue::path();
    let mut last = None;
    loop {
        tokio::time::sleep(Duration::from_millis(1000)).await;
        let mtime = tokio::fs::metadata(&path)
            .await
            .ok()
            .and_then(|m| m.modified().ok());
        if mtime != last {
            last = mtime;
            let _ = tx.send(LiveEvent {
                tick: true,
                ..Default::default()
            });
        }
    }
}

async fn index() -> Html<&'static str> {
    Html(UI)
}

#[derive(Serialize)]
struct OrgView {
    name: String,
    projects: Vec<ProjectCard>,
    stacks: Vec<shalt_core::StackChoice>,
    parallel: shalt_core::parallel::ParallelView,
    diagram: String,
    pipeline: String,
}

#[derive(Serialize)]
struct ProjectCard {
    id: String,
    name: String,
    path: String,
    green: i64,
    red: i64,
    pending: i64,
    stale: i64,
    total: i64,
    remaining: i64,
    pct: f64,
    missing: bool,
    paused: bool,
    yolo: bool,
    yolo_mode: String,
    pause_reason: String,
    notice: String,
    running: usize,
    waiting: usize,
    blocked: usize,
    phase: String,
    state: String,
    detail: String,
    issue: Option<shalt_core::jobs::Issue>,
    progress: Vec<shalt_core::org::ProgressPoint>,
    stack: String,
    stack_chosen: bool,
    gate: String,
    work_started: bool,
    resume: Option<ResumeView>,
    now: Option<NowView>,
    estimated: i64,
    spent: i64,
    forecast_secs: i64,
    spent_secs: i64,
}

#[derive(Serialize)]
struct NowView {
    rid: String,
    name: String,
    line: String,
    job_id: String,
    kind: String,
    status: String,
    backend: String,
    model: String,
    log: Vec<String>,
}

fn job_log_tail(j: &shalt_core::jobs::Job, n: usize) -> Vec<String> {
    shalt_core::jobs::activity_lines(&j.log, n)
}

fn card_now(
    jobs: &[&shalt_core::jobs::Job],
    ledger: Option<&Ledger>,
) -> Option<NowView> {
    let j = shalt_core::jobs::pick_live_job(jobs.iter().copied())?;
    let name = if !j.rid.is_empty() {
        ledger
            .and_then(|l| l.entries.get(&j.rid))
            .map(|e| e.name.clone())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| j.rid.clone())
    } else {
        j.prompt
            .lines()
            .next()
            .unwrap_or("Working")
            .chars()
            .take(80)
            .collect()
    };
    Some(NowView {
        rid: j.rid.clone(),
        name,
        line: shalt_core::jobs::status_line(j),
        job_id: j.id.clone(),
        kind: shalt_core::jobs::kind_phase(j.kind).into(),
        status: format!("{:?}", j.status).to_lowercase(),
        backend: j.backend.clone(),
        model: j.model.clone(),
        log: job_log_tail(j, 8),
    })
}

#[derive(Serialize)]
struct ResumeView {
    id: String,
    kind: String,
    status: String,
}

async fn api_org() -> Json<OrgView> {
    let mut org = Org::load();
    if org.explain_pauses() {
        let _ = org.save();
    }
    let q = JobQueue::load();
    let projects: Vec<ProjectCard> = org
        .projects
        .iter()
        .map(|p| {
            let path = PathBuf::from(&p.path);
            let missing = !path.exists();
            let led = if missing {
                None
            } else {
                Some(Ledger::load(&path.join(".shalt/ledger.json")).unwrap_or_default())
            };
            let (green, red, pending, stale, total) = if let Some(led) = led.as_ref() {
                let s = led.summary();
                let g = s.get("green").and_then(|v| v.as_i64()).unwrap_or(0);
                let r = s.get("red").and_then(|v| v.as_i64()).unwrap_or(0);
                let mut pd = s.get("pending").and_then(|v| v.as_i64()).unwrap_or(0);
                let st = s.get("stale").and_then(|v| v.as_i64()).unwrap_or(0);
                let mut tot = s.get("total").and_then(|v| v.as_i64()).unwrap_or(0)
                    - s.get("orphan").and_then(|v| v.as_i64()).unwrap_or(0);
                if tot == 0 {
                    if let Ok(features) = load_specs(&path.join("spec"), false) {
                        let n = features.iter().map(|f| f.scenarios.len() as i64).sum();
                        if n > 0 {
                            pd = n;
                            tot = n;
                        }
                    }
                }
                (g, r, pd, st, tot)
            } else {
                (0, 0, 0, 0, 0)
            };
            let remaining = (red + pending + stale).max(0);
            let pct = if total > 0 {
                (1000.0 * green as f64 / total as f64).round() / 10.0
            } else {
                0.0
            };
            let progress = if missing {
                vec![]
            } else {
                shalt_core::org::record_progress(&p.id, remaining, green, total)
            };
            let jobs: Vec<_> = q.jobs.iter().filter(|j| j.project_id == p.id).collect();
            let st = shalt_core::jobs::summarize_jobs(&jobs, p.paused);
            let stack = if missing {
                String::new()
            } else {
                shalt_core::Config::load(&path)
                    .map(|c| c.stack)
                    .unwrap_or_default()
            };
            let gate = if missing {
                String::new()
            } else {
                shalt_core::work_gate(&path).to_string()
            };
            let work_started = !missing && shalt_core::talk::work_started(&path);
            let resume = q.resume_target(&p.id).map(|j| ResumeView {
                id: j.id.clone(),
                kind: format!("{:?}", j.kind).to_lowercase(),
                status: format!("{:?}", j.status).to_lowercase(),
            });
            let detail = if p.paused && !p.pause_reason.is_empty() {
                p.pause_reason.clone()
            } else {
                st.detail
            };
            let spend = if missing {
                shalt_core::tokens::SpendSnapshot::default()
            } else {
                let board = shalt_core::Board::load(&path.join(".shalt/board.json"));
                shalt_core::tokens::spend_snapshot(
                    &board,
                    &q.jobs,
                    led.as_ref().unwrap_or(&Ledger::default()),
                    &p.id,
                )
            };
            ProjectCard {
                id: p.id.clone(),
                name: p.name.clone(),
                path: p.path.clone(),
                green,
                red,
                pending,
                stale,
                total,
                remaining,
                pct,
                missing,
                paused: p.paused,
                yolo: p.yolo_mode_enum().asks_all(),
                yolo_mode: p.yolo_mode_enum().as_str().into(),
                pause_reason: p.pause_reason.clone(),
                notice: p.notice.clone(),
                running: st.running,
                waiting: st.waiting,
                blocked: st.blocked,
                phase: st.phase,
                state: st.state,
                detail,
                issue: st.issue,
                progress,
                stack: stack.clone(),
                stack_chosen: shalt_core::config::stack_is_set(&stack),
                gate,
                work_started,
                resume,
                now: card_now(&jobs, led.as_ref()),
                estimated: spend.estimated,
                spent: spend.spent,
                forecast_secs: spend.forecast_secs,
                spent_secs: spend.spent_secs,
            }
        })
        .collect();
    let diagram = shalt_core::mermaid_org(
        &org.name,
        &projects
            .iter()
            .map(|p| (p.id.clone(), p.name.clone(), p.state.clone()))
            .collect::<Vec<_>>(),
    );
    Json(OrgView {
        name: org.name,
        projects,
        stacks: shalt_core::stack_choices(),
        parallel: shalt_core::parallel_view(&q.jobs),
        diagram,
        pipeline: shalt_core::mermaid_pipeline(),
    })
}

async fn api_models() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "models": shalt_core::roster_models(),
        "agents": shalt_core::agent_roster(),
        "stacks": shalt_core::stack_choices(),
    }))
}

async fn api_keys() -> Json<shalt_core::KeysStatus> {
    Json(shalt_core::keys_status())
}

async fn api_keys_save(Json(body): Json<shalt_core::KeysPatch>) -> impl IntoResponse {
    match shalt_core::save_keys(body) {
        Ok(status) => Json(status).into_response(),
        Err(e) => (axum::http::StatusCode::BAD_REQUEST, e).into_response(),
    }
}

#[derive(Deserialize)]
struct BrowseQuery {
    dir: Option<String>,
}

async fn api_browse(Query(q): Query<BrowseQuery>) -> impl IntoResponse {
    let raw = q
        .dir
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("~");
    match list_dirs(std::path::Path::new(raw)) {
        Ok(listing) => Json(listing).into_response(),
        Err(e) => (axum::http::StatusCode::BAD_REQUEST, e).into_response(),
    }
}

#[derive(serde::Deserialize)]
struct ComposeBody {
    prompt: String,
    #[serde(default)]
    backend: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    path: Option<String>,
    /// Parent or vacant folder for a new project.
    #[serde(default)]
    dir: Option<String>,
    #[serde(default)]
    stack: Option<String>,
}

async fn api_compose(State(app): State<Arc<App>>, Json(body): Json<ComposeBody>) -> impl IntoResponse {
    let started = if let Some(path) = body.path.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        onboard_project(std::path::Path::new(path), &body.prompt, &body.backend, &body.model)
    } else {
        start_project(ComposeRequest {
            prompt: body.prompt,
            backend: body.backend,
            model: body.model,
            name: body.name,
            stack: body.stack.unwrap_or_default(),
            dir: body.dir.filter(|s| !s.trim().is_empty()).map(std::path::PathBuf::from),
        })
    };
    match started {
        Ok((project, job)) => {
            let _ = shalt_core::claim_play(&project.id);
            spawn_job(&app, job.id.clone());
            Json(serde_json::json!({ "project": project, "job": job })).into_response()
        }
        Err(e) => (axum::http::StatusCode::BAD_REQUEST, e).into_response(),
    }
}

#[derive(Deserialize)]
struct ImportBody {
    pack: shalt_core::PlanPack,
    #[serde(default)]
    dir: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

async fn api_export(Path(id): Path<String>) -> impl IntoResponse {
    let org = Org::load();
    let Some(p) = org.get(&id) else {
        return (axum::http::StatusCode::NOT_FOUND, "no such project").into_response();
    };
    match shalt_core::export_pack(std::path::Path::new(&p.path), &p.name) {
        Ok(pack) => Json(pack).into_response(),
        Err(e) => (axum::http::StatusCode::BAD_REQUEST, e).into_response(),
    }
}

async fn api_sketch_css() -> impl IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "text/css; charset=utf-8")],
        shalt_core::SKETCH_CSS,
    )
}

async fn api_mockup(
    Path((id, rest)): Path<(String, String)>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let org = Org::load();
    let Some(p) = org.get(&id) else {
        return (axum::http::StatusCode::NOT_FOUND, "unknown project").into_response();
    };
    let Some(rest) = shalt_core::normalize_rel(&rest) else {
        return (axum::http::StatusCode::BAD_REQUEST, "bad path").into_response();
    };
    let root = std::path::PathBuf::from(&p.path);
    let file = root.join("mockups").join(&rest);
    let Ok(bytes) = std::fs::read(&file) else {
        return (axum::http::StatusCode::NOT_FOUND, "no such mockup").into_response();
    };
    let ctype = if rest.ends_with(".css") {
        "text/css; charset=utf-8"
    } else if rest.ends_with(".js") {
        "text/javascript; charset=utf-8"
    } else if rest.ends_with(".svg") {
        "image/svg+xml"
    } else {
        "text/html; charset=utf-8"
    };
    if !rest.ends_with(".html") && !rest.ends_with(".htm") {
        return ([(axum::http::header::CONTENT_TYPE, ctype)], bytes).into_response();
    }
    let html = String::from_utf8_lossy(&bytes).into_owned();
    let green = q.get("green").cloned().unwrap_or_default();
    let rid = q.get("rid").cloned().unwrap_or_default();
    let page = shalt_core::assemble_mockup(&root, &id, &rest, &html, &green, &rid);
    (
        [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
        page,
    )
        .into_response()
}

async fn api_markup(Path((id, rid)): Path<(String, String)>) -> impl IntoResponse {
    let org = Org::load();
    let Some(p) = org.get(&id) else {
        return (axum::http::StatusCode::NOT_FOUND, "unknown project").into_response();
    };
    let root = PathBuf::from(&p.path);
    let markup = shalt_core::markups::load_markup_any(&root, &rid);
    Json(markup).into_response()
}

async fn api_import(Json(body): Json<ImportBody>) -> impl IntoResponse {
    let dir = body
        .dir
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from);
    match shalt_core::import_pack(&body.pack, dir.as_deref(), body.name.as_deref()) {
        Ok(project) => Json(serde_json::json!({ "ok": true, "project": project })).into_response(),
        Err(e) => (axum::http::StatusCode::BAD_REQUEST, e).into_response(),
    }
}

#[derive(Serialize)]
struct ProjectView {
    id: String,
    name: String,
    path: String,
    stories: Vec<StoryView>,
    board: shalt_core::board::Board,
    summary: serde_json::Value,
    draft: shalt_core::draft::DraftView,
    live: bool,
    paused: bool,
    yolo: bool,
    yolo_mode: String,
    pause_reason: String,
    notice: String,
    running: usize,
    progress: Vec<shalt_core::org::ProgressPoint>,
    talk: shalt_core::talk::SpecTalk,
    work_started: bool,
    queue: Vec<BoardRow>,
    agents: Vec<AgentView>,
    sprints: Vec<shalt_core::board::Sprint>,
    retro: Option<shalt_core::board::SprintRetro>,
    suggest: i64,
    plan: String,
    command: shalt_core::tokens::CommandCenter,
    roster: Vec<shalt_core::AgentInfo>,
    issue: Option<shalt_core::jobs::Issue>,
    focus: shalt_core::tokens::WorkFocus,
    stack: String,
    stack_chosen: bool,
    gate: String,
    stack_note: String,
    stacks: Vec<shalt_core::StackChoice>,
    steps_dir: String,
    src_dir: String,
    test_files: Vec<TestFileView>,
    src_files: Vec<TestFileView>,
    parallel: shalt_core::parallel::ParallelView,
    work_map: shalt_core::WorkMap,
    diagrams: shalt_core::Diagrams,
    films: Vec<shalt_core::Film>,
    journey_tests: Vec<shalt_core::JourneyTests>,
    focus_journey: String,
    kit: Option<shalt_core::DesignKit>,
    interview: shalt_core::Interview,
    product_look: bool,
    markup: bool,
    asks: Vec<shalt_core::jobs::AskRecord>,
    journal: shalt_core::Journal,
    resume: Option<ResumeView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    harness_error: Option<String>,
    /// Cheap fingerprint of spec / drawings / journal / plan / ledger.
    doc: String,
}

#[derive(Serialize)]
struct ProjectNowView {
    id: String,
    live: bool,
    paused: bool,
    yolo: bool,
    yolo_mode: String,
    pause_reason: String,
    notice: String,
    running: usize,
    summary: serde_json::Value,
    agents: Vec<AgentView>,
    command: shalt_core::tokens::CommandCenter,
    issue: Option<shalt_core::jobs::Issue>,
    focus: shalt_core::tokens::WorkFocus,
    focus_journey: String,
    journey_tests: Vec<shalt_core::JourneyTests>,
    gate: String,
    resume: Option<ResumeView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    harness_error: Option<String>,
    doc: String,
}

#[derive(Serialize)]
struct TestFileView {
    path: String,
    bytes: usize,
    body: String,
}

fn zone_files(root: &std::path::Path, rel: String) -> Vec<TestFileView> {
    shalt_core::list_step_files(root, &rel)
        .into_iter()
        .take(48)
        .map(|path| {
            let raw = std::fs::read_to_string(root.join(&path)).unwrap_or_default();
            let bytes = raw.len();
            let body = if raw.chars().count() > 80_000 {
                let mut s: String = raw.chars().take(80_000).collect();
                s.push_str("\n… truncated …\n");
                s
            } else {
                raw
            };
            TestFileView { path, bytes, body }
        })
        .collect()
}

#[derive(Serialize)]
struct BoardRow {
    rid: String,
    name: String,
    rank: i64,
    status: String,
    sprint: String,
    estimate: i64,
    forecast_secs: i64,
    spent_tokens: i64,
    spent_secs: i64,
    epic: String,
    backend: String,
    model: String,
    #[serde(default)]
    goal_id: String,
    #[serde(default)]
    milestone_id: String,
    #[serde(default)]
    file: String,
}

#[derive(Serialize)]
struct AgentView {
    id: String,
    kind: String,
    status: String,
    line: String,
    log: Vec<String>,
    backend: String,
    model: String,
    tokens: i64,
    #[serde(default)]
    rid: String,
    #[serde(default)]
    created_at: String,
    #[serde(default)]
    turn: u32,
}

#[derive(Serialize)]
struct StoryView {
    file: String,
    name: String,
    narrative: String,
    tasks: Vec<TaskView>,
}

#[derive(Serialize)]
struct TaskView {
    rid: Option<String>,
    name: String,
    status: String,
    holdout: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    failure: Option<String>,
}

#[derive(Deserialize, Default)]
struct ProjectQuery {
    #[serde(default)]
    lite: Option<String>,
    #[serde(default)]
    now: Option<String>,
}

fn project_doc(path: &std::path::Path, led: &Ledger, gate: &str) -> String {
    let s = led.summary();
    format!(
        "{}:g{}:r{}:p{}:t{}:{}",
        shalt_core::org::document_stamp(path),
        s.get("green").and_then(|v| v.as_i64()).unwrap_or(0),
        s.get("red").and_then(|v| v.as_i64()).unwrap_or(0),
        s.get("pending").and_then(|v| v.as_i64()).unwrap_or(0),
        s.get("total").and_then(|v| v.as_i64()).unwrap_or(0),
        gate
    )
}

fn agent_views(q: &JobQueue, id: &str, log_take: usize) -> Vec<AgentView> {
    q.jobs
        .iter()
        .filter(|j| {
            j.project_id == id
                && matches!(
                    j.status,
                    JobStatus::Running
                        | JobStatus::Pending
                        | JobStatus::Waiting
                        | JobStatus::Paused
                )
        })
        .map(|j| {
            let kind = match j.kind {
                JobKind::Author => "author",
                JobKind::Design => "design",
                JobKind::Steps => "steps",
                JobKind::Build => "build",
                JobKind::Run => "run",
                JobKind::Verify => "verify",
                JobKind::Mutate => "mutate",
                JobKind::Diagrams => "diagrams",
                JobKind::Plan => "plan",
                JobKind::Ux => "ux",
            };
            let log: Vec<String> = shalt_core::jobs::activity_lines(&j.log, log_take);
            AgentView {
                id: j.id.clone(),
                kind: kind.into(),
                status: format!("{:?}", j.status).to_lowercase(),
                line: shalt_core::jobs::status_line(j),
                log,
                backend: j.backend.clone(),
                model: j.model.clone(),
                tokens: shalt_core::tokens::job_tokens(j),
                rid: j.rid.clone(),
                created_at: j.created_at.clone(),
                turn: shalt_core::jobs::job_turn(&j.log),
            }
        })
        .collect()
}

fn api_project_now(app: &App, id: &str) -> Json<ProjectNowView> {
    let org = Org::load();
    let pref = org.get(id);
    let path = pref
        .map(|p| PathBuf::from(&p.path))
        .unwrap_or_else(|| app.root.clone());
    let led = Ledger::load(&path.join(".shalt/ledger.json")).unwrap_or_default();
    let q = JobQueue::load();
    let board = Board::load(&path.join(".shalt/board.json"));
    let live = q.jobs.iter().any(|j| {
        j.project_id == id
            && matches!(
                j.status,
                JobStatus::Running | JobStatus::Waiting | JobStatus::Pending | JobStatus::Paused
            )
    });
    let paused = pref.map(|p| p.paused).unwrap_or(false);
    let running = q
        .jobs
        .iter()
        .filter(|j| {
            j.project_id == id && matches!(j.status, JobStatus::Running | JobStatus::Pending)
        })
        .count();
    let proj_jobs: Vec<_> = q.jobs.iter().filter(|j| j.project_id == id).collect();
    let issue = shalt_core::jobs::summarize_jobs(&proj_jobs, paused).issue;
    let focus = shalt_core::tokens::work_focus(
        &board,
        &led,
        &q.jobs,
        id,
        board.active_sprint().map(|s| s.id.as_str()),
    );
    let command = shalt_core::tokens::command_center(&board, &q.jobs, &led, id, None);
    let gate = shalt_core::work_gate(&path).to_string();
    Json(ProjectNowView {
        id: id.to_string(),
        live,
        paused,
        yolo: pref.map(|p| p.yolo_mode_enum().asks_all()).unwrap_or(false),
        yolo_mode: pref.map(|p| p.yolo_mode_enum().as_str().to_string()).unwrap_or_else(|| "off".into()),
        pause_reason: pref.map(|p| p.pause_reason.clone()).unwrap_or_default(),
        notice: pref.map(|p| p.notice.clone()).unwrap_or_default(),
        running,
        summary: serde_json::to_value(led.summary()).unwrap_or_default(),
        agents: agent_views(&q, id, 8),
        command,
        issue,
        focus,
        focus_journey: board.focus_journey.clone(),
        journey_tests: {
            let features = shalt_core::load_specs(&path.join("spec"), false).unwrap_or_default();
            let defs = shalt_core::load_step_defs(&path);
            shalt_core::journey_tests(&features, &defs)
        },
        gate: gate.clone(),
        resume: q.resume_target(id).map(|j| ResumeView {
            id: j.id.clone(),
            kind: format!("{:?}", j.kind).to_lowercase(),
            status: format!("{:?}", j.status).to_lowercase(),
        }),
        harness_error: led.display_harness_error(),
        doc: project_doc(&path, &led, &gate),
    })
}

async fn api_project(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Query(q): Query<ProjectQuery>,
) -> impl IntoResponse {
    if q.now.as_deref() == Some("1") {
        return api_project_now(&app, &id).into_response();
    }
    let lite = q.lite.as_deref() == Some("1");
    let mut org = Org::load();
    if org.explain_pauses() {
        let _ = org.save();
    }
    let pref = org.get(&id);
    let name = pref.map(|p| p.name.clone()).unwrap_or_else(|| id.clone());
    let path_s = pref.map(|p| p.path.clone()).unwrap_or_default();
    let path = pref
        .map(|p| PathBuf::from(&p.path))
        .unwrap_or_else(|| app.root.clone());
    let mut led = Ledger::load(&path.join(".shalt/ledger.json")).unwrap_or_default();
    let features = load_specs(&path.join("spec"), false).unwrap_or_default();
    led.sync_spec(&features);
    let mut board = Board::load(&path.join(".shalt/board.json"));
    board.sync_new_rids(&features);
    board.sync_epics(&features);
    let stories = features
        .iter()
        .map(|f| StoryView {
            file: f.file.clone(),
            name: f.name.clone(),
            narrative: f.story().one_line(),
            tasks: f
                .scenarios
                .iter()
                .map(|s| {
                    let ent = s.rid.as_ref().and_then(|r| led.entries.get(r));
                    TaskView {
                        rid: s.rid.clone(),
                        name: s.name.clone(),
                        status: ent
                            .map(|e| e.status.clone())
                            .unwrap_or_else(|| "pending".into()),
                        holdout: s.is_holdout(),
                        failure: ent.and_then(|e| e.failure.clone()).filter(|f| !f.is_empty()),
                    }
                })
                .collect(),
        })
        .collect();
    let q = JobQueue::load();
    let running = q
        .jobs
        .iter()
        .filter(|j| {
            j.project_id == id && matches!(j.status, JobStatus::Running | JobStatus::Pending)
        })
        .count();
    let live = q.jobs.iter().any(|j| {
        j.project_id == id
            && matches!(
                j.status,
                JobStatus::Running | JobStatus::Waiting | JobStatus::Pending | JobStatus::Paused
            )
    });
    let paused = pref.map(|p| p.paused).unwrap_or(false);
    let progress = shalt_core::org::progress_for(&id);
    if shalt_core::alloc::prepare_board(&mut board, &led, &q.jobs, &id, &features) {
        let _ = board.save(&path.join(".shalt/board.json"));
    }
    let incoming = Draft::for_project(&path.join("spec"), &q.jobs, &id);
    let mut queue: Vec<BoardRow> = board
        .items
        .iter()
        .map(|it| {
            let e = led.entries.get(&it.rid);
            BoardRow {
                rid: it.rid.clone(),
                name: e
                    .map(|e| e.name.clone())
                    .filter(|s| !s.is_empty())
                    .or_else(|| {
                        features
                            .iter()
                            .flat_map(|f| f.scenarios.iter())
                            .find(|s| s.rid.as_deref() == Some(it.rid.as_str()))
                            .map(|s| s.name.clone())
                    })
                    .unwrap_or_else(|| it.rid.clone()),
                rank: it.rank,
                status: e
                    .map(|e| e.status.clone())
                    .unwrap_or_else(|| "pending".into()),
                sprint: it
                    .sprint_id
                    .clone()
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "backlog".into()),
                estimate: it.token_estimate,
                forecast_secs: it.time_estimate_secs,
                spent_tokens: q
                    .jobs
                    .iter()
                    .filter(|j| j.project_id == id && j.rid == it.rid)
                    .map(shalt_core::tokens::job_tokens)
                    .sum(),
                spent_secs: q
                    .jobs
                    .iter()
                    .filter(|j| j.project_id == id && j.rid == it.rid)
                    .map(shalt_core::tokens::job_secs)
                    .sum(),
                epic: e.map(|e| e.epic.clone()).unwrap_or_default(),
                backend: it.backend.clone(),
                model: it.model.clone(),
                goal_id: it.goal_id.clone().unwrap_or_default(),
                milestone_id: it.milestone_id.clone().unwrap_or_default(),
                file: e.map(|e| e.feature_file.clone()).unwrap_or_default(),
            }
        })
        .collect();
    queue.sort_by_key(|r| r.rank);
    let agents = agent_views(&q, &id, 20);
    let retro_id = board
        .active_sprint()
        .or_else(|| board.sprints.iter().rev().find(|s| s.closed_at.is_some()))
        .map(|s| s.id.clone());
    let retro = retro_id
        .as_ref()
        .and_then(|sid| shalt_core::tokens::retro(&board, &q.jobs, &id, sid));
    let suggest = shalt_core::tokens::suggest_estimate(&board);
    let sprints = board.sprints.clone();
    let plan_fallback = q
        .jobs
        .iter()
        .rev()
        .find(|j| j.project_id == id && j.kind == JobKind::Author)
        .map(|j| j.prompt.clone())
        .unwrap_or_default();
    let plan = shalt_core::talk::load_plan(&path, &plan_fallback);
    let command = shalt_core::tokens::command_center(&board, &q.jobs, &led, &id, None);
    let proj_jobs: Vec<_> = q.jobs.iter().filter(|j| j.project_id == id).collect();
    let issue = shalt_core::jobs::summarize_jobs(&proj_jobs, paused).issue;
    let focus = shalt_core::tokens::work_focus(
        &board,
        &led,
        &q.jobs,
        &id,
        board.active_sprint().map(|s| s.id.as_str()),
    );
    let work_map = {
        let sprint = board.active_sprint().map(|s| s.id.clone());
        shalt_core::work_map(&board, &led, &q.jobs, &id, sprint.as_deref())
    };
    let gate = shalt_core::work_gate(&path).to_string();
    let doc = project_doc(&path, &led, &gate);
    let entries: Vec<_> = led.entries.values().cloned().collect();
    let diagrams = if lite {
        shalt_core::Diagrams {
            usecase: String::new(),
            breakdown: String::new(),
            pipeline: String::new(),
            work: String::new(),
            flow: String::new(),
        }
    } else {
        shalt_core::project_diagrams(&entries, &work_map, &gate, &features)
    };
    let films = shalt_core::films(&path, &features, &led);
    let journal = shalt_core::Journal::load(&path);
    let focus_journey = board.focus_journey.clone();
    let journey_tests = {
        let defs = shalt_core::load_step_defs(&path);
        shalt_core::journey_tests(&features, &defs)
    };
    Json(ProjectView {
        id: id.clone(),
        name,
        path: path_s,
        stories,
        board,
        summary: serde_json::to_value(led.summary()).unwrap_or_default(),
        draft: incoming.view(),
        live,
        paused,
        yolo: pref.map(|p| p.yolo_mode_enum().asks_all()).unwrap_or(false),
        yolo_mode: pref.map(|p| p.yolo_mode_enum().as_str().to_string()).unwrap_or_else(|| "off".into()),
        pause_reason: pref.map(|p| p.pause_reason.clone()).unwrap_or_default(),
        notice: pref.map(|p| p.notice.clone()).unwrap_or_default(),
        running,
        progress,
        talk: shalt_core::talk::SpecTalk::load(&path),
        work_started: shalt_core::talk::work_started(&path),
        queue,
        agents,
        sprints,
        retro,
        suggest,
        plan,
        command,
        roster: shalt_core::agent_roster(),
        issue,
        focus,
        stack: {
            shalt_core::Config::load(&path)
                .map(|c| c.stack)
                .unwrap_or_default()
        },
        stack_chosen: shalt_core::config::stack_is_set(
            &shalt_core::Config::load(&path)
                .map(|c| c.stack)
                .unwrap_or_default(),
        ),
        gate,
        stack_note: shalt_core::Config::load(&path)
            .ok()
            .and_then(|c| shalt_core::config::stack_support_note(&c.stack).map(|s| s.to_string()))
            .unwrap_or_default(),
        stacks: shalt_core::stack_choices(),
        steps_dir: {
            let steps = shalt_core::Config::load(&path)
                .map(|c| c.steps)
                .unwrap_or_else(|_| "tests".into());
            if steps.is_empty() {
                "tests".into()
            } else {
                steps
            }
        },
        src_dir: {
            let src = shalt_core::Config::load(&path)
                .map(|c| c.src)
                .unwrap_or_else(|_| "src".into());
            if src.is_empty() {
                "src".into()
            } else {
                src
            }
        },
        parallel: shalt_core::parallel_view(&q.jobs),
        work_map,
        diagrams,
        films,
        journey_tests,
        focus_journey,
        kit: shalt_core::load_kit(&path),
        interview: shalt_core::load_interview(&path),
        product_look: shalt_core::has_final_look(&path),
        markup: shalt_core::markup_enabled(&path),
        asks: q.asks_for_project(&id),
        journal,
        harness_error: led.display_harness_error(),
        doc,
        resume: q.resume_target(&id).map(|j| ResumeView {
            id: j.id.clone(),
            kind: format!("{:?}", j.kind).to_lowercase(),
            status: format!("{:?}", j.status).to_lowercase(),
        }),
        test_files: if lite {
            vec![]
        } else {
            zone_files(&path, {
                let steps = shalt_core::Config::load(&path)
                    .map(|c| c.steps)
                    .unwrap_or_else(|_| "tests".into());
                if steps.is_empty() {
                    "tests".into()
                } else {
                    steps
                }
            })
        },
        src_files: if lite {
            vec![]
        } else {
            zone_files(&path, {
                let src = shalt_core::Config::load(&path)
                    .map(|c| c.src)
                    .unwrap_or_else(|_| "src".into());
                if src.is_empty() {
                    "src".into()
                } else {
                    src
                }
            })
        },
    })
    .into_response()
}

#[derive(serde::Deserialize)]
struct ProjectAction {
    #[serde(default)]
    action: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    file: Option<String>,
    #[serde(default)]
    line: Option<usize>,
    #[serde(default)]
    steps: Option<Vec<String>>,
    #[serde(default)]
    holdout: Option<bool>,
    #[serde(default)]
    rid: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    tokens: Option<i64>,
    #[serde(default)]
    sprint: Option<String>,
    #[serde(default)]
    epic: Option<String>,
    #[serde(default)]
    backend: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    job: Option<String>,
    #[serde(default)]
    pool: Option<Vec<String>>,
    #[serde(default)]
    prefer: Option<String>,
    #[serde(default)]
    stack: Option<String>,
    #[serde(default)]
    goal: Option<String>,
    #[serde(default)]
    milestone: Option<String>,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    yolo: Option<bool>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    question: Option<String>,
    #[serde(default)]
    post: Option<String>,
    #[serde(default)]
    parent: Option<String>,
    #[serde(default)]
    markup: Option<shalt_core::Markup>,
    #[serde(default)]
    interview: Option<shalt_core::Interview>,
}

fn project_spec_dir(id: &str) -> Result<PathBuf, String> {
    let org = Org::load();
    let p = org.get(id).ok_or_else(|| format!("no project {id}"))?;
    Ok(PathBuf::from(&p.path).join("spec"))
}

fn sync_job_draft_file(project_id: &str, file: &str, body: Option<&str>) {
    let key = if file.starts_with("spec/") {
        file.to_string()
    } else {
        format!("spec/{file}")
    };
    let mut q = JobQueue::load();
    let mut changed = false;
    for j in &mut q.jobs {
        if j.project_id != project_id {
            continue;
        }
        changed = true;
        if let Some(b) = body {
            j.draft.put(&key, b);
        } else {
            j.draft.files.remove(&key);
        }
    }
    if changed {
        let _ = q.save();
    }
}

fn restart_after_edit(id: &str) {
    let org = Org::load();
    let Some(p) = org.get(id) else {
        return;
    };
    let root = PathBuf::from(&p.path);
    if shalt_core::talk::work_started(&root) {
        let mut q = JobQueue::load();
        q.abandon_for_restart(id);
        let _ = q.save();
    }
}

fn reveal_in_project(root: &std::path::Path, rel: &str) -> Result<String, String> {
    let root = root
        .canonicalize()
        .map_err(|e| format!("{}: {e}", root.display()))?;
    let rel = rel.trim().trim_start_matches("./").trim_start_matches('/');
    if rel.contains("..") {
        return Err("path must stay in the project".into());
    }
    let candidate = if rel.is_empty() {
        root.clone()
    } else {
        root.join(rel)
    };
    let shown = if candidate.exists() {
        candidate
            .canonicalize()
            .map_err(|e| format!("{}: {e}", candidate.display()))?
    } else if let Some(parent) = candidate.parent() {
        if parent.exists() {
            parent
                .canonicalize()
                .unwrap_or_else(|_| parent.to_path_buf())
        } else {
            root.clone()
        }
    } else {
        root.clone()
    };
    if !shown.starts_with(&root) {
        return Err("path must stay in the project".into());
    }
    let status = if cfg!(target_os = "macos") {
        Command::new("open")
            .args(["-R", &shown.display().to_string()])
            .status()
    } else if cfg!(target_os = "windows") {
        Command::new("explorer")
            .args(["/select,", &shown.display().to_string()])
            .status()
    } else {
        let dir = if shown.is_dir() {
            shown.clone()
        } else {
            shown.parent().unwrap_or(&shown).to_path_buf()
        };
        Command::new("xdg-open").arg(&dir).status()
    };
    match status {
        Ok(s) if s.success() => Ok(shown.display().to_string()),
        Ok(s) => Err(format!("could not reveal ({s})")),
        Err(e) => Err(e.to_string()),
    }
}

fn pause_project_work(id: &str) -> bool {
    pause_project_work_reason(id, Some(shalt_core::org::YOU_PAUSED))
}

fn pause_project_work_reason(id: &str, reason: Option<&str>) -> bool {
    let mut org = Org::load();
    if !org.pause(id, true, reason) {
        return false;
    }
    let _ = org.save();
    let mut q = JobQueue::load();
    let _ = q.pause_project(id);
    let _ = q.save();
    true
}

fn play_project_work(app: &App, id: &str) -> Result<(), String> {
    let org = Org::load();
    let Some(pref) = org.get(id) else {
        return Err(format!("no such project {id}"));
    };
    let root = PathBuf::from(&pref.path);
    let mut q = JobQueue::load();
    if q.reopen_cut_short_author(id).is_some() {
        let _ = q.save();
    }
    if !q.authoring_open(id)
        && shalt_core::next_stage(&root) == shalt_core::Stage::Language
    {
        return Err("Pick a build language first, then Play writes tests.".into());
    }
    shalt_core::claim_play(id)?;
    let mut q = JobQueue::load();
    let cap = shalt_core::Capacity::load();
    let mut ids = q.resumable_for_project(id);
    if q.authoring_open(id) {
        ids.retain(|jid| {
            q.get(jid)
                .map(|j| j.kind == JobKind::Author)
                .unwrap_or(false)
        });
    }
    let mut started = 0usize;
    for jid in ids {
        let Some(j) = q.get(&jid).cloned() else {
            continue;
        };
        if !shalt_core::can_admit(&j, &q.jobs, &cap).ok() {
            continue;
        }
        q.set_status(&jid, JobStatus::Running);
        q.append(&jid, "playing — model is on this project");
        let _ = q.save();
        if !app.live.lock().unwrap().contains(&jid) {
            spawn_job(app, jid);
        }
        started += 1;
    }
    if JobQueue::load().authoring_open(id) {
        return Ok(());
    }
    if started == 0 {
        match shalt_core::continue_project(id) {
            Ok(Some(job)) => spawn_job(app, job.id),
            Ok(None) => {}
            Err(e) => return Err(e),
        }
    }
    if JobQueue::load().authoring_open(id) {
        return Ok(());
    }
    loop {
        match shalt_core::continue_project(id) {
            Ok(Some(job)) => spawn_job(app, job.id),
            Ok(None) => break,
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

async fn api_project_action(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Json(body): Json<ProjectAction>,
) -> impl IntoResponse {
    let mut org = Org::load();
    match body.action.as_str() {
        "pause" => {
            if !pause_project_work(&id) {
                return (axum::http::StatusCode::NOT_FOUND, "no such project").into_response();
            }
            Json(serde_json::json!({ "ok": true, "paused": true, "id": id })).into_response()
        }
        "switch_model" => {
            let backend = body.backend.unwrap_or_default();
            let model = body.model.unwrap_or_default();
            match shalt_core::switch_play_model(
                &id,
                &backend,
                &model,
                body.job.as_deref(),
                body.rid.as_deref(),
            ) {
                Ok(job) => {
                    if !app.live.lock().unwrap().contains(&job.id)
                        && JobQueue::load().wants_worker(&job.id)
                    {
                        spawn_job(&app, job.id.clone());
                    }
                    Json(serde_json::json!({
                        "ok": true,
                        "paused": false,
                        "id": id,
                        "job": job.id,
                        "backend": job.backend,
                        "model": job.model
                    }))
                    .into_response()
                }
                Err(e) if e.starts_with("no such") => {
                    (axum::http::StatusCode::NOT_FOUND, e).into_response()
                }
                Err(e) => (axum::http::StatusCode::BAD_REQUEST, e).into_response(),
            }
        }
        "correct_ask" => {
            let qtext = body.question.unwrap_or_default();
            let answer = body.content.or(body.message).unwrap_or_default();
            let mut q = JobQueue::load();
            if !q.correct_ask(&id, &qtext, &answer) {
                return (axum::http::StatusCode::BAD_REQUEST, "no matching question").into_response();
            }
            let _ = q.save();
            Json(serde_json::json!({ "ok": true, "asks": q.asks_for_project(&id) })).into_response()
        }
        "yolo" => {
            let mode = body
                .mode
                .as_deref()
                .and_then(shalt_core::YoloMode::parse)
                .or_else(|| body.yolo.map(|on| if on { shalt_core::YoloMode::All } else { shalt_core::YoloMode::Off }))
                .unwrap_or_else(|| {
                    org.get(&id)
                        .map(|p| p.yolo_mode_enum().next())
                        .unwrap_or(shalt_core::YoloMode::All)
                });
            if !org.set_yolo_mode(&id, mode) {
                return (axum::http::StatusCode::NOT_FOUND, "no such project").into_response();
            }
            let _ = org.save();
            let mut adopted = Vec::new();
            if mode.asks_all() {
                let mut q = JobQueue::load();
                adopted = q.adopt_guesses_for_project(&id);
                for jid in &adopted {
                    q.append(jid, "yolo: took the guess and continued");
                }
                let _ = q.save();
                for jid in &adopted {
                    if !app.live.lock().unwrap().contains(jid) {
                        spawn_job(&app, jid.clone());
                    }
                }
            }
            Json(serde_json::json!({
                "ok": true,
                "id": id,
                "yolo": mode.asks_all(),
                "yolo_mode": mode.as_str(),
                "adopted": adopted
            })).into_response()
        }
        "play" => match play_project_work(&app, &id) {
            Ok(()) => Json(serde_json::json!({ "ok": true, "paused": false, "id": id })).into_response(),
            Err(e) if e.starts_with("no such") => {
                (axum::http::StatusCode::NOT_FOUND, e).into_response()
            }
            Err(e) if e.contains("language") => {
                (axum::http::StatusCode::CONFLICT, e).into_response()
            }
            Err(e) => (axum::http::StatusCode::BAD_REQUEST, e).into_response(),
        },
        "reveal" => {
            let org = Org::load();
            let Some(pref) = org.get(&id) else {
                return (axum::http::StatusCode::NOT_FOUND, "no project").into_response();
            };
            let root = PathBuf::from(&pref.path);
            let steps = shalt_core::Config::load(&root)
                .map(|c| c.steps)
                .unwrap_or_else(|_| "tests".into());
            let rel = body
                .file
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(steps);
            match reveal_in_project(&root, &rel) {
                Ok(shown) => Json(serde_json::json!({ "ok": true, "path": shown })).into_response(),
                Err(e) => (axum::http::StatusCode::BAD_REQUEST, e).into_response(),
            }
        }
        "focus_journey" => {
            let org = Org::load();
            let Some(pref) = org.get(&id) else {
                return (axum::http::StatusCode::NOT_FOUND, "no such project").into_response();
            };
            let root = PathBuf::from(&pref.path);
            let mut board = Board::load(&root.join(".shalt/board.json"));
            let want = body
                .epic
                .unwrap_or_default()
                .trim()
                .to_string();
            board.focus_journey = want.clone();
            if let Err(e) = board.save(&root.join(".shalt/board.json")) {
                return (axum::http::StatusCode::BAD_REQUEST, e.to_string()).into_response();
            }
            Json(serde_json::json!({ "ok": true, "focus_journey": want })).into_response()
        }
        "dismiss_notice" => {
            if !org.clear_notice(&id) {
                return (axum::http::StatusCode::NOT_FOUND, "no such project").into_response();
            }
            let _ = org.save();
            Json(serde_json::json!({ "ok": true, "id": id })).into_response()
        }
        "stack" => {
            let stack = body.stack.unwrap_or_default();
            match shalt_core::restack_project(&id, &stack) {
                Ok(r) => Json(r).into_response(),
                Err(e) if e.contains("Pause Play") => {
                    (axum::http::StatusCode::CONFLICT, e).into_response()
                }
                Err(e) if e.starts_with("no project") => {
                    (axum::http::StatusCode::NOT_FOUND, e).into_response()
                }
                Err(e) => (axum::http::StatusCode::BAD_REQUEST, e).into_response(),
            }
        }
        "rename" => {
            let name = body.name.unwrap_or_default();
            if !org.rename(&id, &name) {
                return (axum::http::StatusCode::BAD_REQUEST, "could not rename").into_response();
            }
            let _ = org.save();
            Json(org.get(&id).cloned()).into_response()
        }
        "remove" => {
            if !org.remove(&id) {
                return (axum::http::StatusCode::NOT_FOUND, "no such project").into_response();
            }
            let _ = org.save();
            Json(serde_json::json!({ "ok": true, "id": id })).into_response()
        }
        "estimate" | "sprint_open" | "sprint_close" | "sprint_assign" | "assign_agent"
        | "epic_estimate" | "epic_agent" | "pool_set" | "allocate" | "reallocate"
        | "goal_set" | "milestone_set" | "place" => {
            let spec_dir = match project_spec_dir(&id) {
                Ok(p) => p,
                Err(e) => return (axum::http::StatusCode::NOT_FOUND, e).into_response(),
            };
            let root = spec_dir.parent().unwrap_or(spec_dir.as_path()).to_path_buf();
            let board_path = root.join(".shalt/board.json");
            let mut board = Board::load(&board_path);
            match body.action.as_str() {
                "estimate" => {
                    let rid = body.rid.unwrap_or_default();
                    let n = body.tokens.unwrap_or(0);
                    if rid.is_empty() || !board.set_estimate(&rid, n) {
                        return (axum::http::StatusCode::BAD_REQUEST, "unknown ticket").into_response();
                    }
                    let _ = board.save(&board_path);
                    Json(serde_json::json!({ "ok": true, "rid": rid, "estimate": n })).into_response()
                }
                "sprint_open" => {
                    let q = JobQueue::load();
                    let led = Ledger::load(&root.join(".shalt/ledger.json")).unwrap_or_default();
                    let last = board
                        .sprints
                        .iter()
                        .rev()
                        .find(|s| s.closed_at.is_some())
                        .and_then(|s| shalt_core::tokens::retro(&board, &q.jobs, &id, &s.id));
                    if let Some(r) = last {
                        match shalt_core::sprint::apply_next_sprint(&root, &mut board, &led, &r, &[]) {
                            Ok(s) => {
                                let _ = board.save(&board_path);
                                return Json(s).into_response();
                            }
                            Err(e) => {
                                return (axum::http::StatusCode::BAD_REQUEST, e).into_response();
                            }
                        }
                    }
                    let title = body.name.unwrap_or_default();
                    let s = board.open_sprint(&title);
                    let _ = shalt_core::sprint::seat_sprint_slice(&mut board, &led);
                    let _ = board.save(&board_path);
                    Json(s).into_response()
                }
                "sprint_close" => {
                    let q = JobQueue::load();
                    let led = Ledger::load(&root.join(".shalt/ledger.json")).unwrap_or_default();
                    match shalt_core::sprint::close_and_learn(
                        &root, &mut board, &q.jobs, &led, &id,
                    ) {
                        Ok(r) => {
                            let yolo = shalt_core::org::Org::yolo_plan(&id);
                            if yolo {
                                let _ = shalt_core::sprint::apply_next_sprint(
                                    &root, &mut board, &led, &r, &[],
                                );
                            }
                            let _ = board.save(&board_path);
                            Json(r).into_response()
                        }
                        Err(e) => (axum::http::StatusCode::BAD_REQUEST, e).into_response(),
                    }
                }
                "assign_agent" => {
                    let backend = body.backend.unwrap_or_default();
                    let model = body.model.unwrap_or_default();
                    if let Some(epic) = body.epic.filter(|s| !s.is_empty()) {
                        if !board.set_epic_agent(&epic, &backend, &model) {
                            return (axum::http::StatusCode::BAD_REQUEST, "unknown epic").into_response();
                        }
                        let _ = board.save(&board_path);
                        return Json(serde_json::json!({ "ok": true, "epic": epic, "backend": backend, "model": model })).into_response();
                    }
                    let rid = body.rid.unwrap_or_default();
                    if rid.is_empty() || !board.set_item_agent(&rid, &backend, &model) {
                        return (axum::http::StatusCode::BAD_REQUEST, "unknown ticket").into_response();
                    }
                    let _ = board.save(&board_path);
                    Json(serde_json::json!({ "ok": true, "rid": rid, "backend": backend, "model": model })).into_response()
                }
                "epic_estimate" => {
                    let epic = body.epic.unwrap_or_default();
                    let n = body.tokens.unwrap_or(0);
                    if epic.is_empty() || !board.set_epic_estimate(&epic, n) {
                        return (axum::http::StatusCode::BAD_REQUEST, "unknown epic").into_response();
                    }
                    let _ = board.save(&board_path);
                    Json(serde_json::json!({ "ok": true, "epic": epic, "estimate": n })).into_response()
                }
                "epic_agent" => {
                    let epic = body.epic.unwrap_or_default();
                    let backend = body.backend.unwrap_or_default();
                    let model = body.model.unwrap_or_default();
                    if epic.is_empty() || !board.set_epic_agent(&epic, &backend, &model) {
                        return (axum::http::StatusCode::BAD_REQUEST, "unknown epic").into_response();
                    }
                    let _ = board.save(&board_path);
                    Json(serde_json::json!({ "ok": true, "epic": epic, "backend": backend, "model": model })).into_response()
                }
                "pool_set" => {
                    let mut slots = Vec::new();
                    for spec in body.pool.unwrap_or_default() {
                        let (backend, model) = shalt_core::tokens::parse_agent(&spec);
                        if !backend.is_empty() {
                            slots.push(shalt_core::board::PoolSlot { backend, model });
                        }
                    }
                    if slots.is_empty() {
                        slots = shalt_core::alloc::default_pool();
                    }
                    board.pool = slots;
                    if let Some(p) = body.prefer.filter(|s| matches!(s.as_str(), "cheap" | "balanced" | "fast")) {
                        board.prefer = p;
                    } else if board.prefer.is_empty() {
                        board.prefer = "balanced".into();
                    }
                    let q = JobQueue::load();
                    let led = Ledger::load(&root.join(".shalt/ledger.json")).unwrap_or_default();
                    let features = load_specs(&root.join("spec"), false).unwrap_or_default();
                    let _ = shalt_core::alloc::prepare_board(
                        &mut board,
                        &led,
                        &q.jobs,
                        &id,
                        &features,
                    );
                    let _ = board.save(&board_path);
                    Json(serde_json::json!({ "ok": true, "pool": board.pool, "prefer": board.prefer })).into_response()
                }
                "allocate" | "reallocate" => {
                    let q = JobQueue::load();
                    let n = shalt_core::alloc::reallocate_now(
                        &mut board,
                        &Ledger::load(&root.join(".shalt/ledger.json")).unwrap_or_default(),
                        &q.jobs,
                        &id,
                    )
                    .len();
                    let _ = board.save(&board_path);
                    Json(serde_json::json!({ "ok": true, "changed": n, "prefer": board.prefer })).into_response()
                }
                "goal_set" => {
                    let title = body.name.or(body.content).unwrap_or_default();
                    let g = board.upsert_goal(body.goal.as_deref(), &title);
                    let _ = board.save(&board_path);
                    Json(g).into_response()
                }
                "milestone_set" => {
                    let title = body.name.or(body.content).unwrap_or_default();
                    let m = board.upsert_milestone(
                        body.milestone.as_deref(),
                        &title,
                        body.target.as_deref(),
                    );
                    let _ = board.save(&board_path);
                    Json(m).into_response()
                }
                "place" => {
                    let rid = body.rid.unwrap_or_default();
                    if rid.is_empty() {
                        return (axum::http::StatusCode::BAD_REQUEST, "unknown ticket").into_response();
                    }
                    let goal = body.goal.clone();
                    let milestone = body.milestone.clone();
                    if !board.assign(&rid, goal.clone(), milestone.clone(), None) {
                        return (axum::http::StatusCode::BAD_REQUEST, "unknown ticket").into_response();
                    }
                    let _ = board.save(&board_path);
                    Json(serde_json::json!({ "ok": true, "rid": rid, "goal": goal, "milestone": milestone })).into_response()
                }
                _ => {
                    let rid = body.rid.unwrap_or_default();
                    let sp = body.sprint.unwrap_or_default();
                    let val = if sp.is_empty() || sp == "backlog" {
                        None
                    } else {
                        Some(sp)
                    };
                    if let Some(it) = board.items.iter_mut().find(|i| i.rid == rid) {
                        it.sprint_id = val;
                    } else {
                        return (axum::http::StatusCode::BAD_REQUEST, "unknown ticket").into_response();
                    }
                    let _ = board.save(&board_path);
                    Json(serde_json::json!({ "ok": true, "rid": rid })).into_response()
                }
            }
        }
        "save_plan" => {
            if shalt_core::talk::edits_need_pause(&id) {
                return (
                    axum::http::StatusCode::CONFLICT,
                    "Pause Play first, then edit. Play again to restart.",
                )
                    .into_response();
            }
            let org = Org::load();
            let Some(pref) = org.get(&id) else {
                return (axum::http::StatusCode::NOT_FOUND, "no project").into_response();
            };
            let root = PathBuf::from(&pref.path);
            let text = body.content.unwrap_or_default();
            match shalt_core::talk::save_plan(&root, &text) {
                Ok(()) => {
                    restart_after_edit(&id);
                    Json(serde_json::json!({ "ok": true })).into_response()
                }
                Err(e) => (axum::http::StatusCode::BAD_REQUEST, e.to_string()).into_response(),
            }
        }
        "save_file" => {
            if shalt_core::talk::edits_need_pause(&id) {
                return (
                    axum::http::StatusCode::CONFLICT,
                    "Pause Play first, then edit. Play again to restart.",
                )
                    .into_response();
            }
            let spec_dir = match project_spec_dir(&id) {
                Ok(p) => p,
                Err(e) => return (axum::http::StatusCode::NOT_FOUND, e).into_response(),
            };
            let file = body.file.unwrap_or_default();
            let content = body.content.unwrap_or_default();
            match spec::put_spec_file(&spec_dir, &file, &content) {
                Ok(rel) => {
                    let root = spec_dir.parent().unwrap_or(spec_dir.as_path()).to_path_buf();
                    let _ = spec::stamp_rids(&spec_dir);
                    let stamped = std::fs::read_to_string(spec_dir.join(&rel)).ok();
                    sync_job_draft_file(&id, &rel, stamped.as_deref().or(Some(&content)));
                    if let Ok(features) = load_specs(&spec_dir, false) {
                        let mut led = Ledger::load(&root.join(".shalt/ledger.json")).unwrap_or_default();
                        led.sync_spec(&features);
                        let _ = led.save(&root.join(".shalt/ledger.json"));
                        let mut board = Board::load(&root.join(".shalt/board.json"));
                        board.sync_new_rids(&features);
                        board.sync_epics(&features);
                        let qnow = JobQueue::load();
                        shalt_core::alloc::allocate_unassigned_now(
                            &mut board,
                            &led,
                            &qnow.jobs,
                            &id,
                        );
                        let _ = board.save(&root.join(".shalt/board.json"));
                    }
                    let warn = match shalt_core::spec::parse_text(&content, &rel) {
                        Ok(None) => Some("file is empty or not a Feature"),
                        Err(_) => Some("Spec did not parse; saved anyway so you can fix it"),
                        Ok(Some(_)) => None,
                    };
                    restart_after_edit(&id);
                    Json(serde_json::json!({ "ok": true, "file": format!("spec/{rel}"), "warn": warn })).into_response()
                }
                Err(e) => (axum::http::StatusCode::BAD_REQUEST, e).into_response(),
            }
        }
        "save_markup" => {
            let org = Org::load();
            let Some(pref) = org.get(&id) else {
                return (axum::http::StatusCode::NOT_FOUND, "no project").into_response();
            };
            let root = PathBuf::from(&pref.path);
            let mut m = body.markup.unwrap_or_default();
            if m.rid.trim().is_empty() {
                m.rid = body.rid.unwrap_or_default();
            }
            match shalt_core::save_markup(&root, m) {
                Ok(saved) => Json(serde_json::json!({ "ok": true, "markup": saved })).into_response(),
                Err(e) => (axum::http::StatusCode::BAD_REQUEST, e).into_response(),
            }
        }
        "save_interview" => {
            let org = Org::load();
            let Some(pref) = org.get(&id) else {
                return (axum::http::StatusCode::NOT_FOUND, "no project").into_response();
            };
            let root = PathBuf::from(&pref.path);
            let iv = body.interview.unwrap_or_default();
            match shalt_core::save_interview(&root, iv) {
                Ok(saved) => Json(serde_json::json!({ "ok": true, "interview": saved })).into_response(),
                Err(e) => (axum::http::StatusCode::BAD_REQUEST, e).into_response(),
            }
        }
        "journal_comment" => {
            let org = Org::load();
            let Some(pref) = org.get(&id) else {
                return (axum::http::StatusCode::NOT_FOUND, "no project").into_response();
            };
            let root = PathBuf::from(&pref.path);
            let post = body.post.or(body.rid).unwrap_or_default();
            let parent = body.parent.unwrap_or_default();
            let text = body
                .message
                .or(body.content)
                .unwrap_or_default();
            let by = std::env::var("USER").unwrap_or_else(|_| "You".into());
            match shalt_core::journal::comment(&root, &post, &parent, &by, &text) {
                Ok(c) => {
                    let journal = shalt_core::Journal::load(&root);
                    Json(serde_json::json!({ "ok": true, "comment": c, "journal": journal })).into_response()
                }
                Err(e) if e.starts_with("no post") || e.starts_with("no such") => {
                    (axum::http::StatusCode::NOT_FOUND, e).into_response()
                }
                Err(e) => (axum::http::StatusCode::BAD_REQUEST, e).into_response(),
            }
        }
        "chat" => {
            let text = body.message.unwrap_or_default();
            match tokio::task::spawn_blocking(move || shalt_core::revise_spec(&id, &text)).await {
                Ok(Ok(talk)) => Json(talk).into_response(),
                Ok(Err(e)) => {
                    let code = if e.contains("pause this project") {
                        axum::http::StatusCode::CONFLICT
                    } else if e.contains("empty") {
                        axum::http::StatusCode::BAD_REQUEST
                    } else {
                        axum::http::StatusCode::BAD_GATEWAY
                    };
                    (code, e).into_response()
                }
                Err(e) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
            }
        }
        "delete_task" | "edit_task" | "promote_task" => {
            if body.action != "promote_task" && shalt_core::talk::edits_need_pause(&id) {
                return (
                    axum::http::StatusCode::CONFLICT,
                    "Pause Play first, then edit. Play again to restart.",
                )
                    .into_response();
            }
            let spec_dir = match project_spec_dir(&id) {
                Ok(p) => p,
                Err(e) => return (axum::http::StatusCode::NOT_FOUND, e).into_response(),
            };
            let file = body.file.unwrap_or_default();
            let line = body.line.unwrap_or(0);
            if file.is_empty() || line == 0 {
                return (axum::http::StatusCode::BAD_REQUEST, "file and line required").into_response();
            }
            let root = spec_dir.parent().unwrap_or(spec_dir.as_path()).to_path_buf();
            let result = match body.action.as_str() {
                "delete_task" => {
                    match spec::drop_scenario_blocks(&spec_dir, &[(file.clone(), line)]) {
                        Ok(n) if n > 0 => {
                            if let Some(rid) = body.rid.filter(|s| s.starts_with("S-")) {
                                let mut board = Board::load(&root.join(".shalt/board.json"));
                                board.unschedule(&rid);
                                let _ = board.save(&root.join(".shalt/board.json"));
                            }
                            let gone = !spec_dir.join(&file).exists();
                            let body = if gone {
                                None
                            } else {
                                std::fs::read_to_string(spec_dir.join(&file)).ok()
                            };
                            sync_job_draft_file(&id, &file, body.as_deref());
                            Ok(serde_json::json!({ "ok": true, "removed": n }))
                        }
                        Ok(_) => Err("no scenario at that line".into()),
                        Err(e) => Err(e.to_string()),
                    }
                }
                "edit_task" => {
                    let name = body.name.unwrap_or_default();
                    let steps = body.steps.unwrap_or_default();
                    spec::rewrite_scenario(&spec_dir, &file, line, &name, &steps, body.holdout)
                        .map(|_| {
                            let text = std::fs::read_to_string(spec_dir.join(&file)).unwrap_or_default();
                            sync_job_draft_file(&id, &file, Some(&text));
                            serde_json::json!({ "ok": true })
                        })
                }
                _ => {
                    let rid = match spec::stamp_scenario(&spec_dir, &file, line) {
                        Ok(Some(r)) => r,
                        Ok(None) => {
                            return (axum::http::StatusCode::BAD_REQUEST, "could not stamp").into_response();
                        }
                        Err(e) => return (axum::http::StatusCode::BAD_REQUEST, e).into_response(),
                    };
                    let text = std::fs::read_to_string(spec_dir.join(&file)).unwrap_or_default();
                    sync_job_draft_file(&id, &file, Some(&text));
                    let mut board = Board::load(&root.join(".shalt/board.json"));
                    board.promote(&rid);
                    let _ = board.save(&root.join(".shalt/board.json"));
                    Ok(serde_json::json!({ "ok": true, "rid": rid }))
                }
            };
            match result {
                Ok(v) => {
                    if body.action != "promote_task" {
                        restart_after_edit(&id);
                    }
                    Json(v).into_response()
                }
                Err(e) => (axum::http::StatusCode::BAD_REQUEST, e).into_response(),
            }
        }
        other => (axum::http::StatusCode::BAD_REQUEST, format!("unknown action {other}")).into_response(),
    }
}

async fn api_jobs() -> Json<JobQueue> {
    Json(JobQueue::load())
}

async fn api_job(Path(id): Path<String>) -> impl IntoResponse {
    let q = JobQueue::load();
    match q.get(&id) {
        Some(j) => {
            let draft = if let Some(p) = Org::load().get(&j.project_id) {
                Draft::for_project(
                    &PathBuf::from(&p.path).join("spec"),
                    std::slice::from_ref(&j),
                    &j.project_id,
                )
            } else {
                j.draft.clone()
            };
            let issue = matches!(j.status, JobStatus::Failed | JobStatus::Interrupted)
                .then(|| shalt_core::jobs::diagnose(&j));
            let project_gate = Org::load()
                .get(&j.project_id)
                .map(|p| shalt_core::work_gate(std::path::Path::new(&p.path)))
                .unwrap_or("plan");
            let live = q.jobs.iter().any(|x| {
                x.project_id == j.project_id
                    && matches!(
                        x.status,
                        JobStatus::Running
                            | JobStatus::Waiting
                            | JobStatus::Pending
                            | JobStatus::Paused
                    )
            });
            Json(serde_json::json!({
                "job": j,
                "system": author_system_prompt(),
                "user": author_prompt(&j.prompt, &j.turns),
                "draft": draft.view(),
                "issue": issue,
                "pipeline": shalt_core::mermaid_pipeline_at(project_gate),
                "live": live,
            }))
            .into_response()
        }
        None => (axum::http::StatusCode::NOT_FOUND, "no such job").into_response(),
    }
}

#[derive(serde::Deserialize)]
struct JobAction {
    #[serde(default)]
    action: String,
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    answer: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    backend: Option<String>,
    #[serde(default)]
    model: Option<String>,
}

async fn api_job_action(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Json(body): Json<JobAction>,
) -> impl IntoResponse {
    let mut q = JobQueue::load();
    if q.get(&id).is_none() {
        return (axum::http::StatusCode::NOT_FOUND, "no such job").into_response();
    }
    if let Some(p) = body.prompt.as_deref() {
        q.set_prompt(&id, p);
    }
    match body.action.as_str() {
        "pause" => {
            q.set_status(&id, JobStatus::Paused);
        }
        "resume" => {
            q.set_status(&id, JobStatus::Running);
            let live = app.live.lock().unwrap().contains(&id);
            if !live {
                drop(q.save());
                spawn_job(&app, id.clone());
            }
        }
        "cancel" => {
            q.set_status(&id, JobStatus::Interrupted);
        }
        "answer" | "adopt" => {
            let ok = if body.action == "adopt" && body.answer.as_deref().unwrap_or("").trim().is_empty() {
                q.adopt_chat(&id, None)
            } else {
                let a = body
                    .answer
                    .as_deref()
                    .or(body.prompt.as_deref())
                    .unwrap_or("")
                    .trim();
                if a.is_empty() {
                    return (axum::http::StatusCode::BAD_REQUEST, "answer is empty").into_response();
                }
                q.set_answer(&id, a)
            };
            if !ok {
                return (axum::http::StatusCode::BAD_REQUEST, "nothing to add to the spec").into_response();
            }
            let live = app.live.lock().unwrap().contains(&id);
            if !live {
                drop(q.save());
                spawn_job(&app, id.clone());
            }
        }
        "answer_agent" => {
            let backend = body.backend.unwrap_or_default();
            let model = body.model.unwrap_or_default();
            if !q.set_answer_agent(&id, &backend, &model) {
                return (axum::http::StatusCode::NOT_FOUND, "no such job").into_response();
            }
        }
        "decide" | "chat" => {
            let text = body.message.unwrap_or_default();
            let backend = body.backend.clone();
            let model = body.model.clone();
            let decide = body.action == "decide";
            let id = id.clone();
            return match tokio::task::spawn_blocking(move || {
                if decide {
                    shalt_core::decide_on_job(
                        &id,
                        backend.as_deref(),
                        model.as_deref(),
                    )
                } else {
                    shalt_core::chat_on_job_with(
                        &id,
                        &text,
                        backend.as_deref(),
                        model.as_deref(),
                    )
                }
            })
            .await
            {
                Ok(Ok(j)) => Json(serde_json::json!({
                    "ok": true,
                    "id": j.id,
                    "status": format!("{:?}", j.status).to_lowercase(),
                    "kind": shalt_core::jobs::kind_phase(j.kind),
                    "question": j.question,
                    "backend": j.backend,
                    "model": j.model,
                    "answer_backend": j.answer_backend,
                    "answer_model": j.answer_model,
                    "turns": j.turns,
                }))
                .into_response(),
                Ok(Err(e)) => {
                    let code = if e.contains("no job") {
                        axum::http::StatusCode::NOT_FOUND
                    } else if e.contains("empty") || e.contains("no open") {
                        axum::http::StatusCode::BAD_REQUEST
                    } else {
                        axum::http::StatusCode::BAD_GATEWAY
                    };
                    (code, e).into_response()
                }
                Err(e) => (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    e.to_string(),
                )
                    .into_response(),
            };
        }
        "save" | "" => {}
        other => {
            return (axum::http::StatusCode::BAD_REQUEST, format!("unknown action {other}")).into_response();
        }
    }
    let _ = q.save();
    let q = JobQueue::load();
    Json(q.get(&id).cloned()).into_response()
}

#[derive(serde::Deserialize)]
struct Enqueue {
    kind: String,
    project: String,
    #[serde(default)]
    prompt: String,
    #[serde(default)]
    rid: String,
    #[serde(default)]
    backend: String,
    #[serde(default)]
    model: String,
}

async fn api_enqueue(State(app): State<Arc<App>>, Json(body): Json<Enqueue>) -> impl IntoResponse {
    let kind = match body.kind.as_str() {
        "author" => JobKind::Author,
        "design" => JobKind::Design,
        "steps" => JobKind::Steps,
        "build" => JobKind::Build,
        "run" => JobKind::Run,
        "verify" => JobKind::Verify,
        _ => return (axum::http::StatusCode::BAD_REQUEST, "unknown kind").into_response(),
    };
    let mut q = JobQueue::load();
    let mut backend = body.backend.trim().to_string();
    let mut model = body.model.trim().to_string();
    if backend.is_empty() && model.is_empty() {
        if let Some(prev) = q.jobs.iter().rev().find(|j| {
            j.project_id == body.project && (!j.backend.is_empty() || !j.model.is_empty())
        }) {
            backend = prev.backend.clone();
            model = prev.model.clone();
        } else {
            backend = "qwen".into();
        }
    }
    let j = q.enqueue_full(kind, &body.project, &body.prompt, &backend, &model);
    if !body.rid.trim().is_empty() {
        q.set_work(&j.id, "", body.rid.trim());
    }
    let id = j.id.clone();
    let out = q.get(&id).cloned().unwrap_or(j);
    let _ = q.save();
    spawn_job(&app, id);
    Json(out).into_response()
}
