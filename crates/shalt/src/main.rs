use clap::{Parser, Subcommand};
use shalt_core::backends::FixtureBackend;
use shalt_core::board::{verify_drift, Board};
use shalt_core::config::{write_config, Config};
use shalt_core::integrity::audit;
use shalt_core::jobs::{JobKind, JobQueue};
use shalt_core::ledger::{Ledger, GREEN, ORPHAN, PENDING, RED, STALE};
use shalt_core::org::Org;
use shalt_core::roles::run_role;
use shalt_core::runner::{harness_report, run_suite};
use shalt_core::spec::{holdout_rids, load_specs, stamp_rids};
use shalt_core::{Backend, RoleError};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "shalt", version, about = "English → Gherkin → tests → code, with a spec-bound ledger")]
struct Cli {
    #[arg(long, default_value = ".")]
    root: PathBuf,
    #[arg(long, default_value = "fixture")]
    backend: String,
    #[arg(long)]
    fixtures: Option<PathBuf>,
    #[arg(long)]
    model: Option<String>,
    #[arg(long)]
    base_url: Option<String>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    Init {
        #[arg(long, default_value = "python")]
        stack: String,
        #[arg(long, default_value = "")]
        name: String,
    },
    Author { request: String },
    Approve {
        #[arg(long)]
        yes: bool,
        #[arg(long, default_value = "local")]
        by: String,
    },
    Steps,
    Build {
        #[arg(long, default_value_t = 6)]
        max_turns: usize,
        #[arg(long)]
        strict: bool,
    },
    Run,
    Status,
    Verify,
    Tree,
    Stories,
    Org {
        #[command(subcommand)]
        action: OrgCmd,
    },
    Board {
        #[arg(long)]
        project: Option<String>,
        #[command(subcommand)]
        action: Option<BoardCmd>,
    },
    Job {
        #[command(subcommand)]
        action: JobCmd,
    },
    Ui {
        /// Bind this port when starting. Ignored if a shalt ui is already up.
        #[arg(long, global = true)]
        port: Option<u16>,
        #[arg(long, global = true)]
        no_open: bool,
        #[command(subcommand)]
        action: Option<UiAction>,
    },
    Diagrams,
    Dashboard,
    Mutate {
        #[arg(long, default_value = "auto")]
        engine: String,
        #[arg(long, default_value_t = 30)]
        budget: usize,
        #[arg(long, default_value_t = 0)]
        seed: u64,
    },
}

#[derive(Subcommand)]
enum UiAction {
    /// Show the running UI, if any
    Status,
    /// Stop the running UI
    Stop,
    /// Stop the running UI, then start it again
    Restart,
}

#[derive(Subcommand)]
enum OrgCmd {
    List,
    Add { path: PathBuf },
    Remove { id: String },
}

#[derive(Subcommand)]
enum BoardCmd {
    List,
    Unschedule { rid: String },
}

#[derive(Subcommand)]
enum JobCmd {
    List,
    Add {
        kind: String,
        #[arg(long)]
        project: String,
    },
    Show { id: String },
    Pause { id: String },
    Resume { id: String },
}

fn backend(cli: &Cli) -> Result<Box<dyn Backend>, i32> {
    match cli.backend.as_str() {
        "fixture" => {
            let dir = cli.fixtures.clone().ok_or_else(|| {
                eprintln!("--fixtures is required for the fixture backend");
                1
            })?;
            Ok(Box::new(FixtureBackend::new(dir)))
        }
        "grok" | "openai" | "qwen" | "ollama" => {
            match shalt_core::OpenAICompatBackend::from_preset(
                &cli.backend,
                cli.model.as_deref(),
                cli.base_url.as_deref(),
            ) {
                Ok(b) => Ok(Box::new(b)),
                Err(e) => {
                    eprintln!("{e}");
                    Err(1)
                }
            }
        }
        other => {
            eprintln!("unknown backend {other:?}");
            Err(1)
        }
    }
}

fn ledger_path(root: &Path) -> PathBuf {
    root.join(".shalt/ledger.json")
}

fn sync(root: &Path) -> Result<(Ledger, Vec<shalt_core::Feature>), i32> {
    let features = match load_specs(&root.join("spec"), true) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("spec does not parse: {}", e.message());
            return Err(4);
        }
    };
    let mut led = Ledger::load(&ledger_path(root)).unwrap_or_default();
    led.sync_spec(&features);
    let _ = led.save(&ledger_path(root));
    Ok((led, features))
}

fn main() {
    let cli = Cli::parse();
    let code = match run(cli) {
        Ok(c) => c,
        Err(c) => c,
    };
    std::process::exit(code);
}

fn run(cli: Cli) -> Result<i32, i32> {
    let root = cli.root.canonicalize().unwrap_or_else(|_| cli.root.clone());
    match &cli.cmd {
        Cmd::Init { stack, name } => {
            let preset = write_config(&root, stack, name).map_err(|e| {
                eprintln!("{e}");
                1
            })?;
            let cfg = Config::load(&root).unwrap_or_default();
            for d in ["spec", "contract", ".shalt"] {
                std::fs::create_dir_all(root.join(d)).ok();
            }
            std::fs::create_dir_all(root.join(&cfg.steps)).ok();
            std::fs::create_dir_all(root.join(&cfg.src)).ok();
            if stack == "python" {
                std::fs::write(
                    root.join(&cfg.steps).join("conftest.py"),
                    shalt_core::config::python_reporter_template(),
                )
                .ok();
            }
            std::fs::write(
                root.join(".shalt/.gitignore"),
                "stage/\nbackup/\nlast_run.json\nmessages.ndjson\ncucumber.json\n",
            )
            .ok();
            Ledger::default().save(&ledger_path(&root)).ok();
            println!("initialised shalt workspace at {}  ({})", root.display(), preset.label);
            Ok(0)
        }
        Cmd::Author { request } => {
            let mut b = backend(&cli)?;
            let prompt = format!(
                "Translate this request into Gherkin feature files under spec/.\n\nREQUEST:\n{request}\n"
            );
            match run_role(&root, "author", &prompt, b.as_mut(), false) {
                Ok(res) => {
                    println!("author wrote {} file(s):", res.wrote.len());
                    for w in res.wrote {
                        println!("  {w}");
                    }
                    let _ = sync(&root);
                    println!("\nReview spec/ then run: shalt approve");
                    Ok(0)
                }
                Err(RoleError::Integrity(e)) => {
                    eprintln!("turn rejected: {e}");
                    Ok(2)
                }
                Err(e) => {
                    eprintln!("{e}");
                    Ok(1)
                }
            }
        }
        Cmd::Approve { yes, by } => {
            if !yes {
                eprintln!("refusing without --yes in this build (non-interactive)");
                return Ok(1);
            }
            let features = load_specs(&root.join("spec"), true).map_err(|e| {
                eprintln!("{}", e.message());
                4
            })?;
            if features.is_empty() {
                eprintln!("no feature files in spec/");
                return Ok(1);
            }
            let minted = stamp_rids(&root.join("spec")).map_err(|e| {
                eprintln!("{e}");
                1
            })?;
            let (mut led, features) = sync(&root)?;
            let hashes: serde_json::Map<String, serde_json::Value> = features
                .iter()
                .flat_map(|f| f.scenarios.iter().map(move |s| (f, s)))
                .filter_map(|(f, s)| {
                    s.rid.as_ref().map(|r| (r.clone(), serde_json::Value::String(s.spec_hash(&f.background))))
                })
                .collect();
            led.spec_lock = serde_json::json!({
                "approved_by": by,
                "approved_at": chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
                "scenario_count": features.iter().map(|f| f.scenarios.len()).sum::<usize>(),
                "scenario_hashes": hashes,
            });
            led.save(&ledger_path(&root)).ok();
            println!("approved by {by}; {} new scenario id(s) stamped into the spec.", minted.len());
            Ok(0)
        }
        Cmd::Steps => {
            let led = Ledger::load(&ledger_path(&root)).unwrap_or_default();
            if led.spec_lock.as_object().map(|o| o.is_empty()).unwrap_or(true) {
                eprintln!("spec is not approved yet -- run `shalt approve` first");
                return Ok(1);
            }
            let mut b = backend(&cli)?;
            match run_role(&root, "stepwright", "Write step definitions under steps/ and contract/interface.md", b.as_mut(), false) {
                Ok(res) => {
                    println!("stepwright wrote {} file(s)", res.wrote.len());
                    Ok(0)
                }
                Err(RoleError::Integrity(e)) => {
                    eprintln!("turn rejected: {e}");
                    Ok(2)
                }
                Err(e) => {
                    eprintln!("{e}");
                    Ok(1)
                }
            }
        }
        Cmd::Run => {
            let (mut led, _) = sync(&root)?;
            let cfg = Config::load(&root).unwrap_or_default();
            let run = run_suite(&root, &cfg);
            if run.harness_error {
                eprintln!("the test harness failed to run");
                eprintln!("{}", harness_report(&run));
                return Ok(3);
            }
            let out = led.apply_run(&run.results, &run.run_id, &run.collection_error);
            led.save(&ledger_path(&root)).ok();
            if !out.regressions.is_empty() {
                println!("{} REGRESSION(S)", out.regressions.len());
            }
            print_status(&led);
            Ok(0)
        }
        Cmd::Build { max_turns, strict } => {
            let (mut led, features) = sync(&root)?;
            if led.spec_lock.as_object().map(|o| o.is_empty()).unwrap_or(true) {
                eprintln!("spec is not approved yet -- run `shalt approve` first");
                return Ok(1);
            }
            let cfg = Config::load(&root).unwrap_or_default();
            let mut b = backend(&cli)?;
            let held = holdout_rids(&features);
            let live: Vec<_> = led.entries.iter().filter(|(_, e)| e.status != ORPHAN).map(|(r, _)| r.clone()).collect();
            let visible: Vec<_> = live.iter().filter(|r| !held.contains(*r)).cloned().collect();
            if !held.is_empty() {
                println!("{} scenario(s) held out from the implementer", held.len());
            }
            for turn in 1..=*max_turns {
                let run = run_suite(&root, &cfg);
                if run.harness_error {
                    eprintln!("turn {turn}: the test harness failed to run");
                    eprintln!("{}", harness_report(&run));
                    return Ok(3);
                }
                led.apply_run(&run.results, &format!("turn{turn}"), &run.collection_error);
                led.save(&ledger_path(&root)).ok();
                let red_visible: Vec<_> = visible
                    .iter()
                    .filter(|r| matches!(led.entries[*r].status.as_str(), RED | PENDING | STALE))
                    .cloned()
                    .collect();
                println!("\nturn {turn}: {}/{} visible green", visible.len() - red_visible.len(), visible.len());
                if red_visible.is_empty() {
                    break;
                }
                match run_role(&root, "implementer", "Make the failing scenarios pass", b.as_mut(), true) {
                    Ok(res) => println!("  implementer wrote: {}", res.wrote.join(", ")),
                    Err(RoleError::Integrity(e)) => {
                        println!("\nturn {turn} REJECTED -- {e}");
                        println!("  nothing from this turn was kept; the spec and tests are untouched.");
                        if *strict {
                            return Ok(2);
                        }
                    }
                    Err(e) => {
                        eprintln!("{e}");
                        return Ok(1);
                    }
                }
            }
            let run = run_suite(&root, &cfg);
            led.apply_run(&run.results, "final", &run.collection_error);
            led.save(&ledger_path(&root)).ok();
            let overfit: Vec<_> = held
                .iter()
                .filter(|r| led.entries.get(*r).map(|e| e.status != GREEN).unwrap_or(false))
                .cloned()
                .collect();
            let vis_green = visible.iter().all(|r| led.entries[r].status == GREEN);
            if vis_green && !overfit.is_empty() {
                println!("OVERFIT: every visible scenario is green but held-out scenarios fail.");
                for r in &overfit {
                    println!("  {r}  {}", led.entries[r].name);
                }
            }
            print_status(&led);
            Ok(0)
        }
        Cmd::Status => {
            let (led, _) = sync(&root)?;
            print_status(&led);
            Ok(0)
        }
        Cmd::Verify => {
            let (led, features) = sync(&root)?;
            let mut problems = audit(&led, &features);
            let board = Board::load(&root.join(".shalt/board.json"));
            problems.extend(verify_drift(&board, &features));
            if problems.is_empty() {
                println!("verify: ok");
                Ok(0)
            } else {
                for p in &problems {
                    println!("  {p}");
                }
                Ok(1)
            }
        }
        Cmd::Tree => {
            let (led, features) = sync(&root)?;
            for f in features {
                println!("STORY {}  {}", f.name, f.story().one_line());
                for s in f.scenarios {
                    let st = s
                        .rid
                        .as_ref()
                        .and_then(|r| led.entries.get(r))
                        .map(|e| e.status.as_str())
                        .unwrap_or(PENDING);
                    let mark = if s.is_holdout() { " [holdout]" } else { "" };
                    println!("   {} {} {}{}", status_glyph(st), s.name, s.rid.unwrap_or_default(), mark);
                }
            }
            Ok(0)
        }
        Cmd::Stories => {
            let (_, features) = sync(&root)?;
            for f in features {
                let st = f.story();
                if st.complete() {
                    println!("{}  {}", f.file, st.one_line());
                } else {
                    println!("{}  missing {}", f.file, st.missing().join(", "));
                }
            }
            Ok(0)
        }
        Cmd::Org { action } => match action {
            OrgCmd::List => {
                let org = Org::load();
                println!("{}", org.name);
                for p in org.projects {
                    println!("  {}  {}  {}", p.id, p.name, p.path);
                }
                Ok(0)
            }
            OrgCmd::Add { path } => {
                let mut org = Org::load();
                match org.add(path) {
                    Ok(p) => {
                        org.save().ok();
                        println!("added {} at {}", p.id, p.path);
                        Ok(0)
                    }
                    Err(e) => {
                        eprintln!("{e}");
                        Ok(1)
                    }
                }
            }
            OrgCmd::Remove { id } => {
                let mut org = Org::load();
                if org.remove(id) {
                    org.save().ok();
                    println!("removed {id}");
                    Ok(0)
                } else {
                    eprintln!("no project {id}");
                    Ok(1)
                }
            }
        },
        Cmd::Board { project: _, action } => {
            let mut board = Board::load(&root.join(".shalt/board.json"));
            let (_, features) = sync(&root)?;
            board.sync_new_rids(&features);
            match action {
                Some(BoardCmd::Unschedule { rid }) => {
                    board.unschedule(rid);
                    board.save(&root.join(".shalt/board.json")).ok();
                    println!("unscheduled {rid}");
                }
                _ => {
                    for it in &board.items {
                        println!("  {}  rank {}", it.rid, it.rank);
                    }
                }
            }
            board.save(&root.join(".shalt/board.json")).ok();
            Ok(0)
        }
        Cmd::Job { action } => match action {
            JobCmd::List => {
                let q = JobQueue::load();
                for j in q.jobs {
                    println!("  {}  {:?}  {}  {:?}", j.id, j.kind, j.project_id, j.status);
                }
                Ok(0)
            }
            JobCmd::Show { id } => {
                let q = JobQueue::load();
                match q.get(id) {
                    Some(j) => {
                        println!("{}  {:?}  {}", j.id, j.status, j.project_id);
                        println!("model  {} / {}", j.backend, j.model);
                        println!("--- prompt ---");
                        println!("{}", shalt_core::author_user_prompt(&j.prompt));
                        println!("--- log ---");
                        print!("{}", j.log);
                        Ok(0)
                    }
                    None => {
                        eprintln!("no job {id}");
                        Ok(1)
                    }
                }
            }
            JobCmd::Pause { id } => {
                let mut q = JobQueue::load();
                if q.get(id).is_none() {
                    eprintln!("no job {id}");
                    return Ok(1);
                }
                q.set_status(id, shalt_core::jobs::JobStatus::Paused);
                q.append(id, "pause requested");
                q.save().ok();
                println!("paused {id}");
                Ok(0)
            }
            JobCmd::Resume { id } => {
                let mut q = JobQueue::load();
                if q.get(id).is_none() {
                    eprintln!("no job {id}");
                    return Ok(1);
                }
                q.set_status(id, shalt_core::jobs::JobStatus::Running);
                q.append(id, "resume");
                q.save().ok();
                println!("resumed {id} (if no worker is attached, re-run from the UI)");
                Ok(0)
            }
            JobCmd::Add { kind, project } => {
                let kind = match kind.as_str() {
                    "author" => JobKind::Author,
                    "steps" => JobKind::Steps,
                    "build" => JobKind::Build,
                    "run" => JobKind::Run,
                    "verify" => JobKind::Verify,
                    other => {
                        eprintln!("unknown kind {other}");
                        return Ok(1);
                    }
                };
                let mut q = JobQueue::load();
                let j = q.enqueue(kind, project);
                q.save().ok();
                println!("queued {}", j.id);
                Ok(0)
            }
        },
        Cmd::Ui { port, no_open, action } => match action {
            None => cmd_ui_start(&root, *port, !*no_open),
            Some(UiAction::Status) => cmd_ui_status(),
            Some(UiAction::Stop) => cmd_ui_stop(),
            Some(UiAction::Restart) => {
                let _ = cmd_ui_stop();
                cmd_ui_start(&root, *port, !*no_open)
            }
        },
        Cmd::Diagrams => {
            let (led, _) = sync(&root)?;
            let entries: Vec<_> = led.entries.values().cloned().collect();
            match shalt_core::viz::write_mermaid(&root, &entries) {
                Ok(written) => {
                    println!("wrote {} file(s):", written.len());
                    for w in written {
                        println!("  {}", w.display());
                    }
                    Ok(0)
                }
                Err(e) => {
                    eprintln!("{e}");
                    Ok(1)
                }
            }
        }
        Cmd::Dashboard => {
            let (led, _) = sync(&root)?;
            let name = Config::load(&root).ok().and_then(|c| {
                if c.name.is_empty() {
                    None
                } else {
                    Some(c.name)
                }
            }).unwrap_or_else(|| root.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "project".into()));
            match shalt_core::viz::write_dashboard(&root, &led, &name) {
                Ok(p) => {
                    let s = led.summary();
                    println!(
                        "wrote {}  ({} upheld / {} scenarios, {}%)",
                        p.display(),
                        s.get("green").and_then(|v| v.as_i64()).unwrap_or(0),
                        s.get("total").and_then(|v| v.as_i64()).unwrap_or(0),
                        s.get("completion_pct").and_then(|v| v.as_f64()).unwrap_or(0.0)
                    );
                    Ok(0)
                }
                Err(e) => {
                    eprintln!("{e}");
                    Ok(1)
                }
            }
        }
        Cmd::Mutate { engine, budget, seed } => {
            let (mut led, _) = sync(&root)?;
            let cfg = Config::load(&root).unwrap_or_default();
            println!("mutating {}/ — engine {engine}, budget {budget}, seed {seed}", cfg.src);
            let report = shalt_core::run_campaign(&root, &cfg, engine, *budget, *seed);
            if !report.error.is_empty() {
                eprintln!("cannot run: {}", report.error);
                return Ok(1);
            }
            led.apply_mutation(&report);
            led.save(&ledger_path(&root)).ok();
            println!(
                "\nmutation score {}%  ({} killed, {} survived, {} invalid)",
                report.score(),
                report.killed().len(),
                report.survived().len(),
                report.invalid().len()
            );
            if !report.survived().is_empty() {
                println!("\n{} mutation(s) survived — no scenario noticed:", report.survived().len());
                for m in report.survived().iter().take(15) {
                    println!("  {}", m.describe());
                }
            }
            let blind = report.blind_spots();
            if !blind.is_empty() {
                println!("\nBLIND SPOTS — mutation(s) ran inside scenarios that stayed green:");
                for (rid, ms) in &blind {
                    println!("  {rid}");
                    for m in ms {
                        println!("      missed by  {}", m.describe());
                    }
                }
            }
            Ok(0)
        }
    }
}

fn reopen_ui(port: u16, open: bool) -> Result<i32, i32> {
    let url = format!("http://127.0.0.1:{port}/");
    println!("already running at {url}");
    if open {
        let _ = std::process::Command::new("open").arg(&url).spawn();
    }
    println!("  shalt ui status | shalt ui stop | shalt ui restart");
    Ok(0)
}

fn print_in_use(port: u16) {
    eprintln!("port {port} is already in use");
    if let Some((pid, who)) = shalt_core::uis::occupant(port) {
        eprintln!("  pid {pid}  {who}");
    }
    eprintln!("  shalt ui            # pick the next free port");
    eprintln!("  shalt ui --port n");
}

fn cmd_ui_start(root: &Path, port: Option<u16>, open: bool) -> Result<i32, i32> {
    if let Some(u) = shalt_core::uis::current() {
        if let Some(p) = port {
            if p != u.port {
                eprintln!("shalt ui is already on {} — one instance. shalt ui stop, then --port {p}.", u.url);
            }
        }
        return reopen_ui(u.port, open);
    }
    let preferred = port.unwrap_or(7700);
    if port.is_some() {
        return serve_ui(root, preferred, open);
    }
    for p in preferred..=7799 {
        if shalt_core::uis::occupant(p).is_some() {
            continue;
        }
        match serve_ui(root, p, open) {
            Err(1) => continue,
            other => return other,
        }
    }
    eprintln!("no free port in {preferred}–7799");
    print_in_use(preferred);
    Err(1)
}

fn serve_ui(root: &Path, port: u16, open: bool) -> Result<i32, i32> {
    let rt = tokio::runtime::Runtime::new().unwrap();
    match rt.block_on(crate::server::serve(root.to_path_buf(), port, open)) {
        Ok(()) => Ok(0),
        Err(e) if e.starts_with("ADDR_IN_USE:") => {
            print_in_use(port);
            Err(1)
        }
        Err(e) => {
            eprintln!("{e}");
            Err(1)
        }
    }
}

fn cmd_ui_status() -> Result<i32, i32> {
    match shalt_core::uis::current() {
        Some(u) => {
            println!("up  pid {}  {}  {}", u.pid, u.url, u.root);
            Ok(0)
        }
        None => {
            println!("shalt ui is not running");
            if let Some((pid, who)) = shalt_core::uis::occupant(7700) {
                println!("port 7700 is pid {pid} ({who})");
            }
            Ok(0)
        }
    }
}

fn cmd_ui_stop() -> Result<i32, i32> {
    match shalt_core::uis::stop_current() {
        Ok(Some(u)) => {
            println!("stopped pid {}  {}", u.pid, u.url);
            Ok(0)
        }
        Ok(None) => {
            println!("shalt ui is not running");
            Ok(0)
        }
        Err(e) => {
            eprintln!("{e}");
            Err(1)
        }
    }
}

fn status_glyph(st: &str) -> &'static str {
    match st {
        GREEN => "+",
        RED => "x",
        STALE => "~",
        PENDING => ".",
        ORPHAN => "o",
        _ => "?",
    }
}

fn print_status(led: &Ledger) {
    let s = led.summary();
    println!(
        "[{}]  {}%  {} upheld / {} failing / {} stale / {} no test",
        bar(s.get("green").and_then(|v| v.as_i64()).unwrap_or(0), s.get("total").and_then(|v| v.as_i64()).unwrap_or(0)),
        s.get("completion_pct").and_then(|v| v.as_f64()).unwrap_or(0.0),
        s.get("green").and_then(|v| v.as_i64()).unwrap_or(0),
        s.get("red").and_then(|v| v.as_i64()).unwrap_or(0),
        s.get("stale").and_then(|v| v.as_i64()).unwrap_or(0),
        s.get("pending").and_then(|v| v.as_i64()).unwrap_or(0),
    );
}

fn bar(green: i64, total: i64) -> String {
    let n: usize = 28;
    let filled = if total == 0 { 0 } else { (n as i64 * green / total) as usize };
    format!("{}{}", "#".repeat(filled), "-".repeat(n.saturating_sub(filled)))
}

mod server;
