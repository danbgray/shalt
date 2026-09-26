#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod install_cli;

use std::path::PathBuf;
use std::time::Duration;
use tauri::{WebviewUrl, WebviewWindowBuilder};

fn home_root() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

fn pick_port() -> Result<u16, String> {
    for p in 7700u16..=7799 {
        if shalt_core::uis::occupant(p).is_some() {
            continue;
        }
        if std::net::TcpListener::bind(("127.0.0.1", p)).is_ok() {
            return Ok(p);
        }
    }
    Err("no free port in 7700–7799".into())
}

async fn wait_url(timeout: Duration) -> Option<String> {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if let Some(u) = shalt_core::uis::current() {
            return Some(u.url);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    None
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            install_cli::install_bundled_cli();
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let url = if let Some(u) = shalt_core::uis::current() {
                    u.url
                } else {
                    let port = match pick_port() {
                        Ok(p) => p,
                        Err(e) => {
                            eprintln!("shalt: {e}");
                            return;
                        }
                    };
                    let root = home_root();
                    tauri::async_runtime::spawn(async move {
                        if let Err(e) = shalt::server::serve(root, port, false).await {
                            eprintln!("shalt desk: {e}");
                        }
                    });
                    match wait_url(Duration::from_secs(8)).await {
                        Some(u) => u,
                        None => {
                            eprintln!("shalt desk did not come up on {port}");
                            return;
                        }
                    }
                };
                let parsed = match url.parse() {
                    Ok(u) => u,
                    Err(e) => {
                        eprintln!("shalt desk url: {e}");
                        return;
                    }
                };
                let _ = WebviewWindowBuilder::new(&handle, "main", WebviewUrl::External(parsed))
                    .title("Shalt")
                    .inner_size(1320.0, 860.0)
                    .min_inner_size(880.0, 560.0)
                    .build();
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Shalt failed to start");
}
