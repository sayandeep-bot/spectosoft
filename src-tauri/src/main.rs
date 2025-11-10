// src/main.rs

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod activity_service;
mod screenshot_service;
mod video_service;
// MODIFIED: Add `retry_all_pending_activities` to the import list
use activity_service::{
    retry_all_pending_activities, start_activity_logging_service, stop_activity_logging_service,
    ActivityLoggerState,
};
use reqwest::blocking::Client;
use screenshot_service::{start_screenshot_service, stop_screenshot_service};
use video_service::{
    retry_all_pending_videos, start_video_recording_service, stop_video_recording_service,
    VideoServiceState,
};

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};
use tauri::Manager;

// Define a combined AppState for managing all services
pub struct MainAppState {
    // Screenshot service state
    pub screenshot_is_running: Arc<Mutex<bool>>,
    // Activity logger service state
    pub activity_logger_state: ActivityLoggerState,
    pub video_service_state: VideoServiceState,
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_fs::init())
        .manage(MainAppState {
            // Manage the combined state
            screenshot_is_running: Arc::new(Mutex::new(false)),
            activity_logger_state: ActivityLoggerState {
                is_activity_logging_running: Arc::new(Mutex::new(false)),
                meta_lock: Arc::new(Mutex::new(())),
                keystroke_buffer: Arc::new(Mutex::new(HashMap::new())),
                mouse_click_count: Arc::new(Mutex::new(0)),
                mouse_scroll_count: Arc::new(Mutex::new(0)),
            },
            video_service_state: VideoServiceState {
                is_running: Arc::new(Mutex::new(false)),
            },
        })
        .setup(|app| {
            // --- Existing Screenshot Pending Check (No changes needed) ---
            let screenshot_pending_dir = {
                let tmp = app
                    .path()
                    .app_data_dir()
                    .unwrap_or_default()
                    .join("screenshots_pending");
                tmp
            };

            if screenshot_pending_dir.exists() {
                let client = Client::new();
                let dir_clone = screenshot_pending_dir.clone();
                let client_clone = client.clone();
                thread::spawn(move || {
                    println!("🔄 Startup: Checking for pending screenshots...");
                    // Give the app a moment to start up before hitting the network
                    thread::sleep(Duration::from_secs(2));
                    screenshot_service::retry_all_pending(&client_clone, &dir_clone);
                });
            }

            // VVVV --- NEW: Activity Log Pending Check --- VVVV
            let activity_pending_dir = {
                // IMPORTANT: This path must exactly match the one in `activity_service.rs`
                let tmp = app
                    .path()
                    .app_data_dir()
                    .unwrap_or_default()
                    .join("activity_logs_pending");
                tmp
            };

            if activity_pending_dir.exists() {
                let client = Client::new();
                let dir_clone = activity_pending_dir.clone();
                let client_clone = client.clone();
                thread::spawn(move || {
                    println!("🔄 Startup: Checking for pending activity logs...");
                    // Stagger the startup checks slightly to avoid network congestion
                    thread::sleep(Duration::from_secs(4));
                    activity_service::retry_all_pending_activities(&client_clone, &dir_clone);
                });
            }
            // ^^^^ --- NEW: Activity Log Pending Check --- ^^^^
            let video_pending_dir = app
                .path()
                .app_data_dir()
                .unwrap()
                .join("video_recordings_pending");
            if video_pending_dir.exists() {
                let client = Client::new();
                let dir_clone = video_pending_dir.clone();
                thread::spawn(move || {
                    println!("🔄 Startup: Checking for pending videos...");
                    thread::sleep(Duration::from_secs(6)); // Stagger the check
                    retry_all_pending_videos(&client, &dir_clone);
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            start_screenshot_service,
            stop_screenshot_service,
            start_activity_logging_service,
            stop_activity_logging_service,
            start_video_recording_service,
            stop_video_recording_service,
        ])
        .run(tauri::generate_context!())
        .expect("❌ Error while running Tauri app");
}
