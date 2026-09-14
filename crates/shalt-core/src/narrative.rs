use regex::Regex;
use std::sync::OnceLock;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Story {
    pub actor: String,
    pub capability: String,
    pub benefit: String,
    pub raw: String,
}

impl Story {
    pub fn complete(&self) -> bool {
        !self.actor.is_empty() && !self.capability.is_empty()
    }

    pub fn missing(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.actor.is_empty() {
            out.push("actor (\"As a ...\")");
        }
        if self.capability.is_empty() {
            out.push("capability (\"I want ...\")");
        }
        if self.benefit.is_empty() {
            out.push("benefit (\"So that ...\")");
        }
        out
    }

    pub fn one_line(&self) -> String {
        if !self.complete() {
            return self
                .raw
                .trim()
                .lines()
                .next()
                .unwrap_or("")
                .to_string();
        }
        let mut s = format!("As a {}, I want {}", self.actor, self.capability);
        if !self.benefit.is_empty() {
            s.push_str(&format!(", so that {}", self.benefit));
        }
        s
    }
}

fn actor_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?im)^\s*As\s+(?:an?|the)\s+(?P<v>.+?)\s*[,.]?\s*$").unwrap())
}
fn want_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?im)^\s*I\s+(?:want|need|would\s+like|can|do)\s+(?:to\s+)?(?P<v>.+?)\s*[,.]?\s*$")
            .unwrap()
    })
}
fn benefit_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?im)^\s*(?:So\s+that|In\s+order\s+to)\s+(?P<v>.+?)\s*[,.]?\s*$").unwrap()
    })
}

fn grab(re: &Regex, text: &str) -> String {
    re.captures(text)
        .and_then(|c| c.name("v"))
        .map(|m| m.as_str().trim().to_string())
        .unwrap_or_default()
}

pub fn parse_story(description: &str) -> Story {
    Story {
        actor: grab(actor_re(), description),
        capability: grab(want_re(), description),
        benefit: grab(benefit_re(), description),
        raw: description.to_string(),
    }
}

pub fn slug(value: &str, fallback: &str) -> String {
    let s: String = value
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() {
        fallback.to_string()
    } else {
        s
    }
}
