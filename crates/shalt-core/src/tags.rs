//! Cucumber-style tag expressions and feature locators.

use crate::spec::{Feature, Scenario};
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub enum TagExpr {
    All,
    Tag(String),
    Not(Box<TagExpr>),
    And(Box<TagExpr>, Box<TagExpr>),
    Or(Box<TagExpr>, Box<TagExpr>),
}

impl TagExpr {
    pub fn parse(s: &str) -> Result<Self, String> {
        let t = s.trim();
        if t.is_empty() {
            return Ok(TagExpr::All);
        }
        let tokens = tokenize(t)?;
        let mut p = Parser { tokens, i: 0 };
        let expr = p.parse_or()?;
        p.expect_end()?;
        Ok(expr)
    }

    pub fn matches(&self, tags: &[String]) -> bool {
        match self {
            TagExpr::All => true,
            TagExpr::Tag(want) => tags.iter().any(|t| t == want),
            TagExpr::Not(inner) => !inner.matches(tags),
            TagExpr::And(a, b) => a.matches(tags) && b.matches(tags),
            TagExpr::Or(a, b) => a.matches(tags) || b.matches(tags),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Locator {
    pub path: String,
    pub line: Option<usize>,
}

impl Locator {
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        if let Some((path, rest)) = s.rsplit_once(':') {
            if looks_like_feature_path(path) && rest.chars().all(|c| c.is_ascii_digit()) {
                let line = rest.parse().ok()?;
                return Some(Locator {
                    path: path.to_string(),
                    line: Some(line),
                });
            }
        }
        if looks_like_feature_path(s) {
            return Some(Locator {
                path: s.to_string(),
                line: None,
            });
        }
        None
    }
}

pub fn looks_like_feature_arg(s: &str) -> bool {
    Locator::parse(s).is_some() || glob_feature_pattern(s)
}

fn glob_feature_pattern(s: &str) -> bool {
    s.contains('*') && s.contains(".feature")
}

fn looks_like_feature_path(s: &str) -> bool {
    let base = Path::new(s)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(s);
    base.ends_with(".feature")
}

#[derive(Debug, Clone, Copy)]
pub struct Pick {
    pub feature: usize,
    pub scenario: usize,
}

pub fn filter_scenarios(features: &[Feature], locators: &[Locator], tags: &TagExpr) -> Vec<Pick> {
    let mut out = Vec::new();
    for (fi, f) in features.iter().enumerate() {
        for (si, sc) in f.scenarios.iter().enumerate() {
            if !locators_match(f, sc, locators) {
                continue;
            }
            if tags.matches(&sc.all_tags()) {
                out.push(Pick {
                    feature: fi,
                    scenario: si,
                });
            }
        }
    }
    out
}

fn locators_match(f: &Feature, sc: &Scenario, locators: &[Locator]) -> bool {
    if locators.is_empty() {
        return true;
    }
    locators.iter().any(|loc| {
        if !path_matches(&f.file, &loc.path) {
            return false;
        }
        match loc.line {
            None => true,
            Some(n) => line_in_scenario(f, sc, n),
        }
    })
}

fn path_matches(feature_file: &str, locator: &str) -> bool {
    let a = Path::new(feature_file);
    let b = Path::new(locator);
    a == b
        || a.ends_with(b)
        || b.ends_with(a)
        || a.file_name() == b.file_name() && a.file_name().is_some()
}

fn line_in_scenario(f: &Feature, sc: &Scenario, n: usize) -> bool {
    if sc.line == n || sc.block_start() == n {
        return true;
    }
    let start = sc.block_start();
    if n < start {
        return false;
    }
    let next = f
        .scenarios
        .iter()
        .map(|s| s.block_start())
        .filter(|a| *a > start)
        .min()
        .unwrap_or(usize::MAX);
    n < next
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Tag(String),
    And,
    Or,
    Not,
    Lp,
    Rp,
}

fn tokenize(s: &str) -> Result<Vec<Tok>, String> {
    let mut out = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            ' ' | '\t' | '\n' | '\r' => i += 1,
            '(' => {
                out.push(Tok::Lp);
                i += 1;
            }
            ')' => {
                out.push(Tok::Rp);
                i += 1;
            }
            ',' => {
                out.push(Tok::Or);
                i += 1;
            }
            '~' => {
                out.push(Tok::Not);
                i += 1;
            }
            '@' => {
                let start = i;
                i += 1;
                while i < chars.len() && is_tag_char(chars[i]) {
                    i += 1;
                }
                let t: String = chars[start..i].iter().collect();
                if t == "@" {
                    return Err("empty tag".into());
                }
                out.push(Tok::Tag(t));
            }
            c if c.is_ascii_alphabetic() => {
                let start = i;
                while i < chars.len() && chars[i].is_ascii_alphabetic() {
                    i += 1;
                }
                let w: String = chars[start..i].iter().collect();
                match w.to_ascii_lowercase().as_str() {
                    "and" => out.push(Tok::And),
                    "or" => out.push(Tok::Or),
                    "not" => out.push(Tok::Not),
                    other => return Err(format!("unexpected {other:?} in tag expression")),
                }
            }
            c => return Err(format!("unexpected {c:?} in tag expression")),
        }
    }
    Ok(out)
}

fn is_tag_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | ':')
}

struct Parser {
    tokens: Vec<Tok>,
    i: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.i)
    }
    fn bump(&mut self) -> Option<Tok> {
        let t = self.tokens.get(self.i).cloned();
        if t.is_some() {
            self.i += 1;
        }
        t
    }
    fn parse_or(&mut self) -> Result<TagExpr, String> {
        let mut left = self.parse_and()?;
        while matches!(self.peek(), Some(Tok::Or)) {
            self.bump();
            let right = self.parse_and()?;
            left = TagExpr::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }
    fn parse_and(&mut self) -> Result<TagExpr, String> {
        let mut left = self.parse_not()?;
        while matches!(self.peek(), Some(Tok::And)) {
            self.bump();
            let right = self.parse_not()?;
            left = TagExpr::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }
    fn parse_not(&mut self) -> Result<TagExpr, String> {
        if matches!(self.peek(), Some(Tok::Not)) {
            self.bump();
            return Ok(TagExpr::Not(Box::new(self.parse_not()?)));
        }
        self.parse_primary()
    }
    fn parse_primary(&mut self) -> Result<TagExpr, String> {
        match self.bump() {
            Some(Tok::Tag(t)) => Ok(TagExpr::Tag(t)),
            Some(Tok::Lp) => {
                let inner = self.parse_or()?;
                match self.bump() {
                    Some(Tok::Rp) => Ok(inner),
                    _ => Err("missing ) in tag expression".into()),
                }
            }
            other => Err(format!("expected tag, got {other:?}")),
        }
    }
    fn expect_end(&self) -> Result<(), String> {
        if self.i < self.tokens.len() {
            Err("trailing tokens in tag expression".into())
        } else {
            Ok(())
        }
    }
}
