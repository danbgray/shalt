use crate::narrative::{parse_story, Story};
use regex::Regex;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use thiserror::Error;

pub const RID_PREFIX: &str = "@rid:";
pub const HOLDOUT_TAG: &str = "@holdout";
pub const EPIC_PREFIX: &str = "@epic:";

fn rid_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"@rid:(S-[0-9a-f]{8})").unwrap())
}
fn epic_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^@epic:([A-Za-z0-9_.\-]+)$").unwrap())
}

pub fn new_rid() -> String {
    format!("S-{:08x}", rand::random::<u32>())
}

fn sha_text(text: &str) -> String {
    let mut h = Sha256::new();
    h.update(text.as_bytes());
    format!("sha256:{}", &hex::encode(h.finalize())[..32])
}

pub fn file_hash(p: &Path) -> std::io::Result<String> {
    let bytes = fs::read(p)?;
    let mut h = Sha256::new();
    h.update(&bytes);
    Ok(hex::encode(h.finalize())[..16].to_string())
}

fn norm_tag(t: &str) -> String {
    let t = t.trim();
    if t.starts_with('@') {
        t.to_string()
    } else {
        format!("@{t}")
    }
}

fn step_lines(steps: &[gherkin::Step]) -> Vec<String> {
    let mut out = Vec::new();
    for s in steps {
        let mut line = format!("{} {}", s.keyword.trim(), s.value.trim());
        if let Some(doc) = &s.docstring {
            line.push_str(&format!("\n<<<{}>>>", doc.trim()));
        }
        if let Some(tbl) = &s.table {
            for row in &tbl.rows {
                line.push_str(&format!("\n|{}|", row.join("|")));
            }
        }
        out.push(line);
    }
    out
}

fn examples_lines(examples: &[gherkin::Examples]) -> Vec<String> {
    let mut out = Vec::new();
    for ex in examples {
        if let Some(tbl) = &ex.table {
            for row in &tbl.rows {
                out.push(format!("|{}|", row.join("|")));
            }
        }
    }
    out
}

#[derive(Debug, Clone)]
pub struct Scenario {
    pub rid: Option<String>,
    pub name: String,
    pub keyword: String,
    pub tags: Vec<String>,
    pub steps: Vec<String>,
    pub examples: Vec<String>,
    pub feature_name: String,
    pub feature_file: String,
    pub line: usize,
    pub tag_lines: Vec<usize>,
    pub rid_count: usize,
    pub inherited_tags: Vec<String>,
    pub oracles: Vec<crate::oracles::ThenOracle>,
}

impl Scenario {
    pub fn all_tags(&self) -> Vec<String> {
        let mut t = self.tags.clone();
        for it in &self.inherited_tags {
            if !t.contains(it) {
                t.push(it.clone());
            }
        }
        t
    }

    pub fn block_start(&self) -> usize {
        self.tag_lines.iter().copied().min().unwrap_or(self.line)
    }

    pub fn is_holdout(&self) -> bool {
        self.tags.iter().any(|t| t == HOLDOUT_TAG)
    }

    pub fn epic(&self) -> String {
        for t in self.all_tags() {
            if let Some(c) = epic_re().captures(&t) {
                return c[1].to_string();
            }
        }
        String::new()
    }

    pub fn canonical(&self, background: &[String]) -> String {
        let mut tags: Vec<_> = self
            .tags
            .iter()
            .filter(|t| !t.starts_with(RID_PREFIX))
            .cloned()
            .collect();
        tags.sort();
        format!(
            "FEATURE:{}\nBACKGROUND:\n{}\nTAGS:{}\n{}:{}\nSTEPS:\n{}\nEXAMPLES:\n{}",
            self.feature_name.trim(),
            background.join("\n"),
            tags.join(","),
            self.keyword.trim().to_uppercase(),
            self.name.trim(),
            self.steps.join("\n"),
            self.examples.join("\n")
        )
    }

    pub fn spec_hash(&self, background: &[String]) -> String {
        sha_text(&self.canonical(background))
    }
}

#[derive(Debug, Clone)]
pub struct Feature {
    pub name: String,
    pub file: String,
    pub tags: Vec<String>,
    pub background: Vec<String>,
    pub scenarios: Vec<Scenario>,
    pub description: String,
}

impl Feature {
    pub fn story(&self) -> Story {
        parse_story(&self.description)
    }

    /// `As a …`, else the first Gherkin step that names who acts.
    pub fn inferred_actor(&self) -> String {
        let st = self.story();
        if !st.actor.is_empty() {
            return st.actor;
        }
        for sc in &self.scenarios {
            for step in &sc.steps {
                if let Some(a) = crate::narrative::actor_from_step(step) {
                    return a;
                }
            }
        }
        String::new()
    }

    pub fn epic(&self) -> String {
        for t in &self.tags {
            if let Some(c) = epic_re().captures(t) {
                return c[1].to_string();
            }
        }
        let p = Path::new(&self.file);
        let parts: Vec<_> = p.iter().collect();
        if parts.len() > 1 {
            parts[0].to_string_lossy().into_owned()
        } else {
            String::new()
        }
    }

    pub fn blocks(&self, total_lines: usize) -> Vec<(&Scenario, usize, usize)> {
        let mut anchors: Vec<usize> = self.scenarios.iter().map(|s| s.block_start()).collect();
        anchors.sort_unstable();
        anchors.dedup();
        self.scenarios
            .iter()
            .map(|sc| {
                let start = sc.block_start();
                let end = anchors
                    .iter()
                    .copied()
                    .find(|&a| a > start)
                    .unwrap_or(total_lines + 1);
                (sc, start, end)
            })
            .collect()
    }
}

#[derive(Debug, Error)]
#[error("spec parse error")]
pub struct SpecParseError {
    pub errors: HashMap<String, String>,
}

impl SpecParseError {
    pub fn message(&self) -> String {
        self.errors
            .iter()
            .map(|(k, v)| format!("{k}: {v}"))
            .collect::<Vec<_>>()
            .join("; ")
    }
}

fn rids_in_tags(tags: &[String]) -> Vec<String> {
    tags.iter()
        .filter_map(|t| {
            rid_re()
                .captures(t)
                .map(|c| c[1].to_string())
        })
        .collect()
}

fn tag_lines_for(raw_lines: &[String], scenario_line: usize) -> Vec<usize> {
    let mut lines = Vec::new();
    if scenario_line == 0 {
        return lines;
    }
    let mut i = scenario_line; // 1-indexed scenario keyword line
    // walk up through tag lines
    while i > 1 {
        let prev = raw_lines.get(i - 2).map(|s| s.trim().to_string()).unwrap_or_default();
        if prev.starts_with('@') {
            lines.push(i - 1);
            i -= 1;
        } else if prev.is_empty() {
            // blank line between tags? stop unless next-up is a tag
            break;
        } else {
            break;
        }
    }
    lines.sort_unstable();
    lines
}

fn convert_scenario(
    sc: &gherkin::Scenario,
    feature_name: &str,
    rel: &str,
    inherited: &[String],
    extra_bg: &[String],
    raw_lines: &[String],
    extra_tags: &[String],
) -> Scenario {
    let mut tags: Vec<String> = sc.tags.iter().map(|t| norm_tag(t)).collect();
    tags.extend(extra_tags.iter().cloned());
    let rids = rids_in_tags(&tags);
    let line = sc.position.line as usize;
    Scenario {
        rid: rids.first().cloned(),
        rid_count: rids.len(),
        name: sc.name.clone(),
        keyword: sc.keyword.clone(),
        tags,
        steps: {
            let mut s = extra_bg.to_vec();
            s.extend(step_lines(&sc.steps));
            // background is hashed separately; don't duplicate into steps
            let _ = extra_bg;
            step_lines(&sc.steps)
        },
        examples: examples_lines(&sc.examples),
        feature_name: feature_name.to_string(),
        feature_file: rel.to_string(),
        line,
        tag_lines: tag_lines_for(raw_lines, line),
        inherited_tags: inherited.to_vec(),
        oracles: Vec::new(),
    }
}

pub fn parse_text(source: &str, rel: &str) -> Result<Option<Feature>, String> {
    let gf = gherkin::Feature::parse(source, gherkin::GherkinEnv::default())
        .map_err(|e| e.to_string())?;
    let raw_lines: Vec<String> = source.split('\n').map(|s| s.trim_end_matches('\r').to_string()).collect();
    let feature_tags: Vec<String> = gf.tags.iter().map(|t| norm_tag(t)).collect();
    let mut background = Vec::new();
    if let Some(bg) = &gf.background {
        background.extend(step_lines(&bg.steps));
    }
    let mut scenarios = Vec::new();
    for sc in &gf.scenarios {
        scenarios.push(convert_scenario(
            sc,
            &gf.name,
            rel,
            &feature_tags,
            &[],
            &raw_lines,
            &[],
        ));
    }
    for rule in &gf.rules {
        let mut rule_bg = background.clone();
        if let Some(bg) = &rule.background {
            rule_bg.extend(step_lines(&bg.steps));
        }
        let rule_tags: Vec<String> = rule.tags.iter().map(|t| norm_tag(t)).collect();
        for sc in &rule.scenarios {
            let mut inherited = feature_tags.clone();
            inherited.extend(rule_tags.iter().cloned());
            let scn = convert_scenario(sc, &gf.name, rel, &inherited, &[], &raw_lines, &[]);
            let _ = rule_bg;
            scenarios.push(scn);
        }
    }
    // Fix: rule background should be in Feature.background only at feature level.
    // Python walks children and extends a single background list including rule backgrounds.
    // Recompute background the Python way: feature background only on Feature.background;
    // rule background is included when hashing scenarios under that rule via... wait.
    // Python `_walk_children` extends `background` for the whole feature including rules.
    // That means a rule background is appended to the shared list and applies to ALL subsequent
    // scenarios including those after the rule? That's a sequential walk. Feature-level
    // background first, then rule backgrounds accumulate.
    // For hashing, Scenario.spec_hash(f.background) uses the Feature.background list.
    // So rule backgrounds ARE in Feature.background in Python if they appeared as children.
    // I'll flatten: feature bg + all rule bgs into Feature.background to match sequential extend.
    for rule in &gf.rules {
        if let Some(bg) = &rule.background {
            background.extend(step_lines(&bg.steps));
        }
    }

    let mut feature = Feature {
        name: gf.name,
        file: rel.to_string(),
        tags: feature_tags,
        background,
        scenarios,
        description: gf.description.unwrap_or_default(),
    };
    crate::oracles::attach_oracles(&mut feature, source);
    Ok(Some(feature))
}

pub fn gherkin_scenario_count(features: &[Feature]) -> usize {
    features.iter().map(|f| f.scenarios.len()).sum()
}

/// Markdown bullets and prose are not a spec. Play needs `Scenario:` lines.
pub fn looks_like_gherkin(text: &str) -> bool {
    text.lines().any(|l| {
        let t = l.trim();
        t.starts_with("Scenario:") || t.starts_with("Scenario Outline:")
    })
}

/// Author writes must be a parseable Feature with Scenario + When + Then.
/// Markdown headings and bullet lists are refused.
pub fn spec_write_error(path: &str, content: &str) -> Option<String> {
    let p = path.replace('\\', "/");
    let name = p.rsplit('/').next().unwrap_or(&p);
    if p.contains("spec/") || p.starts_with("spec/") || name.ends_with(".feature") {
        if !name.ends_with(".feature") {
            return Some("write spec/*.feature only".into());
        }
    } else {
        return Some("author writes spec/*.feature only".into());
    }
    if content.lines().any(|l| {
        let t = l.trim_start();
        t.starts_with("# ") || t.starts_with("## ") || t.starts_with("### ")
    }) {
        return Some(concat!(
            "do not use markdown headings. Write exactly:\n",
            "@epic:recipes\n",
            "Feature: Share recipes\n",
            "  Scenario: Author publishes a titled recipe\n",
            "    Given I am signed in as \"maya@example.com\"\n",
            "    When I create a recipe titled \"Weeknight Tomato Pasta\"\n",
            "    Then the recipe \"Weeknight Tomato Pasta\" is public\n",
        ).into());
    }
    if !content.lines().any(|l| l.trim().starts_with("Feature:")) {
        return Some("missing Feature: line".into());
    }
    if !looks_like_gherkin(content) {
        return Some("missing Scenario: line. Bullet lists are not a spec.".into());
    }
    if !content.contains("@epic:") {
        return Some("tag the Feature with @epic:<area> (one word)".into());
    }
    match parse_text(content, name) {
        Err(e) => Some(format!("spec does not parse: {}", e.lines().next().unwrap_or("error"))),
        Ok(None) => Some("spec parsed empty".into()),
        Ok(Some(f)) => {
            if f.scenarios.is_empty() {
                return Some("Feature has no Scenario".into());
            }
            for s in &f.scenarios {
                if s.steps.is_empty() {
                    return Some(format!("Scenario {:?} has no steps", s.name));
                }
                let when = s.steps.iter().any(|st| {
                    let k = st.split_whitespace().next().unwrap_or("");
                    k.eq_ignore_ascii_case("when")
                });
                let then = s.steps.iter().any(|st| {
                    let k = st.split_whitespace().next().unwrap_or("");
                    k.eq_ignore_ascii_case("then")
                });
                if !when || !then {
                    return Some(format!(
                        "Scenario {:?} needs When and Then with concrete values",
                        s.name
                    ));
                }
            }
            if let Some(err) = crate::oracles::missing_observe(content) {
                return Some(err);
            }
            None
        }
    }
}

#[cfg(test)]
mod spec_write_tests {
    use super::spec_write_error;

    #[test]
    fn markdown_essay_is_refused() {
        let md = "# Feature: Recipe Sharing\n\n### Acceptance Criteria\n- Authors can write a recipe\n";
        let err = spec_write_error("spec/recipe.feature", md).expect("refused");
        assert!(
            err.contains("markdown") || err.contains("Scenario") || err.contains("Feature"),
            "{err}"
        );
    }

    #[test]
    fn scenario_with_when_then_is_ok() {
        let ok = "@epic:recipes\nFeature: Recipes\n  Scenario: Publish a link\n    Given I am signed in as \"maya@example.com\"\n    When I publish \"Weeknight Tomato Pasta\"\n    Then the recipe is public\n    #observe: unsigned GET of the public share URL shows the title\n";
        assert_eq!(spec_write_error("spec/recipes.feature", ok), None);
    }

    #[test]
    fn scenario_without_then_is_refused() {
        let bad = "@epic:recipes\nFeature: Recipes\n  Scenario: Incomplete\n    Given I am signed in as \"maya@example.com\"\n";
        assert!(spec_write_error("spec/recipes.feature", bad).unwrap().contains("When"));
    }

    #[test]
    fn then_without_observe_is_refused() {
        let bad = "@epic:recipes\nFeature: Recipes\n  Scenario: Publish a link\n    Given I am signed in as \"maya@example.com\"\n    When I publish \"Weeknight Tomato Pasta\"\n    Then the recipe is public\n";
        let err = spec_write_error("spec/recipes.feature", bad).expect("refused");
        assert!(err.contains("#observe:"), "{err}");
    }
}

pub fn spec_is_playable(root: &Path) -> bool {
    load_specs(&root.join("spec"), false)
        .map(|fs| fs.iter().any(|f| !f.scenarios.is_empty()))
        .unwrap_or(false)
}

/// First spec from the interview. Author 2B+ may override. Does not clobber
/// a spec that already has Scenario: lines.
pub fn seed_spec_from_plan(root: &Path, plan: &str) -> Result<Vec<String>, String> {
    let spec = root.join("spec");
    fs::create_dir_all(&spec).map_err(|e| e.to_string())?;
    let existing = load_specs(&spec, false).unwrap_or_default();
    if gherkin_scenario_count(&existing) > 0 {
        return Ok(Vec::new());
    }
    let p = plan.to_ascii_lowercase();
    let mut wrote = Vec::new();
    for (name, body, hit) in seed_feature_bodies() {
        if !hit(&p) {
            continue;
        }
        if let Some(err) = spec_write_error(&format!("spec/{name}"), body) {
            return Err(err);
        }
        let path = spec.join(name);
        fs::write(&path, body).map_err(|e| e.to_string())?;
        wrote.push(format!("spec/{name}"));
    }
    Ok(wrote)
}

fn seed_feature_bodies() -> Vec<(&'static str, &'static str, fn(&str) -> bool)> {
    vec![
        (
            "recipes.feature",
            r#"@epic:recipes
Feature: Create and publish recipes
  Scenario: Author publishes a titled recipe
    Given I am signed in as "maya@example.com"
    When I create a recipe titled "Weeknight Tomato Pasta"
    And I add ingredient "tomatoes"
    And I add step 1 "Boil water"
    And I publish "Weeknight Tomato Pasta"
    Then the recipe "Weeknight Tomato Pasta" is public
    #observe: unsigned GET of the public share URL shows title Weeknight Tomato Pasta
"#,
            |p| p.contains("recipe") && (p.contains("title") || p.contains("publish")),
        ),
        (
            "sharing.feature",
            r#"@epic:sharing
Feature: Public recipe link
  Scenario: Anyone opens a public recipe without an account
    Given a public recipe "Weeknight Tomato Pasta" at "/r/weeknight-tomato-pasta"
    When an anonymous viewer opens "/r/weeknight-tomato-pasta"
    Then they see title "Weeknight Tomato Pasta"
    #observe: unsigned GET of /r/weeknight-tomato-pasta shows title Weeknight Tomato Pasta
"#,
            |p| p.contains("public") && (p.contains("link") || p.contains("account")),
        ),
        (
            "video.feature",
            r#"@epic:video
Feature: Recipe video timestamps
  Scenario: Cook jumps to a tagged step
    Given I am signed in as "maya@example.com"
    And a recipe "Weeknight Tomato Pasta" owned by "maya@example.com"
    And the recipe has full video "https://example.com/pasta.mp4"
    And step 1 of "Weeknight Tomato Pasta" is "Boil water"
    When I tag step 1 at timestamp "00:00:12"
    Then step 1 of "Weeknight Tomato Pasta" links to "00:00:12"
    #observe: opening the step clip starts at 00:00:12
"#,
            |p| p.contains("video") || p.contains("timestamp"),
        ),
        (
            "packets.feature",
            r#"@epic:ingredients
Feature: Ingredient packets
  Scenario: Author builds a packet with an Amazon Fresh link
    Given I am signed in as "maya@example.com"
    And a public recipe "Weeknight Tomato Pasta" with ingredients:
      | name     | quantity |
      | tomatoes | 4        |
    When the author creates packet "sauce" from all ingredients
    And the author attaches Amazon Fresh order link "https://fresh.amazon.com/sauce" to packet "sauce"
    Then packet "sauce" contains 1 items
    #observe: packet sauce item count is 1
    And the packet is linked to recipe "Weeknight Tomato Pasta"
    #observe: packet sauce names recipe Weeknight Tomato Pasta
    And packet "sauce" has Amazon Fresh order link "https://fresh.amazon.com/sauce"
    #observe: packet sauce order URL is the Amazon Fresh link https://fresh.amazon.com/sauce
"#,
            |p| p.contains("amazon") || p.contains("packet") || p.contains("ingredient"),
        ),
        (
            "patrons.feature",
            r#"@epic:patrons
Feature: Patron subscriptions
  Scenario: Reader subscribes at five dollars a month
    Given I am signed in as "maya@example.com"
    When I enable patronage at "$5" per month
    Then my profile shows patronage available at "$5" per month
    #observe: patronage offer for maya@example.com is $5 per month
  Scenario: Patron opens a patron-only recipe
    Given "alex@example.com" offers patronage at "$5" per month
    And "sam@example.com" is an active patron of "alex@example.com"
    And a patron-only recipe "Patron Pasta" owned by "alex@example.com"
    When "sam@example.com" opens the share URL for "Patron Pasta"
    Then they see title "Patron Pasta"
    #observe: signed-in patron GET of the share URL shows title Patron Pasta
"#,
            |p| p.contains("patron") || p.contains("$5") || p.contains("substack"),
        ),
        (
            "todo.feature",
            r#"@epic:tasks
Feature: Todo list
  Scenario: Add a titled task
    Given the list is empty
    When I add a task titled "Buy milk"
    Then the list shows "Buy milk" as active
    #observe: the active list contains Buy milk
  Scenario: Mark a task done
    Given a task "Buy milk" is active
    When I mark "Buy milk" done
    Then "Buy milk" is completed
    #observe: Buy milk is in the completed list and not in the active list
  Scenario: Filter active tasks
    Given a task "Buy milk" is completed
    And a task "Call Sam" is active
    When I filter to active
    Then the visible list is only "Call Sam"
    #observe: the visible list is only Call Sam
  Scenario: Clear completed tasks
    Given a task "Buy milk" is completed
    And a task "Call Sam" is active
    When I clear completed
    Then the list is only "Call Sam"
    #observe: completed is empty and Call Sam remains
"#,
            |p| {
                p.contains("todo")
                    || (p.contains("task") && (p.contains("done") || p.contains("list")))
            },
        ),
    ]
}

#[cfg(test)]
mod spec_seed_tests {
    use super::*;
    use std::fs;

    #[test]
    fn seed_bodies_pass_spec_write_error() {
        for (name, body, _) in seed_feature_bodies() {
            assert_eq!(
                spec_write_error(&format!("spec/{name}"), body),
                None,
                "{name} is not a valid spec"
            );
        }
    }

    #[test]
    fn seed_from_recipe_interview() {
        let t = tempfile::TempDir::new().unwrap();
        let plan = concat!(
            "A tool to share and create cooking recipes. Authors write a recipe with a title, ",
            "ingredients, and ordered steps, then publish a public link anyone can open without an account. ",
            "They can attach one full-recipe video and tag timestamps on each step. ",
            "Ingredients group into packets with an Amazon Fresh order link. ",
            "Patrons work the way Substack does: a reader subscribes at $5 a month."
        );
        let wrote = seed_spec_from_plan(t.path(), plan).unwrap();
        assert!(
            wrote.iter().any(|f| f.ends_with("recipes.feature")),
            "{wrote:?}"
        );
        assert!(wrote.iter().any(|f| f.ends_with("sharing.feature")), "{wrote:?}");
        assert!(wrote.iter().any(|f| f.ends_with("video.feature")), "{wrote:?}");
        assert!(wrote.iter().any(|f| f.ends_with("packets.feature")), "{wrote:?}");
        assert!(wrote.iter().any(|f| f.ends_with("patrons.feature")), "{wrote:?}");
        assert!(spec_is_playable(t.path()));
        assert_eq!(seed_spec_from_plan(t.path(), plan).unwrap().len(), 0);
    }

    #[test]
    fn seed_from_todo_interview() {
        let t = tempfile::TempDir::new().unwrap();
        let wrote = seed_spec_from_plan(
            t.path(),
            "A todo list. Add a task with a title. Mark it done. Filter active vs completed.",
        )
        .unwrap();
        assert!(wrote.iter().any(|f| f.ends_with("todo.feature")), "{wrote:?}");
        assert!(spec_is_playable(t.path()));
        assert!(!wrote.iter().any(|f| f.contains("recipes.feature")));
    }

    #[test]
    fn seed_does_not_clobber_existing_scenarios() {
        let t = tempfile::TempDir::new().unwrap();
        fs::create_dir_all(t.path().join("spec")).unwrap();
        fs::write(
            t.path().join("spec/keep.feature"),
            "@epic:keep\nFeature: Keep\n  Scenario: Already here\n    When I publish \"x\"\n    Then it is public\n",
        )
        .unwrap();
        let wrote = seed_spec_from_plan(t.path(), "recipes title publish video patron $5").unwrap();
        assert!(wrote.is_empty());
        let body = fs::read_to_string(t.path().join("spec/keep.feature")).unwrap();
        assert!(body.contains("Already here"));
    }
}

pub fn load_specs(spec_dir: &Path, strict: bool) -> Result<Vec<Feature>, SpecParseError> {
    let mut out = Vec::new();
    let mut errors = HashMap::new();
    if !spec_dir.exists() {
        return Ok(out);
    }
    let mut paths: Vec<PathBuf> = Vec::new();
    for entry in walkdir::WalkDir::new(spec_dir).into_iter().filter_map(|e| e.ok()) {
        if entry.file_type().is_file()
            && entry.path().extension().and_then(|s| s.to_str()) == Some("feature")
        {
            paths.push(entry.path().to_path_buf());
        }
    }
    paths.sort();
    for p in paths {
        let rel = p
            .strip_prefix(spec_dir)
            .unwrap_or(&p)
            .to_string_lossy()
            .replace('\\', "/");
        match fs::read_to_string(&p) {
            Ok(text) => match parse_text(&text, &rel) {
                Ok(Some(f)) => out.push(f),
                Ok(None) => {}
                Err(e) => {
                    errors.insert(rel, e.lines().next().unwrap_or("parse error").chars().take(200).collect());
                }
            },
            Err(e) => {
                errors.insert(rel, e.to_string());
            }
        }
    }
    if !errors.is_empty() && strict {
        return Err(SpecParseError { errors });
    }
    Ok(out)
}

/// Write a `.feature` under spec/. Rejects path escape.
pub fn put_spec_file(spec_dir: &Path, file: &str, body: &str) -> Result<String, String> {
    let rel = file
        .trim()
        .trim_start_matches("./")
        .trim_start_matches("spec/")
        .trim_start_matches('/');
    if rel.is_empty()
        || rel.contains("..")
        || std::path::Path::new(rel).is_absolute()
        || rel.as_bytes().contains(&b'\\')
    {
        return Err("spec path must stay under spec/".into());
    }
    if !rel.ends_with(".feature") {
        return Err("only .feature files can be saved here".into());
    }
    let dest = spec_dir.join(rel);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(&dest, body).map_err(|e| e.to_string())?;
    Ok(rel.to_string())
}

pub fn stamp_rids(spec_dir: &Path) -> std::io::Result<HashMap<String, String>> {
    let mut minted = HashMap::new();
    let mut paths: Vec<PathBuf> = walkdir::WalkDir::new(spec_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file() && e.path().extension().and_then(|s| s.to_str()) == Some("feature"))
        .map(|e| e.into_path())
        .collect();
    paths.sort();
    let mut existing: HashSet<String> = HashSet::new();
    for path in &paths {
        let text = fs::read_to_string(path)?;
        for cap in rid_re().captures_iter(&text) {
            existing.insert(cap[1].to_string());
        }
    }
    for path in &paths {
        let raw = fs::read(path)?;
        let newline: &[u8] = if raw.windows(2).any(|w| w == b"\r\n") {
            b"\r\n"
        } else {
            b"\n"
        };
        let text = String::from_utf8_lossy(&raw);
        let rel = path
            .strip_prefix(spec_dir)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let feat = match parse_text(&text.replace("\r\n", "\n"), &rel) {
            Ok(Some(f)) => f,
            _ => continue,
        };
        let mut lines: Vec<String> = if newline == b"\r\n" {
            text.replace("\r\n", "\n").split('\n').map(|s| s.to_string()).collect()
        } else {
            text.split('\n').map(|s| s.to_string()).collect()
        };
        // drop trailing empty from split
        if lines.last().map(|s| s.is_empty()).unwrap_or(false) {
            lines.pop();
        }
        let mut scenarios = feat.scenarios;
        scenarios.sort_by_key(|s| std::cmp::Reverse(s.block_start()));
        for sc in scenarios {
            if sc.rid.is_some() {
                continue;
            }
            let mut rid = new_rid();
            while existing.contains(&rid) {
                rid = new_rid();
            }
            existing.insert(rid.clone());
            minted.insert(rid.clone(), sc.name.clone());
            let idx = sc.block_start().saturating_sub(1);
            let indent = lines
                .get(idx)
                .map(|line| {
                    line.chars().take_while(|c| c.is_whitespace()).collect::<String>()
                })
                .unwrap_or_else(|| "  ".to_string());
            lines.insert(idx, format!("{indent}{RID_PREFIX}{rid}"));
        }
        let mut out = lines.join(std::str::from_utf8(newline).unwrap());
        out.push_str(std::str::from_utf8(newline).unwrap());
        fs::write(path, out.as_bytes())?;
    }
    Ok(minted)
}

pub fn duplicate_rids(features: &[Feature]) -> Vec<(String, String)> {
    let mut problems = Vec::new();
    for f in features {
        for s in &f.scenarios {
            if s.rid_count > 1 {
                problems.push((f.file.clone(), s.name.clone()));
            }
        }
    }
    let mut seen: HashMap<String, String> = HashMap::new();
    for f in features {
        for s in &f.scenarios {
            if let Some(rid) = &s.rid {
                if let Some(prev) = seen.get(rid) {
                    problems.push((
                        f.file.clone(),
                        format!("{} reuses id {rid} from {prev}", s.name),
                    ));
                } else {
                    seen.insert(rid.clone(), s.name.clone());
                }
            }
        }
    }
    problems
}

/// Stamp `@rid:` on one scenario (`file` relative to spec/, `line` is Scenario: or tag line).
pub fn stamp_scenario(spec_dir: &Path, file: &str, line: usize) -> Result<Option<String>, String> {
    let path = spec_dir.join(file);
    let raw = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let newline = if raw.contains("\r\n") { "\r\n" } else { "\n" };
    let feat = parse_text(&raw.replace("\r\n", "\n"), file)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("{file} is not a feature"))?;
    let sc = feat
        .scenarios
        .iter()
        .find(|s| s.line == line || s.block_start() == line)
        .ok_or_else(|| format!("no scenario at {file}:{line}"))?;
    if let Some(rid) = &sc.rid {
        return Ok(Some(rid.clone()));
    }
    let mut existing: HashSet<String> = HashSet::new();
    if let Ok(all) = load_specs(spec_dir, true) {
        for f in all {
            for s in f.scenarios {
                if let Some(r) = s.rid {
                    existing.insert(r);
                }
            }
        }
    }
    let mut rid = new_rid();
    while existing.contains(&rid) {
        rid = new_rid();
    }
    let mut lines: Vec<String> = raw.replace("\r\n", "\n").split('\n').map(|s| s.to_string()).collect();
    if lines.last().map(|s| s.is_empty()).unwrap_or(false) {
        lines.pop();
    }
    let idx = sc.block_start().saturating_sub(1);
    let indent = lines
        .get(idx)
        .map(|line| line.chars().take_while(|c| c.is_whitespace()).collect::<String>())
        .unwrap_or_else(|| "  ".into());
    lines.insert(idx, format!("{indent}{RID_PREFIX}{rid}"));
    let mut out = lines.join(newline);
    out.push_str(newline);
    fs::write(&path, out).map_err(|e| e.to_string())?;
    Ok(Some(rid))
}

/// Replace a scenario's name and steps. Keeps existing @rid / @holdout unless `holdout` is set.
pub fn rewrite_scenario(
    spec_dir: &Path,
    file: &str,
    line: usize,
    name: &str,
    steps: &[String],
    holdout: Option<bool>,
) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("scenario name is empty".into());
    }
    let path = spec_dir.join(file);
    let raw = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let newline = if raw.contains("\r\n") { "\r\n" } else { "\n" };
    let normalized = raw.replace("\r\n", "\n");
    let feat = parse_text(&normalized, file)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("{file} is not a feature"))?;
    let sc = feat
        .scenarios
        .iter()
        .find(|s| s.line == line || s.block_start() == line)
        .ok_or_else(|| format!("no scenario at {file}:{line}"))?;
    let mut lines: Vec<String> = normalized.split('\n').map(|s| s.to_string()).collect();
    if lines.last().map(|s| s.is_empty()).unwrap_or(false) {
        lines.pop();
    }
    let blocks = feat.blocks(lines.len());
    let (_, start, end) = blocks
        .into_iter()
        .find(|(_, s, _)| *s == sc.block_start())
        .ok_or_else(|| "could not locate scenario block".to_string())?;
    let sidx = start.saturating_sub(1);
    let eidx = end.saturating_sub(1).min(lines.len());
    let indent = lines
        .get(sidx)
        .map(|l| l.chars().take_while(|c| c.is_whitespace()).collect::<String>())
        .unwrap_or_else(|| "  ".into());
    let keep_holdout = holdout.unwrap_or_else(|| sc.is_holdout());
    let mut block = Vec::new();
    if keep_holdout {
        block.push(format!("{indent}{HOLDOUT_TAG}"));
    }
    if let Some(rid) = &sc.rid {
        block.push(format!("{indent}{RID_PREFIX}{rid}"));
    }
    block.push(format!("{indent}Scenario: {name}"));
    for step in steps {
        let t = step.trim();
        if t.is_empty() {
            continue;
        }
        block.push(format!("{indent}  {t}"));
    }
    block.push(String::new());
    lines.splice(sidx..eidx, block);
    let mut out = lines.join(newline);
    if !out.ends_with(newline) {
        out.push_str(newline);
    }
    fs::write(&path, out).map_err(|e| e.to_string())?;
    Ok(())
}

/// Drop scenario blocks from feature files. `drop` is `(relative path, block_start line)`.
/// Empty feature files are removed.
pub fn drop_scenario_blocks(spec_dir: &Path, drop: &[(String, usize)]) -> std::io::Result<usize> {
    if drop.is_empty() {
        return Ok(0);
    }
    let mut by_file: HashMap<String, Vec<usize>> = HashMap::new();
    for (file, start) in drop {
        by_file.entry(file.clone()).or_default().push(*start);
    }
    let mut removed = 0usize;
    for (rel, mut starts) in by_file {
        starts.sort_unstable();
        starts.dedup();
        let path = spec_dir.join(&rel);
        let raw = fs::read_to_string(&path)?;
        let newline = if raw.contains("\r\n") { "\r\n" } else { "\n" };
        let feat = match parse_text(&raw.replace("\r\n", "\n"), &rel) {
            Ok(Some(f)) => f,
            _ => continue,
        };
        let mut lines: Vec<String> = raw.replace("\r\n", "\n").split('\n').map(|s| s.to_string()).collect();
        if lines.last().map(|s| s.is_empty()).unwrap_or(false) {
            lines.pop();
        }
        let blocks = feat.blocks(lines.len());
        let mut ranges: Vec<(usize, usize)> = blocks
            .into_iter()
            .filter(|(_, start, _)| starts.contains(start))
            .map(|(_, start, end)| (start, end))
            .collect();
        ranges.sort_by_key(|(s, _)| std::cmp::Reverse(*s));
        for (start, end) in ranges {
            let s = start.saturating_sub(1);
            let e = end.saturating_sub(1).min(lines.len());
            if s < e {
                lines.drain(s..e);
                removed += 1;
            }
        }
        let leftover = parse_text(&(lines.join("\n") + "\n"), &rel)
            .ok()
            .flatten();
        if leftover.as_ref().map(|f| f.scenarios.is_empty()).unwrap_or(true) {
            fs::remove_file(&path)?;
        } else {
            let mut out = lines.join(newline);
            out.push_str(newline);
            fs::write(&path, out)?;
        }
    }
    Ok(removed)
}

pub fn strip_holdouts(text: &str) -> String {
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let normalized = text.replace("\r\n", "\n");
    let feat = match parse_text(&normalized, "<memory>") {
        Ok(Some(f)) => f,
        _ => return text.to_string(),
    };
    let mut lines: Vec<&str> = normalized.split('\n').collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    let ranges: Vec<_> = feat
        .blocks(lines.len())
        .into_iter()
        .filter(|(sc, _, _)| sc.is_holdout())
        .map(|(_, start, end)| (start, end))
        .collect();
    let mut lines: Vec<String> = lines.into_iter().map(|s| s.to_string()).collect();
    for (start, end) in ranges.into_iter().rev() {
        let s = start.saturating_sub(1);
        let e = end.saturating_sub(1).min(lines.len());
        if s < e {
            lines.drain(s..e);
        }
    }
    let mut out = lines.join(newline);
    out.push_str(newline);
    out
}

pub fn holdout_rids(features: &[Feature]) -> HashSet<String> {
    features
        .iter()
        .flat_map(|f| f.scenarios.iter())
        .filter(|s| s.is_holdout())
        .filter_map(|s| s.rid.clone())
        .collect()
}
