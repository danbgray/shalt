use clap::{Parser, Subcommand};
use shalt_core::backends::FixtureBackend;
use shalt_core::board::{verify_drift, Board};
use shalt_core::config::{write_config, Config};
use shalt_core::integrity::audit;
use shalt_core::jobs::{JobKind, JobQueue};
use shalt_core::ledger::{Ledger, GREEN, ORPHAN, PENDING, RED, STALE};
use shalt_core::org::Org;
use shalt_core::roles::run_role;
use shalt_core::runner::run_suite;
use shalt_core::spec::{drop_scenario_blocks, holdout_rids, load_specs, stamp_rids};
use std::io::{self, BufRead, IsTerminal, Write};
use shalt_core::{author_user_prompt, Backend, OpenAICompatBackend, RoleError};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

const COMMANDS: &[&str] = &[
    "init", "author", "approve", "steps", "build", "run", "status", "verify", "tree", "spec",
    "stories", "onboard", "org", "board", "job", "ui", "diagrams", "dashboard", "mutate", "do",
    "play", "loop", "sprint", "models", "help",
];

fn parse_existing_dir(s: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(s);
    if !p.is_dir() {
        return Err(format!("{s} is not a directory"));
    }
    Ok(p)
}
const VALUE_FLAGS: &[&str] = &[
    "--root", "--backend", "--fixtures", "--base-url", "--port",
];

#[derive(Parser)]
#[command(name = "shalt", version, about = "English → Gherkin → tests → code, with a spec-bound ledger")]
struct Cli {
    #[arg(long, default_value = ".")]
    root: PathBuf,
    #[arg(long, default_value = "fixture")]
    backend: String,
    #[arg(long)]
    fixtures: Option<PathBuf>,
    /// Model id (`--model=qwen3.5:2b`). Pass `--model` alone to list and pick.
    #[arg(long, global = true, require_equals = true, num_args = 0..=1, default_missing_value = "")]
    model: Option<String>,
    #[arg(long)]
    base_url: Option<String>,
    /// Accept every generated scenario (skip y/n). Also required for `shalt approve`.
    #[arg(long, short = 'y', global = true)]
    yes: bool,
    #[arg(long, global = true, default_value = "local")]
    by: String,
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
    Approve,
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
    /// Wrap an existing repo in shalt (Gherkin around code that already exists).
    Onboard {
        /// Directory of the existing project
        #[arg(value_name = "PATH", value_parser = parse_existing_dir)]
        path: PathBuf,
        /// Optional note for the author
        #[arg(short, long, default_value = "")]
        prompt: String,
    },
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
    Sprint {
        #[command(subcommand)]
        action: SprintCmd,
    },
    Spec {
        #[command(subcommand)]
        action: SpecCmd,
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
    /// List available models and pick a default
    Models,
    /// English sentence → spec → tests
    Do {
        #[arg(trailing_var_arg = true, required = true, allow_hyphen_values = true)]
        sentence: Vec<String>,
    },
    /// Thin loop: spec → tests → code until green. Same engine as UI Play.
    #[command(alias = "loop")]
    Play {
        #[arg(long, default_value_t = 8)]
        max_steps: usize,
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
    /// Change the display name. The id (and jobs) stay the same.
    Rename {
        id: String,
        #[arg(trailing_var_arg = true, required = true)]
        name: Vec<String>,
    },
    /// Drop from the org catalog. Does not delete files on disk.
    Remove { id: String },
    /// Park this project's running jobs so another project can use the model.
    Pause { id: String },
    /// Resume this project and pause the others.
    Play { id: String },
}

#[derive(Subcommand)]
enum BoardCmd {
    List,
    Unschedule { rid: String },
    Promote { rid: String },
    Estimate { rid: String, tokens: i64 },
}

#[derive(Subcommand)]
enum SprintCmd {
    List,
    Open {
        #[arg(trailing_var_arg = true)]
        title: Vec<String>,
    },
    Close {
        id: Option<String>,
    },
    Assign {
        rid: String,
        id: String,
    },
    Retro {
        id: Option<String>,
    },
}

#[derive(Subcommand)]
enum SpecCmd {
    /// Remove a scenario from spec/ (does not delete the feature's other scenarios).
    Delete { file: String, line: usize },
    /// Stamp a rid if needed and put the scenario at the front of the board.
    Promote { file: String, line: usize },
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

fn inject_do(mut args: Vec<OsString>) -> Vec<OsString> {
    let mut i = 1usize;
    while i < args.len() {
        let s = args[i].to_string_lossy();
        if s == "--" {
            i += 1;
            break;
        }
        if s.starts_with('-') {
            if s == "--model" {
                if let Some(next) = args.get(i + 1).and_then(|a| a.to_str()) {
                    if looks_like_model(next) {
                        args[i] = OsString::from(format!("--model={next}"));
                        args.remove(i + 1);
                        i += 1;
                        continue;
                    }
                }
                i += 1;
                continue;
            }
            if VALUE_FLAGS.iter().any(|f| *f == s) {
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        break;
    }
    let pos = args.get(i).and_then(|a| a.to_str()).unwrap_or("");
    if pos.is_empty() {
        if has_bare_model(&args)
            || args.iter().any(|a| a.to_string_lossy().starts_with("--model="))
        {
            args.insert(1, "models".into());
        }
        return args;
    }
    if COMMANDS.contains(&pos) {
        return args;
    }
    args.insert(i, "do".into());
    args
}

fn looks_like_model(s: &str) -> bool {
    if s.starts_with('-') {
        return false;
    }
    s.contains(':')
        || s.contains('/')
        || s.starts_with("grok")
        || s.starts_with("gpt")
        || s.starts_with("o1")
        || s.starts_with("o3")
        || s.starts_with("o4")
        || s.starts_with("qwen")
        || s.starts_with("llama")
        || s.starts_with("mistral")
        || s.starts_with("gemma")
        || s.starts_with("phi")
        || s.starts_with("deepseek")
        || s.starts_with("claude")
        || s.starts_with("command")
}

fn has_bare_model(args: &[OsString]) -> bool {
    args.iter().any(|a| a == "--model")
        && !args.iter().any(|a| a.to_string_lossy().starts_with("--model="))
}

fn main() {
    let args: Vec<OsString> = inject_do(std::env::args_os().collect());
    if args.len() == 1 {
        let stem = Path::new(&args[0])
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("shalt");
        if stem == "shall" {
            eprintln!("shall <what the system shall do>\n  English → Gherkin. Each scenario is y/n unless you pass --yes, then tests.");
            std::process::exit(2);
        }
    }
    let cli = Cli::parse_from(args);
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
        Cmd::Models => cmd_models(&cli),
        Cmd::Do { sentence } => cmd_specify(&cli, &root, &sentence.join(" ")),
        Cmd::Play { max_steps } => cmd_play(&root, *max_steps),
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
        Cmd::Approve => {
            let yes = cli.yes;
            let by = &cli.by;
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
                eprintln!("{}\n{}", run.stderr, run.stdout);
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
                    eprintln!("{}\n{}", run.stderr, run.stdout);
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
        Cmd::Onboard { path, prompt } => {
            let backend = if cli.backend == "fixture" {
                ""
            } else {
                cli.backend.as_str()
            };
            match shalt_core::onboard_project(path, prompt, backend, cli.model.as_deref().unwrap_or("")) {
                Ok((p, job)) => {
                    println!("onboarded {} as {}", p.path, p.id);
                    println!("job {}", job.id);
                    match shalt_core::execute_author(&job.id) {
                        Ok(s) => {
                            println!("{s}");
                            Ok(0)
                        }
                        Err(e) => {
                            eprintln!("{e}");
                            Ok(1)
                        }
                    }
                }
                Err(e) => {
                    eprintln!("{e}");
                    Ok(1)
                }
            }
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
                let stack = shalt_core::config::detect_stack(path);
                let name = path
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "project".into());
                if let Err(e) = shalt_core::config::ensure_workspace(path, stack, &name) {
                    eprintln!("{e}");
                    return Ok(1);
                }
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
            OrgCmd::Rename { id, name } => {
                let name = name.join(" ");
                let mut org = Org::load();
                if org.rename(id, &name) {
                    org.save().ok();
                    println!("renamed {id} → {name}");
                    Ok(0)
                } else {
                    eprintln!("could not rename {id}");
                    Ok(1)
                }
            }
            OrgCmd::Remove { id } => {
                let mut org = Org::load();
                if org.remove(id) {
                    org.save().ok();
                    println!("removed {id} (files on disk kept)");
                    Ok(0)
                } else {
                    eprintln!("no project {id}");
                    Ok(1)
                }
            }
            OrgCmd::Pause { id } => {
                let mut org = Org::load();
                if !org.set_paused(id, true) {
                    eprintln!("no project {id}");
                    return Ok(1);
                }
                org.save().ok();
                let mut q = JobQueue::load();
                let n = q.pause_project(id).len();
                q.save().ok();
                println!("paused {id} ({n} job(s) parked)");
                Ok(0)
            }
            OrgCmd::Play { id } => cmd_play_id(id, 8),
        },
        Cmd::Board { project: _, action } => {
            let mut board = Board::load(&root.join(".shalt/board.json"));
            let (led, features) = sync(&root)?;
            board.sync_new_rids(&features);
            match action {
                Some(BoardCmd::Unschedule { rid }) => {
                    board.unschedule(rid);
                    board.save(&root.join(".shalt/board.json")).ok();
                    println!("unscheduled {rid}");
                }
                Some(BoardCmd::Promote { rid }) => {
                    board.promote(rid);
                    board.save(&root.join(".shalt/board.json")).ok();
                    println!("promoted {rid}");
                }
                Some(BoardCmd::Estimate { rid, tokens }) => {
                    if board.set_estimate(rid, *tokens) {
                        board.save(&root.join(".shalt/board.json")).ok();
                        println!("{rid} estimate {tokens} tokens");
                    } else {
                        eprintln!("no ticket {rid} on the board");
                        return Ok(1);
                    }
                }
                _ => {
                    for it in &board.items {
                        let title = led
                            .entries
                            .get(&it.rid)
                            .map(|e| e.name.as_str())
                            .filter(|s| !s.is_empty())
                            .unwrap_or("");
                        let sp = it.sprint_id.as_deref().unwrap_or("backlog");
                        println!(
                            "  {}  {}  est {}  {}  rank {}",
                            it.rid,
                            if title.is_empty() { "—" } else { title },
                            it.token_estimate,
                            sp,
                            it.rank
                        );
                    }
                }
            }
            board.save(&root.join(".shalt/board.json")).ok();
            Ok(0)
        }
        Cmd::Sprint { action } => {
            let mut board = Board::load(&root.join(".shalt/board.json"));
            let q = JobQueue::load();
            let pid = root
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let project_id = Org::load()
                .find_by_path(&root)
                .map(|p| p.id.clone())
                .unwrap_or(pid);
            match action {
                SprintCmd::List => {
                    for s in &board.sprints {
                        let mark = if s.closed_at.is_some() {
                            "closed"
                        } else if s.enabled {
                            "open"
                        } else {
                            "idle"
                        };
                        println!("  {}  {}  {mark}", s.id, s.title);
                    }
                    if board.sprints.is_empty() {
                        println!("no sprints — `shalt sprint open W38`");
                    }
                }
                SprintCmd::Open { title } => {
                    let s = board.open_sprint(&title.join(" "));
                    board.save(&root.join(".shalt/board.json")).ok();
                    println!("opened {}  {}", s.id, s.title);
                }
                SprintCmd::Close { id } => {
                    let sid = id
                        .clone()
                        .or_else(|| board.active_sprint().map(|s| s.id.clone()));
                    let Some(sid) = sid else {
                        eprintln!("no open sprint");
                        return Ok(1);
                    };
                    let Some(r) = shalt_core::tokens::retro(&board, &q.jobs, &project_id, &sid)
                    else {
                        eprintln!("no sprint {sid}");
                        return Ok(1);
                    };
                    board.close_sprint(&sid, r.estimated, r.spent);
                    board.save(&root.join(".shalt/board.json")).ok();
                    print_retro(&r);
                }
                SprintCmd::Assign { rid, id } => {
                    let val = if id == "backlog" { None } else { Some(id.clone()) };
                    if let Some(it) = board.items.iter_mut().find(|i| i.rid == *rid) {
                        it.sprint_id = val;
                        board.save(&root.join(".shalt/board.json")).ok();
                        println!("{rid} → {id}");
                    } else {
                        eprintln!("no ticket {rid}");
                        return Ok(1);
                    }
                }
                SprintCmd::Retro { id } => {
                    let sid = id
                        .clone()
                        .or_else(|| board.active_sprint().map(|s| s.id.clone()))
                        .or_else(|| {
                            board
                                .sprints
                                .iter()
                                .rev()
                                .find(|s| s.closed_at.is_some())
                                .map(|s| s.id.clone())
                        });
                    let Some(sid) = sid else {
                        eprintln!("no sprint");
                        return Ok(1);
                    };
                    match shalt_core::tokens::retro(&board, &q.jobs, &project_id, &sid) {
                        Some(r) => print_retro(&r),
                        None => {
                            eprintln!("no sprint {sid}");
                            return Ok(1);
                        }
                    }
                }
            }
            Ok(0)
        }
        Cmd::Spec { action } => {
            let spec = root.join("spec");
            match action {
                SpecCmd::Delete { file, line } => {
                    match shalt_core::spec::drop_scenario_blocks(&spec, &[(file.clone(), *line)]) {
                        Ok(0) => {
                            eprintln!("no scenario at {file}:{line}");
                            Ok(1)
                        }
                        Ok(n) => {
                            println!("removed {n} scenario(s) from {file}");
                            Ok(0)
                        }
                        Err(e) => {
                            eprintln!("{e}");
                            Ok(1)
                        }
                    }
                }
                SpecCmd::Promote { file, line } => {
                    match shalt_core::spec::stamp_scenario(&spec, file, *line) {
                        Ok(Some(rid)) => {
                            let mut board = Board::load(&root.join(".shalt/board.json"));
                            board.promote(&rid);
                            board.save(&root.join(".shalt/board.json")).ok();
                            println!("promoted {rid}");
                            Ok(0)
                        }
                        Ok(None) => {
                            eprintln!("could not stamp {file}:{line}");
                            Ok(1)
                        }
                        Err(e) => {
                            eprintln!("{e}");
                            Ok(1)
                        }
                    }
                }
            }
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
                        println!("{}", shalt_core::jobs::status_line(j));
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

fn print_models(models: &[shalt_core::ModelChoice]) {
    for (i, m) in models.iter().enumerate() {
        println!("  {:>2}  {:<36} {}", i + 1, m.id, m.kind);
    }
}

fn pick_model() -> Result<shalt_core::ModelChoice, i32> {
    let models = shalt_core::list_models();
    if models.is_empty() {
        eprintln!("no models found. Start Ollama, or set XAI_API_KEY / OPENAI_API_KEY.");
        return Err(1);
    }
    println!("available models:");
    print_models(&models);
    if !io::stdin().is_terminal() {
        eprintln!("not a terminal; pass --model=<id>");
        return Err(2);
    }
    print!("pick a model (number or id): ");
    let _ = io::stdout().flush();
    let mut line = String::new();
    if io::stdin().lock().read_line(&mut line).is_err() {
        return Err(1);
    }
    let choice = line.trim();
    if choice.is_empty() {
        eprintln!("no selection");
        return Err(2);
    }
    let picked = if let Ok(n) = choice.parse::<usize>() {
        n.checked_sub(1).and_then(|i| models.get(i)).cloned()
    } else {
        models.iter().find(|m| m.id == choice || m.id.starts_with(choice)).cloned()
    };
    let Some(picked) = picked else {
        eprintln!("unknown model {choice:?}");
        return Err(1);
    };
    if let Err(e) = shalt_core::api::save_preferred_model(&picked.backend, &picked.id) {
        eprintln!("could not save default: {e}");
    } else {
        println!("default model: {} ({})", picked.id, picked.backend);
    }
    Ok(picked)
}

fn cmd_models(cli: &Cli) -> Result<i32, i32> {
    if let Some(id) = cli.model.as_deref().filter(|s| !s.is_empty()) {
        let models = shalt_core::list_models();
        let backend = models
            .iter()
            .find(|m| m.id == id)
            .map(|m| m.backend.as_str())
            .unwrap_or_else(|| infer_backend(id, cli));
        shalt_core::api::save_preferred_model(backend, id).map_err(|e| {
            eprintln!("{e}");
            1
        })?;
        println!("default model: {id} ({backend})");
        return Ok(0);
    }
    pick_model().map(|_| 0)
}

fn infer_backend<'a>(id: &str, cli: &'a Cli) -> &'a str {
    if cli.backend != "fixture" {
        return cli.backend.as_str();
    }
    if id.contains("grok") {
        "grok"
    } else if id.contains("gpt") || id.starts_with("o1") || id.starts_with("o3") || id.starts_with("o4") {
        "openai"
    } else {
        "ollama"
    }
}

fn resolve_model(cli: &Cli) -> Result<(String, String), i32> {
    if let Some(id) = cli.model.as_deref() {
        if id.is_empty() {
            let p = pick_model()?;
            return Ok((p.backend, p.id));
        }
        let backend = shalt_core::list_models()
            .iter()
            .find(|m| m.id == id)
            .map(|m| m.backend.clone())
            .unwrap_or_else(|| infer_backend(id, cli).to_string());
        return Ok((backend, id.to_string()));
    }
    if cli.backend != "fixture" {
        return Ok((cli.backend.clone(), String::new()));
    }
    if let Some((b, id)) = shalt_core::api::preferred_model() {
        return Ok((b, id));
    }
    let ollama_up = shalt_core::api::ollama_reachable();
    let grok = shalt_core::api::xai_api_key().is_some();
    if ollama_up {
        Ok(("ollama".into(), String::new()))
    } else if grok {
        Ok(("grok".into(), String::new()))
    } else {
        eprintln!("no model available: `shall --model` to list, or start Ollama / set XAI_API_KEY");
        Err(1)
    }
}

fn live_backend(cli: &Cli) -> Result<OpenAICompatBackend, i32> {
    let (preset, model_id) = resolve_model(cli)?;
    let model = if model_id.is_empty() { None } else { Some(model_id.as_str()) };
    let mut b = OpenAICompatBackend::from_preset(&preset, model, cli.base_url.as_deref()).map_err(|e| {
        eprintln!("{e}");
        1
    })?;
    eprintln!("using {preset} / {}", b.model);
    b.on_progress = Some(Box::new(|line| eprintln!("{line}")));
    if !cli.yes && io::stdin().is_terminal() {
        b.on_ask = Some(Box::new(|question: &str, guess: &str| {
            loop {
                println!("\n? {question}");
                if !guess.is_empty() {
                    println!("  suggested: {guess}");
                }
                print!("> ");
                let _ = io::stdout().flush();
                let mut line = String::new();
                io::stdin().lock().read_line(&mut line).map_err(|e| e.to_string())?;
                let a = line.trim().to_string();
                if !a.is_empty() {
                    return Ok(a);
                }
                eprintln!("  type an answer (or --yes next time to skip questions)");
            }
        }));
    }
    Ok(b)
}

fn prompt_yn(label: &str) -> Result<bool, i32> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    loop {
        print!("{label} [y/n] ");
        let _ = stdout.flush();
        let mut line = String::new();
        if stdin.lock().read_line(&mut line).is_err() {
            return Err(1);
        }
        match line.trim().to_lowercase().as_str() {
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => eprintln!("  type y or n"),
        }
    }
}

fn review_scenarios(root: &Path) -> Result<usize, i32> {
    if !io::stdin().is_terminal() {
        eprintln!("not a terminal; pass --yes to accept every scenario");
        return Err(2);
    }
    let spec = root.join("spec");
    let features = load_specs(&spec, true).map_err(|e| {
        eprintln!("{}", e.message());
        4
    })?;
    let mut drop: Vec<(String, usize)> = Vec::new();
    let mut kept = 0usize;
    for f in &features {
        let path = spec.join(&f.file);
        let raw = std::fs::read_to_string(&path).unwrap_or_default();
        let lines: Vec<String> = raw.replace("\r\n", "\n").split('\n').map(|s| s.to_string()).collect();
        let n = if lines.last().map(|s| s.is_empty()).unwrap_or(false) {
            lines.len() - 1
        } else {
            lines.len()
        };
        println!("\nFeature: {}  ({})", f.name, f.file);
        for (sc, start, end) in f.blocks(n) {
            let s = start.saturating_sub(1);
            let e = end.saturating_sub(1).min(n);
            let body = if s < e { lines[s..e].join("\n") } else { sc.name.clone() };
            println!("\n{body}\n");
            if prompt_yn("Keep this scenario?")? {
                kept += 1;
            } else {
                drop.push((f.file.clone(), start));
                println!("  dropped.");
            }
        }
    }
    if !drop.is_empty() {
        drop_scenario_blocks(&spec, &drop).map_err(|e| {
            eprintln!("{e}");
            1
        })?;
    }
    Ok(kept)
}

fn print_retro(r: &shalt_core::board::SprintRetro) {
    let state = if r.closed { "closed" } else { "open" };
    println!("{}  {}  {state}", r.sprint_id, r.title);
    println!(
        "  tickets {}  estimated {}  spent {}",
        r.tickets, r.estimated, r.spent
    );
    match r.accuracy {
        Some(a) => println!(
            "  accuracy {a:.1}× estimate  bias {}  next ticket ~{}",
            r.bias, r.suggest
        ),
        None => println!("  set token estimates on tickets to measure accuracy"),
    }
}

fn cmd_play(root: &Path, max_steps: usize) -> Result<i32, i32> {
    if !root.join("shalt.toml").exists() && !root.join("spec").exists() {
        eprintln!("no shalt workspace at {}", root.display());
        return Ok(1);
    }
    let pref = Org::ensure_registered(root).map_err(|e| {
        eprintln!("{e}");
        1
    })?;
    println!("PLAY project={} path={}", pref.id, pref.path);
    cmd_play_id(&pref.id, max_steps)
}

fn cmd_play_id(id: &str, max_steps: usize) -> Result<i32, i32> {
    match shalt_core::play_loop(id, max_steps) {
        Ok(ticks) if ticks.is_empty() => {
            println!("PLAY idle — need a spec, or work is already green");
            Ok(0)
        }
        Ok(_) => Ok(0),
        Err(e) => {
            eprintln!("PLAY failed: {e}");
            Ok(1)
        }
    }
}

fn cmd_specify(cli: &Cli, root: &Path, sentence: &str) -> Result<i32, i32> {
    let sentence = sentence.trim();
    if sentence.is_empty() {
        eprintln!("shall <what the system shall do>");
        return Err(2);
    }
    if !root.join("shalt.toml").exists() {
        let name = root
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "project".into());
        shalt_core::config::init_workspace(root, "python", &name).map_err(|e| {
            eprintln!("{e}");
            1
        })?;
        println!("initialised workspace at {}", root.display());
    }
    let mut b = live_backend(cli)?;
    println!("authoring spec…");
    match run_role(root, "author", &author_user_prompt(sentence), &mut b, false) {
        Ok(res) => {
            println!("author wrote {} file(s):", res.wrote.len());
            for w in &res.wrote {
                println!("  {w}");
            }
        }
        Err(RoleError::Integrity(e)) => {
            eprintln!("turn rejected: {e}");
            return Ok(2);
        }
        Err(e) => {
            eprintln!("{e}");
            return Ok(1);
        }
    }
    let _ = sync(root);
    if !cli.yes {
        match review_scenarios(root) {
            Ok(0) => {
                eprintln!("nothing kept; not writing tests");
                return Ok(0);
            }
            Ok(_) => {}
            Err(c) => return Err(c),
        }
    }
    let by = if cli.by == "local" {
        std::env::var("USER").unwrap_or_else(|_| "local".into())
    } else {
        cli.by.clone()
    };
    let features = load_specs(&root.join("spec"), true).map_err(|e| {
        eprintln!("{}", e.message());
        4
    })?;
    if features.is_empty() {
        eprintln!("author wrote no feature files");
        return Ok(1);
    }
    stamp_rids(&root.join("spec")).map_err(|e| {
        eprintln!("{e}");
        1
    })?;
    let (mut led, features) = sync(root)?;
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
    led.save(&ledger_path(root)).ok();
    println!("writing tests…");
    let mut b = live_backend(cli)?;
    match run_role(
        root,
        "stepwright",
        "Write step definitions under steps/ and contract/interface.md",
        &mut b,
        false,
    ) {
        Ok(res) => {
            println!("stepwright wrote {} file(s):", res.wrote.len());
            for w in res.wrote {
                println!("  {w}");
            }
            println!("\nNext: shalt build");
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

#[cfg(test)]
mod arg_tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<OsString> {
        v.iter().map(|x| OsString::from(*x)).collect()
    }
    fn out(v: Vec<OsString>) -> Vec<String> {
        v.into_iter().map(|x| x.to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn sentence_becomes_do() {
        assert_eq!(
            out(inject_do(s(&["shall", "total", "invoices", "exactly"]))),
            ["shall", "do", "total", "invoices", "exactly"]
        );
    }

    #[test]
    fn known_command_untouched() {
        assert_eq!(out(inject_do(s(&["shalt", "ui"]))), ["shalt", "ui"]);
        assert_eq!(out(inject_do(s(&["shall", "status"]))), ["shall", "status"]);
    }

    #[test]
    fn flags_then_sentence() {
        assert_eq!(
            out(inject_do(s(&["shall", "--backend", "qwen", "total", "invoices"]))),
            ["shall", "--backend", "qwen", "do", "total", "invoices"]
        );
    }

    #[test]
    fn bare_model_flag_lists() {
        assert_eq!(out(inject_do(s(&["shall", "--model"]))), ["shall", "models", "--model"]);
    }

    #[test]
    fn model_then_sentence_still_does() {
        assert_eq!(
            out(inject_do(s(&["shall", "--model", "total", "invoices"]))),
            ["shall", "--model", "do", "total", "invoices"]
        );
    }

    #[test]
    fn named_model_then_sentence() {
        assert_eq!(
            out(inject_do(s(&["shall", "--model", "qwen3.5:2b", "total", "invoices"]))),
            ["shall", "--model=qwen3.5:2b", "do", "total", "invoices"]
        );
    }
}
