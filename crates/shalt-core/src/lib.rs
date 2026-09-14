//! Shalt core: spec identity, ledger, isolation, overlay, jobs.

pub mod api;
pub mod backends;
pub mod board;
pub mod config;
pub mod integrity;
pub mod jobs;
pub mod ledger;
pub mod mutate;
pub mod narrative;
pub mod org;
pub mod reports;
pub mod roles;
pub mod runner;
pub mod spec;
pub mod viz;

pub use api::OpenAICompatBackend;
pub use backends::{Backend, FixtureBackend};
pub use board::Board;
pub use config::Config;
pub use integrity::{IntegrityViolation, ALL_ZONES, READS, ZONES};
pub use jobs::JobQueue;
pub use ledger::{Entry, Ledger, RunResult, GREEN, ORPHAN, PENDING, RED, STALE};
pub use mutate::{run_campaign, MutationReport};
pub use org::Org;
pub use roles::{run_role, RoleError, RoleResult};
pub use spec::{load_specs, stamp_rids, Feature, Scenario, SpecParseError};

pub const LEDGER_SCHEMA: &str = "shalt.ledger/1";
pub const BOARD_SCHEMA: &str = "shalt.board/1";
pub const JOBS_SCHEMA: &str = "shalt.jobs/1";
