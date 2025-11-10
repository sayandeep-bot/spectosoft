use chrono::{Datelike, Utc};
use fs_extra::dir;
use image::RgbaImage;
use reqwest::blocking::Client;
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};
use tauri::Manager; // Ensure Manager is imported for app.path()
use uuid::Uuid;
use xcap::Monitor;

// This line is crucial: it brings MainAppState from main.rs into scope
use super::MainAppState;

// We no longer need this AppState struct here, as MainAppState is managing it.
// #[derive(Clone)]
// pub struct AppState {
//     pub is_running: Arc<Mutex<bool>>,
// }

#[tauri::command]
pub fn start_screenshot_service(app: tauri::AppHandle, state: tauri::State<MainAppState>) {
    // FIX: Access `screenshot_is_running` from MainAppState
    let is_running = state.screenshot_is_running.clone();
    {
        let mut running = is_running.lock().unwrap();
        if *running {
            println!("⚠️ Screenshot service already running");
            return;
        }
        *running = true;
    }

    thread::spawn(move || {
        let pending_dir = get_pending_dir(&app);
        if !pending_dir.exists() {
            if let Err(e) = dir::create_all(&pending_dir, false) {
                eprintln!("❌ Failed to create pending dir: {}", e);
                return;
            }
        }

        let client = Client::new();

        // Retry thread (every 5 min)
        {
            let retry_client = client.clone();
            let retry_dir = pending_dir.clone();
            // FIX: Access `screenshot_is_running` from MainAppState
            let retry_is_running = is_running.clone(); // `is_running` already holds the Arc to screenshot_is_running
            thread::spawn(move || loop {
                {
                    let running = retry_is_running.lock().unwrap();
                    if !*running {
                        println!("🛑 Retry thread stopped");
                        break;
                    }
                }
                println!("\n🔁 ===== RETRY CYCLE STARTED =====");
                retry_all_pending(&retry_client, &retry_dir);
                println!("===== RETRY CYCLE ENDED =====\n");
                thread::sleep(Duration::from_secs(300));
            });
        }

        // Screenshot loop (every 10 sec)
        loop {
            {
                // FIX: `is_running` already holds the Arc to screenshot_is_running
                let running = is_running.lock().unwrap();
                if !*running {
                    println!("🛑 Screenshot service stopped");
                    break;
                }
            }

            if let Err(e) = take_save_and_try_upload(&client, &pending_dir) {
                eprintln!("⚠️ Screenshot error: {}", e);
            }

            thread::sleep(Duration::from_secs(10));
        }
    });
}

#[tauri::command]
pub fn stop_screenshot_service(state: tauri::State<MainAppState>) {
    // FIX: Access `screenshot_is_running` from MainAppState
    let mut running = state.screenshot_is_running.lock().unwrap();
    *running = false;
    println!("🛑 Screenshot service manually stopped");
}

fn get_pending_dir(app: &tauri::AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| PathBuf::from("./data"))
        .join("screenshots_pending")
}

fn get_today_folder(base_dir: &PathBuf) -> PathBuf {
    let now = Utc::now();
    base_dir.join(format!(
        "{:04}-{:02}-{:02}",
        now.year(),
        now.month(),
        now.day()
    ))
}

fn take_save_and_try_upload(
    client: &Client,
    base_dir: &PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    let monitors = Monitor::all()?;
    let monitor = monitors.first().ok_or("No monitor found")?;
    let rgba_image: RgbaImage = monitor.capture_image()?;

    let timestamp = Utc::now().format("%Y%m%d_%H%M%S_%3f").to_string();
    let filename = format!("screenshot_{}_{}.png", timestamp, Uuid::new_v4());

    let today_dir = get_today_folder(base_dir);
    dir::create_all(&today_dir, false)?;
    let filepath = today_dir.join(&filename);

    rgba_image.save(&filepath)?;
    println!("📸 Screenshot saved: {}", filepath.display());

    match try_upload_file(client, &filepath) {
        Ok(_) => {
            println!("✅ Uploaded immediately: {}", filename);
            if let Err(e) = fs::remove_file(&filepath) {
                eprintln!("⚠️ Failed to delete {}: {}", filepath.display(), e);
            } else {
                println!("🗑️ Deleted after successful upload: {}", filename);
            }
        }
        Err(e) => println!("💾 Upload failed, kept on disk: {} - {}", filename, e),
    }

    Ok(())
}

fn try_upload_file(client: &Client, filepath: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let url = "http://192.168.1.26:3000/api/v1/upload";
    let filename = filepath
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown.png");

    let file_data = fs::read(filepath)?;
    let form = reqwest::blocking::multipart::Form::new().part(
        "file",
        reqwest::blocking::multipart::Part::bytes(file_data)
            .file_name(filename.to_string())
            .mime_str("image/png")?,
    );

    let response = client
        .post(url)
        .multipart(form)
        .timeout(Duration::from_secs(10))
        .send()?;
    let status = response.status();
    let text = response.text().unwrap_or_default();

    if status.is_success() {
        println!("✅ Upload success: {} ({})", filename, status);
        Ok(())
    } else {
        Err(format!("Upload failed: {} ({}) - {}", filename, status, text).into())
    }
}

pub fn retry_all_pending(client: &Client, base_dir: &PathBuf) {
    let date_dirs = match fs::read_dir(base_dir) {
        Ok(dirs) => dirs,
        Err(e) => {
            eprintln!("⚠️ Failed to read pending dir: {}", e);
            return;
        }
    };

    let mut total_found = 0;
    let mut total_uploaded = 0;
    let mut total_failed = 0;

    for date_dir_entry in date_dirs.flatten() {
        let dir_path = date_dir_entry.path();
        if !dir_path.is_dir() {
            continue;
        }

        let files = match fs::read_dir(&dir_path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("⚠️ Failed to read folder {}: {}", dir_path.display(), e);
                continue;
            }
        };

        let mut png_files: Vec<PathBuf> = files
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("png"))
            .collect();

        if png_files.is_empty() {
            continue;
        }

        png_files.sort();
        total_found += png_files.len();
        println!(
            "📂 Found {} pending in {}",
            png_files.len(),
            dir_path.display()
        );

        for file in png_files {
            let filename = file
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown.png");
            match try_upload_file(client, &file) {
                Ok(_) => {
                    println!("✅ Retry upload success: {}", filename);
                    total_uploaded += 1;
                    if let Err(e) = fs::remove_file(&file) {
                        eprintln!("⚠️ Failed to delete {}: {}", filename, e);
                    } else {
                        println!("🗑️ Deleted after successful retry: {}", filename);
                    }
                }
                Err(e) => {
                    println!("❌ Retry failed: {} - {}", filename, e);
                    total_failed += 1;
                }
            }
        }
    }

    println!(
        "📊 Retry summary: Found={}, Uploaded={}, Failed={}, Remaining={}",
        total_found,
        total_uploaded,
        total_failed,
        total_found - total_uploaded
    );
}
