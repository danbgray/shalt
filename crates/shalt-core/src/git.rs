//! GitHub URLs become a local tree. Auth stays with git itself — we never print tokens.

use std::path::{Path, PathBuf};
use std::process::Command;

pub fn looks_like_git_source(raw: &str) -> bool {
    let s = raw.trim();
    if s.is_empty() || Path::new(s).is_dir() {
        return false;
    }
    s.starts_with("git@")
        || s.starts_with("ssh://")
        || s.starts_with("https://github.com/")
        || s.starts_with("http://github.com/")
        || s.starts_with("github.com/")
        || s.ends_with(".git")
}

pub fn github_clone_name(raw: &str) -> Option<String> {
    let s = raw
        .trim()
        .trim_end_matches('/')
        .trim_end_matches(".git");
    let s = s.strip_prefix("git@github.com:").unwrap_or(s);
    let s = s.strip_prefix("ssh://git@github.com/").unwrap_or(s);
    let s = s.strip_prefix("https://github.com/").unwrap_or(s);
    let s = s.strip_prefix("http://github.com/").unwrap_or(s);
    let s = s.strip_prefix("github.com/").unwrap_or(s);
    let name = s.rsplit('/').next().unwrap_or("").trim();
    if name.is_empty() || name.contains(':') {
        None
    } else {
        Some(name.to_string())
    }
}

pub fn normalize_github_url(raw: &str) -> String {
    let s = raw.trim();
    if s.starts_with("github.com/") {
        format!("https://{s}")
    } else {
        s.to_string()
    }
}

/// Clone `src` into `dest` (parent created). Returns the repo root.
pub fn clone_git_source(src: &str, dest: &Path) -> Result<PathBuf, String> {
    if dest.exists() {
        if dest.join(".git").exists() {
            return Ok(dest.to_path_buf());
        }
        return Err(format!("{} already exists", dest.display()));
    }
    if let Some(p) = dest.parent() {
        std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
    }
    let url = normalize_github_url(src);
    let out = Command::new("git")
        .args(["clone", "--", &url, &dest.display().to_string()])
        .output()
        .map_err(|e| format!("git clone: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let err = err.lines().find(|l| !l.is_empty()).unwrap_or("git clone failed");
        return Err(err.to_string());
    }
    Ok(dest.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_https_and_ssh_names() {
        assert_eq!(
            github_clone_name("https://github.com/rivletio/shalt.git").as_deref(),
            Some("shalt")
        );
        assert_eq!(
            github_clone_name("git@github.com:rivletio/secure-job-envelope").as_deref(),
            Some("secure-job-envelope")
        );
        assert_eq!(
            github_clone_name("github.com/acme/desk").as_deref(),
            Some("desk")
        );
        assert!(looks_like_git_source("https://github.com/acme/desk"));
        assert!(!looks_like_git_source("."));
    }
}
