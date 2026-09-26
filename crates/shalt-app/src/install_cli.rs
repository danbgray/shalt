//! Copy the bundled `shalt` CLI onto PATH so a friend who installs the app
//! also gets the command-line tool.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const MARKER: &str = "# shalt (Shalt.app)";
const PATH_LINE: &str = r#"export PATH="$HOME/.local/bin:$PATH""#;

fn bundled_cli() -> Option<PathBuf> {
    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let direct = dir.join("shalt-cli");
    if direct.is_file() {
        return Some(direct);
    }
    let rd = fs::read_dir(&dir).ok()?;
    for e in rd.flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        if name == "Shalt" || name == "Shalt.exe" {
            continue;
        }
        if name == "shalt-cli" || name.starts_with("shalt-cli-") {
            let p = e.path();
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

fn copy_cli(src: &Path, dest: &Path) -> std::io::Result<()> {
    if let Some(dir) = dest.parent() {
        fs::create_dir_all(dir)?;
    }
    if let (Ok(s), Ok(d)) = (fs::metadata(src), fs::metadata(dest)) {
        if s.len() == d.len() {
            if let (Ok(sm), Ok(dm)) = (s.modified(), d.modified()) {
                if dm >= sm {
                    return Ok(());
                }
            }
        }
    }
    fs::copy(src, dest)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dest, fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

fn ensure_local_bin_on_path(home: &Path) {
    let zprofile = home.join(".zprofile");
    let zshrc = home.join(".zshrc");
    let already = [&zprofile, &zshrc].iter().any(|p| {
        fs::read_to_string(p)
            .map(|s| {
                s.contains(MARKER)
                    || s.contains("$HOME/.local/bin")
                    || s.contains("${HOME}/.local/bin")
                    || s.contains("~/.local/bin")
            })
            .unwrap_or(false)
    });
    if already {
        return;
    }
    let block = format!("\n{MARKER}\n{PATH_LINE}\n");
    if let Ok(mut f) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&zprofile)
    {
        let _ = f.write_all(block.as_bytes());
    }
}

/// Install the CLI next to common PATH prefixes. Best-effort; never fatal.
pub fn install_bundled_cli() {
    let Some(src) = bundled_cli() else {
        return;
    };
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let mut dests = vec![home.join(".local/bin/shalt")];
    let cargo_bin = home.join(".cargo/bin");
    if cargo_bin.is_dir() {
        dests.push(cargo_bin.join("shalt"));
    }
    for dir in ["/opt/homebrew/bin", "/usr/local/bin"] {
        let d = PathBuf::from(dir);
        if d.is_dir() {
            dests.push(d.join("shalt"));
        }
    }
    for dest in dests {
        let _ = copy_cli(&src, &dest);
    }
    ensure_local_bin_on_path(&home);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_export_is_idempotent() {
        let dir = tempfile::TempDir::new().unwrap();
        let home = dir.path();
        ensure_local_bin_on_path(home);
        ensure_local_bin_on_path(home);
        let raw = fs::read_to_string(home.join(".zprofile")).unwrap();
        assert_eq!(raw.matches(MARKER).count(), 1);
        assert!(raw.contains("$HOME/.local/bin"));
    }
}
