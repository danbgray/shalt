//! Colour for Gherkin and CLI output. Honours NO_COLOR, FORCE_COLOR, and tty.

use std::io::IsTerminal;
use std::sync::atomic::{AtomicU8, Ordering};

const OFF: u8 = 0;
const ANSI: u8 = 1;
const TRUE: u8 = 2;

static MODE: AtomicU8 = AtomicU8::new(OFF);

pub fn init(pref: &str) {
    let forced = std::env::var("FORCE_COLOR").is_ok() || std::env::var("CLICOLOR_FORCE").ok().map(|v| v != "0").unwrap_or(false);
    let no = std::env::var("NO_COLOR").is_ok();
    let tty = std::io::stdout().is_terminal();
    let on = match pref {
        "always" => true,
        "never" => false,
        _ if no => false,
        _ if forced => true,
        _ => tty,
    };
    let truecolor = on
        && std::env::var("COLORTERM")
            .map(|v| v.contains("truecolor") || v.contains("24bit"))
            .unwrap_or(false);
    MODE.store(
        if !on {
            OFF
        } else if truecolor {
            TRUE
        } else {
            ANSI
        },
        Ordering::Relaxed,
    );
}

fn level() -> u8 {
    MODE.load(Ordering::Relaxed)
}

pub fn on() -> bool {
    level() != OFF
}

fn wrap(code: &str, s: &str) -> String {
    if !on() {
        return s.to_string();
    }
    format!("\x1b[{code}m{s}\x1b[0m")
}

fn rgb(r: u8, g: u8, b: u8, fallback: &str, s: &str) -> String {
    match level() {
        OFF => s.to_string(),
        TRUE => format!("\x1b[38;2;{r};{g};{b}m{s}\x1b[0m"),
        _ => wrap(fallback, s),
    }
}

pub fn ok(s: &str) -> String {
    rgb(61, 186, 122, "32", s)
}
pub fn bad(s: &str) -> String {
    rgb(224, 93, 78, "31", s)
}
pub fn warn(s: &str) -> String {
    rgb(212, 160, 23, "33", s)
}
pub fn accent(s: &str) -> String {
    rgb(196, 163, 90, "33", s)
}
pub fn mute(s: &str) -> String {
    wrap("2", s)
}
pub fn bold(s: &str) -> String {
    wrap("1", s)
}
pub fn italic(s: &str) -> String {
    wrap("3", s)
}
pub fn keyword(s: &str) -> String {
    rgb(107, 140, 206, "36", s)
}

pub fn status(st: &str, s: &str) -> String {
    match st {
        "green" => ok(s),
        "red" => bad(s),
        "stale" => warn(s),
        "orphan" => mute(s),
        _ => mute(s),
    }
}

/// Highlight a Gherkin document. No-op when colour is off.
pub fn gherkin(text: &str) -> String {
    if !on() {
        return text.to_string();
    }
    text.split_inclusive('\n').map(gherkin_line).collect()
}

fn starts_kw(t: &str, kw: &str) -> bool {
    t.starts_with(kw) && (t.len() == kw.len() || t.as_bytes().get(kw.len()).map(|c| c.is_ascii_whitespace() || *c == b':').unwrap_or(true))
}

fn paint_quotes(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('"') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        if let Some(j) = after.find('"') {
            let inner = &after[..j];
            out.push_str(&accent(&format!("\"{inner}\"")));
            rest = &after[j + 1..];
        } else {
            out.push('"');
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

fn paint_step(line: &str) -> String {
    let trimmed = line.trim_start();
    let pad = &line[..line.len() - trimmed.len()];
    let kw = ["Given ", "When ", "Then ", "And ", "But "]
        .iter()
        .find(|k| trimmed.starts_with(*k))
        .copied()
        .unwrap_or("");
    if kw.is_empty() {
        return paint_quotes(line);
    }
    format!("{pad}{}{}", keyword(kw.trim_end()), paint_quotes(&trimmed[kw.len()..]))
}

fn gherkin_line(line: &str) -> String {
    let nl = if line.ends_with('\n') { "\n" } else { "" };
    let body = line.trim_end_matches('\n');
    let t = body.trim_start();
    let painted = if t.starts_with('#') {
        mute(body)
    } else if t.starts_with("@rid:") {
        mute(body)
    } else if t.starts_with("@holdout") {
        warn(body)
    } else if t.starts_with("@epic:") {
        accent(body)
    } else if t.starts_with('@') {
        mute(body)
    } else if starts_kw(t, "Feature:") || starts_kw(t, "Rule:") {
        bold(&keyword(body))
    } else if starts_kw(t, "Scenario:")
        || t.starts_with("Scenario Outline:")
        || starts_kw(t, "Background:")
        || starts_kw(t, "Examples:")
    {
        bold(&keyword(body))
    } else if t.starts_with("Given ")
        || t.starts_with("When ")
        || t.starts_with("Then ")
        || t.starts_with("And ")
        || t.starts_with("But ")
    {
        paint_step(body)
    } else if t.starts_with("As a") || t.starts_with("As an") || t.starts_with("I want") || t.starts_with("So that") {
        italic(body)
    } else {
        paint_quotes(body)
    };
    format!("{painted}{nl}")
}

/// Colour a live author/stepwright progress line.
pub fn progress(line: &str) -> String {
    if !on() {
        return line.to_string();
    }
    let t = line.trim_start();
    if t.starts_with('?') {
        return accent(line);
    }
    if t.starts_with("you:") || t.starts_with("  you:") {
        return italic(line);
    }
    if t.starts_with("[write_file]") {
        return ok(line);
    }
    if t.starts_with("[ask_human]") {
        return accent(line);
    }
    if t.starts_with("[done]") {
        return mute(line);
    }
    if t.starts_with("step ") || t.starts_with("contacting ") || t.starts_with("still waiting") || t.starts_with("tokens ") {
        return mute(line);
    }
    if t.contains("REJECTED") || t.starts_with("OVERFIT") {
        return bold(&bad(line));
    }
    line.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    static LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn gherkin_is_plain_when_colour_is_off() {
        let _g = LOCK.lock().unwrap();
        init("never");
        let src = "Feature: X\n  Scenario: A\n    Given \"1.00\"\n";
        assert_eq!(gherkin(src), src);
        assert!(!gherkin(src).contains('\u{1b}'));
    }

    #[test]
    fn gherkin_marks_keywords_when_forced() {
        let _g = LOCK.lock().unwrap();
        init("always");
        let out = gherkin("  Given a thing\n");
        assert!(out.contains('\u{1b}'), "{out:?}");
        assert!(out.contains("Given"), "{out:?}");
    }
}
