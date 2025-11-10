use super::MainAppState;
use chrono::{Datelike, Utc};
use oneshot;
use reqwest::blocking::Client;
use std::{
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager};
use uuid::Uuid;
use windows::core::{Result, HSTRING};
use windows::Win32::Media::Audio::{
    eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator, AUDCLNT_SHAREMODE_SHARED,
    AUDCLNT_STREAMFLAGS_LOOPBACK,
};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::{
    Foundation::HWND,
    Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC,
        GetDIBits, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
        HGDIOBJ, ROP_CODE,
    },
    UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN},
};

#[derive(Clone, Debug)]
struct AudioFormat {
    sample_rate: u32,
    channels: u32,
    bits_per_sample: u32,
}

#[derive(Clone)]
pub struct VideoServiceState {
    pub is_running: std::sync::Arc<std::sync::Mutex<bool>>,
}

struct Recorder {
    path: PathBuf,
    duration_secs: u64,
}

// Thread 2: Audio Producer
fn run_audio_capture(
    stop_signal: Arc<AtomicBool>,
    audio_sender: mpsc::Sender<(Vec<f32>, i64)>, // Sends (data, timestamp)
    format_sender: oneshot::Sender<AudioFormat>,
) -> Result<()> {
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?;
        let audio_client: windows::Win32::Media::Audio::IAudioClient =
            device.Activate(CLSCTX_ALL, None)?;
        let wave_format_ptr = audio_client.GetMixFormat()?;
        let wave_format = *wave_format_ptr;
        let audio_format = AudioFormat {
            sample_rate: wave_format.nSamplesPerSec,
            channels: wave_format.nChannels as u32,
            bits_per_sample: wave_format.wBitsPerSample as u32,
        };
        format_sender.send(audio_format.clone()).unwrap();
        audio_client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_LOOPBACK,
            100_000_000, // 10-second buffer
            0,
            wave_format_ptr,
            None,
        )?;
        CoTaskMemFree(Some(wave_format_ptr as *const _));
        let capture_client: windows::Win32::Media::Audio::IAudioCaptureClient =
            audio_client.GetService()?;
        audio_client.Start()?;

        let mut audio_timestamp = 0i64;

        while !stop_signal.load(Ordering::SeqCst) {
            let packet_size = capture_client.GetNextPacketSize()?;
            if packet_size == 0 {
                thread::sleep(Duration::from_millis(1));
                continue;
            }
            let mut data_ptr = std::ptr::null_mut();
            let mut num_frames_available = 0;
            let mut flags = 0;
            capture_client.GetBuffer(
                &mut data_ptr,
                &mut num_frames_available,
                &mut flags,
                None,
                None,
            )?;

            if num_frames_available > 0 {
                let num_samples = num_frames_available as usize * audio_format.channels as usize;
                let samples_slice = std::slice::from_raw_parts(data_ptr as *const f32, num_samples);
                let duration =
                    (10_000_000 * num_frames_available as i64) / audio_format.sample_rate as i64;

                if audio_sender
                    .send((samples_slice.to_vec(), audio_timestamp))
                    .is_err()
                {
                    break;
                }

                audio_timestamp += duration;
                capture_client.ReleaseBuffer(num_frames_available)?;
            }
        }
        audio_client.Stop()?;
        CoUninitialize();
    }
    Ok(())
}

// Thread 1: Video Producer
fn run_video_capture(
    stop_signal: Arc<AtomicBool>,
    video_sender: mpsc::Sender<(Vec<u8>, i64)>, // Sends (data, timestamp)
    width: u32,
    height: u32,
    frame_rate: u32,
    total_frames: u32,
) {
    let frame_duration = Duration::from_nanos(1_000_000_000 / frame_rate as u64);
    let mut video_timestamp = 0i64;
    let frame_duration_100ns = 10_000_000i64 / frame_rate as i64;
    let mut next_frame_time = Instant::now();

    for _ in 0..total_frames {
        if stop_signal.load(Ordering::SeqCst) {
            break;
        }

        let frame_data = capture_screen(width, height);
        if video_sender.send((frame_data, video_timestamp)).is_err() {
            break;
        }
        video_timestamp += frame_duration_100ns;

        next_frame_time += frame_duration;
        let now = Instant::now();
        if next_frame_time > now {
            thread::sleep(next_frame_time - now);
        }
    }
}

impl Recorder {
    fn new(path: PathBuf) -> std::result::Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Self {
            path,
            duration_secs: 30,
        })
    }

    // Main Thread: The Consumer/Encoder
    fn record(&mut self) -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let width = unsafe { GetSystemMetrics(SM_CXSCREEN) as u32 };
        let height = unsafe { GetSystemMetrics(SM_CYSCREEN) as u32 };
        let frame_rate = 30u32;
        let total_frames_to_capture = (self.duration_secs * frame_rate as u64) as u32;

        unsafe { MFStartup(MF_VERSION, 0)? };
        let sink_writer =
            unsafe { MFCreateSinkWriterFromURL(&HSTRING::from(self.path.to_str().unwrap()), None, None)? };

        let stop_signal = Arc::new(AtomicBool::new(false));

        let (audio_sender, audio_receiver) = mpsc::channel();
        let (format_sender, format_receiver) = oneshot::channel();
        let audio_stop_signal = stop_signal.clone();
        thread::spawn(move || {
            if let Err(e) = run_audio_capture(audio_stop_signal, audio_sender, format_sender) {
                eprintln!("[AUDIO_THREAD] Capture thread failed: {}", e);
            }
        });
        let audio_format = format_receiver.recv()?;

        let (video_sender, video_receiver) = mpsc::channel();
        let video_stop_signal = stop_signal.clone();
        thread::spawn(move || {
            run_video_capture(
                video_stop_signal,
                video_sender,
                width,
                height,
                frame_rate,
                total_frames_to_capture,
            );
        });

        let (video_stream_index, audio_stream_index) = unsafe {
            let out_video_mt = MFCreateMediaType()?;
            out_video_mt.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            out_video_mt.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
            out_video_mt.SetUINT32(&MF_MT_AVG_BITRATE, 2_500_000)?;
            out_video_mt.SetUINT64(&MF_MT_FRAME_SIZE, ((width as u64) << 32) | height as u64)?;
            out_video_mt.SetUINT64(&MF_MT_FRAME_RATE, ((frame_rate as u64) << 32) | 1)?;
            out_video_mt.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
            out_video_mt.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1u64 << 32) | 1)?;
            let video_stream_index = sink_writer.AddStream(&out_video_mt)?;

            let in_video_mt = MFCreateMediaType()?;
            in_video_mt.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            in_video_mt.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
            in_video_mt.SetUINT64(&MF_MT_FRAME_SIZE, ((width as u64) << 32) | height as u64)?;
            in_video_mt.SetUINT64(&MF_MT_FRAME_RATE, ((frame_rate as u64) << 32) | 1)?;
            in_video_mt.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
            in_video_mt.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1u64 << 32) | 1)?;
            sink_writer.SetInputMediaType(video_stream_index, &in_video_mt, None)?;

            let out_audio_mt = MFCreateMediaType()?;
            out_audio_mt.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
            out_audio_mt.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC)?;
            out_audio_mt.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, audio_format.sample_rate)?;
            out_audio_mt.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, audio_format.channels)?;
            let audio_stream_index = sink_writer.AddStream(&out_audio_mt)?;

            let in_audio_mt = MFCreateMediaType()?;
            in_audio_mt.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
            in_audio_mt.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_Float)?;
            in_audio_mt.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, audio_format.sample_rate)?;
            in_audio_mt.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, audio_format.channels)?;
            in_audio_mt.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, audio_format.bits_per_sample)?;
            let block_alignment = audio_format.channels * (audio_format.bits_per_sample / 8);
            in_audio_mt.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, block_alignment)?;
            let bytes_per_second = audio_format.sample_rate * block_alignment;
            in_audio_mt.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, bytes_per_second)?;
            sink_writer.SetInputMediaType(audio_stream_index, &in_audio_mt, None)?;
            (video_stream_index, audio_stream_index)
        };

        unsafe { sink_writer.BeginWriting()? };
        
        for _ in 0..total_frames_to_capture {
            let (video_data, video_ts) = video_receiver.recv()?;

            // Drain and write any audio that arrived before this video frame's timestamp
            loop {
                match audio_receiver.try_recv() {
                    Ok((audio_data, audio_ts)) => {
                        if audio_ts < video_ts {
                            unsafe {
                                let audio_bytes: Vec<u8> = audio_data.iter().flat_map(|&f| f.to_le_bytes()).collect();
                                let num_sample_frames = audio_data.len() as i64 / audio_format.channels as i64;
                                let duration = (10_000_000 * num_sample_frames) / audio_format.sample_rate as i64;
                                let sample = MFCreateSample()?;
                                let buffer = MFCreateMemoryBuffer(audio_bytes.len() as u32)?;
                                let mut data_ptr = std::ptr::null_mut();
                                buffer.Lock(&mut data_ptr, None, None)?;
                                std::ptr::copy_nonoverlapping(audio_bytes.as_ptr(), data_ptr, audio_bytes.len());
                                buffer.Unlock()?;
                                buffer.SetCurrentLength(audio_bytes.len() as u32)?;
                                sample.AddBuffer(&buffer)?;
                                sample.SetSampleTime(audio_ts)?;
                                sample.SetSampleDuration(duration)?;
                                sink_writer.WriteSample(audio_stream_index, &sample)?;
                            }
                        } else {
                            // This audio belongs to the next interval, leave it in the queue
                            break;
                        }
                    }
                    Err(_) => { break; }
                }
            }

            // Write the video frame for the current timestamp
            unsafe {
                let corrected_frame = flip_frame_vertically(&video_data, width, height);
                let sample = MFCreateSample()?;
                let buffer = MFCreateMemoryBuffer(corrected_frame.len() as u32)?;
                let mut data_ptr = std::ptr::null_mut();
                buffer.Lock(&mut data_ptr, None, None)?;
                std::ptr::copy_nonoverlapping(corrected_frame.as_ptr(), data_ptr, corrected_frame.len());
                buffer.Unlock()?;
                buffer.SetCurrentLength(corrected_frame.len() as u32)?;
                sample.AddBuffer(&buffer)?;
                sample.SetSampleTime(video_ts)?;
                let video_frame_duration = 10_000_000i64 / frame_rate as i64;
                sample.SetSampleDuration(video_frame_duration)?;
                sink_writer.WriteSample(video_stream_index, &sample)?;
            }
        }

        stop_signal.store(true, Ordering::SeqCst);
        unsafe {
            sink_writer.Finalize()?;
            MFShutdown()?;
        }
        Ok(())
    }
}

fn capture_screen(width: u32, height: u32) -> Vec<u8> {
    unsafe {
        let hdc_screen = GetDC(HWND(0));
        let hdc_mem = CreateCompatibleDC(hdc_screen);
        let hbm_mem = CreateCompatibleBitmap(hdc_screen, width as i32, height as i32);
        let old_bmp = SelectObject(hdc_mem, HGDIOBJ(hbm_mem.0));
        BitBlt(hdc_mem, 0, 0, width as i32, height as i32, hdc_screen, 0, 0, ROP_CODE(0x00CC0020));
        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0 as u32,
                ..Default::default()
            },
            ..Default::default()
        };
        let buffer_size = (width * height * 4) as usize;
        let mut bits = vec![0u8; buffer_size];
        GetDIBits(hdc_mem, hbm_mem, 0, height, Some(bits.as_mut_ptr() as *mut std::ffi::c_void), &mut bmi, DIB_RGB_COLORS);
        SelectObject(hdc_mem, old_bmp);
        DeleteObject(HGDIOBJ(hbm_mem.0));
        DeleteDC(hdc_mem);
        ReleaseDC(HWND(0), hdc_screen);
        bits
    }
}

fn flip_frame_vertically(frame: &[u8], width: u32, height: u32) -> Vec<u8> {
    let stride = width as usize * 4;
    let mut flipped_frame = Vec::with_capacity(frame.len());
    for y in (0..height as usize).rev() {
        let start = y * stride;
        let end = start + stride;
        flipped_frame.extend_from_slice(&frame[start..end]);
    }
    flipped_frame
}

fn get_pending_dir(app: &tauri::AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| PathBuf::from("./data")) // Fallback for robustness
        .join("video_recordings_pending")
}

fn get_today_pending_folder(base_dir: &PathBuf) -> PathBuf {
    let now = Utc::now();
    base_dir.join(format!("{:04}-{:02}-{:02}", now.year(), now.month(), now.day()))
}

fn try_upload_video_file(client: &Client, filepath: &PathBuf) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let url = "http://192.168.1.26:3000/api/v1/upload-video";
    let filename = filepath.file_name().and_then(|n| n.to_str()).unwrap_or("unknown.mp4");
    let file_data = fs::read(filepath)?;
    let form = reqwest::blocking::multipart::Form::new().part(
        "file",
        reqwest::blocking::multipart::Part::bytes(file_data)
            .file_name(filename.to_string())
            .mime_str("video/mp4")?,
    );
    let response = client.post(url).multipart(form).timeout(Duration::from_secs(60)).send()?;
    if response.status().is_success() { Ok(()) } else { Err(format!("[UPLOAD] Upload failed for {}: {} - {}", filename, response.status(), response.text().unwrap_or_default()).into()) }
}

pub fn retry_all_pending_videos(client: &Client, base_dir: &PathBuf) {
    if let Ok(date_dirs) = fs::read_dir(base_dir) {
        for date_dir_entry in date_dirs.flatten() {
            let dir_path = date_dir_entry.path();
            if !dir_path.is_dir() { continue; }
            if let Ok(file_entries) = fs::read_dir(&dir_path) {
                for file_entry in file_entries.flatten() {
                    let file_path = file_entry.path();
                    if file_path.extension().and_then(|s| s.to_str()) != Some("mp4") { continue; }
                    if try_upload_video_file(client, &file_path).is_ok() {
                        let _ = fs::remove_file(&file_path);
                    }
                }
            }
        }
    }
}

fn run_video_recorder(app: AppHandle, state: VideoServiceState) {
    let client = Client::new();
    let pending_dir = get_pending_dir(&app);
    let is_running = state.is_running.clone();
    println!("🎥 Starting video recording service loop...");
    loop {
        if !*is_running.lock().unwrap() {
            println!("[LOOP] `is_running` is false. Stopping video recording thread.");
            break;
        }
        let today_dir = get_today_pending_folder(&pending_dir);
        if let Err(e) = fs::create_dir_all(&today_dir) {
            eprintln!("[LOOP] ERROR: Failed to create video directory {}: {}", today_dir.display(), e);
            thread::sleep(Duration::from_secs(30));
            continue;
        }
        let timestamp = Utc::now().format("%Y-%m-%d_%H%M%S_%3f").to_string();
        let unique_id = Uuid::new_v4();
        let filename = format!("video_{}_{}.mp4", timestamp, unique_id);
        let filepath = today_dir.join(&filename);
        
        println!("[LOOP] Starting new recording: {}", filename);
        let recording_succeeded = match Recorder::new(filepath.clone()) {
            Ok(mut recorder) => match recorder.record() {
                Ok(()) => { println!("[LOOP] Recording completed successfully!"); true }
                Err(e) => {
                    eprintln!("[LOOP] Recording failed: {}", e);
                    let _ = fs::remove_file(&filepath);
                    false
                }
            },
            Err(e) => { eprintln!("[LOOP] Failed to init recorder: {}", e); false }
        };
        
        if recording_succeeded {
            if try_upload_video_file(&client, &filepath).is_err() {
                println!("[LOOP] Upload failed, video saved for retry: {}", filepath.display());
            } else {
                println!("[LOOP] Upload successful, deleting local file.");
                let _ = fs::remove_file(&filepath);
            }
            println!("[LOOP] Sleeping 5 seconds before next recording...");
            thread::sleep(Duration::from_secs(5));
        } else {
            println!("[LOOP] Recording failed, sleeping 30 seconds before retry...");
            thread::sleep(Duration::from_secs(30));
        }
    }
    println!("🛑 Video recording service loop terminated.");
}

#[tauri::command]
pub fn start_video_recording_service(app: AppHandle, state: tauri::State<'_, MainAppState>) {
    let video_state = state.video_service_state.clone();
    let is_running = video_state.is_running.clone();
    {
        let mut running_flag = is_running.lock().unwrap();
        if *running_flag {
            println!("[COMMAND] Service already running, ignoring start request.");
            return;
        }
        *running_flag = true;
    }
    let recorder_app = app.clone();
    thread::spawn(move || {
        run_video_recorder(recorder_app, video_state);
    });
    let retry_app = app.clone();
    let retry_is_running = is_running.clone();
    thread::spawn(move || {
        let client = Client::new();
        let pending_dir = get_pending_dir(&retry_app);
        loop {
            thread::sleep(Duration::from_secs(300));
            if !*retry_is_running.lock().unwrap() {
                break;
            }
            retry_all_pending_videos(&client, &pending_dir);
        }
    });
    println!("[COMMAND] Video recording service started successfully.");
}

#[tauri::command]
pub fn stop_video_recording_service(state: tauri::State<'_, MainAppState>) {
    let mut is_running = state.video_service_state.is_running.lock().unwrap();
    *is_running = false;
    println!("🛑 Video recording service manually stopped. Threads will terminate after their current cycle.");
}