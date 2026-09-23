//! Shalt core: spec identity, ledger, isolation, overlay, jobs.

pub mod alloc;
pub mod api;
pub mod audit;
pub mod backends;
pub mod bindings;
pub mod board;
pub mod brief;
pub mod compose;
pub mod config;
pub mod credentials;
pub mod git;
pub mod deps;
pub mod draft;
pub mod integrity;
pub mod jobs;
pub mod journal;
pub mod ledger;
pub mod markups;
pub mod mockups;
pub mod mutate;
pub mod narrative;
pub mod org;
pub mod pack;
pub mod parallel;
pub mod pipeline;
pub mod reports;
pub mod roles;
pub mod runner;
pub mod scaffold;
pub mod spec;
pub mod sprint;
pub mod tags;
pub mod talk;
pub mod tokens;
pub mod uis;
pub mod ux;
pub mod viz;

pub use api::{
    agent_roster, keys_status, list_models, roster_models, save_keys, xai_api_key, AgentInfo,
    KeySlot, KeysPatch, KeysStatus, ModelChoice, OpenAICompatBackend, DEFAULT_GROK_MODEL,
    DEFAULT_QWEN_MODEL,
};
pub use credentials::{Credentials, DEFAULT_HOST};
pub use git::{clone_git_source, github_clone_name, looks_like_git_source, normalize_github_url};
pub use compose::{
    author_prompt, author_prompt_with_spec, author_system_prompt, author_user_prompt, chat_on_job, chat_on_job_with,
    spec_snapshot,
    backend_quota_exhausted, decide_on_job, designer_user_prompt, designer_user_prompt_for, execute_author, execute_design, grok_failover_target_if, grok_is_usable, last_assistant_on, list_dirs,
    local_failover_target_if, looks_like_cloud_quota, looks_like_local_stall, onboard_project, pick_local_model, restack_project, start_project, ASK_CHAT_SYSTEM,
    ASK_DECIDE, ComposeRequest, DirListing,
};
pub use config::{restack, stack_choices, RestackReport, StackChoice};
pub use pipeline::{
    continue_project, exclusive_unpause, execute_job, next_stage, play_chains_after, play_loop,
    play_stop_reason,
    play_step, switch_play_model, work_gate, PlayOutcome, PlayTick, Stage,
};
pub use backends::{Backend, FixtureBackend};
pub use bindings::{
    journey_tests, load_step_defs, pick_steps_journey, stepwright_focus_prompt, steps_needed,
    JourneyTests,
};
pub use board::Board;
pub use config::Config;
pub use draft::{Draft, DraftView};
pub use integrity::{IntegrityViolation, ALL_ZONES, READS, ZONES};
pub use jobs::JobQueue;
pub use journal::Journal;
pub use ledger::{Entry, Ledger, RunResult, GREEN, ORPHAN, PENDING, RED, STALE};
pub use mutate::{run_campaign, MutationReport};
pub use org::{Org, YoloMode};
pub use pack::{export_pack, import_pack, PlanPack};
pub use deps::{work_map, WorkMap};
pub use parallel::{
    can_admit, claim_play, jobs_overlap, next_fillable, view as parallel_view, Admit, Capacity,
    SlotKind,
};
pub use roles::{focused_stepwright_rels, run_role, run_role_focused, RoleError, RoleResult};
pub use spec::{load_specs, put_spec_file, stamp_rids, Feature, Scenario, SpecParseError};
pub use sprint::{sprint_brief, SPRINT_TICKET_CAP};
pub use runner::list_step_files;
pub use scaffold::{
    apply_js_contract_stubs, apply_step_stubs, has_final_look, parse_js_contract, promote_prototype,
    quarantine_duplicate_step_files, steps_source_ok, write_js_world_if_missing, ContractModule,
    DUP_STEPS_DIR,
};
pub use tags::{filter_scenarios, looks_like_feature_arg, Locator, TagExpr};
pub use markups::{
    designer_markup_prompt, embed_tag, load_interview, load_markup, markup_enabled, page_polish_prompt,
    polish_prompt, save_interview, save_markup, Interview, Markup,
};
pub use mockups::{
    assemble_mockup, design_needed, ensure_thumb, films, frame_is_drawn, html_to_thumb_svg,
    is_html_document, kit_has_platform, kit_platform, load_kit, mockup_inject, normalize_platform,
    pick_design_journey,
    normalize_rel, promote_final_if_green, promote_sketch_to_final, refresh_thumbs, thumb_rel,
    verify_mockups, DesignKit, Film, Frame, SKETCH_CSS,
};
pub use narrative::{actor_from_step, parse_story};
pub use viz::{
    mermaid_org, mermaid_pipeline, mermaid_pipeline_at, mermaid_user_flow, project_diagrams,
    Diagrams,
};
pub use talk::{revise_spec, SpecTalk};

pub const LEDGER_SCHEMA: &str = "shalt.ledger/1";
pub const BOARD_SCHEMA: &str = "shalt.board/1";
pub const JOBS_SCHEMA: &str = "shalt.jobs/1";
