use clap::{Parser, Subcommand};
use shalt_core::backends::FixtureBackend;
use shalt_core::board::{verify_drift, Board};
use shalt_core::config::Config;
use shalt_core::integrity::audit_in;
use shalt_core::jobs::{JobKind, JobQueue};
use shalt_core::ledger::{Ledger, GREEN, ORPHAN, PENDING, RED, STALE};
use shalt_core::org::Org;
use shalt_core::roles::run_role;
use shalt_core::runner::{run_suite, run_suite_with};
use shalt_core::spec::{drop_scenario_blocks, holdout_rids, load_specs, stamp_rids};
use std::collections::HashSet;
use shalt_core::tags::{filter_scenarios, looks_like_feature_arg, Locator, Pick, TagExpr};
use std::io::{self, BufRead, IsTerminal, Write};
use shalt_core::{author_prompt_with_spec, spec_snapshot, Backend, OpenAICompatBackend, RoleError, RoleResult};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

const COMMANDS: &[&str] = &[
    "init", "author", "approve", "steps", "build", "run", "status", "verify", "tree", "spec",
    "stories", "onboard", "org", "board", "job", "ui", "diagrams", "dashboard", "mutate", "do",
    "play", "loop", "sprint", "models", "plan", "stack", "stop", "design", "journal", "login",
    "logout", "whoami", "connect", "help",
];

fn parse_existing_dir(s: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(s);
    if !p.is_dir() {
        return Err(format!("{s} is not a directory"));
    }
    Ok(p)
}
const VALUE_FLAGS: &[&str] = &[
    "--root", "--backend", "--fixtures", "--base-url", "--port", "--color", "--by", "--tags",
    "--format",
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
    /// auto, always, never. NO_COLOR always wins.
    #[arg(long, global = true, default_value = "auto")]
    color: String,
    /// Cucumber tag expression (`@wip`, `not @holdout`, `@wip or @slow`).
    #[arg(long, global = true)]
    tags: Option<String>,
    /// auto, pretty, progress, play. auto is pretty on a tty, progress when piped.
    #[arg(long = "format", global = true, default_value = "auto")]
    formatter: String,
    /// List matching scenarios without running the harness.
    #[arg(long, global = true)]
    dry_run: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    Init {
        /// rust and javascript are supported. python is next. Other cucumber-family presets should run.
        #[arg(long, default_value = "rust")]
        stack: String,
        #[arg(long, default_value = "")]
        name: String,
    },
    Author { request: String },
    Approve,
    Steps,
    Build {
        #[arg(long, default_value_t = 12)]
        max_turns: usize,
        #[arg(long)]
        strict: bool,
    },
    Run {
        /// Feature files (`spec/foo.feature` or `spec/foo.feature:12`)
        #[arg(value_name = "FEATURE")]
        features: Vec<String>,
    },
    Status,
    Verify,
    Tree,
    Stories,
    /// Wrap an existing repo in shalt. Path or a GitHub URL (`github.com/org/repo`).
    Onboard {
        /// Directory, or GitHub URL to clone then wrap
        #[arg(value_name = "PATH_OR_URL")]
        source: String,
        /// Clone destination when SOURCE is a GitHub URL (default: ./<repo>)
        #[arg(long)]
        dir: Option<PathBuf>,
        /// Optional note for the author
        #[arg(short, long, default_value = "")]
        prompt: String,
    },
    /// Sign in to shalt.dev with GitHub (browser).
    Login {
        #[arg(long, default_value = "https://shalt.dev")]
        host: String,
    },
    Logout,
    Whoami,
    /// Connect this machine to a Space, or GitHub.
    Connect {
        #[command(subcommand)]
        action: Option<ConnectCmd>,
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
        /// Keep the server in this terminal (Ctrl-C stops it). Default detaches so the command returns.
        #[arg(long, global = true)]
        foreground: bool,
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
        /// Take every guess (`all`) or only sprint planning. Stays until `org yolo ID off`.
        #[arg(long)]
        yolo: bool,
    },
    /// Stop the UI, Play, and every other running shalt process.
    Stop,
    /// Draw storyboards (HTML prototypes) under mockups/.
    Design,
    /// Print the project journal, or comment on a post.
    Journal {
        #[command(subcommand)]
        action: Option<JournalCmd>,
    },
    /// Export or import a portable plan (English + Gherkin + model pool).
    Plan {
        #[command(subcommand)]
        action: PlanCmd,
    },
    /// Show or change the workspace build language (`rust`, `javascript`, `python`, …).
    Stack {
        #[arg(value_name = "LANG")]
        name: Option<String>,
    },
}

#[derive(Subcommand)]
enum JournalCmd {
    /// Comment on a post (`shalt journal` prints post ids).
    Comment {
        post: String,
        #[arg(long)]
        reply: Option<String>,
        #[arg(trailing_var_arg = true, required = true, allow_hyphen_values = true)]
        text: Vec<String>,
    },
}

#[derive(Subcommand)]
enum ConnectCmd {
    /// Sign in with GitHub on shalt.dev
    Github,
}

#[derive(Subcommand)]
enum PlanCmd {
    /// Write a `.shalt-plan.json` you can hand to another project.
    Export {
        #[arg(value_name = "FILE")]
        file: Option<PathBuf>,
    },
    /// Start a new project from a plan pack. Does not Play.
    Import {
        file: PathBuf,
        #[arg(long)]
        dir: Option<PathBuf>,
        #[arg(long)]
        name: Option<String>,
    },
}

#[derive(Subcommand)]
enum UiAction {
    /// Show the running UI, if any
    Status,
    /// Stop the running UI
    Stop,
    /// Stop the running UI, start a detached server, return when it answers.
    Restart,
}

#[derive(Subcommand)]
enum OrgCmd {
    List,
    Add {
        #[arg(value_parser = parse_existing_dir)]
        path: PathBuf,
    },
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
    /// Take guesses instead of asking. `on` or `off`; omit to show.
    Yolo {
        id: String,
        state: Option<String>,
    },
}

#[derive(Subcommand)]
enum BoardCmd {
    List,
    Unschedule { rid: String },
    Promote { rid: String },
    Estimate { rid: String, tokens: i64 },
    /// Assign who runs this ticket: `grok::grok-4` or `qwen`.
    Agent { rid: String, spec: String },
    /// Set an epic's token budget and/or agent.
    Epic {
        name: String,
        #[arg(long)]
        tokens: Option<i64>,
        #[arg(long)]
        agent: Option<String>,
    },
    /// Models this project may use: `qwen::qwen3.8:27b-mlx grok::grok-4`
    Pool {
        specs: Vec<String>,
        #[arg(long, default_value = "balanced")]
        prefer: String,
    },
    /// Random, then fit, assignment of unassigned tickets from the pool.
    Allocate,
    /// Add or rename a goal. `shalt board goal g-envelope "Shops exchange a packet"`
    Goal {
        id: String,
        #[arg(trailing_var_arg = true)]
        title: Vec<String>,
    },
    /// Add or rename a milestone. `shalt board milestone m-format "0.0.1 format locked"`
    Milestone {
        id: String,
        #[arg(long)]
        target: Option<String>,
        #[arg(trailing_var_arg = true)]
        title: Vec<String>,
    },
    /// Seat a ticket on a goal and/or milestone.
    Place {
        rid: String,
        #[arg(long)]
        goal: Option<String>,
        #[arg(long)]
        milestone: Option<String>,
    },
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
    /// Jobs that are blocked on a human answer (same as the UI dialog).
    Waiting,
    /// Print the open question. ID optional if only one job is waiting.
    Ask { id: Option<String> },
    /// Chat on a waiting job. Same loop as the desk: the model replies; ANSWER: is a draft.
    /// `--backend` / `--model` pick who drafts (Play stays on the job's agent).
    Chat {
        id: String,
        message: String,
    },
    /// Ask the answer agent to decide and draft ANSWER:.
    Decide {
        id: String,
    },
    /// Commit the agreed answer and unblock Play.
    Answer {
        id: String,
        text: String,
    },
}

fn resolve_waiting<'a>(
    q: &'a JobQueue,
    id: Option<&str>,
) -> Result<&'a shalt_core::jobs::Job, String> {
    if let Some(id) = id.map(str::trim).filter(|s| !s.is_empty()) {
        return q.get(id).ok_or_else(|| format!("no job {id}"));
    }
    let waiting: Vec<_> = q
        .jobs
        .iter()
        .filter(|j| j.status == shalt_core::jobs::JobStatus::Waiting)
        .collect();
    match waiting.as_slice() {
        [] => Err("no job is waiting on you".into()),
        [j] => Ok(*j),
        many => Err(format!(
            "several waiting jobs; pass an id: {}",
            many.iter()
                .map(|j| j.id.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        )),
    }
}

fn print_waiting_turn(j: &shalt_core::jobs::Job) {
    println!("{}  {:?}  {}  {:?}", j.id, j.kind, j.project_id, j.status);
    println!("{}", shalt_core::jobs::status_line(j));
    let turn = j.turns.iter().rev().find(|t| t.answer.is_empty());
    let qn = turn
        .map(|t| t.question.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(j.question.as_str());
    if !qn.is_empty() {
        println!("--- question ---");
        println!("{qn}");
    }
    if !j.answer_backend.is_empty() || !j.answer_model.is_empty() {
        println!(
            "--- answer agent ---  {} / {}",
            j.answer_backend,
            j.answer_model
        );
    } else if !j.backend.is_empty() {
        println!("--- play agent ---  {} / {}", j.backend, j.model);
    }
    if let Some(t) = turn {
        if !t.guess.is_empty() {
            println!("--- guess ---");
            println!("{}", t.guess);
        }
        if !t.chat.is_empty() {
            println!("--- chat ---");
            for m in &t.chat {
                let who = if m.role == "user" {
                    "you".into()
                } else if !m.backend.is_empty() {
                    format!("{} {}", m.backend, m.model)
                } else {
                    "shalt".into()
                };
                println!("{who}: {}", m.content);
            }
        }
        if let Some(last) = shalt_core::last_assistant_on(j) {
            let parsed = shalt_core::jobs::parse_chat_answer(last);
            if parsed != last.trim() {
                println!("--- draft answer ---");
                println!("{parsed}");
            }
        }
    }
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
                Ok(mut b) => {
                    b.on_progress = Some(Box::new(|line| {
                        eprintln!("{line}");
                    }));
                    Ok(Box::new(b))
                }
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

fn print_tokens(res: &RoleResult) {
    if res.prompt_tokens + res.completion_tokens > 0 {
        println!(
            "tokens {} (prompt {} · completion {})",
            res.prompt_tokens + res.completion_tokens,
            res.prompt_tokens,
            res.completion_tokens
        );
    }
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
    if looks_like_feature_arg(pos) {
        args.insert(i, "run".into());
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
    term::init(&cli.color);
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
            let preset = shalt_core::config::init_workspace(&root, stack, name).map_err(|e| {
                eprintln!("{e}");
                1
            })?;
            println!("initialised shalt workspace at {}  ({})", root.display(), preset.label);
            if let Some(note) = shalt_core::config::stack_support_note(stack) {
                println!("note: {note}");
            }
            Ok(0)
        }
        Cmd::Models => cmd_models(&cli),
        Cmd::Do { sentence } => cmd_specify(&cli, &root, &sentence.join(" ")),
        Cmd::Play { max_steps, yolo } => cmd_play(&root, *max_steps, *yolo),
        Cmd::Stop => cmd_stop(),
        Cmd::Design => cmd_design(&cli, &root),
        Cmd::Journal { action } => match action {
            None => cmd_journal(&root),
            Some(JournalCmd::Comment { post, reply, text }) => {
                cmd_journal_comment(&root, &post, reply.as_deref(), &text.join(" "))
            }
        },
        Cmd::Stack { name } => match name {
            None => {
                let cfg = Config::load(&root).unwrap_or_default();
                let label = shalt_core::config::preset(&cfg.stack)
                    .map(|p| p.label)
                    .unwrap_or("");
                println!("{}  {label}", cfg.stack);
                for c in shalt_core::stack_choices() {
                    let on = if c.id == cfg.stack { "*" } else { " " };
                    println!("{on} {}  {}  ({})", c.id, c.label, c.support);
                }
                Ok(0)
            }
            Some(s) => {
                let report = match Org::load().find_by_path(&root).cloned() {
                    Some(p) => shalt_core::restack_project(&p.id, s),
                    None => shalt_core::restack(&root, s),
                }
                .map_err(|e| {
                    eprintln!("{e}");
                    1
                })?;
                println!("{}", report.note);
                for r in &report.removed {
                    println!("  removed {r}");
                }
                Ok(0)
            }
        },
        Cmd::Plan { action } => match action {
            PlanCmd::Export { file } => {
                let name = Org::load()
                    .find_by_path(&root)
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| {
                        root.file_name()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "project".into())
                    });
                let pack = shalt_core::export_pack(&root, &name).map_err(|e| {
                    eprintln!("{e}");
                    1
                })?;
                let json = serde_json::to_string_pretty(&pack).map_err(|e| {
                    eprintln!("{e}");
                    1
                })?;
                match file {
                    Some(p) if p.as_os_str() == "-" => {
                        println!("{json}");
                    }
                    Some(p) => {
                        std::fs::write(p, json + "\n").map_err(|e| {
                            eprintln!("{e}");
                            1
                        })?;
                        println!("exported {}", p.display());
                    }
                    None => {
                        let p = PathBuf::from(format!("{name}.shalt-plan.json"));
                        std::fs::write(&p, json + "\n").map_err(|e| {
                            eprintln!("{e}");
                            1
                        })?;
                        println!("exported {}", p.display());
                    }
                }
                Ok(0)
            }
            PlanCmd::Import { file, dir, name } => {
                let raw = std::fs::read_to_string(file).map_err(|e| {
                    eprintln!("{}: {e}", file.display());
                    1
                })?;
                let pack: shalt_core::PlanPack = serde_json::from_str(&raw).map_err(|e| {
                    eprintln!("not a shalt plan pack: {e}");
                    1
                })?;
                let project = shalt_core::import_pack(&pack, dir.as_deref(), name.as_deref())
                    .map_err(|e| {
                        eprintln!("{e}");
                        1
                    })?;
                println!("imported {} at {}", project.id, project.path);
                println!("Play to start implementing. Spec and plan are already on disk.");
                Ok(0)
            }
        },
        Cmd::Author { request } => {
            let mut b = backend(&cli)?;
            let prompt = author_prompt_with_spec(request, &[], false, &spec_snapshot(&root));
            match run_role(&root, "author", &prompt, b.as_mut(), false) {
                Ok(res) => {
                    println!("author wrote {} file(s):", res.wrote.len());
                    for w in &res.wrote {
                        println!("  {w}");
                    }
                    print_tokens(&res);
                    shalt_core::talk::seed_plan(&root, request);
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
            let cfg = Config::load(&root).unwrap_or_default();
            let features = load_specs(&root.join("spec"), true).map_err(|e| {
                eprintln!("{}", e.message());
                4
            })?;
            if let Ok(Some(p)) = shalt_core::write_js_world_if_missing(&root) {
                println!(
                    "template {}",
                    p.strip_prefix(&root).unwrap_or(&p).display()
                );
            }
            let mut any = false;
            for _ in 0..8 {
                let defs = shalt_core::load_step_defs(&root);
                let Some(journey) =
                    shalt_core::pick_steps_journey(&features, &defs, "")
                else {
                    break;
                };
                any = true;
                println!("steps for journey {journey}");
                let prompt =
                    shalt_core::stepwright_focus_prompt(&features, &journey, &cfg.stack);
                match run_role(&root, "stepwright", &prompt, b.as_mut(), false) {
                    Ok(res) => {
                        println!("stepwright wrote {} file(s)", res.wrote.len());
                        for w in &res.wrote {
                            println!("  {w}");
                        }
                        print_tokens(&res);
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
            }
            if let Ok(stubs) = shalt_core::apply_js_contract_stubs(&root) {
                for s in stubs {
                    println!("stub {s}");
                }
            }
            if !any {
                println!("all journeys already have step bindings");
            }
            Ok(0)
        }
        Cmd::Run { features } => cmd_run(&cli, &root, features),
        Cmd::Build { max_turns, strict } => {
            let (mut led, features) = sync(&root)?;
            if led.spec_lock.as_object().map(|o| o.is_empty()).unwrap_or(true) {
                eprintln!("spec is not approved yet -- run `shalt approve` first");
                return Ok(1);
            }
            let cfg = Config::load(&root).unwrap_or_default();
            if let Ok(stubs) = shalt_core::apply_js_contract_stubs(&root) {
                for s in stubs {
                    println!("stub {s}");
                }
            }
            let mut b = backend(&cli)?;
            let held = holdout_rids(&features);
            let mut skip = held.clone();
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
                shalt_core::promote_final_if_green(&root, &led);
                led.save(&ledger_path(&root)).ok();
                let red_visible: Vec<_> = visible
                    .iter()
                    .filter(|r| matches!(led.entries[*r].status.as_str(), RED | PENDING | STALE))
                    .cloned()
                    .collect();
                println!(
                    "\nturn {turn}: {}/{} visible {}",
                    visible.len() - red_visible.len(),
                    visible.len(),
                    term::ok("green")
                );
                if red_visible.is_empty() {
                    break;
                }
                let Some((feat, scen)) = led.next_ungreen(&features, &skip) else {
                    break;
                };
                let focus_rid = scen.rid.clone().unwrap_or_default();
                let focus_name = scen.name.clone();
                let epic = feat.epic();
                skip.insert(focus_rid.clone());
                println!("  ticket {focus_rid} ({focus_name}) · {epic}");
                let mut allow = HashSet::new();
                allow.insert(focus_rid.clone());
                let mut dump = shalt_core::runner::failure_digest(&run, Some(&allow), 8);
                if !run.collection_error.is_empty() {
                    dump = format!(
                        "{dump}\n\nSUITE:\n{}",
                        run.collection_error.chars().take(2000).collect::<String>()
                    );
                }
                let prompt = format!(
                    "Make scenario {focus_rid} ({focus_name}) pass. That ticket is this job. Other failing scenarios are other tickets — do not try to finish the whole spec in this turn. Fill the src/ stub the failing test imports. Do not rewrite every file.\n\nTEST OUTPUT:\n{dump}"
                );
                match run_role(&root, "implementer", &prompt, b.as_mut(), true) {
                    Ok(res) => {
                        println!("  implementer wrote: {}", res.wrote.join(", "));
                        print_tokens(&res);
                    }
                    Err(RoleError::Integrity(e)) => {
                        println!("\n{}", term::bad(&format!("turn {turn} REJECTED -- {e}")));
                        println!("  {}", term::mute("nothing from this turn was kept; the spec and tests are untouched."));
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
            shalt_core::promote_final_if_green(&root, &led);
            led.save(&ledger_path(&root)).ok();
            let overfit: Vec<_> = held
                .iter()
                .filter(|r| led.entries.get(*r).map(|e| e.status != GREEN).unwrap_or(false))
                .cloned()
                .collect();
            let vis_green = visible.iter().all(|r| led.entries[r].status == GREEN);
            if vis_green && !overfit.is_empty() {
                println!("{}", term::bad("OVERFIT: every visible scenario is green but held-out scenarios fail."));
                for r in &overfit {
                    println!("  {r}  {}", led.entries[r].name);
                }
            }
            print_report(&cli, &led, &features, &all_picks(&features), None, None);
            Ok(0)
        }
        Cmd::Status => {
            let (led, features) = sync(&root)?;
            let picks = selected(&features, &[], cli.tags.as_deref())?;
            print_report(&cli, &led, &features, &picks, None, None);
            Ok(0)
        }
        Cmd::Verify => {
            let (led, features) = sync(&root)?;
            let mut problems = audit_in(Some(&root), &led, &features);
            let board = Board::load(&root.join(".shalt/board.json"));
            problems.extend(verify_drift(&board, &features));
            problems.extend(shalt_core::verify_mockups(&root, &features));
            if problems.is_empty() {
                println!("{}", term::ok("verify: ok"));
                Ok(0)
            } else {
                for p in &problems {
                    println!("  {}", term::warn(p));
                }
                Ok(1)
            }
        }
        Cmd::Tree => {
            let (led, features) = sync(&root)?;
            for f in features {
                println!(
                    "{} {}  {}",
                    term::keyword("STORY"),
                    term::bold(&f.name),
                    term::italic(&f.story().one_line())
                );
                for s in f.scenarios {
                    let st = s
                        .rid
                        .as_ref()
                        .and_then(|r| led.entries.get(r))
                        .map(|e| e.status.as_str())
                        .unwrap_or(PENDING);
                    let mark = if s.is_holdout() {
                        format!(" {}", term::warn("[holdout]"))
                    } else {
                        String::new()
                    };
                    println!(
                        "   {} {} {}{}",
                        term::status(st, status_glyph(st)),
                        s.name,
                        term::mute(&s.rid.unwrap_or_default()),
                        mark
                    );
                }
            }
            Ok(0)
        }
        Cmd::Stories => {
            let (_, features) = sync(&root)?;
            for f in features {
                let st = f.story();
                if st.complete() {
                    println!("{}  {}", term::mute(&f.file), term::italic(&st.one_line()));
                } else {
                    println!("{}  {}", term::mute(&f.file), term::warn(&format!("missing {}", st.missing().join(", "))));
                }
            }
            Ok(0)
        }
        Cmd::Login { host } => cmd_login(host),
        Cmd::Logout => {
            shalt_core::Credentials::clear().ok();
            println!("signed out");
            Ok(0)
        }
        Cmd::Whoami => cmd_whoami(),
        Cmd::Connect { action } => match action {
            None | Some(ConnectCmd::Github) => cmd_login("https://shalt.dev"),
        },
        Cmd::Onboard { source, dir, prompt } => {
            let path = match resolve_onboard_source(source, dir.as_deref()) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("{e}");
                    return Ok(1);
                }
            };
            let backend = if cli.backend == "fixture" {
                ""
            } else {
                cli.backend.as_str()
            };
            match shalt_core::onboard_project(&path, prompt, backend, cli.model.as_deref().unwrap_or("")) {
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
                    let flags = [
                        p.paused.then_some("paused"),
                        match p.yolo_mode_enum() {
                            shalt_core::YoloMode::All => Some("yolo-all"),
                            shalt_core::YoloMode::Plan => Some("yolo-plan"),
                            shalt_core::YoloMode::Off => None,
                        },
                    ]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" ");
                    if flags.is_empty() {
                        println!("  {}  {}  {}", p.id, p.name, p.path);
                    } else {
                        println!("  {}  {}  {}  {}", p.id, p.name, flags, p.path);
                    }
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
                if !org.pause(id, true, Some(shalt_core::org::YOU_PAUSED)) {
                    eprintln!("no project {id}");
                    return Ok(1);
                }
                org.save().ok();
                let mut q = JobQueue::load();
                let n = q.pause_project(id).len();
                q.save().ok();
                println!("paused {id} ({n} job(s) parked) — {}", shalt_core::org::YOU_PAUSED);
                Ok(0)
            }
            OrgCmd::Play { id } => cmd_play_id(id, 8),
            OrgCmd::Yolo { id, state } => {
                let mut org = Org::load();
                let Some(p) = org.get(id).cloned() else {
                    eprintln!("no project {id}");
                    return Ok(1);
                };
                match state.as_deref().map(|s| s.trim().to_ascii_lowercase()).as_deref() {
                    None => {
                        println!("{}  yolo {}", id, p.yolo_mode_enum().as_str());
                        Ok(0)
                    }
                    Some("on") | Some("true") | Some("1") | Some("all") => {
                        org.set_yolo_mode(id, shalt_core::YoloMode::All);
                        org.save().ok();
                        let mut q = JobQueue::load();
                        let n = q.adopt_guesses_for_project(id).len();
                        q.save().ok();
                        println!("yolo all for {id}{}", if n > 0 { format!(" — took {n} guess(es)") } else { String::new() });
                        Ok(0)
                    }
                    Some("plan") | Some("planning") => {
                        org.set_yolo_mode(id, shalt_core::YoloMode::Plan);
                        org.save().ok();
                        println!("yolo plan for {id} — sprint planning takes the guess; spec/tests/code still ask");
                        Ok(0)
                    }
                    Some("off") | Some("false") | Some("0") => {
                        org.set_yolo_mode(id, shalt_core::YoloMode::Off);
                        org.save().ok();
                        println!("yolo off for {id}");
                        Ok(0)
                    }
                    Some(other) => {
                        eprintln!("yolo state must be off, plan, or all, not {other}");
                        Ok(1)
                    }
                }
            }
        },
        Cmd::Board { project: _, action } => {
            let mut board = Board::load(&root.join(".shalt/board.json"));
            let (led, features) = sync(&root)?;
            board.sync_new_rids(&features);
            board.sync_epics(&features);
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
                Some(BoardCmd::Agent { rid, spec }) => {
                    let (backend, model) = shalt_core::tokens::parse_agent(spec);
                    if board.set_item_agent(rid, &backend, &model) {
                        board.save(&root.join(".shalt/board.json")).ok();
                        println!(
                            "{rid} agent {}",
                            shalt_core::tokens::agent_label(&backend, &model)
                        );
                    } else {
                        eprintln!("no ticket {rid} on the board");
                        return Ok(1);
                    }
                }
                Some(BoardCmd::Epic { name, tokens, agent }) => {
                    if let Some(n) = tokens {
                        if !board.set_epic_estimate(name, *n) {
                            eprintln!("could not set epic {name}");
                            return Ok(1);
                        }
                    }
                    if let Some(spec) = agent {
                        let (backend, model) = shalt_core::tokens::parse_agent(spec);
                        if !board.set_epic_agent(name, &backend, &model) {
                            eprintln!("could not set epic {name}");
                            return Ok(1);
                        }
                    }
                    if tokens.is_none() && agent.is_none() {
                        eprintln!("pass --tokens N and/or --agent qwen::model");
                        return Ok(1);
                    }
                    board.save(&root.join(".shalt/board.json")).ok();
                    let e = board.epics.iter().find(|e| e.name == *name);
                    match e {
                        Some(e) => println!(
                            "epic {}  est {}  agent {}",
                            e.name,
                            e.token_estimate,
                            shalt_core::tokens::agent_label(&e.backend, &e.model)
                        ),
                        None => println!("epic {name}"),
                    }
                }
                Some(BoardCmd::Pool { specs, prefer }) => {
                    let mut slots = Vec::new();
                    for spec in specs {
                        let (backend, model) = shalt_core::tokens::parse_agent(spec);
                        if !backend.is_empty() {
                            slots.push(shalt_core::board::PoolSlot { backend, model });
                        }
                    }
                    if slots.is_empty() {
                        slots = shalt_core::alloc::default_pool();
                    }
                    board.pool = slots;
                    board.prefer = if matches!(prefer.as_str(), "cheap" | "balanced" | "fast") {
                        prefer.clone()
                    } else {
                        "balanced".into()
                    };
                    board.save(&root.join(".shalt/board.json")).ok();
                    println!("pool {}  prefer {}", board.pool.len(), board.prefer);
                    for s in &board.pool {
                        println!(
                            "  {}",
                            shalt_core::tokens::agent_label(&s.backend, &s.model)
                        );
                    }
                }
                Some(BoardCmd::Allocate) => {
                    let q = shalt_core::jobs::JobQueue::load();
                    let n = shalt_core::alloc::allocate_unassigned_now(
                        &mut board,
                        &led,
                        &q.jobs,
                        &Org::load()
                            .find_by_path(&root)
                            .map(|p| p.id.clone())
                            .unwrap_or_default(),
                    )
                    .len();
                    board.save(&root.join(".shalt/board.json")).ok();
                    println!("allocated {n} unassigned ticket(s)  prefer {}", board.prefer);
                }
                Some(BoardCmd::Goal { id, title }) => {
                    let title = title.join(" ");
                    let g = board.upsert_goal(Some(id), &title);
                    board.save(&root.join(".shalt/board.json")).ok();
                    println!("goal {}  {}", g.id, g.title);
                }
                Some(BoardCmd::Milestone { id, target, title }) => {
                    let title = title.join(" ");
                    let m = board.upsert_milestone(Some(id), &title, target.as_deref());
                    board.save(&root.join(".shalt/board.json")).ok();
                    println!(
                        "milestone {}  {}{}",
                        m.id,
                        m.title,
                        m.target
                            .as_deref()
                            .map(|t| format!("  target {t}"))
                            .unwrap_or_default()
                    );
                }
                Some(BoardCmd::Place { rid, goal, milestone }) => {
                    if !board.assign(rid, goal.clone(), milestone.clone(), None) {
                        eprintln!("no ticket {rid} on the board");
                        return Ok(1);
                    }
                    board.save(&root.join(".shalt/board.json")).ok();
                    println!(
                        "{rid}  goal {}  milestone {}",
                        goal.as_deref().unwrap_or("—"),
                        milestone.as_deref().unwrap_or("—")
                    );
                }
                _ => {
                    for g in &board.goals {
                        println!("  goal {}  {}", g.id, g.title);
                    }
                    for m in &board.milestones {
                        println!(
                            "  milestone {}  {}{}",
                            m.id,
                            m.title,
                            m.target
                                .as_deref()
                                .map(|t| format!("  target {t}"))
                                .unwrap_or_default()
                        );
                    }
                    for e in &board.epics {
                        println!(
                            "  epic {}  est {}  agent {}",
                            e.name,
                            e.token_estimate,
                            shalt_core::tokens::agent_label(&e.backend, &e.model)
                        );
                    }
                    for it in &board.items {
                        let title = led
                            .entries
                            .get(&it.rid)
                            .map(|e| e.name.as_str())
                            .filter(|s| !s.is_empty())
                            .unwrap_or("");
                        let sp = it.sprint_id.as_deref().unwrap_or("backlog");
                        let who = shalt_core::tokens::agent_label(&it.backend, &it.model);
                        let goal = it.goal_id.as_deref().unwrap_or("—");
                        let mile = it.milestone_id.as_deref().unwrap_or("—");
                        println!(
                            "  {}  {}  est {}  {}  {}  {}  {}  rank {}",
                            it.rid,
                            if title.is_empty() { "—" } else { title },
                            it.token_estimate,
                            if who.is_empty() { "inherit" } else { who.as_str() },
                            sp,
                            goal,
                            mile,
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
                let mut jobs = q.jobs.clone();
                jobs.sort_by_key(|j| {
                    if j.status == shalt_core::jobs::JobStatus::Waiting {
                        0
                    } else {
                        1
                    }
                });
                for j in &jobs {
                    if j.status == shalt_core::jobs::JobStatus::Waiting {
                        println!(
                            "  {}  {:?}  {}  waiting  {}",
                            j.id,
                            j.kind,
                            j.project_id,
                            shalt_core::jobs::status_line(j)
                        );
                    } else {
                        println!("  {}  {:?}  {}  {:?}", j.id, j.kind, j.project_id, j.status);
                    }
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
                        print_waiting_turn(j);
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
            JobCmd::Waiting => {
                let q = JobQueue::load();
                let waiting: Vec<_> = q
                    .jobs
                    .iter()
                    .filter(|j| j.status == shalt_core::jobs::JobStatus::Waiting)
                    .collect();
                if waiting.is_empty() {
                    println!("no job is waiting on you");
                    return Ok(0);
                }
                for j in waiting {
                    print_waiting_turn(j);
                    println!(
                        "next: shalt job chat {} \"…\"  or  shalt job answer {} \"…\"",
                        j.id, j.id
                    );
                    println!();
                }
                Ok(0)
            }
            JobCmd::Ask { id } => {
                let q = JobQueue::load();
                match resolve_waiting(&q, id.as_deref()) {
                    Ok(j) => {
                        print_waiting_turn(j);
                        println!(
                            "next: shalt job chat {} \"…\"  ·  shalt job decide {}  ·  shalt job answer {} \"…\"",
                            j.id, j.id, j.id
                        );
                        Ok(0)
                    }
                    Err(e) => {
                        eprintln!("{e}");
                        Ok(1)
                    }
                }
            }
            JobCmd::Chat { id, message } => {
                let backend = (cli.backend != "fixture").then_some(cli.backend.as_str());
                let model = cli.model.as_deref().filter(|s| !s.is_empty());
                match shalt_core::chat_on_job_with(id, message, backend, model) {
                    Ok(j) => {
                        println!("you: {message}");
                        if let Some(reply) = shalt_core::last_assistant_on(&j) {
                            println!("shalt:\n{reply}");
                            let parsed = shalt_core::jobs::parse_chat_answer(reply);
                            if parsed != reply.trim() {
                                println!("---");
                                println!("draft answer:\n{parsed}");
                                println!("commit: shalt job answer {} \"…\"", j.id);
                            }
                        } else {
                            eprintln!("no reply from the model");
                            return Ok(1);
                        }
                        Ok(0)
                    }
                    Err(e) => {
                        eprintln!("{e}");
                        Ok(1)
                    }
                }
            }
            JobCmd::Decide { id } => {
                let backend = (cli.backend != "fixture").then_some(cli.backend.as_str());
                let model = cli.model.as_deref().filter(|s| !s.is_empty());
                match shalt_core::decide_on_job(id, backend, model) {
                    Ok(j) => {
                        if let Some(reply) = shalt_core::last_assistant_on(&j) {
                            println!("shalt:\n{reply}");
                            let parsed = shalt_core::jobs::parse_chat_answer(reply);
                            if parsed != reply.trim() {
                                println!("---");
                                println!("draft answer:\n{parsed}");
                                println!("commit: shalt job answer {} \"…\"", j.id);
                            }
                        } else {
                            eprintln!("no reply from the model");
                            return Ok(1);
                        }
                        Ok(0)
                    }
                    Err(e) => {
                        eprintln!("{e}");
                        Ok(1)
                    }
                }
            }
            JobCmd::Answer { id, text } => {
                let mut q = JobQueue::load();
                if q.get(id).is_none() {
                    eprintln!("no job {id}");
                    return Ok(1);
                }
                if !q.set_answer(id, text) {
                    eprintln!("could not set the answer");
                    return Ok(1);
                }
                q.append(id, "answered from the CLI");
                q.save().ok();
                println!("answered {id} — Play continues if a worker is attached");
                Ok(0)
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
                    "design" => JobKind::Design,
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
        Cmd::Ui { port, no_open, foreground, action } => match action {
            None => cmd_ui_start(&root, *port, !*no_open, *foreground),
            Some(UiAction::Status) => cmd_ui_status(),
            Some(UiAction::Stop) => cmd_ui_stop(),
            Some(UiAction::Restart) => {
                let _ = cmd_ui_stop();
                cmd_ui_start(&root, *port, !*no_open, *foreground)
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
    println!("  shalt ui status | shalt ui stop | shalt ui restart | shalt stop");
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

fn resolve_onboard_source(source: &str, dir: Option<&Path>) -> Result<PathBuf, String> {
    let src = source.trim();
    let as_path = PathBuf::from(src);
    if as_path.is_dir() {
        return Ok(as_path);
    }
    if shalt_core::looks_like_git_source(src) {
        let name = shalt_core::github_clone_name(src)
            .ok_or_else(|| format!("could not read a repo name from {src}"))?;
        let dest = dir
            .map(|d| d.to_path_buf())
            .unwrap_or_else(|| PathBuf::from(&name));
        eprintln!("cloning {} → {}", shalt_core::normalize_github_url(src), dest.display());
        return shalt_core::clone_git_source(src, &dest);
    }
    Err(format!("{src} is not a directory or a GitHub URL"))
}

fn cmd_login(host: &str) -> Result<i32, i32> {
    let host = host.trim().trim_end_matches('/').to_string();
    let listener = match std::net::TcpListener::bind("127.0.0.1:0") {
        Ok(l) => l,
        Err(e) => {
            eprintln!("cannot listen on localhost: {e}");
            return Ok(1);
        }
    };
    let port = match listener.local_addr() {
        Ok(a) => a.port(),
        Err(e) => {
            eprintln!("{e}");
            return Ok(1);
        }
    };
    let next = format!("http://127.0.0.1:{port}/ok");
    let url = format!(
        "{host}/api/github/start?next={}",
        url_encode(&next)
    );
    println!("Sign in with GitHub: {url}");
    let _ = std::process::Command::new("open").arg(&url).spawn();
    let token = match read_cli_token(&listener) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{e}");
            eprintln!("Onboard a clone without login:  shalt onboard github.com/org/repo");
            return Ok(1);
        }
    };
    if token == "not-configured" {
        eprintln!("GitHub login is not enabled on {host} yet.");
        eprintln!("Create an OAuth App at https://github.com/settings/developers");
        eprintln!("  Homepage: {host}");
        eprintln!("  Callback: {host}/api/github/callback");
        eprintln!("Then set GITHUB_CLIENT_ID and GITHUB_CLIENT_SECRET on the host.");
        eprintln!("Until then:  git clone git@github.com:org/repo.git && shalt onboard ./repo");
        return Ok(1);
    }
    let user = whoami_on_host(&host, &token).unwrap_or_default();
    let cred = shalt_core::Credentials {
        host: host.clone(),
        user: user.clone(),
        token,
    };
    if let Err(e) = cred.save() {
        eprintln!("{e}");
        return Ok(1);
    }
    if user.is_empty() {
        println!("signed in to {host}");
    } else {
        println!("signed in to {host} as {user}");
    }
    Ok(0)
}

fn cmd_whoami() -> Result<i32, i32> {
    let c = shalt_core::Credentials::load();
    if !c.signed_in() {
        println!("not signed in. shalt login");
        return Ok(1);
    }
    match whoami_on_host(&c.host, &c.token) {
        Ok(u) => {
            println!("{u}  {}", c.host);
            Ok(0)
        }
        Err(e) => {
            eprintln!("{e}");
            Ok(1)
        }
    }
}

fn whoami_on_host(host: &str, token: &str) -> Result<String, String> {
    let url = format!("{}/api/me", host.trim_end_matches('/'));
    let resp = ureq::get(&url)
        .set("Authorization", &format!("Bearer {token}"))
        .call()
        .map_err(|e| e.to_string())?;
    let v: serde_json::Value = resp.into_json().map_err(|e| e.to_string())?;
    Ok(v.get("login")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string())
}

fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn read_cli_token(listener: &std::net::TcpListener) -> Result<String, String> {
    listener
        .set_nonblocking(false)
        .map_err(|e| e.to_string())?;
    let (mut stream, _) = listener.accept().map_err(|e| e.to_string())?;
    let mut buf = [0u8; 4096];
    let n = std::io::Read::read(&mut stream, &mut buf).map_err(|e| e.to_string())?;
    let req = String::from_utf8_lossy(&buf[..n]);
    let line = req.lines().next().unwrap_or("");
    let token = line
        .split_whitespace()
        .nth(1)
        .and_then(|p| p.split('?').nth(1))
        .unwrap_or("")
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .find(|(k, _)| *k == "token")
        .map(|(_, v)| v.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "no token in callback".to_string())?;
    let body = "<html><body>Signed in. You can close this tab.</body></html>";
    let _ = std::io::Write::write_all(
        &mut stream,
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .as_bytes(),
    );
    Ok(token)
}

fn cmd_ui_start(root: &Path, port: Option<u16>, open: bool, foreground: bool) -> Result<i32, i32> {
    if let Some(u) = shalt_core::uis::current() {
        if let Some(p) = port {
            if p != u.port {
                eprintln!("shalt ui is already on {} — one instance. shalt ui stop, then --port {p}.", u.url);
            }
        }
        return reopen_ui(u.port, open);
    }
    let serve = foreground
        || std::env::var(shalt_core::uis::SERVE_ENV).ok().as_deref() == Some("1");
    if serve {
        return serve_ui_blocking(root, port, open);
    }
    let exe = std::env::current_exe().map_err(|e| {
        eprintln!("{e}");
        1
    })?;
    let pid = shalt_core::uis::spawn_detached(&exe, root, port).map_err(|e| {
        eprintln!("could not start shalt ui: {e}");
        1
    })?;
    match shalt_core::uis::wait_until_up(std::time::Duration::from_secs(8)) {
        Some(u) => {
            println!("shalt ui on {}  pid {}", u.url, u.pid);
            if open {
                let _ = std::process::Command::new("open").arg(&u.url).spawn();
            }
            Ok(0)
        }
        None => {
            eprintln!(
                "shalt ui pid {pid} did not answer /api/health. log: {}",
                shalt_core::uis::log_path().display()
            );
            let _ = shalt_core::uis::stop_pid(pid);
            Err(1)
        }
    }
}

fn serve_ui_blocking(root: &Path, port: Option<u16>, open: bool) -> Result<i32, i32> {
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

fn cmd_journal(root: &Path) -> Result<i32, i32> {
    let j = shalt_core::Journal::load(root);
    if j.issues.is_empty() {
        println!("no journal yet — agents file a dispatch when a turn finishes");
        return Ok(0);
    }
    println!("The journal  ·  Vol. {}", if j.volume == 0 { 1 } else { j.volume });
    for issue in &j.issues {
        println!();
        println!("— No. {}  {} —", issue.number, issue.date);
        for d in &issue.dispatches {
            let who = [d.desk.as_str(), d.model.as_str(), d.backend.as_str()]
                .into_iter()
                .find(|s| !s.is_empty())
                .unwrap_or("agent");
            println!();
            let kind = if d.form == "feature" { "Feature" } else { "Progress" };
            println!("{}", d.title);
            println!("  {kind} · {who} · {}", d.id);
            println!();
            for para in d.body.split("\n\n") {
                let t = para.trim();
                if !t.is_empty() {
                    println!("{t}");
                    println!();
                }
            }
            for c in &d.comments {
                let nest = if c.parent.is_empty() { "  " } else { "    " };
                println!("{nest}{} · {} · {}", c.by, c.id, c.at);
                for line in c.body.lines() {
                    println!("{nest}{line}");
                }
                println!();
            }
        }
    }
    Ok(0)
}

fn cmd_journal_comment(root: &Path, post: &str, reply: Option<&str>, text: &str) -> Result<i32, i32> {
    let by = std::env::var("USER").unwrap_or_else(|_| "You".into());
    match shalt_core::journal::comment(root, post, reply.unwrap_or(""), &by, text) {
        Ok(c) => {
            println!("comment {} on {post}", c.id);
            Ok(0)
        }
        Err(e) => {
            eprintln!("{e}");
            Ok(1)
        }
    }
}

fn cmd_design(cli: &Cli, root: &Path) -> Result<i32, i32> {
    let mut b = backend(cli)?;
    let prompt = shalt_core::designer_user_prompt(root);
    match run_role(root, "designer", &prompt, b.as_mut(), false) {
        Ok(res) => {
            println!("designer wrote {} file(s)", res.wrote.len());
            for w in &res.wrote {
                println!("  {w}");
            }
            print_tokens(&res);
            if let Ok(promoted) = shalt_core::promote_prototype(root) {
                for p in promoted {
                    println!("product {p}");
                }
            }
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

fn cmd_stop() -> Result<i32, i32> {
    let r = shalt_core::uis::stop_everything(std::process::id());
    if r.procs.is_empty() {
        println!("no shalt processes running");
    } else {
        for p in &r.procs {
            let args = if p.args.chars().count() > 88 {
                format!("{}…", p.args.chars().take(87).collect::<String>())
            } else {
                p.args.clone()
            };
            println!("stopped pid {}  {args}", p.pid);
        }
    }
    if r.jobs > 0 {
        println!("parked {} job(s)", r.jobs);
    }
    if r.projects > 0 {
        println!("paused {} project(s) — Play to start again", r.projects);
    }
    if !r.failed.is_empty() {
        for pid in &r.failed {
            eprintln!("pid {pid} did not exit");
        }
        return Err(1);
    }
    Ok(0)
}

fn print_models(models: &[shalt_core::ModelChoice]) {
    for (i, m) in models.iter().enumerate() {
        let kind = if m.kind == "local" {
            term::ok(&m.kind)
        } else {
            term::accent(&m.kind)
        };
        println!("  {:>2}  {:<36} {}", i + 1, m.id, kind);
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
    eprintln!("{} {} / {}", term::mute("using"), term::accent(&preset), term::accent(&b.model));
    b.on_progress = Some(Box::new(|line| {
        use std::io::Write;
        let painted = term::progress(line);
        let err = std::io::stderr();
        let mut err = err.lock();
        if shalt_core::jobs::heartbeat_line(line) && std::io::stderr().is_terminal() {
            let _ = write!(err, "\r{painted}        ");
            let _ = err.flush();
        } else {
            let _ = write!(err, "\r\x1b[K");
            let _ = writeln!(err, "{painted}");
        }
    }));
    if !cli.yes && io::stdin().is_terminal() {
        b.on_ask = Some(Box::new(|question: &str, guess: &str| {
            loop {
                println!("\n{}", term::accent(&format!("? {question}")));
                if !guess.is_empty() {
                    println!("  {}", term::mute(&format!("suggested: {guess}")));
                }
                print!("{} ", term::accent(">"));
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

enum Keep {
    Yes,
    No,
    All,
}

fn prompt_keep(label: &str) -> Result<Keep, i32> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    loop {
        print!("{} {} ", label, term::mute("[y/n/a]"));
        let _ = stdout.flush();
        let mut line = String::new();
        if stdin.lock().read_line(&mut line).is_err() {
            return Err(1);
        }
        match line.trim().to_lowercase().as_str() {
            "y" | "yes" => return Ok(Keep::Yes),
            "n" | "no" => return Ok(Keep::No),
            "a" | "all" => return Ok(Keep::All),
            _ => eprintln!("  type y, n, or a (approve all remaining)"),
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
    let mut accept_rest = false;
    for f in &features {
        let path = spec.join(&f.file);
        let raw = std::fs::read_to_string(&path).unwrap_or_default();
        let lines: Vec<String> = raw.replace("\r\n", "\n").split('\n').map(|s| s.to_string()).collect();
        let n = if lines.last().map(|s| s.is_empty()).unwrap_or(false) {
            lines.len() - 1
        } else {
            lines.len()
        };
        println!(
            "\n{}  ({})",
            term::gherkin(&format!("Feature: {}", f.name)).trim_end(),
            term::mute(&f.file)
        );
        for (sc, start, end) in f.blocks(n) {
            let s = start.saturating_sub(1);
            let e = end.saturating_sub(1).min(n);
            let body = if s < e { lines[s..e].join("\n") } else { sc.name.clone() };
            println!("\n{}\n", term::gherkin(&body));
            if accept_rest {
                kept += 1;
                continue;
            }
            match prompt_keep("Keep this scenario?")? {
                Keep::Yes => kept += 1,
                Keep::No => {
                    drop.push((f.file.clone(), start));
                    println!("  {}", term::bad("dropped."));
                }
                Keep::All => {
                    kept += 1;
                    accept_rest = true;
                    println!("  {}", term::ok("approved this and the rest."));
                }
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
        None => println!("  forecasts fill in as shalt learns; spent vs forecast is the retro"),
    }
}

fn cmd_play(root: &Path, max_steps: usize, yolo: bool) -> Result<i32, i32> {
    if !root.join("shalt.toml").exists() && !root.join("spec").exists() {
        eprintln!("no shalt workspace at {}", root.display());
        return Ok(1);
    }
    let pref = Org::ensure_registered(root).map_err(|e| {
        eprintln!("{e}");
        1
    })?;
    if yolo {
        let mut org = Org::load();
        org.set_yolo(&pref.id, true);
        org.save().ok();
        let mut q = JobQueue::load();
        let n = q.adopt_guesses_for_project(&pref.id).len();
        q.save().ok();
        println!(
            "yolo on for {}{}",
            pref.id,
            if n > 0 {
                format!(" — took {n} guess(es)")
            } else {
                String::new()
            }
        );
    }
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
        let stack = shalt_core::config::detect_stack(root);
        shalt_core::config::init_workspace(root, stack, &name).map_err(|e| {
            eprintln!("{e}");
            1
        })?;
        println!("initialised workspace at {}", root.display());
    }
    let mut b = live_backend(cli)?;
    println!("authoring spec…");
    match run_role(
        root,
        "author",
        &author_prompt_with_spec(sentence, &[], false, &spec_snapshot(root)),
        &mut b,
        false,
    ) {
        Ok(res) => {
            println!("author wrote {} file(s):", res.wrote.len());
            for w in &res.wrote {
                println!("  {w}");
            }
            print_tokens(&res);
            shalt_core::talk::seed_plan(root, sentence);
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
        shalt_core::api::stepwright_user_prompt(&Config::load(root).map(|c| c.stack).unwrap_or_default()),
        &mut b,
        false,
    ) {
        Ok(res) => {
            println!("stepwright wrote {} file(s):", res.wrote.len());
            for w in &res.wrote {
                println!("  {w}");
            }
            print_tokens(&res);
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

fn parse_tags(expr: Option<&str>) -> Result<TagExpr, i32> {
    match expr {
        None | Some("") => Ok(TagExpr::All),
        Some(s) => TagExpr::parse(s).map_err(|e| {
            eprintln!("{e}");
            2
        }),
    }
}

fn selected(
    features: &[shalt_core::Feature],
    locators: &[Locator],
    tags: Option<&str>,
) -> Result<Vec<Pick>, i32> {
    let expr = parse_tags(tags)?;
    Ok(filter_scenarios(features, locators, &expr))
}

fn all_picks(features: &[shalt_core::Feature]) -> Vec<Pick> {
    filter_scenarios(features, &[], &TagExpr::All)
}

fn glob_match(name: &str, pat: &str) -> bool {
    if let Some((pre, suf)) = pat.split_once('*') {
        name.starts_with(pre) && name.ends_with(suf) && name.len() >= pre.len() + suf.len()
    } else {
        name == pat
    }
}

fn glob_features(root: &Path, pattern: &str) -> Vec<String> {
    let p = Path::new(pattern);
    let parent = p.parent().unwrap_or(Path::new("."));
    let file_pat = p.file_name().and_then(|s| s.to_str()).unwrap_or("*.feature");
    let mut dirs = vec![parent.to_path_buf()];
    let under_root = root.join(parent);
    if under_root != parent {
        dirs.push(under_root);
    }
    let mut out = Vec::new();
    for dir in dirs {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for ent in rd.flatten() {
            let name = ent.file_name();
            let name = name.to_string_lossy();
            if glob_match(&name, file_pat) {
                out.push(ent.path().display().to_string());
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

fn expand_feature_args(root: &Path, args: &[String]) -> Result<Vec<Locator>, i32> {
    let mut out = Vec::new();
    for a in args {
        if a.contains('*') {
            let expanded = glob_features(root, a);
            if expanded.is_empty() {
                eprintln!("no feature files match {a}");
                return Err(1);
            }
            for p in expanded {
                match Locator::parse(&p) {
                    Some(l) => out.push(l),
                    None => {
                        eprintln!("not a feature file: {p}");
                        return Err(1);
                    }
                }
            }
        } else {
            match Locator::parse(a) {
                Some(l) => out.push(l),
                None => {
                    eprintln!("not a feature file: {a}");
                    return Err(1);
                }
            }
        }
    }
    Ok(out)
}

fn pytest_k(names: &[String]) -> String {
    names
        .iter()
        .map(|n| {
            if n.chars().any(|c| c.is_whitespace() || "()'\"".contains(c)) {
                format!("\"{}\"", n.replace('"', ""))
            } else {
                n.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" or ")
}

fn pytest_filter(
    cfg: &Config,
    features: &[shalt_core::Feature],
    picks: &[Pick],
) -> Vec<String> {
    let total: usize = features.iter().map(|f| f.scenarios.len()).sum();
    if picks.is_empty() || picks.len() == total {
        return vec![];
    }
    if !cfg.command.contains("pytest") {
        return vec![];
    }
    let names: Vec<String> = picks
        .iter()
        .map(|p| features[p.feature].scenarios[p.scenario].name.clone())
        .collect();
    vec!["-k".into(), pytest_k(&names)]
}

fn print_report(
    cli: &Cli,
    led: &Ledger,
    features: &[shalt_core::Feature],
    picks: &[Pick],
    results: Option<&std::collections::HashMap<String, shalt_core::RunResult>>,
    duration: Option<f64>,
) {
    pretty::render(
        features,
        picks,
        led,
        results,
        pretty::resolve(&cli.formatter),
        duration,
    );
}

fn cmd_run(cli: &Cli, root: &Path, features: &[String]) -> Result<i32, i32> {
    let (mut led, all) = sync(root)?;
    let locators = if features.is_empty() {
        vec![]
    } else {
        expand_feature_args(root, features)?
    };
    let picks = selected(&all, &locators, cli.tags.as_deref())?;
    if picks.is_empty() {
        print_report(cli, &led, &all, &picks, None, Some(0.0));
        return Ok(0);
    }
    if cli.dry_run {
        print_report(cli, &led, &all, &picks, None, None);
        return Ok(0);
    }
    let cfg = Config::load(root).unwrap_or_default();
    let extra = pytest_filter(&cfg, &all, &picks);
    let run = run_suite_with(root, &cfg, &extra);
    if run.harness_error {
        eprintln!("the test harness failed to run");
        eprintln!("{}\n{}", run.stderr, run.stdout);
        return Ok(3);
    }
    let out = led.apply_run(&run.results, &run.run_id, &run.collection_error);
    shalt_core::promote_final_if_green(root, &led);
    led.save(&ledger_path(root)).ok();
    if !out.regressions.is_empty() {
        println!("{} REGRESSION(S)", out.regressions.len());
    }
    print_report(cli, &led, &all, &picks, Some(&run.results), Some(run.duration));
    let any_red = picks.iter().any(|p| {
        pretty::kind_of(&all[p.feature].scenarios[p.scenario], &led, Some(&run.results))
            == pretty::Kind::Failed
    });
    Ok(if any_red { 1 } else { 0 })
}

mod pretty;
mod server;
mod term;

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
        assert_eq!(out(inject_do(s(&["shalt", "stop"]))), ["shalt", "stop"]);
        assert_eq!(out(inject_do(s(&["shalt", "design"]))), ["shalt", "design"]);
        assert_eq!(out(inject_do(s(&["shalt", "journal"]))), ["shalt", "journal"]);
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

    #[test]
    fn feature_path_becomes_run() {
        assert_eq!(
            out(inject_do(s(&["shalt", "spec/foo.feature"]))),
            ["shalt", "run", "spec/foo.feature"]
        );
        assert_eq!(
            out(inject_do(s(&["shalt", "spec/foo.feature:12", "--tags", "@wip"]))),
            ["shalt", "run", "spec/foo.feature:12", "--tags", "@wip"]
        );
    }

    #[test]
    fn spec_subcommand_is_not_a_feature_path() {
        assert_eq!(
            out(inject_do(s(&["shalt", "spec", "delete", "invoices.feature", "4"]))),
            ["shalt", "spec", "delete", "invoices.feature", "4"]
        );
    }

    #[test]
    fn tags_then_feature_is_run() {
        assert_eq!(
            out(inject_do(s(&["shalt", "--tags", "@wip", "spec/foo.feature"]))),
            ["shalt", "--tags", "@wip", "run", "spec/foo.feature"]
        );
    }
}
