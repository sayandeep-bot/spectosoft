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
use super::MainAppState;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ActivityType {
    KeyboardInput,
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
}

/// Returns the base directory for activity logs with date-wise folder structure.
/// Path: C:\Users\Sayandeep Dey\AppData\Roaming\main-dashboard\activity_logs\YYYY-MM-DD
fn get_activity_logs_base_dir() -> PathBuf {
    // Base path in AppData Roaming
    let base_path = PathBuf::from(r"C:\Users\Sayandeep Dey\AppData\Roaming\main-dashboard\activity_logs");
    
    // Get current date in YYYY-MM-DD format
    let now = Utc::now();
    let date_folder = format!("{:04}-{:02}-{:02}", now.year(), now.month(), now.day());
    
    // Combine base path with date folder
    base_path.join(date_folder)
}

/// Writes a vector of activities to a new, uniquely timestamped JSON file.
pub fn write_activities_to_new_file(
    activity_entries: Vec<ActivityMeta>,
    meta_lock: &Arc<Mutex<()>>,
) -> std::io::Result<()> {
    if activity_entries.is_empty() {
        return Ok(());
    }

    let _guard = meta_lock.lock().unwrap();

    // Get the date-specific directory
    let base_dir = get_activity_logs_base_dir();
    println!("[DEBUG] Activity logs directory: {}", base_dir.display());

    // Create the directory structure (including parent directories)
    fs::create_dir_all(&base_dir)?;

    let timestamp = Utc::now().format("%Y-%m-%d_%H%M%S_%3f").to_string();
    let unique_id = Uuid::new_v4();
    let filename = format!("activity_{}_{}.json", timestamp, unique_id);
    let log_file_path = base_dir.join(&filename);

    println!("[IMPORTANT] Generating UNIQUE filename: {}", log_file_path.display());

    let log_data = LogData { activities: activity_entries };
    let json = serde_json::to_vec_pretty(&log_data)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

    let mut f = fs::File::create(&log_file_path)?;
    f.write_all(&json)?;
    f.sync_all()?;
    
    println!("[SUCCESS] Logged activities to {}", log_file_path.display());
    Ok(())
}


/// Keyboard monitoring thread
fn run_keyboard_monitor(state: ActivityLoggerState) {
    let is_running = state.is_activity_logging_running.clone();
    let keystroke_buffer = state.keystroke_buffer.clone();

    if let Err(error) = listen(move |event| {
        if !*is_running.lock().unwrap() {
            return;
        }
        
        if let EventType::KeyPress(key) = event.event_type {
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
        }
    }) {
        eprintln!("[ERROR] Keyboard monitor failed: {:?}", error);
    }
}


/// Main monitoring thread for window focus, browser activity, and periodic logging.
fn run_main_monitor(state: ActivityLoggerState) {
    let lock = state.meta_lock.clone();
    let is_running = state.is_activity_logging_running.clone();
    let keystroke_buffer = state.keystroke_buffer.clone();

    let mut last_window_title = String::new();

    println!("Starting main monitor...");
    loop {
        {
            if !*is_running.lock().unwrap() {
                println!("Stopping main monitor thread.");
                break;
            }
        }

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

        if !activities_to_log.is_empty() {
            if let Err(e) = write_activities_to_new_file(activities_to_log, &lock) {
                eprintln!("[ERROR] CRITICAL: Failed to write activity log file: {}", e);
            }
        }
    }
}

// --- Tauri Commands ---

#[tauri::command]
pub fn start_activity_logging_service(state: tauri::State<'_, MainAppState>) {
    let is_running = state.activity_logger_state.is_activity_logging_running.clone();
    let meta_lock = state.activity_logger_state.meta_lock.clone();
    let keystroke_buffer = state.activity_logger_state.keystroke_buffer.clone();

    {
        let mut running_flag = is_running.lock().unwrap();
        if *running_flag {
            println!("⚠️ Activity logging service already running.");
            return;
        }
        *running_flag = true;
    }

    println!("Starting activity logging service...");

    let keyboard_state = ActivityLoggerState {
        is_activity_logging_running: is_running.clone(),
        meta_lock: meta_lock.clone(),
        keystroke_buffer: keystroke_buffer.clone(),
    };
    thread::spawn(move || run_keyboard_monitor(keyboard_state));
    
    let main_monitor_state = ActivityLoggerState {
        is_activity_logging_running: is_running.clone(),
        meta_lock: meta_lock.clone(),
        keystroke_buffer: keystroke_buffer.clone(),
    };
    thread::spawn(move || run_main_monitor(main_monitor_state));

    println!("Activity logging services started successfully.");
}

#[tauri::command]
pub fn stop_activity_logging_service(state: tauri::State<'_, MainAppState>) {
    let mut is_running = state.activity_logger_state.is_activity_logging_running.lock().unwrap();
    *is_running = false;
    println!("🛑 Activity logging service manually stopped. It may take a moment for threads to terminate.");
}