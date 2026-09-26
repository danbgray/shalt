//! The project journal is a periodical the agents file, not a job-log dump.

use shalt_core::jobs::{JobKind, JobQueue};
use shalt_core::journal::{comment, parse_dispatch, publish, Journal};
use tempfile::TempDir;

#[test]
fn parse_takes_the_last_journal_block() {
    let text = "wrote spec/a.feature\nJOURNAL: First try\nok\nJOURNAL: The envelope is a file\nNot a platform. Shops already have those, and this file is only the handshake between them.\n";
    let (title, body) = parse_dispatch(text).unwrap();
    assert_eq!(title, "The envelope is a file");
    assert!(body.contains("Not a platform"));
    assert!(!body.contains("First try"));
}

#[test]
fn parse_ignores_tool_traces_and_short_ok() {
    assert!(parse_dispatch("JOURNAL: ok\n").is_none());
    assert!(parse_dispatch("no dispatch here").is_none());
}

#[test]
fn publish_files_today_and_does_not_double_a_job() {
    let t = TempDir::new().unwrap();
    let mut q = JobQueue::default();
    let mut job = q.enqueue(JobKind::Design, "envelope");
    job.backend = "grok".into();
    job.model = "grok-4".into();
    let body = "JOURNAL: A tray, not a table\nThe inbox is a shallow tray of sealed work. I drew it in pencil so it cannot be mistaken for the product.\n";
    assert!(publish(t.path(), &job, body).unwrap());
    assert!(!publish(t.path(), &job, body).unwrap(), "same job files once");
    let j = Journal::load(t.path());
    assert_eq!(j.issues.len(), 1);
    assert_eq!(j.issues[0].number, 1);
    assert_eq!(j.issues[0].dispatches.len(), 1);
    assert_eq!(j.issues[0].dispatches[0].kind, "design");
    assert_eq!(j.issues[0].dispatches[0].title, "A tray, not a table");
    assert!(j.issues[0].dispatches[0].body.contains("shallow tray"));
}

#[test]
fn a_second_day_opens_the_next_issue() {
    let t = TempDir::new().unwrap();
    let mut q = JobQueue::default();
    let a = q.enqueue(JobKind::Author, "envelope");
    let b = q.enqueue(JobKind::Steps, "envelope");
    let mut j = Journal::default();
    j.file_on_date(
        t.path(),
        "2026-09-17",
        shalt_core::journal::dispatch_from(&a, "JOURNAL: Day one\nThe spec names the envelope as a file you can hash.\n"),
    )
    .unwrap();
    j.file_on_date(
        t.path(),
        "2026-09-18",
        shalt_core::journal::dispatch_from(&b, "JOURNAL: Day two\nThe oracle binds each sealed job to a traveler id.\n"),
    )
    .unwrap();
    let got = Journal::load(t.path());
    assert_eq!(got.issues.len(), 2);
    assert_eq!(got.issues[0].date, "2026-09-18");
    assert_eq!(got.issues[0].number, 2);
    assert_eq!(got.issues[1].date, "2026-09-17");
    assert_eq!(got.issues[1].number, 1);
}

#[test]
fn feature_block_is_a_longer_post_and_clears_pending() {
    let t = TempDir::new().unwrap();
    shalt_core::journal::note_event(
        t.path(),
        "first-green",
        "The first scenario just went green.",
    )
    .unwrap();
    let mut q = JobQueue::default();
    let job = q.enqueue(JobKind::Build, "envelope");
    let essay = "FEATURE: The first green light\n\
         A bound test passed. That is not a vibe and not a screenshot — it is the spec, word for word, held by an oracle the implementer never saw.\n\
         What became true is small on purpose: one sealed handshake, one traveler id, one shop that can hash the file the same way we do.\n\
         The rest of the board is still pencil. Journeys wait. The inbox is a tray in a sketch. Green here is a foothold, not the product.\n\
         Tomorrow the next ticket takes the same oath. If the spec is wrong we rewrite the spec, not the test.\n";
    assert!(publish(t.path(), &job, essay).unwrap());
    let j = Journal::load(t.path());
    assert!(j.pending.is_empty(), "{:?}", j.pending);
    let d = &j.issues[0].dispatches[0];
    assert_eq!(d.form, "feature");
    assert_eq!(d.event, "first-green");
    assert!(d.body.contains("foothold"));
}

#[test]
fn progress_and_feature_can_share_a_turn() {
    let t = TempDir::new().unwrap();
    let mut q = JobQueue::default();
    let job = q.enqueue(JobKind::Author, "envelope");
    let text = "JOURNAL: Spec grew a spine\n\
         I wrote the first journeys so the envelope is a file you can hash, not a platform anyone has to join.\n\
         FEATURE: Opening essay\n\
         This system shall be a content-addressed handshake between shops that already have tools.\n\
         The work is not to replace their ERPs. It is to make a traveler that any conforming bench can verify without us in the room.\n\
         What follows is journeys, then pictures, then tests that cannot see the code, then code that cannot see the tests.\n\
         That order is the product claim. Green will mean the wording held, not that an agent was pleased with itself.\n";
    assert!(publish(t.path(), &job, text).unwrap());
    let j = Journal::load(t.path());
    let forms: Vec<_> = j.issues[0]
        .dispatches
        .iter()
        .map(|d| d.form.as_str())
        .collect();
    assert!(forms.contains(&"progress"), "{forms:?}");
    assert!(forms.contains(&"feature"), "{forms:?}");
}

#[test]
fn a_human_can_comment_and_reply_on_a_dispatch() {
    let t = TempDir::new().unwrap();
    let mut q = JobQueue::default();
    let job = q.enqueue(JobKind::Design, "envelope");
    assert!(publish(
        t.path(),
        &job,
        "JOURNAL: A tray, not a table\nThe inbox is a shallow tray of sealed work. I drew it in pencil so it cannot be mistaken for the product.\n",
    )
    .unwrap());
    let j = Journal::load(t.path());
    let post = j.issues[0].dispatches[0].id.clone();
    assert!(post.starts_with("p-"), "{post}");
    let c = comment(t.path(), &post, "", "You", "Keep the tray. Don't add a table.").unwrap();
    assert!(c.id.starts_with("c-"));
    assert!(c.parent.is_empty());
    let r = comment(t.path(), &post, &c.id, "You", "And no inbox tabs.").unwrap();
    assert_eq!(r.parent, c.id);
    let got = Journal::load(t.path());
    assert_eq!(got.issues[0].dispatches[0].comments.len(), 2);
    assert!(comment(t.path(), &post, "", "You", "   ").is_err());
    assert!(comment(t.path(), "p-missing", "", "You", "hello there").is_err());
    assert!(comment(t.path(), &post, "c-nope", "You", "hello there").is_err());
}
