// src/main.rs

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod screenshot_service;
mod activity_service; 

use screenshot_service::{start_screenshot_service, stop_screenshot_service};
use activity_service::{start_activity_logging_service, stop_activity_logging_service, ActivityLoggerState}; 
use reqwest::blocking::Client;
// You need to add HashMap for the keystroke_buffer initialization
use std::{collections::HashMap, sync::{Arc, Mutex}, thread, time::Duration};
use tauri::Manager;

// Define a combined AppState for managing all services
pub struct MainAppState {
    // Screenshot service state
    pub screenshot_is_running: Arc<Mutex<bool>>,
    // Activity logger service state
    pub activity_logger_state: ActivityLoggerState,
}

fn main() {

    tauri::Builder::default()
        .plugin(tauri_plugin_fs::init())
        .manage(MainAppState { // Manage the combined state
            screenshot_is_running: Arc::new(Mutex::new(false)),
            // This is the block that needs to be fixed
            activity_logger_state: ActivityLoggerState {
                is_activity_logging_running: Arc::new(Mutex::new(false)),
                meta_lock: Arc::new(Mutex::new(())),
                // FIX: Add the missing keystroke_buffer field here
                keystroke_buffer: Arc::new(Mutex::new(HashMap::new())),
            },
        })
        .setup(|app| {
            let app_handle = app.handle().clone();
            let pending_dir = {
                let tmp = app.path().app_data_dir().unwrap_or_default().join("screenshots_pending");
                tmp
            };

            if pending_dir.exists() {
                let client = Client::new();
                let dir_clone = pending_dir.clone();
                let client_clone = client.clone();
                thread::spawn(move || {
                    println!("🔄 Startup: Checking for pending screenshots...");
                    thread::sleep(Duration::from_secs(2));
                    screenshot_service::retry_all_pending(&client_clone, &dir_clone);
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            start_screenshot_service,
            stop_screenshot_service,
            start_activity_logging_service,
            stop_activity_logging_service,
        ])
        .run(tauri::generate_context!())
        .expect("❌ Error while running Tauri app");
}