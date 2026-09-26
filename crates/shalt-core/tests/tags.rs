use shalt_core::spec::{Feature, Scenario};
use shalt_core::tags::{filter_scenarios, looks_like_feature_arg, Locator, TagExpr};

fn sc(name: &str, tags: &[&str], file: &str, line: usize) -> Scenario {
    Scenario {
        rid: None,
        name: name.into(),
        keyword: "Scenario".into(),
        tags: tags.iter().map(|t| t.to_string()).collect(),
        steps: vec!["Given a".into(), "Then b".into()],
        examples: vec![],
        feature_name: "F".into(),
        feature_file: file.into(),
        line,
        tag_lines: vec![line.saturating_sub(1)],
        rid_count: 0,
        inherited_tags: vec![],
        oracles: vec![],
    }
}

fn feat(file: &str, scenarios: Vec<Scenario>) -> Feature {
    Feature {
        name: "F".into(),
        file: file.into(),
        tags: vec![],
        background: vec![],
        scenarios,
        description: String::new(),
    }
}

#[test]
fn empty_expression_matches_all() {
    let e = TagExpr::parse("").unwrap();
    assert!(e.matches(&["@wip".into()]));
    assert!(e.matches(&[]));
}

#[test]
fn single_tag() {
    let e = TagExpr::parse("@wip").unwrap();
    assert!(e.matches(&["@wip".into()]));
    assert!(!e.matches(&["@slow".into()]));
}

#[test]
fn not_and_or() {
    let e = TagExpr::parse("@wip and not @holdout").unwrap();
    assert!(e.matches(&["@wip".into()]));
    assert!(!e.matches(&["@wip".into(), "@holdout".into()]));
    let e = TagExpr::parse("@wip or @slow").unwrap();
    assert!(e.matches(&["@slow".into()]));
    let e = TagExpr::parse("not @wip").unwrap();
    assert!(e.matches(&[]));
}

#[test]
fn comma_is_or() {
    let e = TagExpr::parse("@wip,@slow").unwrap();
    assert!(e.matches(&["@slow".into()]));
}

#[test]
fn epic_and_rid_tags() {
    let e = TagExpr::parse("@epic:billing").unwrap();
    assert!(e.matches(&["@epic:billing".into()]));
    let e = TagExpr::parse("@rid:S-ab12cd34").unwrap();
    assert!(e.matches(&["@rid:S-ab12cd34".into()]));
}

#[test]
fn parens() {
    let e = TagExpr::parse("(@wip or @slow) and not @holdout").unwrap();
    assert!(e.matches(&["@slow".into()]));
    assert!(!e.matches(&["@slow".into(), "@holdout".into()]));
}

#[test]
fn locator_file_and_line() {
    let loc = Locator::parse("spec/invoices.feature:12").unwrap();
    assert_eq!(loc.path, "spec/invoices.feature");
    assert_eq!(loc.line, Some(12));
    assert!(looks_like_feature_arg("spec/foo.feature"));
    assert!(looks_like_feature_arg("spec/*.feature"));
    assert!(!looks_like_feature_arg("delete"));
    assert!(!looks_like_feature_arg("spec"));
}

#[test]
fn filter_by_tag_and_line() {
    let features = vec![feat(
        "spec/invoices.feature",
        vec![
            sc("totals", &["@wip"], "spec/invoices.feature", 4),
            sc("overdue", &["@slow"], "spec/invoices.feature", 12),
        ],
    )];
    let tags = TagExpr::parse("@wip").unwrap();
    let picks = filter_scenarios(&features, &[], &tags);
    assert_eq!(picks.len(), 1);
    assert_eq!(features[picks[0].feature].scenarios[picks[0].scenario].name, "totals");

    let loc = Locator::parse("spec/invoices.feature:12").unwrap();
    let picks = filter_scenarios(&features, &[loc], &TagExpr::All);
    assert_eq!(picks.len(), 1);
    assert_eq!(features[picks[0].feature].scenarios[picks[0].scenario].name, "overdue");
}
