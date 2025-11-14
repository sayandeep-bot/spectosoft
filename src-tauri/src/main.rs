// src-tauri/src/main.rs

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// --- Module declarations for your services ---
mod activity_service;
mod screenshot_service;

// --- Imports from other services ---
use activity_service::{
    retry_all_pending_activities, start_activity_logging_service, stop_activity_logging_service,
    ActivityLoggerState,
};
use screenshot_service::{start_screenshot_service, stop_screenshot_service};

// --- Standard, Tauri, and external crate imports ---
use chrono::{Datelike, Utc};
use reqwest::blocking::Client;
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};
use tauri::{command, AppHandle, Manager, State};

// Use the correct library name for your video recorder
use main_dashboard_spinup_lib::video_main::{AudioSource, Container, Recorder, RecorderConfig};

// --- State Management Structs ---
pub struct VideoState {
    pub is_running: Arc<Mutex<bool>>,
    pub stop_handle: Arc<Mutex<Option<Arc<AtomicBool>>>>,
}

pub struct MainAppState {
    pub screenshot_is_running: Arc<Mutex<bool>>,
    pub activity_logger_state: ActivityLoggerState,
    pub video_state: VideoState,
}

// --- Helper Functions ---
fn container_from_str(s: &str) -> Container {
    match s {
        "Avi" => Container::Avi,
        "Webm" => Container::Webm,
        "Mp4" => Container::Mp4,
        _ => Container::Mp4,
    }
}

fn audio_source_from_str(s: &str) -> AudioSource {
    match s {
        "Microphone" => AudioSource::Microphone,
        "System" => AudioSource::System,
        "Both" => AudioSource::Both,
        _ => AudioSource::Both,
    }
}

fn get_dated_folder(base_dir: &PathBuf) -> PathBuf {
    let now = Utc::now();
    base_dir.join(format!(
        "{:04}-{:02}-{:02}",
        now.year(),
        now.month(),
        now.day()
    ))
}

// --- Tauri Commands ---
#[command]
fn start_video_recording(
    app: AppHandle,
    state: State<'_, MainAppState>,
    fps: u32,
    container: String,
    segment_duration: u64,
    audio: bool,
    audio_source: String,
) -> Result<(), String> {
    let video_state = &state.video_state;

    // VVVV --- FIX 1: Adopt the scoped lock pattern from screenshot_service.rs --- VVVV
    {
        let mut is_running = video_state.is_running.lock().unwrap();
        if *is_running {
            println!("⚠️ Video recording is already running.");
            // We return Ok because it's not a failure, the state is just already active.
            // The frontend can handle this gracefully.
            return Ok(());
        }
        *is_running = true;
    } // The lock on `is_running` is released here.
      // ^^^^ --- FIX 1 --- ^^^^

    println!("▶️ Starting video recording...");

    let base_pending_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("video_recordings_pending");

    let output_dir = get_dated_folder(&base_pending_dir);

    let recorder_cfg = RecorderConfig {
        output_dir,
        base_name: "recording".to_string(),
        segment_duration: Duration::from_secs(segment_duration),
        fps: 15,
        container: container_from_str(&container),
        display_index: 0,
        record_all: false,
        combine_all: false,
        flip_vertical: true,
        flip_horizontal: false,
        video_bitrate_kbps: 4000,
        scale_max_width: None,
        include_audio: audio,
        audio_bitrate_kbps: 128,
        audio_source: audio_source_from_str(&audio_source),
    };

    let recorder = Recorder::new(recorder_cfg);
    let stop_flag = recorder.stop_flag();
    *video_state.stop_handle.lock().unwrap() = Some(stop_flag);

    // This thread will run the recorder. We clone the state we need.
    let is_running_clone = video_state.is_running.clone();
    let stop_handle_clone = video_state.stop_handle.clone();
    thread::spawn(move || {
        println!("📹 Recorder thread started.");
        if let Err(e) = recorder.run_blocking() {
            eprintln!("❌ Recorder thread exited with error: {}", e);
        } else {
            println!("📹 Recorder thread finished gracefully.");
        }

        // When the thread finishes (either by stop signal or error),
        // ensure the state is cleaned up.
        *is_running_clone.lock().unwrap() = false;
        *stop_handle_clone.lock().unwrap() = None;
    });

    println!("✅ Video recording started successfully.");
    Ok(())
}

#[command]
fn stop_video_recording(state: State<'_, MainAppState>) -> Result<(), String> {
    let video_state = &state.video_state;

    // VVVV --- FIX 2: Adopt the scoped lock pattern for stopping as well --- VVVV
    // Check if it's running first, without holding the lock for too long.
    if !*video_state.is_running.lock().unwrap() {
        println!("⚠️ Video recording is not running, nothing to stop.");
        return Ok(());
    }

    println!("⏹️ Stopping video recording...");

    // Now get the stop handle and send the signal.
    if let Some(stop_handle) = video_state.stop_handle.lock().unwrap().take() {
        stop_handle.store(true, Ordering::Relaxed);
        // The `is_running` flag will be set to false by the thread itself when it exits.
    } else {
        // This is a fallback in case the handle is missing but state is running
        *video_state.is_running.lock().unwrap() = false;
        return Err("Error: Could not find stop handle, but forced state to 'stopped'.".into());
    }
    // ^^^^ --- FIX 2 --- ^^^^

    println!("✅ Stop signal sent to video recorder.");
    Ok(())
}
// --- Pending File Upload Logic ---
fn try_upload_video_file(
    client: &Client,
    filepath: &PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    let url = "http://192.168.1.26:3000/api/v1/upload";
    let filename = filepath
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown.mp4");
    let mime_type = match filepath.extension().and_then(|s| s.to_str()) {
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        Some("avi") => "video/x-msvideo",
        _ => "application/octet-stream",
    };

    let file_data = fs::read(filepath)?;
    let part = reqwest::blocking::multipart::Part::bytes(file_data)
        .file_name(filename.to_string())
        .mime_str(mime_type)?;

    let form = reqwest::blocking::multipart::Form::new().part("file", part);

    let response = client
        .post(url)
        .multipart(form)
        .timeout(Duration::from_secs(60))
        .send()?;
    if response.status().is_success() {
        println!("✅ Video upload success: {}", filename);
        Ok(())
    } else {
        Err(format!("Video upload failed: {} - {}", filename, response.status()).into())
    }
}

pub fn retry_all_pending_videos(client: &Client, pending_dir: &PathBuf) {
    println!("🔄 Checking for pending videos in {:?}...", pending_dir);
    let date_dirs = match fs::read_dir(pending_dir) {
        Ok(dirs) => dirs,
        Err(_) => return,
    };

    for date_dir_entry in date_dirs.flatten() {
        let dir_path = date_dir_entry.path();
        if !dir_path.is_dir() {
            continue;
        }

        let video_files = match fs::read_dir(&dir_path) {
            Ok(files) => files
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file())
                .collect::<Vec<PathBuf>>(),
            Err(_) => continue,
        };

        if video_files.is_empty() {
            let _ = fs::remove_dir(dir_path);
            continue;
        }

        println!(
            "📂 Found {} pending videos in {}",
            video_files.len(),
            dir_path.display()
        );
        for file in video_files {
            match try_upload_video_file(client, &file) {
                Ok(_) => {
                    if let Err(e) = fs::remove_file(&file) {
                        eprintln!("⚠️ Failed to delete video {}: {}", file.display(), e);
                    }
                }
                Err(e) => eprintln!("❌ Retry failed for video {}: {}", file.display(), e),
            }
        }
    }
}

// --- Main Application Setup ---
fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_fs::init())
        .manage(MainAppState {
            screenshot_is_running: Arc::new(Mutex::new(false)),
            activity_logger_state: ActivityLoggerState {
                is_activity_logging_running: Arc::new(Mutex::new(false)),
                meta_lock: Arc::new(Mutex::new(())),
                keystroke_buffer: Arc::new(Mutex::new(HashMap::new())),
                mouse_click_count: Arc::new(Mutex::new(0)),
                mouse_scroll_count: Arc::new(Mutex::new(0)),
            },
            video_state: VideoState {
                is_running: Arc::new(Mutex::new(false)),
                stop_handle: Arc::new(Mutex::new(None)),
            },
        })
        .setup(|app| {
            let app_data_dir = app.path().app_data_dir().unwrap();
            let client = Client::new();

            let screenshot_pending_dir = app_data_dir.join("screenshots_pending");
            if screenshot_pending_dir.exists() {
                let s_client = client.clone();
                thread::spawn(move || {
                    thread::sleep(Duration::from_secs(2));
                    screenshot_service::retry_all_pending(&s_client, &screenshot_pending_dir);
                });
            }

            let activity_pending_dir = app_data_dir.join("activity_logs_pending");
            if activity_pending_dir.exists() {
                let a_client = client.clone();
                thread::spawn(move || {
                    thread::sleep(Duration::from_secs(4));
                    retry_all_pending_activities(&a_client, &activity_pending_dir);
                });
            }

            let video_pending_dir = app_data_dir.join("video_recordings_pending");
            if video_pending_dir.exists() {
                let v_client = client.clone();
                thread::spawn(move || {
                    thread::sleep(Duration::from_secs(6));
                    retry_all_pending_videos(&v_client, &video_pending_dir);
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            start_screenshot_service,
            stop_screenshot_service,
            start_activity_logging_service,
            stop_activity_logging_service,
            start_video_recording,
            stop_video_recording,
        ])
        .run(tauri::generate_context!())
        .expect("❌ Error while running Tauri app");
}
