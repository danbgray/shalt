//! Bigger-model audit of tests and code written by the fast inner loop.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subject {
    Tests,
    Code,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub pass: bool,
    pub findings: String,
}

pub fn role_for(subject: Subject) -> &'static str {
    match subject {
        Subject::Tests => "auditor",
        Subject::Code => "code_auditor",
    }
}

pub fn prompt(subject: Subject, journey_or_ticket: &str, extra: &str) -> String {
    let focus = if journey_or_ticket.is_empty() {
        String::new()
    } else {
        format!("Focus: {journey_or_ticket}.\n")
    };
    let extra = extra.trim();
    let extra = if extra.is_empty() {
        String::new()
    } else {
        format!("\n\n{extra}\n")
    };
    match subject {
        Subject::Tests => format!(
            "{focus}A smaller model just wrote the tests. You are the AUDITOR, not the author.\n\
             Read the spec and the step files. Do not write files. Do not peek at src/ — it is not there.\n\
             PASS only if each focused scenario has a real Given/When/Then that would fail if the behaviour were missing.\n\
             FAIL if bodies are empty, pending, `assert true`, stubs, or they hard-code the answer instead of calling the contract.\n\
             Call done() with first line PASS or FAIL, then at most 8 short findings.{extra}"
        ),
        Subject::Code => format!(
            "{focus}A smaller model just wrote src/ to make the tests pass. You are the AUDITOR, not the coder.\n\
             Read spec, contract, tests, and src. Do not write files.\n\
             PASS only if the code implements the named behaviour.\n\
             FAIL if it hard-codes the expected output, special-cases this ticket, or leaves `not implemented`.\n\
             Call done() with first line PASS or FAIL, then at most 8 short findings.{extra}"
        ),
    }
}

/// Fail closed: anything other than an explicit PASS is a fail.
pub fn parse_verdict(transcript: &str) -> Verdict {
    let text = transcript.trim();
    if text.is_empty() {
        return Verdict {
            pass: false,
            findings: "auditor returned nothing".into(),
        };
    }
    let mut verdict_line = "";
    for raw in text.lines().rev() {
        let l = raw.trim();
        let body = l
            .strip_prefix("[done]")
            .map(|s| s.trim())
            .unwrap_or(l);
        let u = body.to_ascii_uppercase();
        if u.starts_with("PASS") || u.starts_with("FAIL") {
            verdict_line = body;
            break;
        }
    }
    if verdict_line.is_empty() {
        let u = text.to_ascii_uppercase();
        if u.contains("PASS") && !u.contains("FAIL") {
            return Verdict {
                pass: true,
                findings: String::new(),
            };
        }
        return Verdict {
            pass: false,
            findings: first_findings(text),
        };
    }
    let u = verdict_line.to_ascii_uppercase();
    if u.starts_with("PASS") {
        Verdict {
            pass: true,
            findings: String::new(),
        }
    } else {
        Verdict {
            pass: false,
            findings: first_findings(text),
        }
    }
}

fn first_findings(text: &str) -> String {
    let mut lines: Vec<&str> = text
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with("thinking ·"))
        .collect();
    if lines.len() > 12 {
        lines.truncate(12);
    }
    let s = lines.join("\n");
    s.chars().take(2000).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pass_from_done() {
        let v = parse_verdict("[read_file] spec/a.feature\n[done] PASS — binds the journey");
        assert!(v.pass);
        assert!(v.findings.is_empty());
    }

    #[test]
    fn fail_from_done() {
        let v = parse_verdict("[done] FAIL\n- empty Then\n- pending Given");
        assert!(!v.pass);
        assert!(v.findings.contains("empty Then"));
    }

    #[test]
    fn empty_is_fail() {
        let v = parse_verdict("  ");
        assert!(!v.pass);
    }

    #[test]
    fn roles_split_tests_and_code() {
        assert_eq!(role_for(Subject::Tests), "auditor");
        assert_eq!(role_for(Subject::Code), "code_auditor");
        assert!(prompt(Subject::Tests, "patrons", "").contains("Do not peek at src"));
        assert!(prompt(Subject::Code, "S-1", "").contains("hard-codes"));
    }
}
