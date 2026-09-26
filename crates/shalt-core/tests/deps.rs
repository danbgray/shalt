//! Shared failures wait; independent tickets fan out.

use shalt_core::board::{Board, BoardItem};
use shalt_core::jobs::Job;
use shalt_core::ledger::{Entry, Ledger};

fn ticket(led: &mut Ledger, rid: &str, file: &str, line: usize, fail: &str) {
    let e: Entry = serde_json::from_value(serde_json::json!({
        "rid": rid,
        "name": rid,
        "feature": file,
        "feature_file": file,
        "line": line,
        "status": "red",
        "last_run_at": "2026-09-15T00:00:00Z",
        "failure": fail
    }))
    .unwrap();
    led.entries.insert(rid.into(), e);
}

fn item(rid: &str, rank: i64) -> BoardItem {
    BoardItem {
        rid: rid.into(),
        rank,
        ..Default::default()
    }
}

#[test]
fn shared_compile_error_is_one_ready_ticket() {
    let mut led = Ledger::default();
    let boom = "error: cannot find crate `erp` in this workspace";
    ticket(&mut led, "S-a", "spec/a.feature", 10, boom);
    ticket(&mut led, "S-b", "spec/b.feature", 10, boom);
    ticket(&mut led, "S-c", "spec/c.feature", 10, boom);
    let mut board = Board::default();
    board.items = vec![item("S-a", 1), item("S-b", 2), item("S-c", 3)];
    let map = shalt_core::work_map(&board, &led, &[], "p", None);
    assert_eq!(map.ready.len(), 1, "{:?}", map.ready);
    assert_eq!(map.ready[0].rid, "S-a");
    assert_eq!(map.blocked.len(), 2);
    assert!(map.blocked.iter().all(|n| n.blocked_by == ["S-a"]));
}

#[test]
fn distinct_failures_in_different_files_fan_out() {
    let mut led = Ledger::default();
    ticket(&mut led, "S-a", "spec/a.feature", 10, "assert 1 == 2 on totals");
    ticket(&mut led, "S-b", "spec/b.feature", 10, "assert empty vec on receive");
    let mut board = Board::default();
    board.items = vec![item("S-a", 1), item("S-b", 2)];
    let map = shalt_core::work_map(&board, &led, &[], "p", None);
    assert_eq!(map.ready.len(), 2, "independent tickets start together {:?}", map.ready);
    assert!(map.blocked.is_empty());
}

#[test]
fn later_scenario_in_the_same_file_waits() {
    let mut led = Ledger::default();
    ticket(&mut led, "S-a", "spec/bom.feature", 8, "assert single-level bom");
    ticket(&mut led, "S-b", "spec/bom.feature", 24, "assert multi-level bom");
    let mut board = Board::default();
    board.items = vec![item("S-a", 1), item("S-b", 2)];
    let map = shalt_core::work_map(&board, &led, &[], "p", None);
    assert_eq!(
        map.ready.iter().map(|n| n.rid.as_str()).collect::<Vec<_>>(),
        ["S-a"]
    );
    assert_eq!(map.blocked[0].rid, "S-b");
    assert_eq!(map.blocked[0].blocked_by, ["S-a"]);
}

#[test]
fn running_job_marks_the_ticket_live() {
    let mut led = Ledger::default();
    ticket(&mut led, "S-a", "spec/a.feature", 10, "assert 1 == 2 on totals");
    ticket(&mut led, "S-b", "spec/b.feature", 10, "assert empty vec on receive");
    let mut board = Board::default();
    board.items = vec![item("S-a", 1), item("S-b", 2)];
    let job: Job = serde_json::from_value(serde_json::json!({
        "id": "J-1",
        "kind": "build",
        "project_id": "p",
        "status": "running",
        "rid": "S-a"
    }))
    .unwrap();
    let map = shalt_core::work_map(&board, &led, &[job], "p", None);
    let a = map
        .ready
        .iter()
        .chain(map.blocked.iter())
        .find(|n| n.rid == "S-a")
        .unwrap();
    let b = map
        .ready
        .iter()
        .chain(map.blocked.iter())
        .find(|n| n.rid == "S-b")
        .unwrap();
    assert!(a.running, "live build should light up S-a");
    assert!(!b.running);
}
