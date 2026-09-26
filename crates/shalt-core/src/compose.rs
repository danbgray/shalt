use crate::api::system_for;
use crate::config::{detect_stack, ensure_workspace, init_workspace};
use crate::jobs::{Job, JobKind, JobQueue, JobStatus};
use crate::ledger::Ledger;
use crate::narrative::slug;
use crate::org::{Org, ProjectRef};
use crate::roles::{RoleError, RoleResult};
use crate::spec::load_specs;
use crate::OpenAICompatBackend;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub fn author_user_prompt(request: &str) -> String {
    author_prompt(request, &[])
}

pub fn author_prompt(request: &str, turns: &[crate::jobs::TalkTurn]) -> String {
    author_prompt_mode(request, turns, false)
}

pub fn author_prompt_mode(request: &str, turns: &[crate::jobs::TalkTurn], onboard: bool) -> String {
    author_prompt_with_spec(request, turns, onboard, "")
}

/// `spec_text` is the current spec/ snapshot so the author does not list_files
/// or re-read every feature on each round.
pub fn author_prompt_with_spec(
    request: &str,
    turns: &[crate::jobs::TalkTurn],
    onboard: bool,
    spec_text: &str,
) -> String {
    let mut s = if onboard {
        format!(
            "This is an EXISTING codebase. Write spec/*.feature files (Feature:, Scenario:, Given/When/Then) that describe behaviour the code already implements. Markdown headings and bullets are refused. Do not invent features that are not in the code. Do not modify src/. Ask if the code is ambiguous.\nIf spec/ already has Feature/Scenario files, keep them. Add or finish only what is missing.\n\nNOTE FROM THE HUMAN:\n{request}\n"
        )
    } else {
        format!("Translate this request into spec/*.feature files in this shape:\n@epic:<area>\nFeature: <name>\n  Scenario: <one behaviour>\n    Given ...\n    When ...\n    Then ...\n    #observe: <door the implementation cannot fake>\nMarkdown headings (# ) are refused. #observe: is required after every Then — lock the surface in the spec before Play, like a threshold. Changing Then or #observe: after Play is an amendment. Concrete names and amounts. If spec/ already has Feature/Scenario files, keep them and add only what's missing.\nDo not call list_files. Write each feature file at most once, then done().\n\nREQUEST:\n{request}\n")
    };
    let decided: Vec<_> = turns.iter().filter(|t| !t.answer.trim().is_empty()).collect();
    if !decided.is_empty() {
        s.push_str("\nThe human has already decided the following. Do not ask these again. Write scenarios that cover them.\n");
        for t in decided {
            s.push_str(&format!("\nQ: {}\nA: {}\n", t.question.trim(), t.answer.trim()));
        }
    }
    if spec_text.trim().is_empty() {
        s.push_str("\nNo spec files yet.\n");
    } else if !crate::spec::looks_like_gherkin(spec_text) {
        s.push_str("\nCURRENT SPEC is not valid (no Scenario: lines). Replace those files with Feature/Scenario/Given/When/Then. Markdown bullets are not a spec.\n");
        s.push_str(spec_text.trim());
        s.push_str("\n");
    } else {
        s.push_str("\nCURRENT SPEC:\n");
        s.push_str(spec_text.trim());
        s.push_str("\n\nThis spec was seeded from the interview. Review it. Override any scenario that is wrong, incomplete, or missing an interview fact. Keep Feature/Scenario/When/Then — markdown headings are refused. If the seed already matches the request, call done().\n");
    }
    s
}

const SPEC_SNAPSHOT_CAP: usize = 24_000;

pub fn spec_snapshot(root: &Path) -> String {
    let spec = root.join("spec");
    if !spec.is_dir() {
        return String::new();
    }
    let mut files: Vec<PathBuf> = Vec::new();
    for (p, is_link) in crate::integrity::iter_files(&spec) {
        if is_link {
            continue;
        }
        if p.extension().and_then(|s| s.to_str()) == Some("feature") {
            files.push(p);
        }
    }
    files.sort();
    let mut out = String::new();
    for p in files {
        let rel = p.strip_prefix(root).unwrap_or(&p);
        let body = std::fs::read_to_string(&p).unwrap_or_default();
        let chunk = format!("--- {} ---\n{body}\n", rel.display());
        if out.len() + chunk.len() > SPEC_SNAPSHOT_CAP {
            out.push_str("… (further feature files omitted)\n");
            break;
        }
        out.push_str(&chunk);
    }
    out
}

pub fn author_system_prompt() -> &'static str {
    system_for("author")
}

pub fn designer_user_prompt(root: &Path) -> String {
    designer_user_prompt_for(root, "")
}

pub fn designer_user_prompt_for(root: &Path, focus_rid: &str) -> String {
    designer_user_prompt_scoped(root, focus_rid, "")
}

fn designer_user_prompt_scoped(root: &Path, focus_rid: &str, focus_journey: &str) -> String {
    let features = load_specs(&root.join("spec"), false).unwrap_or_default();
    let films = crate::mockups::films(root, &features, &Ledger::default());
    let journey_nav = {
        let names: Vec<&str> = films
            .iter()
            .filter(|f| f.kind != "none")
            .map(|f| f.journey.as_str())
            .collect();
        if names.is_empty() {
            "this spec's journeys, in spec order".into()
        } else {
            names.join(", ")
        }
    };
    let mut s = format!(
        "Fill the sketch framework. Layout only — shalt's sketch sheet is the look (ink on paper). Do not write an <html> document, product chrome, or a component stylesheet. shalt wraps beat regions and injects primary nav from this spec's journeys ({journey_nav}) plus a beat list.\n\
         One HTML file per journey at mockups/journeys/<journey>/<journey>.html: <section data-rid=\"S-…\">…</section> for each empty beat. main with one H1 and one primary action. Fillable fields. Unproven paths: red stub “This isn't proven yet. The test does not pass.” Never href=\"#\", never alert(). Do not invent journeys.\n\
         Each journey needs mockups/journeys/<journey>/storyboard.json as {{\"journey\",\"kind\":\"ui\",\"spec_hash\":<hash below>,\"frames\":[{{\"rid\",\"file\":\"<journey>.html\",\"caption\"}}]}}. spec_hash must match. kind=none if no UI. Do not write mockups/storyboard.json or HTML at the mockups root.\n\
         Copy titles, emails, prices, URLs, and names from the spec. Do not invent replacements.\n\
         Do not restyle controls. No light text on buttons. tokens.sketch.css if missing is :root vars only (--paper, --ink, --accent). tokens.final.css waits for polish or Build — not this turn.\n\
         Fill empty frames below. Do not rewrite drawn journeys. Do not list_files. Do not write spec/, tests, contract/, src/, or .shalt/. Call done() as soon as every empty beat is drawn.\n\n",
    );
    s.push_str(&crate::markups::designer_markup_prompt(root, focus_journey, focus_rid));
    match crate::mockups::load_kit(root) {
        Some(kit) if crate::mockups::kit_has_platform(&kit) => {
            s.push_str(&format!(
                "Platform is set: {}. Layout HTML only — the sketch sheet is the look. Color, style, fonts, and layout are a separate design interview after every screen is drawn.\n",
                crate::mockups::normalize_platform(&kit.platform)
            ));
            if !kit.style.trim().is_empty() || !kit.color.trim().is_empty() || !kit.layout.trim().is_empty() {
                s.push_str(&format!(
                    "  kit notes: {} · {} · {}\n",
                    kit.style, kit.color, kit.layout
                ));
            }
            s.push('\n');
        }
        Some(_) => {
            s.push_str(
                "Look is partly set but platform is not. FIRST ask_human: Phone, Tablet, or Desktop? Concrete guess: Desktop. Write platform into mockups/kit.json before drawing more HTML. If the human is not answering (yolo), use the guess and set guessed=true.\n\n",
            );
        }
        None => {
            s.push_str(
                "No look kit yet. FIRST ask_human one question: Phone, Tablet, or Desktop? Concrete guess: Desktop. Write mockups/kit.json {platform,guessed}. Style, color, fonts, and layout wait for the design interview after every screen is drawn. tokens.sketch.css if missing is :root {--paper,--ink,--accent} only — no component rules. If the human is not answering (yolo), use the guess, set guessed=true, and continue. Do not draw journey HTML until platform is set.\n\n",
            );
        }
    }
    let journey = focus_journey.trim();
    if !journey.is_empty() {
        s.push_str(&format!(
            "Focus: draw journey `{journey}` only. Other journeys may stay as they are.\n\n"
        ));
    }
    let focus = focus_rid.trim();
    if !focus.is_empty() {
        let name = films
            .iter()
            .flat_map(|f| f.frames.iter())
            .find(|fr| fr.rid == focus)
            .map(|fr| fr.name.as_str())
            .unwrap_or("");
        s.push_str(&format!(
            "Focus: draw only rid `{focus}`{name}. Write its HTML and storyboard.json frame. Other empty frames may stay empty.\n\n",
            name = if name.is_empty() {
                String::new()
            } else {
                format!(" ({name})")
            }
        ));
    }
    if films.is_empty() {
        s.push_str("No journeys in the spec yet.\n");
        return s;
    }
    let mut empty_n = 0usize;
    for film in &films {
        if !journey.is_empty() && film.journey != journey {
            continue;
        }
        if film.kind == "none" {
            s.push_str(&format!(
                "Journey `{j}` kind=none spec_hash={hash} — no screen\n",
                j = film.journey,
                hash = film.spec_hash
            ));
            continue;
        }
        let drawn = film
            .frames
            .iter()
            .filter(|fr| crate::mockups::frame_is_drawn(root, fr))
            .count();
        let total = film.frames.len();
        if drawn == total && focus.is_empty() {
            s.push_str(&format!(
                "Journey `{j}` spec_hash={hash} — {drawn}/{total} drawn\n",
                j = film.journey,
                hash = film.spec_hash
            ));
            continue;
        }
        s.push_str(&format!(
            "Journey `{j}` kind={kind} spec_hash={hash} — {drawn}/{total} drawn\n",
            j = film.journey,
            kind = film.kind,
            hash = film.spec_hash
        ));
        for fr in &film.frames {
            let drawn_fr = crate::mockups::frame_is_drawn(root, fr);
            if drawn_fr && (focus.is_empty() || fr.rid != focus) {
                continue;
            }
            empty_n += usize::from(!drawn_fr);
            let mark = if !focus.is_empty() && fr.rid == focus {
                " ← this beat"
            } else if drawn_fr {
                ""
            } else {
                " (empty)"
            };
            let dest = if fr.file.is_empty() {
                format!("{}.html", film.journey)
            } else {
                std::path::Path::new(&fr.file)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(fr.file.as_str())
                    .to_string()
            };
            s.push_str(&format!(
                "  - {} {}{mark} → mockups/journeys/{j}/{dest}\n",
                fr.rid,
                fr.name,
                j = film.journey
            ));
        }
        s.push('\n');
    }
    if empty_n > 0 && focus.is_empty() {
        s.push_str(&format!(
            "{empty_n} empty UI frame(s) remain. Write them before you stop.\n"
        ));
    }
    s
}

pub fn execute_design(job_id: &str) -> Result<String, String> {
    let mut q = JobQueue::load();
    let mut job = q
        .jobs
        .iter()
        .find(|j| j.id == job_id)
        .cloned()
        .ok_or_else(|| format!("no job {job_id}"))?;
    q.set_status(job_id, JobStatus::Running);
    crate::parallel::mark_start(&mut q, job_id);
    let _ = q.save();
    let org = Org::load();
    let project = org
        .get(&job.project_id)
        .ok_or_else(|| format!("unknown project {}", job.project_id))?;
    let root = PathBuf::from(&project.path);
    let _ = job.draft.restore_to(&root);
    let mut last_transcript = String::new();
    let mut tries = 0usize;
    loop {
        tries += 1;
        if let Some(latest) = JobQueue::load().get(job_id).cloned() {
            job = latest;
        }
        let mut prompt = crate::journal::with_pending(
            &root,
            &designer_user_prompt_scoped(&root, &job.rid, &job.epic),
        );
        if !job.prompt.trim().is_empty() {
            prompt.push_str("\nNOTE:\n");
            prompt.push_str(job.prompt.trim());
            prompt.push('\n');
        }
        let result = run_role_or_failover(&job, &root, "designer", &prompt, false);
        let mut q = JobQueue::load();
        match result {
            Ok(res) => {
                if q.keep_parked(
                    job_id,
                    "model returned after Pause — discarded. Play to continue.",
                ) {
                    let _ = q.save();
                    return Err("stopped (Paused)".into());
                }
                last_transcript = res.transcript.clone();
                let features = load_specs(&root.join("spec"), false).unwrap_or_default();
                crate::mockups::refresh_thumbs(&root, &features);
                if res.wrote.is_empty() && crate::mockups::design_needed(&root, &features) {
                    if tries <= crate::alloc::WRITE_MODELS.len()
                        && hop_write_lane(&mut job, "design dump — next writer draws")
                    {
                        continue;
                    }
                }
                let promoted = crate::scaffold::promote_prototype(&root).unwrap_or_default();
                let summary = if promoted.is_empty() {
                    format!("wrote {}: {}", res.wrote.len(), res.wrote.join(", "))
                } else {
                    format!(
                        "wrote {}: {}; prototype is the product ({})",
                        res.wrote.len(),
                        res.wrote.join(", "),
                        promoted.join(", ")
                    )
                };
                q.append(job_id, &summary);
                q.set_status(job_id, JobStatus::Done);
                let _ = q.save();
                if let Some(j) = q.get(job_id).cloned() {
                    let _ = crate::journal::publish(&root, &j, &last_transcript);
                    let _ = crate::journal::note_design_up(&root);
                    crate::parallel::mark_finish(&j);
                }
                return Ok(summary);
            }
            Err(e) => {
                let stopped = e.to_string();
                let short = crate::jobs::human_error(&stopped);
                if q.keep_parked(
                    job_id,
                    &format!("parked while waiting on the model ({short})"),
                ) {
                    let _ = q.save();
                    return Err(stopped);
                }
                if stopped.contains("model switched") {
                    q.append(
                        job_id,
                        "old model call dropped — Play continues on the new one",
                    );
                    let _ = q.save();
                    return Err(stopped);
                }
                if looks_like_local_stall(&stopped)
                    && tries <= crate::alloc::WRITE_MODELS.len()
                    && hop_write_lane(&mut job, "design stall — next writer draws")
                {
                    continue;
                }
                q.set_error(job_id, &short);
                q.append(job_id, &format!("failed: {short}"));
                let status = if stopped.contains("stopped") {
                    JobStatus::Interrupted
                } else {
                    JobStatus::Failed
                };
                q.set_status(job_id, status);
                let _ = q.save();
                return Err(stopped);
            }
        }
    }
}

pub struct ComposeRequest {
    pub prompt: String,
    pub backend: String,
    pub model: String,
    pub name: Option<String>,
    /// Parent or vacant folder for a new project. Empty = `~/.shalt/projects/<name>`.
    pub dir: Option<PathBuf>,
    /// rust (supported), javascript (supported), python (next), …
    pub stack: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DirEntry {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DirListing {
    pub dir: String,
    pub parent: Option<String>,
    pub entries: Vec<DirEntry>,
}

fn expand_dir(raw: &Path) -> PathBuf {
    let s = raw.to_string_lossy();
    if s == "~" || s.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            if s == "~" {
                return home;
            }
            return home.join(&s[2..]);
        }
    }
    raw.to_path_buf()
}

fn dir_is_vacant(p: &Path) -> bool {
    if !p.exists() {
        return true;
    }
    if p.join("shalt.toml").exists() || p.join("Cargo.toml").exists() || p.join("spec").is_dir() {
        return false;
    }
    let Ok(rd) = std::fs::read_dir(p) else {
        return false;
    };
    !rd.flatten().any(|e| {
        let name = e.file_name();
        let n = name.to_string_lossy();
        !n.starts_with('.')
    })
}

fn unique_child(parent: &Path, name: &str) -> PathBuf {
    let dest = parent.join(name);
    if !dest.exists() {
        return dest;
    }
    parent.join(format!("{name}-{:04x}", rand::random::<u16>()))
}

pub fn resolve_new_project_dir(dir: Option<&Path>, name: &str) -> PathBuf {
    match dir {
        None => {
            let dest = Org::home_dir().join("projects").join(name);
            if dest.exists() {
                Org::home_dir()
                    .join("projects")
                    .join(format!("{name}-{:04x}", rand::random::<u16>()))
            } else {
                dest
            }
        }
        Some(raw) => {
            let parent = expand_dir(raw);
            if dir_is_vacant(&parent) {
                parent
            } else {
                unique_child(&parent, name)
            }
        }
    }
}

pub fn list_dirs(dir: &Path) -> Result<DirListing, String> {
    let dir = expand_dir(dir);
    let dir = if dir.exists() {
        dir.canonicalize().map_err(|e| e.to_string())?
    } else {
        return Err(format!("{} is not a directory", dir.display()));
    };
    if !dir.is_dir() {
        return Err(format!("{} is not a directory", dir.display()));
    }
    let parent = dir.parent().map(|p| p.display().to_string());
    let mut entries = Vec::new();
    let rd = std::fs::read_dir(&dir).map_err(|e| e.to_string())?;
    for e in rd.flatten() {
        let Ok(ft) = e.file_type() else { continue };
        if !ft.is_dir() {
            continue;
        }
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        entries.push(DirEntry {
            name,
            path: e.path().display().to_string(),
        });
    }
    entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(DirListing {
        dir: dir.display().to_string(),
        parent,
        entries,
    })
}

pub fn start_project(req: ComposeRequest) -> Result<(ProjectRef, Job), String> {
    let prompt = req.prompt.trim();
    if prompt.is_empty() {
        return Err("describe the project first".into());
    }
    let backend = resolve_backend(&req.backend)?;
    let name = req
        .name
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| {
            let first = prompt.lines().next().unwrap_or(prompt);
            let s = slug(&first.chars().take(48).collect::<String>(), "project");
            if s.is_empty() { "project".into() } else { s }
        });
    let dir = resolve_new_project_dir(req.dir.as_deref(), &name);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stack = req.stack.trim();
    if stack.is_empty() {
        crate::config::init_plan_workspace(&dir, &name)?;
    } else {
        crate::config::preset(stack).ok_or_else(|| format!("unknown stack {stack:?}"))?;
        init_workspace(&dir, stack, &name)?;
    }
    crate::talk::seed_plan(&dir, prompt);
    let mut org = Org::load();
    let project = org.add(&dir)?;
    org.save().map_err(|e| e.to_string())?;
    let mut q = JobQueue::load();
    let job = q.enqueue_full(JobKind::Author, &project.id, prompt, &backend, &req.model);
    q.save().map_err(|e| e.to_string())?;
    Ok((project, job))
}

/// Change a project's build language. Parks live jobs. Does not Play.
pub fn restack_project(project_id: &str, stack: &str) -> Result<crate::config::RestackReport, String> {
    let org = Org::load();
    let p = org
        .get(project_id)
        .ok_or_else(|| format!("no project {project_id}"))?;
    if !p.paused {
        let q = JobQueue::load();
        let live = q.jobs.iter().any(|j| {
            j.project_id == project_id
                && matches!(
                    j.status,
                    JobStatus::Running | JobStatus::Pending | JobStatus::Waiting
                )
        });
        if live {
            return Err("Pause Play first, then change the build language.".into());
        }
    }
    let report = crate::config::restack(std::path::Path::new(&p.path), stack)?;
    let mut q = JobQueue::load();
    if !q.abandon_for_restart(project_id).is_empty() {
        let _ = q.save();
    }
    Ok(report)
}

fn resolve_backend(backend: &str) -> Result<String, String> {
    match backend {
        "grok" | "openai" | "qwen" | "ollama" => Ok(backend.into()),
        "" => {
            if crate::api::xai_api_key().is_some() {
                Ok("grok".into())
            } else {
                Ok("qwen".into())
            }
        }
        other => Err(format!("unknown backend {other:?}")),
    }
}

pub fn onboard_project(
    path: &std::path::Path,
    prompt: &str,
    backend: &str,
    model: &str,
) -> Result<(ProjectRef, Job), String> {
    let path = path
        .canonicalize()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if !path.is_dir() {
        return Err(format!("{} is not a directory", path.display()));
    }
    let stack = detect_stack(&path);
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "project".into());
    ensure_workspace(&path, stack, &name)?;
    let mut org = Org::load();
    let project = match org.add(&path) {
        Ok(p) => p,
        Err(_) => org
            .projects
            .iter()
            .find(|p| p.path == path.display().to_string() || p.id == name)
            .cloned()
            .ok_or_else(|| format!("{name} is already in the org"))?,
    };
    org.save().map_err(|e| e.to_string())?;
    let note = if prompt.trim().is_empty() {
        "Characterize the existing system. Cover the main user-facing behaviours.".into()
    } else {
        prompt.trim().to_string()
    };
    crate::talk::seed_plan(&path, &note);
    let backend = resolve_backend(backend)?;
    let mut q = JobQueue::load();
    let job = q.enqueue_full(JobKind::Author, &project.id, &note, &backend, model);
    if let Some(j) = q.jobs.iter_mut().find(|j| j.id == job.id) {
        j.onboard = true;
    }
    let job = q.jobs.iter().find(|j| j.id == job.id).cloned().unwrap();
    q.save().map_err(|e| e.to_string())?;
    Ok((project, job))
}

/// Shared LLM controls so pause/play and ask work for author, stepwright, and implementer.
pub fn backend_for_job(job: &Job) -> Result<OpenAICompatBackend, String> {
    let mut backend = OpenAICompatBackend::from_preset(
        if job.backend.is_empty() { "qwen" } else { &job.backend },
        if job.model.is_empty() { None } else { Some(job.model.as_str()) },
        None,
    )?;
    if job.backend.is_empty() || job.model.is_empty() {
        let mut q = JobQueue::load();
        q.set_agent(&job.id, &backend.name, &backend.model);
        let _ = q.save();
    }
    let jid = job.id.clone();
    let jid_prog = jid.clone();
    backend.on_progress = Some(Box::new(move |line: &str| {
        if crate::jobs::heartbeat_line(line) {
            let q = JobQueue::load();
            q.tick(&jid_prog, line);
            return;
        }
        if !crate::jobs::progress_line(line) {
            return;
        }
        let mut q = JobQueue::load();
        q.append(&jid_prog, line);
        let _ = q.save();
    }));
    let mut last_prompt = job.prompt.clone();
    let jid_gate = jid.clone();
    let worker_backend = job.backend.clone();
    let worker_model = job.model.clone();
    backend.on_gate = Some(Box::new(move || {
        loop {
            let q = JobQueue::load();
            let Some(j) = q.get(&jid_gate) else {
                return Err("job disappeared".into());
            };
            match j.status {
                JobStatus::Waiting => {
                    std::thread::sleep(Duration::from_millis(250));
                }
                JobStatus::Paused => {
                    return Err("stopped (Paused)".into());
                }
                JobStatus::Interrupted | JobStatus::Failed | JobStatus::Done => {
                    return Err(format!("stopped ({:?})", j.status));
                }
                JobStatus::Running | JobStatus::Pending => {
                    if (j.backend != worker_backend || j.model != worker_model)
                        && (!j.backend.is_empty() || !j.model.is_empty())
                    {
                        return Err("stopped (model switched)".into());
                    }
                    if j.prompt != last_prompt {
                        last_prompt = j.prompt.clone();
                        return Ok(Some(format!(
                            "The human revised the request. Follow this version now:\n\n{}",
                            last_prompt
                        )));
                    }
                    return Ok(None);
                }
            }
        }
    }));
    let jid_ask = jid.clone();
    let jid_write = jid.clone();
    backend.on_write = Some(Box::new(move |path: &str, content: &str| {
        let mut q = JobQueue::load();
        q.put_file(&jid_write, path, content);
        let _ = q.save();
    }));
    let jid_tok = jid.clone();
    backend.on_usage = Some(Box::new(move |prompt, completion| {
        let mut q = JobQueue::load();
        q.add_tokens(&jid_tok, prompt, completion);
        let _ = q.save();
    }));
    backend.on_ask = Some(Box::new(move |question: &str, guess: &str| {
        let q = JobQueue::load();
        let kind = q.get(&jid_ask).map(|j| j.kind);
        let project_id = q.get(&jid_ask).map(|j| j.project_id.clone()).unwrap_or_default();
        let yolo = crate::org::Org::yolo(&project_id);
        if yolo {
            if let Some(a) = q.remembered_answer(&jid_ask, question) {
                return Ok(crate::jobs::replay_ask(kind, &a));
            }
        } else if let Some(a) = q.remembered_human_answer(&jid_ask, question) {
            return Ok(crate::jobs::replay_ask(kind, &a));
        }
        if yolo {
            let mut q = JobQueue::load();
            if let Some(a) = q.settle_ask(&jid_ask, question, guess) {
                q.append(
                    &jid_ask,
                    &format!("yolo: {}", crate::jobs::ask_short(question)),
                );
                let _ = q.save();
                return Ok(a);
            }
            let root = crate::org::Org::load()
                .get(&project_id)
                .map(|p| std::path::PathBuf::from(&p.path))
                .unwrap_or_default();
            let n = crate::config::guess_tries_for(&root);
            let tries = q.note_bad_guess(&jid_ask, question);
            q.append(
                &jid_ask,
                &format!("yolo: guess refused ({tries}/{n}) — asking again"),
            );
            let _ = q.save();
            if tries < n {
                return Ok(format!(
                    "ERROR: that guess does not answer the question. Call ask_human again with a concrete guess (a date, amount, or rule — not 'make an assumption'). Attempt {tries} of {n}."
                ));
            }
        }
        let mut q = JobQueue::load();
        q.ask(&jid_ask, question, guess);
        let _ = q.save();
        loop {
            std::thread::sleep(Duration::from_millis(250));
            let mut q = JobQueue::load();
            let Some(j) = q.get(&jid_ask).cloned() else {
                return Err("job disappeared".into());
            };
            match j.status {
                JobStatus::Paused => {
                    return Err("stopped (Paused)".into());
                }
                JobStatus::Interrupted | JobStatus::Failed | JobStatus::Done => {
                    return Err(format!("stopped ({:?})", j.status));
                }
                _ => {
                    if let Some(a) = q.take_answer(&jid_ask) {
                        let _ = q.save();
                        return Ok(a);
                    }
                    if crate::org::Org::yolo(&j.project_id) && q.adopt_guess(&jid_ask) {
                        if let Some(a) = q.take_answer(&jid_ask) {
                            q.append(
                                &jid_ask,
                                &format!("yolo: {}", crate::jobs::ask_short(question)),
                            );
                            let _ = q.save();
                            return Ok(a);
                        }
                    }
                    if !crate::org::Org::yolo(&j.project_id) {
                        if let Some(a) = q.remembered_human_answer(&jid_ask, question) {
                            if q.open_turn(&jid_ask).is_some() {
                                q.set_answer(&jid_ask, &a);
                                let _ = q.take_answer(&jid_ask);
                                let _ = q.save();
                            }
                            return Ok(crate::jobs::replay_ask(Some(j.kind), &a));
                        }
                    }
                }
            }
        }
    }));
    Ok(backend)
}

pub fn looks_like_local_stall(err: &str) -> bool {
    let s = err.to_lowercase();
    if s.contains("stopped") || s.contains("paused") || s.contains("integrity") {
        return false;
    }
    s.contains("didn't respond in time")
        || s.contains("timed out")
        || s.contains("timeout")
        || s.contains("could not reach")
        || s.contains("connection refused")
        || (s.contains("11434") && (s.contains("error") || s.contains("reach") || s.contains("busy")))
}

pub fn grok_failover_target_if(job: &Job, grok_ready: bool) -> Option<(String, String)> {
    if !grok_ready {
        return None;
    }
    if !crate::alloc::is_local(&job.backend) {
        return None;
    }
    Some(("grok".into(), crate::api::DEFAULT_GROK_MODEL.into()))
}

pub fn grok_failover_target(job: &Job) -> Option<(String, String)> {
    grok_failover_target_if(job, grok_is_usable())
}

/// Key is present AND recent Grok jobs did not die on credits.
pub fn grok_is_usable() -> bool {
    crate::api::xai_api_key().is_some()
        && !backend_quota_exhausted(&crate::jobs::JobQueue::load().jobs, "grok")
}

pub fn backend_quota_exhausted(jobs: &[Job], backend: &str) -> bool {
    let b = crate::tokens::normalize_backend(backend);
    jobs.iter()
        .rev()
        .filter(|j| crate::tokens::normalize_backend(&j.backend) == b)
        .take(40)
        .any(|j| looks_like_cloud_quota(&format!("{}\n{}", j.log, j.error)))
}

pub fn looks_like_cloud_quota(err: &str) -> bool {
    let s = err.to_lowercase();
    s.contains("out of credits")
        || s.contains("spending limit")
        || s.contains("insufficient_quota")
        || s.contains("insufficient_funds")
        || s.contains("permission-denied")
}

/// Missing cloud key — hop to local instead of idling Play on a failed Design/Build.
pub fn looks_like_missing_key(err: &str) -> bool {
    let s = err.to_lowercase();
    // Humanized "No Anthropic API key" does not contain the substring "no api key".
    (s.contains("api key") && (s.contains("no ") || s.contains("missing")))
        || s.contains("no api key")
}

/// Local Ollama needs no key. Cloud backends need their key present (and Grok not quota-dead).
pub fn backend_key_ready(backend: &str) -> bool {
    match crate::tokens::normalize_backend(backend).as_str() {
        "qwen" | "ollama" | "" => true,
        "grok" => grok_is_usable(),
        "claude" => crate::api::keys_status().anthropic.set,
        "openai" | "codex" => crate::api::keys_status().openai.set,
        other => OpenAICompatBackend::from_preset(other, None, None).is_ok(),
    }
}

pub fn pick_local_model(installed: &[String]) -> String {
    if let Some((_, model)) = crate::alloc::pick_fast_model(installed) {
        return model;
    }
    const PREFER: &[&str] = &[
        crate::api::DEFAULT_QWEN_MODEL,
        "qwen3.8:27b-mlx",
        "qwen3.8:27b-mtp-q4_K_M",
    ];
    for p in PREFER {
        if installed.iter().any(|id| id == p) {
            return (*p).to_string();
        }
    }
    installed
        .iter()
        .find(|id| id.to_lowercase().contains("qwen"))
        .cloned()
        .or_else(|| installed.first().cloned())
        .unwrap_or_else(|| crate::api::DEFAULT_QWEN_MODEL.to_string())
}

pub fn local_failover_target_if(
    job: &Job,
    local_ready: bool,
    installed: &[String],
) -> Option<(String, String)> {
    if !local_ready {
        return None;
    }
    if crate::alloc::is_local(&job.backend) {
        return None;
    }
    Some(("qwen".into(), pick_local_model(installed)))
}

pub fn local_failover_target(job: &Job) -> Option<(String, String)> {
    if !crate::api::ollama_reachable() {
        return None;
    }
    let installed: Vec<String> = crate::api::list_models()
        .into_iter()
        .filter(|m| m.kind == "local" || m.backend == "ollama" || m.backend == "qwen")
        .map(|m| m.id)
        .collect();
    local_failover_target_if(job, true, &installed)
}

fn apply_agent_failover(job_id: &str, backend: &str, model: &str, why: &str) -> Result<Job, String> {
    let mut q = JobQueue::load();
    if q.is_parked(job_id) {
        return Err("stopped (Paused)".into());
    }
    if !q.set_agent(job_id, backend, model) {
        return Err(format!("no job {job_id}"));
    }
    let short = why.lines().next().unwrap_or(why).trim();
    q.append(
        job_id,
        &format!("{short} — failing over to {backend} {model}"),
    );
    let job = q
        .get(job_id)
        .cloned()
        .ok_or_else(|| format!("no job {job_id}"))?;
    q.save().map_err(|e| e.to_string())?;
    if !job.rid.is_empty() {
        if let Some(pref) = Org::load().get(&job.project_id) {
            let board_path = PathBuf::from(&pref.path).join(".shalt/board.json");
            let mut board = crate::board::Board::load(&board_path);
            if board.set_item_agent(&job.rid, backend, model) {
                let _ = board.save(&board_path);
            }
        }
    }
    Ok(job)
}

/// Run the role. If a local model stalls and Grok has a key, switch once and retry.
pub fn run_role_or_failover(
    job: &Job,
    root: &Path,
    role: &str,
    prompt: &str,
    hide_holdouts: bool,
) -> Result<RoleResult, RoleError> {
    let note = crate::jobs::ask_mode_note_for(
        role,
        crate::org::Org::yolo_mode(&job.project_id),
    );
    let packed = crate::brief::uses_packed_brief(role);
    let prompt = if packed {
        if note.trim().is_empty() {
            prompt.to_string()
        } else {
            format!("{note}\n\n{prompt}")
        }
    } else {
        let brief = crate::sprint::sprint_brief(root);
        if brief.trim().is_empty() {
            format!("{note}\n\n{prompt}")
        } else {
            format!("{note}\n\n{brief}\n\n{prompt}")
        }
    };
    let focus = if role == "stepwright" {
        job.epic.as_str()
    } else {
        ""
    };
    let mut live = job.clone();
    let mut backend = match backend_for_job(&live) {
        Ok(b) => b,
        Err(e) if looks_like_missing_key(&e) => {
            let Some((backend, model)) = local_failover_target(&live) else {
                return Err(RoleError::Other(e));
            };
            live = match apply_agent_failover(&live.id, &backend, &model, &e) {
                Ok(j) => j,
                Err(stop) => return Err(RoleError::Other(stop)),
            };
            backend_for_job(&live).map_err(RoleError::Other)?
        }
        Err(e) => return Err(RoleError::Other(e)),
    };
    match crate::roles::run_role_focused(root, role, &prompt, &mut backend, hide_holdouts, focus) {
        Ok(res) => Ok(res),
        Err(RoleError::Integrity(e)) => Err(RoleError::Integrity(e)),
        Err(e) => {
            let msg = e.to_string();
            let failover = if looks_like_local_stall(&msg) {
                grok_failover_target(&live)
            } else if looks_like_cloud_quota(&msg) || looks_like_missing_key(&msg) {
                local_failover_target(&live)
            } else {
                None
            };
            let Some((backend, model)) = failover else {
                return Err(e);
            };
            let job = match apply_agent_failover(&live.id, &backend, &model, &msg) {
                Ok(j) => j,
                Err(stop) => return Err(RoleError::Other(stop)),
            };
            let mut backend = backend_for_job(&job).map_err(RoleError::Other)?;
            crate::roles::run_role_focused(root, role, &prompt, &mut backend, hide_holdouts, focus)
        }
    }
}

fn local_write_chain() -> Vec<String> {
    let mut installed: Vec<String> = crate::api::list_models()
        .into_iter()
        .filter(|m| m.kind == "local" || m.backend == "qwen" || m.backend == "ollama")
        .map(|m| m.id)
        .collect();
    if installed.is_empty() {
        installed = crate::alloc::WRITE_MODELS
            .iter()
            .map(|s| (*s).to_string())
            .collect();
    }
    installed
}

fn hop_write_lane(job: &mut Job, why: &str) -> bool {
    let installed = local_write_chain();
    let skip_flash = crate::api::heavy_review_loaded();
    let Some((backend, model)) =
        crate::alloc::pick_escalate_model_filtered(&installed, &job.model, skip_flash)
    else {
        return false;
    };
    let loaded = crate::api::ollama_loaded();
    if !crate::alloc::hop_up_ok(&loaded, &job.model, &model, chrono::Utc::now()) {
        return false;
    }
    match apply_agent_failover(&job.id, &backend, &model, why) {
        Ok(j) => {
            *job = j;
            true
        }
        Err(_) => false,
    }
}

fn author_accepted_spec(transcript: &str) -> bool {
    transcript.lines().any(|l| {
        let t = l.trim();
        t.starts_with("[done]") && !t.contains("refused")
    })
}

/// Dump or a refused markdown write is not a review. Hop so 4B/8B can override.
fn author_review_holds(transcript: &str, wrote: &[String]) -> bool {
    if transcript.contains("spec format refused") {
        return false;
    }
    author_accepted_spec(transcript) || !wrote.is_empty()
}

#[cfg(test)]
mod author_review_tests {
    use super::{author_accepted_spec, author_review_holds};

    #[test]
    fn dump_is_not_a_review() {
        assert!(!author_review_holds("# Feature: essay\n", &[]));
        assert!(author_accepted_spec("[done] seed matches the interview"));
        assert!(author_review_holds("[done] seed matches the interview", &[]));
        assert!(!author_accepted_spec("[done] refused — spec needs Feature/Scenario/When/Then"));
        assert!(!author_review_holds(
            "spec format refused — hop",
            &["spec/a.feature".into()]
        ));
        assert!(author_review_holds("ok", &["spec/recipes.feature".into()]));
    }
}

pub fn execute_author(job_id: &str) -> Result<String, String> {
    let mut q = JobQueue::load();
    let mut job = q
        .jobs
        .iter()
        .find(|j| j.id == job_id)
        .cloned()
        .ok_or_else(|| format!("no job {job_id}"))?;
    q.set_status(job_id, JobStatus::Running);
    crate::parallel::mark_start(&mut q, job_id);
    let _ = q.save();
    let org = Org::load();
    let project = org
        .get(&job.project_id)
        .ok_or_else(|| format!("unknown project {}", job.project_id))?;
    let root = PathBuf::from(&project.path);
    let _ = job.draft.restore_to(&root);
    let plan = crate::talk::load_plan(&root, &job.prompt);
    let existing = load_specs(&root.join("spec"), false).unwrap_or_default();
    if crate::spec::gherkin_scenario_count(&existing) == 0 {
        match crate::spec::seed_spec_from_plan(&root, &plan) {
            Ok(files) if !files.is_empty() => {
                let mut q = JobQueue::load();
                q.append(
                    job_id,
                    &format!("seeded spec from interview · {}", files.join(", ")),
                );
                let _ = q.save();
            }
            Err(e) => {
                let mut q = JobQueue::load();
                q.append(job_id, &format!("seed spec failed: {e}"));
                let _ = q.save();
            }
            _ => {}
        }
    }
    let mut last_wrote: Vec<String> = Vec::new();
    let mut last_transcript = String::new();
    let mut tries = 0usize;
    loop {
        tries += 1;
        if let Some(latest) = JobQueue::load().get(job_id).cloned() {
            job = latest;
        }
        let prompt = crate::journal::with_pending(
            &root,
            &author_prompt_with_spec(
                &job.prompt,
                &job.turns,
                job.onboard,
                &spec_snapshot(&root),
            ),
        );
        let result = run_role_or_failover(&job, &root, "author", &prompt, false);
        let mut q = JobQueue::load();
        match result {
            Ok(res) => {
                if q.keep_parked(
                    job_id,
                    "model returned after Pause — discarded. Play to continue.",
                ) {
                    let _ = q.save();
                    return Err("stopped (Paused)".into());
                }
                last_wrote = res.wrote.clone();
                last_transcript = res.transcript.clone();
                let features = load_specs(&root.join("spec"), false).unwrap_or_default();
                let playable = crate::spec::gherkin_scenario_count(&features) > 0;
                if playable && author_review_holds(&res.transcript, &res.wrote) {
                    break;
                }
                let why = if res.transcript.contains("spec format refused") {
                    "spec format refused — next writer reviews"
                } else {
                    "author dump — next writer reviews"
                };
                if tries <= crate::alloc::WRITE_MODELS.len() && hop_write_lane(&mut job, why) {
                    continue;
                }
                if playable {
                    q.append(
                        job_id,
                        "seed stands — last writer did not override",
                    );
                    let _ = q.save();
                    break;
                }
                let summary = format!("wrote {}: {}", res.wrote.len(), res.wrote.join(", "));
                q.append(job_id, &summary);
                q.append(job_id, "spec has no Scenario: lines");
                q.set_error(job_id, "spec has no Scenario: lines");
                q.set_status(job_id, JobStatus::Failed);
                let _ = q.save();
                if let Some(j) = q.get(job_id).cloned() {
                    crate::parallel::mark_finish(&j);
                }
                return Err("spec has no Scenario: lines".into());
            }
            Err(e) => {
                let stopped = e.to_string();
                let short = crate::jobs::human_error(&stopped);
                if q.keep_parked(
                    job_id,
                    &format!("parked while waiting on the model ({short})"),
                ) {
                    let _ = q.save();
                    return Err(stopped);
                }
                if stopped.contains("model switched") {
                    q.append(
                        job_id,
                        "old model call dropped — Play continues on the new one",
                    );
                    let _ = q.save();
                    return Err(stopped);
                }
                if looks_like_local_stall(&stopped)
                    && tries <= crate::alloc::WRITE_MODELS.len()
                    && hop_write_lane(&mut job, "author stall — next writer reviews")
                {
                    continue;
                }
                if crate::spec::spec_is_playable(&root)
                    && tries <= crate::alloc::WRITE_MODELS.len()
                    && hop_write_lane(&mut job, "author failed — next writer reviews")
                {
                    continue;
                }
                if crate::spec::spec_is_playable(&root) {
                    q.append(
                        job_id,
                        "seed stands — last writer failed to override",
                    );
                    let _ = q.save();
                    break;
                }
                q.set_error(job_id, &short);
                q.append(job_id, &format!("failed: {short}"));
                let status = if stopped.contains("stopped") {
                    JobStatus::Interrupted
                } else {
                    JobStatus::Failed
                };
                q.set_status(job_id, status);
                let _ = q.save();
                return Err(stopped);
            }
        }
    }
    let mut q = JobQueue::load();
    let _ = crate::spec::stamp_rids(&root.join("spec"));
    let features = load_specs(&root.join("spec"), false).unwrap_or_default();
    let mut led = Ledger::load(&root.join(".shalt/ledger.json")).unwrap_or_default();
    led.sync_spec(&features);
    let _ = led.save(&root.join(".shalt/ledger.json"));
    let mut board = crate::board::Board::load(&root.join(".shalt/board.json"));
    board.sync_new_rids(&features);
    board.sync_epics(&features);
    let qnow = crate::jobs::JobQueue::load();
    crate::alloc::allocate_unassigned_now(
        &mut board,
        &led,
        &qnow.jobs,
        &job.project_id,
    );
    let _ = board.save(&root.join(".shalt/board.json"));
    crate::talk::seed_plan(&root, &job.prompt);
    crate::talk::fold_answers_into_plan(&root, &job.prompt, &job.turns);
    let summary = format!("wrote {}: {}", last_wrote.len(), last_wrote.join(", "));
    q.append(job_id, &summary);
    q.set_status(job_id, JobStatus::Done);
    let _ = q.save();
    if let Some(j) = q.get(job_id).cloned() {
        let _ = crate::journal::publish(&root, &j, &last_transcript);
        let _ = crate::journal::note_spec_born(&root, &features);
        crate::parallel::mark_finish(&j);
    }
    Ok(summary)
}

pub const ASK_CHAT_SYSTEM: &str = "You help a human decide a concrete answer to a blocking question from an agent that is writing spec, tests, or code.\nChat until they agree. Be specific: method names, examples, yes/no, one decision.\nDo not emit ANSWER: until they agree or clearly ask you to decide.\nWhen you have a decision they can accept, end with exactly:\nANSWER: <one short paragraph the implementer or author can follow — not a chat reply>.";

pub const ASK_DECIDE: &str = "Decide. Prefer the guess if it is sound. Otherwise give your best concrete answer — method names, yes/no, one decision.";

/// Same loop as the UI dialog: append a user line, get a model reply, persist it.
pub fn chat_on_job(id: &str, message: &str) -> Result<Job, String> {
    chat_on_job_with(id, message, None, None)
}

/// Ask a (possibly different) agent to decide and draft ANSWER:.
pub fn decide_on_job(
    id: &str,
    backend: Option<&str>,
    model: Option<&str>,
) -> Result<Job, String> {
    chat_on_job_with(id, ASK_DECIDE, backend, model)
}

pub fn chat_on_job_with(
    id: &str,
    message: &str,
    backend: Option<&str>,
    model: Option<&str>,
) -> Result<Job, String> {
    let message = message.trim();
    if message.is_empty() {
        return Err("message is empty".into());
    }
    let mut q = JobQueue::load();
    if q.get(id).is_none() {
        return Err(format!("no job {id}"));
    }
    if q.open_turn(id).is_none() {
        if let Some(j) = q.get(id).cloned() {
            if j.status == JobStatus::Waiting {
                let qn = if j.question.is_empty() {
                    "What should we do?".into()
                } else {
                    j.question.clone()
                };
                q.ask(id, &qn, "");
            }
        }
    }
    let override_b = backend
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "fixture");
    if let Some(b) = override_b {
        q.set_answer_agent(id, b, model.unwrap_or("").trim());
    }
    if !q.append_chat(id, "user", message) {
        return Err("no open question — this job is not waiting on you".into());
    }
    q.save().map_err(|e| e.to_string())?;
    let job = q
        .get(id)
        .cloned()
        .ok_or_else(|| format!("no job {id}"))?;
    let turn = job
        .turns
        .iter()
        .rev()
        .find(|t| t.answer.is_empty())
        .ok_or_else(|| "no open question".to_string())?;
    let mut history: Vec<(String, String)> = vec![(
        "user".into(),
        format!(
            "The spec question is:\n{}\n\nA starting guess is:\n{}",
            turn.question, turn.guess
        ),
    )];
    if !turn.guess.is_empty() {
        history.push(("assistant".into(), turn.guess.clone()));
    }
    for m in &turn.chat {
        history.push((m.role.clone(), m.content.clone()));
    }
    let (agent, mdl) = crate::jobs::resolve_answer_agent(&job, backend, model);
    let mut system = ASK_CHAT_SYSTEM.to_string();
    if agent != job.backend || (!mdl.is_empty() && mdl != job.model) {
        system.push_str(&format!(
            "\nYou are not the blocked worker. That agent is {} {}. You only draft the decision they will follow.",
            job.backend,
            job.model
        ));
    }
    let model_ref = if mdl.is_empty() { None } else { Some(mdl.as_str()) };
    let mut b = OpenAICompatBackend::from_preset(&agent, model_ref, None)?;
    let reply = b.complete(&system, &history)?;
    let mut q = JobQueue::load();
    q.append_chat_from(id, "assistant", &reply, &agent, &mdl);
    q.save().map_err(|e| e.to_string())?;
    q.get(id)
        .cloned()
        .ok_or_else(|| format!("no job {id}"))
}

pub fn last_assistant_on(job: &Job) -> Option<&str> {
    let turn = job.turns.iter().rev().find(|t| t.answer.is_empty())?;
    turn.chat
        .iter()
        .rev()
        .find(|m| m.role == "assistant" && !m.content.trim().is_empty())
        .map(|m| m.content.as_str())
}
