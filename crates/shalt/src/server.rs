use axum::extract::{Path, State};
use axum::response::{Html, IntoResponse, Json};
use axum::routing::{get, post};
use axum::Router;
use serde::Serialize;
use shalt_core::board::Board;
use shalt_core::compose::{execute_author, start_project, ComposeRequest};
use shalt_core::jobs::JobQueue;
use shalt_core::ledger::Ledger;
use shalt_core::list_models;
use shalt_core::org::Org;
use shalt_core::spec::load_specs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

const UI: &str = include_str!("ui.html");

#[derive(Clone)]
struct App {
    root: PathBuf,
}

pub async fn serve(root: PathBuf, port: u16, open: bool) -> Result<(), String> {
    let app = Router::new()
        .route("/", get(index))
        .route("/api/org", get(api_org))
        .route("/api/models", get(api_models))
        .route("/api/compose", post(api_compose))
        .route("/api/project/{id}", get(api_project))
        .route("/api/jobs", get(api_jobs).post(api_enqueue))
        .with_state(Arc::new(App { root }));
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| e.to_string())?;
    if open {
        let url = format!("http://127.0.0.1:{port}/");
        let _ = std::process::Command::new("open").arg(&url).spawn();
    }
    axum::serve(listener, app).await.map_err(|e| e.to_string())
}

async fn index() -> Html<&'static str> {
    Html(UI)
}

#[derive(Serialize)]
struct OrgView {
    name: String,
    projects: Vec<ProjectCard>,
}

#[derive(Serialize)]
struct ProjectCard {
    id: String,
    name: String,
    path: String,
    green: i64,
    total: i64,
    missing: bool,
}

async fn api_org() -> Json<OrgView> {
    let org = Org::load();
    let projects = org
        .projects
        .iter()
        .map(|p| {
            let path = PathBuf::from(&p.path);
            let missing = !path.exists();
            let (green, total) = if missing {
                (0, 0)
            } else {
                let led = Ledger::load(&path.join(".shalt/ledger.json")).unwrap_or_default();
                let s = led.summary();
                (
                    s.get("green").and_then(|v| v.as_i64()).unwrap_or(0),
                    s.get("total").and_then(|v| v.as_i64()).unwrap_or(0)
                        - s.get("orphan").and_then(|v| v.as_i64()).unwrap_or(0),
                )
            };
            ProjectCard {
                id: p.id.clone(),
                name: p.name.clone(),
                path: p.path.clone(),
                green,
                total,
                missing,
            }
        })
        .collect();
    Json(OrgView {
        name: org.name,
        projects,
    })
}

async fn api_models() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "models": list_models() }))
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
}

async fn api_compose(Json(body): Json<ComposeBody>) -> impl IntoResponse {
    match start_project(ComposeRequest {
        prompt: body.prompt,
        backend: body.backend,
        model: body.model,
        name: body.name,
    }) {
        Ok((project, job)) => {
            let job_id = job.id.clone();
            tokio::task::spawn_blocking(move || {
                let _ = execute_author(&job_id);
            });
            Json(serde_json::json!({ "project": project, "job": job })).into_response()
        }
        Err(e) => (axum::http::StatusCode::BAD_REQUEST, e).into_response(),
    }
}

#[derive(Serialize)]
struct ProjectView {
    id: String,
    stories: Vec<StoryView>,
    board: shalt_core::board::Board,
    summary: serde_json::Value,
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
}

async fn api_project(State(app): State<Arc<App>>, Path(id): Path<String>) -> impl IntoResponse {
    let org = Org::load();
    let path = org
        .get(&id)
        .map(|p| PathBuf::from(&p.path))
        .unwrap_or_else(|| app.root.clone());
    let mut led = Ledger::load(&path.join(".shalt/ledger.json")).unwrap_or_default();
    let features = load_specs(&path.join("spec"), false).unwrap_or_default();
    led.sync_spec(&features);
    let mut board = Board::load(&path.join(".shalt/board.json"));
    board.sync_new_rids(&features);
    let stories = features
        .iter()
        .map(|f| StoryView {
            file: f.file.clone(),
            name: f.name.clone(),
            narrative: f.story().one_line(),
            tasks: f
                .scenarios
                .iter()
                .map(|s| TaskView {
                    rid: s.rid.clone(),
                    name: s.name.clone(),
                    status: s
                        .rid
                        .as_ref()
                        .and_then(|r| led.entries.get(r))
                        .map(|e| e.status.clone())
                        .unwrap_or_else(|| "pending".into()),
                    holdout: s.is_holdout(),
                })
                .collect(),
        })
        .collect();
    Json(ProjectView {
        id,
        stories,
        board,
        summary: serde_json::to_value(led.summary()).unwrap_or_default(),
    })
}

async fn api_jobs() -> Json<JobQueue> {
    Json(JobQueue::load())
}

#[derive(serde::Deserialize)]
struct Enqueue {
    kind: String,
    project: String,
}

async fn api_enqueue(Json(body): Json<Enqueue>) -> impl IntoResponse {
    use shalt_core::jobs::JobKind;
    let kind = match body.kind.as_str() {
        "author" => JobKind::Author,
        "steps" => JobKind::Steps,
        "build" => JobKind::Build,
        "run" => JobKind::Run,
        "verify" => JobKind::Verify,
        _ => return (axum::http::StatusCode::BAD_REQUEST, "unknown kind").into_response(),
    };
    let mut q = JobQueue::load();
    let j = q.enqueue(kind, &body.project);
    let _ = q.save();
    Json(j).into_response()
}
