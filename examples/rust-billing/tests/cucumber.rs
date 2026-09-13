//! Step definitions. Written by the stepwright, which sees the approved spec and nothing of
//! the implementation. Imports only the surface declared in contract/interface.md.
use cucumber::{given, then, when, World};

#[derive(Debug, Default, World)]
pub struct BillingWorld {
    days: u32,
    stage: String,
    cents: i64,
    code: String,
    formatted: String,
}

#[given(expr = "an invoice {int} days overdue")]
fn given_days(w: &mut BillingWorld, days: u32) {
    w.days = days;
}

#[when("I ask which reminder is due")]
fn when_ask(w: &mut BillingWorld) {
    w.stage = billing::reminder_stage(w.days).to_string();
}

#[then(expr = "the reminder is {string}")]
fn then_reminder(w: &mut BillingWorld, expected: String) {
    assert_eq!(w.stage, expected, "wrong reminder stage");
}

#[given(expr = "an amount of {int} minor units in {string}")]
fn given_amount(w: &mut BillingWorld, cents: i64, code: String) {
    w.cents = cents;
    w.code = code;
}

#[when("I format it for the customer")]
fn when_format(w: &mut BillingWorld) {
    w.formatted = billing::format_amount(w.cents, &w.code);
}

#[then(expr = "it reads {string}")]
fn then_reads(w: &mut BillingWorld, expected: String) {
    assert_eq!(w.formatted, expected, "wrong formatted amount");
}

fn main() {
    let path = std::env::var("SHALT_REPORT").unwrap_or_else(|_| ".shalt/cucumber.json".into());
    if let Some(dir) = std::path::Path::new(&path).parent() {
        std::fs::create_dir_all(dir).ok();
    }
    let file = std::fs::File::create(&path).expect("could not create the report file");
    futures::executor::block_on(
        BillingWorld::cucumber()
            .with_writer(cucumber::writer::Json::new(file))
            .run("spec"),
    );
}
