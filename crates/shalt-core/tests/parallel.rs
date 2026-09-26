//! Epoch overlap and backend capacity.

use shalt_core::jobs::{epoch_id, JobKind, JobQueue, JobStatus};
use shalt_core::parallel::{
    can_admit, jobs_overlap, retune_caps, Admit, Capacity, Sample, SlotKind,
};

fn job(project: &str, kind: JobKind, rid: &str, backend: &str) -> shalt_core::jobs::Job {
    let mut q = JobQueue::default();
    let j = q.enqueue_full(kind, project, "", backend, "m");
    q.set_work(&j.id, "", rid);
    q.get(&j.id).cloned().unwrap()
}

#[test]
fn writing_tests_is_one_epoch_then_each_ticket_is_a_build_epoch() {
    let t1 = job("erp", JobKind::Steps, "S-1", "qwen");
    let t2 = job("erp", JobKind::Steps, "S-2", "qwen");
    let b1 = job("erp", JobKind::Build, "S-1", "grok");
    let b2 = job("erp", JobKind::Build, "S-2", "grok");
    assert!(jobs_overlap(&t1, &t2), "tests are one workspace");
    assert!(!jobs_overlap(&t1, &b1), "tests then build is sequential, not the same epoch");
    let b1b = job("erp", JobKind::Build, "S-1", "qwen");
    assert!(jobs_overlap(&b1, &b1b), "same ticket build overlaps");
    assert!(!jobs_overlap(&b1, &b2), "different tickets build in parallel");
    assert_eq!(epoch_id("erp", JobKind::Steps, "S-1", ""), "erp/tests");
    assert_eq!(epoch_id("erp", JobKind::Run, "", ""), "erp/run");
    assert_eq!(epoch_id("erp", JobKind::Build, "S-1", ""), "erp/S-1");
}

#[test]
fn spec_authoring_is_one_epoch() {
    let a = job("erp", JobKind::Author, "", "qwen");
    let b = job("erp", JobKind::Author, "", "grok");
    assert!(jobs_overlap(&a, &b));
    assert_eq!(a.epoch, "erp/spec");
}

#[test]
fn local_cap_one_rejects_a_second_qwen_job() {
    let mut q = JobQueue::default();
    let a = q.enqueue_full(JobKind::Steps, "alpha", "", "qwen", "qwen3.8:27b-mlx");
    q.set_work(&a.id, "", "S-1");
    q.set_status(&a.id, JobStatus::Running);
    let b = q.enqueue_full(JobKind::Steps, "beta", "", "qwen", "qwen3.8:27b-mlx");
    q.set_work(&b.id, "", "S-9");
    let cap = Capacity {
        local_cap: 1,
        cloud_cap: 3,
        ..Capacity::default()
    };
    let live_b = q.get(&b.id).unwrap();
    assert!(
        matches!(can_admit(live_b, &q.jobs, &cap), Admit::Full { kind: SlotKind::Local, .. }),
        "{:?}",
        can_admit(live_b, &q.jobs, &cap)
    );
}

#[test]
fn grok_and_qwen_do_not_contend_for_the_same_slot() {
    let mut q = JobQueue::default();
    let a = q.enqueue_full(JobKind::Steps, "alpha", "", "qwen", "qwen3.8:27b-mlx");
    q.set_work(&a.id, "", "S-1");
    q.set_status(&a.id, JobStatus::Running);
    let b = q.enqueue_full(JobKind::Steps, "beta", "", "grok", "grok-4");
    q.set_work(&b.id, "", "S-2");
    let cap = Capacity {
        local_cap: 1,
        cloud_cap: 3,
        ..Capacity::default()
    };
    assert!(can_admit(q.get(&b.id).unwrap(), &q.jobs, &cap).ok());
}

#[test]
fn cloud_cap_rises_when_more_parallel_is_still_faster() {
    let mut samples = Vec::new();
    for n in [1u32, 2, 3] {
        for _ in 0..4 {
            samples.push(Sample {
                kind: "cloud".into(),
                backend: "grok".into(),
                concurrent: n,
                secs: 30 * n as i64, // linear: throughput stays 1/30
                tokens: 1000,
                at: String::new(),
            });
        }
    }
    let (_local, cloud) = retune_caps(samples);
    assert!(cloud >= 2, "flat throughput should keep extra grok slots, got {cloud}");
}

#[test]
fn local_stays_at_one_when_two_is_much_slower() {
    let mut samples = Vec::new();
    for _ in 0..4 {
        samples.push(Sample {
            kind: "local".into(),
            backend: "qwen".into(),
            concurrent: 1,
            secs: 60,
            tokens: 1000,
            at: String::new(),
        });
        samples.push(Sample {
            kind: "local".into(),
            backend: "qwen".into(),
            concurrent: 2,
            secs: 200,
            tokens: 1000,
            at: String::new(),
        });
    }
    let (local, _) = retune_caps(samples);
    assert_eq!(local, 1);
}
