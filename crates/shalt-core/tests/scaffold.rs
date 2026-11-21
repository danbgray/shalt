use shalt_core::config::init_workspace;
use shalt_core::scaffold::{
    apply_js_contract_stubs, apply_step_stubs, archive_src, fill_all_js_pending_oracles,
    fill_js_pending_oracles, fill_js_server, fill_js_store_if_stub, is_fillable_stub,
    parse_js_contract, quarantine_duplicate_step_files, steps_source_ok, write_js_world_if_missing,
};
use shalt_core::list_step_files;
use shalt_core::DUP_STEPS_DIR;
use tempfile::TempDir;

const CONTRACT: &str = r#"# Public surface

## `../src/recipes.js`

- `createUser(email)` → user
- `createRecipe(user, title)` → recipe

## `../src/packets.js`

- `createPacket(recipe, name, ingredientNames)` → packet
"#;

#[test]
fn parse_js_contract_reads_modules_and_fns() {
    let mods = parse_js_contract(CONTRACT);
    assert_eq!(mods.len(), 2);
    assert_eq!(mods[0].path, "src/recipes.js");
    assert_eq!(
        mods[0]
            .fns
            .iter()
            .map(|f| f.name.as_str())
            .collect::<Vec<_>>(),
        vec!["createUser", "createRecipe"]
    );
    assert_eq!(mods[1].path, "src/packets.js");
}

#[test]
fn contract_stubs_fill_missing_modules_and_skip_real_code() {
    let t = TempDir::new().unwrap();
    init_workspace(t.path(), "javascript", "cook").unwrap();
    std::fs::create_dir_all(t.path().join("contract")).unwrap();
    std::fs::write(t.path().join("contract/interface.md"), CONTRACT).unwrap();
    let wrote = apply_js_contract_stubs(t.path()).unwrap();
    assert!(wrote.contains(&"src/recipes.js".into()), "{wrote:?}");
    assert!(wrote.contains(&"src/packets.js".into()), "{wrote:?}");
    let recipes = std::fs::read_to_string(t.path().join("src/recipes.js")).unwrap();
    assert!(recipes.contains("not implemented: createUser"));
    assert!(is_fillable_stub(&recipes));

    std::fs::write(
        t.path().join("src/recipes.js"),
        "export function createUser(email) { return { email }; }\n",
    )
    .unwrap();
    let wrote = apply_js_contract_stubs(t.path()).unwrap();
    assert!(
        !wrote.iter().any(|p| p == "src/recipes.js"),
        "must not clobber a filled module: {wrote:?}"
    );
    let recipes = std::fs::read_to_string(t.path().join("src/recipes.js")).unwrap();
    assert!(recipes.contains("return { email }"));
}

#[test]
fn http_observe_fills_fetch_then() {
    let t = TempDir::new().unwrap();
    std::fs::write(
        t.path().join("shalt.toml"),
        "[project]\nstack = \"javascript\"\n[zones]\nsteps = \"steps\"\nsrc = \"src\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(t.path().join("spec")).unwrap();
    std::fs::create_dir_all(t.path().join("steps")).unwrap();
    std::fs::write(
        t.path().join("spec/sharing.feature"),
        r#"@epic:sharing
Feature: Public recipe link
  Scenario: Anyone opens a public recipe without an account
    Given a public recipe "Weeknight Tomato Pasta" at "/r/weeknight-tomato-pasta"
    When an anonymous viewer opens "/r/weeknight-tomato-pasta"
    Then they see title "Weeknight Tomato Pasta"
    #observe: unsigned GET of /r/weeknight-tomato-pasta shows title Weeknight Tomato Pasta
"#,
    )
    .unwrap();
    std::fs::write(
        t.path().join("steps/sharing.steps.js"),
        concat!(
            "import { Given, When, Then } from '@cucumber/cucumber';\n",
            "Then('they see title {string}', function (a0) {\n  return 'pending';\n});\n",
        ),
    )
    .unwrap();
    let filled = fill_js_pending_oracles(t.path(), "sharing").unwrap();
    assert!(!filled.is_empty(), "{filled:?}");
    let steps = std::fs::read_to_string(t.path().join("steps/sharing.steps.js")).unwrap();
    assert!(steps.contains("fetch("), "{steps}");
    assert!(steps.contains("async function"), "{steps}");
    assert!(!steps.contains("isPublic"), "{steps}");
    assert!(
        !steps.contains("this.baseUrl + this.lastPath ||"),
        "fetch URL must parenthesize path or lastPath is glued onto the port\n{steps}"
    );
    assert!(
        steps.contains("this.baseUrl + (this.lastPath")
            || steps.contains("this.baseUrl + '/r/"),
        "{steps}"
    );
    assert!(fill_js_server(t.path(), false).unwrap());
    let stub = std::fs::read_to_string(t.path().join("src/server.js")).unwrap();
    assert!(stub.contains("404"), "{stub}");
    assert!(fill_js_server(t.path(), true).unwrap());
    let live = std::fs::read_to_string(t.path().join("src/server.js")).unwrap();
    assert!(live.contains("/r/"), "{live}");
    assert!(live.contains("/tasks"), "{live}");
    assert!(live.contains("POST") && live.contains("/recipes"), "{live}");
    assert!(live.contains("recipePage"), "{live}");
    assert!(live.contains("/patrons"), "{live}");
    assert!(live.contains("data-packet"), "{live}");
}

#[test]
fn http_observe_fills_packet_and_video_fetch() {
    let t = TempDir::new().unwrap();
    std::fs::write(
        t.path().join("shalt.toml"),
        "[project]\nstack = \"javascript\"\n[zones]\nsteps = \"steps\"\nsrc = \"src\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(t.path().join("spec")).unwrap();
    std::fs::create_dir_all(t.path().join("steps")).unwrap();
    std::fs::write(
        t.path().join("spec/packets.feature"),
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
    #observe: GET /r/weeknight-tomato-pasta includes packet sauce with 1 item
"#,
    )
    .unwrap();
    std::fs::write(
        t.path().join("spec/video.feature"),
        r#"@epic:video
Feature: Recipe video timestamps
  Scenario: Cook jumps to a tagged step
    Given a recipe "Weeknight Tomato Pasta" owned by "maya@example.com"
    When I tag step 1 at timestamp "00:00:12"
    Then step 1 of "Weeknight Tomato Pasta" links to "00:00:12"
    #observe: GET /r/weeknight-tomato-pasta shows step 1 starting at 00:00:12
"#,
    )
    .unwrap();
    std::fs::write(
        t.path().join("steps/ingredients.steps.js"),
        concat!(
            "import { Given, When, Then } from '@cucumber/cucumber';\n",
            "Then('packet {string} contains 1 items', function (a0) {\n  return 'pending';\n});\n",
        ),
    )
    .unwrap();
    std::fs::write(
        t.path().join("steps/video.steps.js"),
        concat!(
            "import { Given, When, Then } from '@cucumber/cucumber';\n",
            "Then('step 1 of {string} links to {string}', function (a0, a1) {\n  return 'pending';\n});\n",
        ),
    )
    .unwrap();
    fill_js_pending_oracles(t.path(), "ingredients").unwrap();
    fill_js_pending_oracles(t.path(), "video").unwrap();
    let packets = std::fs::read_to_string(t.path().join("steps/ingredients.steps.js")).unwrap();
    assert!(packets.contains("fetch("), "{packets}");
    assert!(!packets.contains("packetItemCount"), "{packets}");
    let video = std::fs::read_to_string(t.path().join("steps/video.steps.js")).unwrap();
    assert!(video.contains("fetch("), "{video}");
    assert!(!video.contains("openStepClip"), "{video}");
}

#[test]
fn world_template_is_written_when_steps_start_not_at_init() {
    let t = TempDir::new().unwrap();
    init_workspace(t.path(), "javascript", "cook").unwrap();
    assert!(
        list_step_files(t.path(), "steps").is_empty(),
        "init still must not fake cucumber steps"
    );
    let wrote = write_js_world_if_missing(t.path()).unwrap();
    assert!(wrote.is_some());
    assert_eq!(
        list_step_files(t.path(), "steps"),
        vec!["steps/world.js".to_string()]
    );
    assert!(write_js_world_if_missing(t.path()).unwrap().is_none());
}

#[test]
fn promote_prototype_copies_journeys_when_final_css_exists() {
    let t = TempDir::new().unwrap();
    init_workspace(t.path(), "javascript", "cook").unwrap();
    std::fs::create_dir_all(t.path().join("mockups/journeys/recipes")).unwrap();
    std::fs::write(
        t.path().join("mockups/tokens.final.css"),
        "/* product */\n:root { --ink: #111; }\n",
    )
    .unwrap();
    std::fs::write(
        t.path().join("mockups/journeys/recipes/recipes.html"),
        r#"<section data-rid="S-aaa11111"><h1>Pasta</h1></section>"#,
    )
    .unwrap();
    std::fs::write(
        t.path().join("spec/recipes.feature"),
        "@epic:recipes\nFeature: Cook\n  @rid:S-aaa11111\n  Scenario: Make pasta\n    When they cook\n    Then pasta exists\n",
    )
    .unwrap();
    assert!(shalt_core::promote_prototype(t.path()).unwrap().is_empty() == false);
    let idx = std::fs::read_to_string(t.path().join("src/ui/index.html")).unwrap();
    assert!(idx.contains("shalt prototype app"), "{idx}");
    let boot = std::fs::read_to_string(t.path().join("src/ui/boot.js")).unwrap();
    assert!(boot.contains("fetch('/recipes')") || boot.contains("fetch('/tasks')"), "{boot}");
    assert!(!boot.contains("../store.js"), "browser boot must not import node:sqlite store\n{boot}");
    let page = std::fs::read_to_string(t.path().join("src/ui/journeys/recipes/recipes.html")).unwrap();
    assert!(page.contains("Pasta"), "{page}");
    assert!(page.contains("tokens.css"), "{page}");
    let css = std::fs::read_to_string(t.path().join("src/ui/tokens.css")).unwrap();
    assert!(css.contains("product"), "{css}");
    std::fs::write(t.path().join("src/ui/boot.js"), "/* custom boot */\n").unwrap();
    shalt_core::promote_prototype(t.path()).unwrap();
    let boot = std::fs::read_to_string(t.path().join("src/ui/boot.js")).unwrap();
    assert_eq!(boot, "/* custom boot */\n");
}

#[test]
fn promote_injects_markup_embed_when_key_present() {
    let t = TempDir::new().unwrap();
    init_workspace(t.path(), "javascript", "cook").unwrap();
    let mut cfg = std::fs::read_to_string(t.path().join("shalt.toml")).unwrap();
    cfg.push_str("\n[markup]\nkey = \"rmk_pub_testonly\"\nsecret = \"rmk_sec_neverprint\"\n");
    std::fs::write(t.path().join("shalt.toml"), cfg).unwrap();
    std::fs::create_dir_all(t.path().join("mockups/journeys/recipes")).unwrap();
    std::fs::write(
        t.path().join("mockups/tokens.final.css"),
        "/* product */\n:root { --ink: #111; }\n",
    )
    .unwrap();
    std::fs::write(
        t.path().join("mockups/journeys/recipes/recipes.html"),
        r#"<section data-rid="S-aaa11111"><h1>Pasta</h1></section>"#,
    )
    .unwrap();
    shalt_core::promote_prototype(t.path()).unwrap();
    let page = std::fs::read_to_string(t.path().join("src/ui/journeys/recipes/recipes.html")).unwrap();
    assert!(page.contains("embed.js"), "{page}");
    assert!(page.contains("rmk_pub_testonly"), "{page}");
    assert!(!page.contains("rmk_sec_neverprint"), "{page}");
    let idx = std::fs::read_to_string(t.path().join("src/ui/index.html")).unwrap();
    assert!(idx.contains("embed.js"), "{idx}");
    assert!(!idx.contains("rmk_sec_neverprint"), "{idx}");
}

#[test]
fn step_stubs_write_pending_js_and_replace_garbage() {
    let t = TempDir::new().unwrap();
    init_workspace(t.path(), "javascript", "cook").unwrap();
    std::fs::write(
        t.path().join("spec/ingredient-packets.feature"),
        r#"@epic:ingredients
Feature: Packets
  Scenario: Group
    Given a public recipe "Weeknight Tomato Pasta"
    When the author creates packet "Pasta night basics"
    Then packet "Pasta night basics" contains 3 items
"#,
    )
    .unwrap();
    let wrote = apply_step_stubs(t.path(), "ingredients").unwrap();
    assert_eq!(wrote, vec!["steps/ingredients.steps.js".to_string()]);
    let body = std::fs::read_to_string(t.path().join("steps/ingredients.steps.js")).unwrap();
    assert!(body.contains("Given('a public recipe {string}'"));
    assert!(body.contains("return 'pending'"));
    assert!(!body.contains("function (not arrow)"));
    assert!(steps_source_ok("steps/ingredients.steps.js", &body));

    std::fs::write(
        t.path().join("steps/ingredients.steps.js"),
        "function (not arrow) World;\nGiven a public recipe\n",
    )
    .unwrap();
    assert!(!steps_source_ok(
        "steps/ingredients.steps.js",
        "function (not arrow) World;\nGiven a public recipe\n"
    ));
    assert!(
        !steps_source_ok(
            "steps/recipes.steps.js",
            "When('I add ingredient {string}', function (name) { this.x = name; });\nWhen('I add ingredient {string}', function (name) { this.x = name; });\n"
        ),
        "the same phrase twice in one file is a parse miss, not a bind"
    );
    apply_step_stubs(t.path(), "ingredients").unwrap();
    let body = std::fs::read_to_string(t.path().join("steps/ingredients.steps.js")).unwrap();
    assert!(body.contains("return 'pending'"), "{body}");
    assert!(!body.contains("function (not arrow)"));
}

#[test]
fn step_stubs_keep_a_binding_file_and_skip_global_phrases() {
    let t = TempDir::new().unwrap();
    init_workspace(t.path(), "javascript", "cook").unwrap();
    std::fs::write(
        t.path().join("spec/patron-support.feature"),
        r#"@epic:patrons
Feature: Patrons
  Scenario: Enable
    Given I am signed in as "maya@example.com"
    When I enable patronage
    Then patronage is available
"#,
    )
    .unwrap();
    let real = r#"
import { Given, When, Then } from '@cucumber/cucumber';
Given('I am signed in as {string}', function (a) {
  this.user = a;
});
When('I enable patronage', function () {
  this.on = true;
});
Then('patronage is available', function () {
  if (!this.on) throw new Error('no');
});
"#;
    std::fs::write(t.path().join("steps/patronage.steps.js"), real).unwrap();
    let defs = shalt_core::bindings::parse_step_defs(real);
    assert_eq!(
        defs.iter().map(|d| d.pattern.clone()).collect::<Vec<_>>(),
        vec![
            "I am signed in as {string}".to_string(),
            "I enable patronage".to_string(),
            "patronage is available".to_string(),
        ],
        "{:?}",
        defs
    );
    let listed = list_step_files(t.path(), "steps");
    assert!(
        listed.iter().any(|p| p.ends_with("patronage.steps.js")),
        "{listed:?}"
    );
    let wrote = apply_step_stubs(t.path(), "patrons").unwrap();
    assert!(wrote.is_empty(), "global phrases already defined: {wrote:?}");
    assert!(!t.path().join("steps/patrons.steps.js").exists());

    std::fs::write(t.path().join("steps/patrons.steps.js"), real).unwrap();
    let again = apply_step_stubs(t.path(), "patrons").unwrap();
    assert!(again.is_empty());
    let kept = std::fs::read_to_string(t.path().join("steps/patrons.steps.js")).unwrap();
    assert_eq!(kept, real);
}

#[test]
fn quarantine_moves_duplicate_step_files_out_of_steps() {
    let t = TempDir::new().unwrap();
    init_workspace(t.path(), "javascript", "cook").unwrap();
    let real = r#"
import { Given } from '@cucumber/cucumber';
Given('I am signed in as {string}', function (a) { this.user = a; });
"#;
    std::fs::write(t.path().join("steps/patronage.steps.js"), real).unwrap();
    std::fs::write(t.path().join("steps/patrons.steps.js"), real).unwrap();
    let moved = quarantine_duplicate_step_files(t.path()).unwrap();
    assert_eq!(moved, vec!["steps/patrons.steps.js".to_string()]);
    assert!(t.path().join("steps/patronage.steps.js").exists());
    assert!(!t.path().join("steps/patrons.steps.js").exists());
    assert!(t.path().join(DUP_STEPS_DIR).join("patrons.steps.js").exists());
    let listed = list_step_files(t.path(), "steps");
    assert!(
        !listed.iter().any(|p| p.contains("patrons.steps.js")),
        "{listed:?}"
    );
}

#[test]
fn rust_step_stubs_append_to_shalt_rs_not_a_second_file() {
    let t = TempDir::new().unwrap();
    init_workspace(t.path(), "rust", "money").unwrap();
    std::fs::write(
        t.path().join("spec/money.feature"),
        r#"@epic:money
Feature: Money
  Scenario: Add
    Given amounts "1.00" and "2.00"
    Then the total is "3.00"
"#,
    )
    .unwrap();
    let wrote = apply_step_stubs(t.path(), "money").unwrap();
    assert_eq!(wrote, vec!["tests/shalt.rs".to_string()]);
    assert!(!t.path().join("tests/money.steps.rs").exists());
    let body = std::fs::read_to_string(t.path().join("tests/shalt.rs")).unwrap();
    assert!(body.contains("#[given(expr = \"amounts {string} and {string}\")]"), "{body}");
    assert!(body.contains("todo!()"), "{body}");
    let again = apply_step_stubs(t.path(), "money").unwrap();
    assert!(again.is_empty(), "{again:?}");
}

#[test]
fn promote_is_a_no_op_without_final_css() {
    let t = TempDir::new().unwrap();
    init_workspace(t.path(), "javascript", "cook").unwrap();
    assert!(shalt_core::promote_prototype(t.path()).unwrap().is_empty());
}

#[test]
fn fill_js_pending_oracles_replaces_pending_create_recipe() {
    let t = TempDir::new().unwrap();
    std::fs::write(
        t.path().join("shalt.toml"),
        "[project]\nstack = \"javascript\"\n[zones]\nsteps = \"steps\"\nsrc = \"src\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(t.path().join("steps")).unwrap();
    std::fs::create_dir_all(t.path().join("src")).unwrap();
    std::fs::write(
        t.path().join("src/recipes.js"),
        "export function createRecipe(o, t) { return { title: t, ingredients: [] }; }\n",
    )
    .unwrap();
    std::fs::write(
        t.path().join("steps/recipes.steps.js"),
        "import { Given, When, Then } from '@cucumber/cucumber';\nimport assert from 'node:assert/strict';\n\nWhen('I create a recipe titled {string}', function (a0) {\n  return 'pending';\n});\nThen('the recipe {string} has 3 ingredients', function (a0) {\n  return 'pending';\n});\n",
    )
    .unwrap();
    let filled = fill_js_pending_oracles(t.path(), "recipes").unwrap();
    assert_eq!(filled.len(), 2, "{filled:?}");
    let body = std::fs::read_to_string(t.path().join("steps/recipes.steps.js")).unwrap();
    assert!(!body.contains("return 'pending'"), "{body}");
    assert!(body.contains("createRecipe"), "{body}");
    assert!(
        body.contains("from '../src/store.js'") || body.contains("from '../src/recipes.js'"),
        "{body}"
    );
}

#[test]
fn fill_js_pending_oracles_covers_vague_author_phrases() {
    let t = TempDir::new().unwrap();
    std::fs::write(
        t.path().join("shalt.toml"),
        "[project]\nstack = \"javascript\"\n[zones]\nsteps = \"steps\"\nsrc = \"src\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(t.path().join("steps")).unwrap();
    std::fs::create_dir_all(t.path().join("src")).unwrap();
    std::fs::write(
        t.path().join("src/store.js"),
        "export function createRecipe() {}\nexport function publish() {}\nexport function resetStore() {}\n",
    )
    .unwrap();
    std::fs::write(
        t.path().join("steps/tool.steps.js"),
        concat!(
            "import { Given, When, Then } from '@cucumber/cucumber';\n",
            "When('an author writes a recipe with a title, ingredients, and ordered steps', function () {\n  return 'pending';\n});\n",
            "When('the tool publishes the recipe publicly without an account', function () {\n  return 'pending';\n});\n",
            "Then('anyone can open the public link to view the content', function () {\n  return 'pending';\n});\n",
        ),
    )
    .unwrap();
    let filled = fill_js_pending_oracles(t.path(), "tool").unwrap();
    assert!(filled.len() >= 3, "{filled:?}");
    let body = std::fs::read_to_string(t.path().join("steps/tool.steps.js")).unwrap();
    assert!(!body.contains("return 'pending'"), "{body}");
    assert!(body.contains("createRecipe"), "{body}");
    assert!(body.contains("publish"), "{body}");
    assert!(body.contains("from '../src/store.js'"), "{body}");
    assert!(t.path().join("src/store.js").is_file());
    let store = std::fs::read_to_string(t.path().join("src/store.js")).unwrap();
    assert!(store.contains("export function createRecipe"), "{store}");
}

#[test]
fn todo_spec_does_not_get_a_recipe_store() {
    let t = TempDir::new().unwrap();
    std::fs::write(
        t.path().join("shalt.toml"),
        "[project]\nstack = \"javascript\"\n[zones]\nsteps = \"steps\"\nsrc = \"src\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(t.path().join("spec")).unwrap();
    std::fs::create_dir_all(t.path().join("steps")).unwrap();
    std::fs::create_dir_all(t.path().join(".shalt")).unwrap();
    std::fs::write(
        t.path().join(".shalt/plan.md"),
        "A todo list. A person adds a task with a title.\n",
    )
    .unwrap();
    std::fs::write(
        t.path().join("spec/todo.feature"),
        r#"@epic:tasks
Feature: Todo list
  Scenario: Add a titled task
    Given the list is empty
    When I add a task titled "Buy milk"
    Then the list shows "Buy milk" as active
    #observe: the active list contains Buy milk
  Scenario: Clear completed tasks
    Given a task "Buy milk" is completed
    When I clear completed
    Then the list is only "Call Sam"
    #observe: completed is empty
"#,
    )
    .unwrap();
    std::fs::write(
        t.path().join("steps/tasks.steps.js"),
        concat!(
            "import { Given, When, Then } from '@cucumber/cucumber';\n",
            "Given('the list is empty', function () {\n  return 'pending';\n});\n",
            "When('I add a task titled {string}', function (a0) {\n  return 'pending';\n});\n",
            "Then('the list shows {string} as active', function (a0) {\n  return 'pending';\n});\n",
            "When('I clear completed', function () {\n  return 'pending';\n});\n",
        ),
    )
    .unwrap();
    let filled = fill_all_js_pending_oracles(t.path()).unwrap();
    assert!(!filled.is_empty(), "{filled:?}");
    let store = std::fs::read_to_string(t.path().join("src/store.js")).unwrap();
    assert!(store.contains("export function addTask"), "{store}");
    assert!(
        store.contains("not implemented: addTask"),
        "steps must not fill a complete store — first run should be able to go red\n{store}"
    );
    assert!(
        !store.contains("export function createRecipe"),
        "todo must not inherit the recipe store\n{store}"
    );
    let steps = std::fs::read_to_string(t.path().join("steps/tasks.steps.js")).unwrap();
    assert!(steps.contains("addTask"), "{steps}");
    assert!(!steps.contains("return 'pending'"), "{steps}");
}

#[test]
fn recipe_store_on_a_todo_spec_is_replaced() {
    let t = TempDir::new().unwrap();
    std::fs::write(
        t.path().join("shalt.toml"),
        "[project]\nstack = \"javascript\"\n[zones]\nsteps = \"steps\"\nsrc = \"src\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(t.path().join("spec")).unwrap();
    std::fs::create_dir_all(t.path().join("src")).unwrap();
    std::fs::write(t.path().join("spec/todo.feature"), "Feature: Todo list\n  Scenario: Add a titled task\n    When I add a task titled \"x\"\n    Then the list shows \"x\" as active\n").unwrap();
    std::fs::write(
        t.path().join("src/store.js"),
        "export function createRecipe() {}\nexport function resetStore() {}\n",
    )
    .unwrap();
    assert!(fill_js_store_if_stub(t.path()).unwrap());
    let store = std::fs::read_to_string(t.path().join("src/store.js")).unwrap();
    assert!(store.contains("addTask"), "{store}");
    assert!(!store.contains("createRecipe"), "{store}");
}

#[test]
fn recipe_store_missing_share_exports_is_rewritten() {
    let t = TempDir::new().unwrap();
    std::fs::write(
        t.path().join("shalt.toml"),
        "[project]\nstack = \"javascript\"\n[zones]\nsteps = \"steps\"\nsrc = \"src\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(t.path().join("spec")).unwrap();
    std::fs::create_dir_all(t.path().join("src")).unwrap();
    std::fs::write(
        t.path().join("spec/recipes.feature"),
        "Feature: Recipes\n  Scenario: Publish\n    When I create a recipe titled \"x\"\n    Then the recipe \"x\" is public\n",
    )
    .unwrap();
    std::fs::write(
        t.path().join("src/store.js"),
        "export function createRecipe() {}\nexport function resetStore() {}\n",
    )
    .unwrap();
    assert!(fill_js_store_if_stub(t.path()).unwrap());
    let store = std::fs::read_to_string(t.path().join("src/store.js")).unwrap();
    assert!(store.contains("export function isPublic"), "{store}");
    assert!(store.contains("export function openShareUrl"), "{store}");
    assert!(store.contains("export function attachAmazonFreshLink"), "{store}");
}

#[test]
fn archive_src_snapshots_then_leaves_a_copy() {
    let t = TempDir::new().unwrap();
    std::fs::create_dir_all(t.path().join("src")).unwrap();
    std::fs::write(t.path().join("src/store.js"), "export function add() {}\n").unwrap();
    let dest = archive_src(t.path()).unwrap().expect("copied");
    assert!(dest.join("store.js").is_file());
    let body = std::fs::read_to_string(dest.join("store.js")).unwrap();
    assert!(body.contains("export function add"));
}

#[test]
fn fill_live_tree_when_env_set() {
    let Ok(root) = std::env::var("SHALT_FILL_ROOT") else {
        return;
    };
    let filled = fill_all_js_pending_oracles(std::path::Path::new(&root)).unwrap();
    eprintln!("filled {}", filled.len());
}
