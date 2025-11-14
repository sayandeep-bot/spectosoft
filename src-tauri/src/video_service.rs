use super::MainAppState;
use chrono::{Datelike, Utc};
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
    audio_sender: mpsc::Sender<(Vec<f32>, i64)>,
    format_sender: mpsc::Sender<AudioFormat>,
) -> Result<()> {
    println!("[AUDIO_CAPTURE] Starting audio capture thread");
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        println!("[AUDIO_CAPTURE] COM initialized");

        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        println!("[AUDIO_CAPTURE] Got device enumerator");

        let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?;
        println!("[AUDIO_CAPTURE] Got default audio endpoint");

        let audio_client: windows::Win32::Media::Audio::IAudioClient =
            device.Activate(CLSCTX_ALL, None)?;
        println!("[AUDIO_CAPTURE] Activated audio client");

        let wave_format_ptr = audio_client.GetMixFormat()?;
        let wave_format = *wave_format_ptr;
        let audio_format = AudioFormat {
            sample_rate: wave_format.nSamplesPerSec,
            channels: wave_format.nChannels as u32,
            bits_per_sample: wave_format.wBitsPerSample as u32,
        };
        println!("[AUDIO_CAPTURE] Audio format: {:?}", audio_format);

        audio_client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_LOOPBACK,
            100_000_000,
            0,
            wave_format_ptr,
            None,
        )?;
        println!("[AUDIO_CAPTURE] Audio client initialized");

        CoTaskMemFree(Some(wave_format_ptr as *const _));
        let capture_client: windows::Win32::Media::Audio::IAudioCaptureClient =
            audio_client.GetService()?;
        audio_client.Start()?;
        println!("[AUDIO_CAPTURE] Audio capture started");

        let _ = format_sender.send(audio_format.clone());

        let mut audio_timestamp = 0i64;
        let mut packet_count = 0;

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
                    println!("[AUDIO_CAPTURE] Channel closed, stopping");
                    break;
                }

                packet_count += 1;
                if packet_count % 100 == 0 {
                    println!(
                        "[AUDIO_CAPTURE] Sent {} audio packets, timestamp: {}",
                        packet_count, audio_timestamp
                    );
                }

                audio_timestamp += duration;
                capture_client.ReleaseBuffer(num_frames_available)?;
            }
        }
        audio_client.Stop()?;
        CoUninitialize();
        println!(
            "[AUDIO_CAPTURE] Audio capture stopped, total packets: {}",
            packet_count
        );
    }
    Ok(())
}

// Thread 1: Video Producer
fn run_video_capture(
    stop_signal: Arc<AtomicBool>,
    video_sender: mpsc::Sender<(Vec<u8>, i64)>,
    width: u32,
    height: u32,
    frame_rate: u32,
    total_frames: u32,
) {
    println!("[VIDEO_CAPTURE] Starting video capture thread");
    println!(
        "[VIDEO_CAPTURE] Resolution: {}x{}, FPS: {}, Total frames: {}",
        width, height, frame_rate, total_frames
    );

    let frame_duration = Duration::from_nanos(1_000_000_000 / frame_rate as u64);
    let mut video_timestamp = 0i64;
    let frame_duration_100ns = 10_000_000i64 / frame_rate as i64;
    let mut next_frame_time = Instant::now();

    for frame_num in 0..total_frames {
        if stop_signal.load(Ordering::SeqCst) {
            println!(
                "[VIDEO_CAPTURE] Stop signal received at frame {}",
                frame_num
            );
            break;
        }

        let frame_data = capture_screen(width, height);
        if video_sender.send((frame_data, video_timestamp)).is_err() {
            println!("[VIDEO_CAPTURE] Channel closed at frame {}", frame_num);
            break;
        }

        if frame_num % 30 == 0 {
            println!(
                "[VIDEO_CAPTURE] Captured frame {}/{}, timestamp: {}",
                frame_num, total_frames, video_timestamp
            );
        }

        video_timestamp += frame_duration_100ns;

        next_frame_time += frame_duration;
        let now = Instant::now();
        if next_frame_time > now {
            thread::sleep(next_frame_time - now);
        }
    }

    println!(
        "[VIDEO_CAPTURE] Video capture complete, sent {} frames",
        total_frames
    );
}

impl Recorder {
    fn new(path: PathBuf) -> std::result::Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        println!("[RECORDER] Creating recorder for path: {}", path.display());
        Ok(Self {
            path,
            duration_secs: 30,
        })
    }

    fn record(&mut self) -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
        println!("[RECORDER] Starting recording process");

        let width = unsafe { GetSystemMetrics(SM_CXSCREEN) as u32 };
        let height = unsafe { GetSystemMetrics(SM_CYSCREEN) as u32 };
        let frame_rate = 30u32;
        let total_frames_to_capture = (self.duration_secs * frame_rate as u64) as u32;

        println!(
            "[RECORDER] Screen: {}x{}, FPS: {}, Duration: {}s, Total frames: {}",
            width, height, frame_rate, self.duration_secs, total_frames_to_capture
        );

        println!("[RECORDER] Initializing Media Foundation");
        unsafe { MFStartup(MF_VERSION, 0)? };

        println!(
            "[RECORDER] Creating sink writer for: {}",
            self.path.display()
        );
        let sink_writer = unsafe {
            MFCreateSinkWriterFromURL(&HSTRING::from(self.path.to_str().unwrap()), None, None)?
        };
        println!("[RECORDER] Sink writer created");

        let stop_signal = Arc::new(AtomicBool::new(false));

        // Start audio capture thread
        let (audio_sender, audio_receiver) = mpsc::channel();
        let (format_sender, format_receiver) = mpsc::channel::<AudioFormat>();
        let audio_stop_signal = stop_signal.clone();
        thread::spawn(move || {
            if let Err(e) = run_audio_capture(audio_stop_signal, audio_sender, format_sender) {
                eprintln!("[AUDIO_THREAD] Capture thread failed: {}", e);
            }
        });

        println!("[RECORDER] Waiting for audio format (with timeout)...");
        let mut has_audio = false;
        let mut audio_format: Option<AudioFormat> = None;
        let wait_start = Instant::now();
        loop {
            if wait_start.elapsed() > Duration::from_secs(5) {
                println!("[RECORDER] Audio initialization timeout - proceeding without audio");
                break;
            }
            match format_receiver.try_recv() {
                Ok(f) => {
                    audio_format = Some(f);
                    has_audio = true;
                    println!("[RECORDER] Got audio format: {:?}", audio_format);
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    thread::sleep(Duration::from_millis(50));
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    println!("[RECORDER] Audio channel disconnected - no audio");
                    break;
                }
            }
        }

        // Start video capture thread
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

        println!("[RECORDER] Configuring media types");
        let video_stream_index = unsafe {
            // VIDEO OUTPUT
            println!("[RECORDER] Creating video output media type");
            let out_video_mt = MFCreateMediaType()?;
            out_video_mt.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            out_video_mt.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
            out_video_mt.SetUINT32(&MF_MT_AVG_BITRATE, 10_000_000)?;
            out_video_mt.SetUINT64(&MF_MT_FRAME_SIZE, ((width as u64) << 32) | height as u64)?;
            out_video_mt.SetUINT64(&MF_MT_FRAME_RATE, ((frame_rate as u64) << 32) | 1)?;
            out_video_mt.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
            out_video_mt.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1u64 << 32) | 1)?;
            let video_stream_index = sink_writer.AddStream(&out_video_mt)?;
            println!("[RECORDER] Video stream index: {}", video_stream_index);

            // VIDEO INPUT
            println!("[RECORDER] Creating video input media type");
            let in_video_mt = MFCreateMediaType()?;
            in_video_mt.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            in_video_mt.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
            in_video_mt.SetUINT64(&MF_MT_FRAME_SIZE, ((width as u64) << 32) | height as u64)?;
            in_video_mt.SetUINT64(&MF_MT_FRAME_RATE, ((frame_rate as u64) << 32) | 1)?;
            in_video_mt.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
            in_video_mt.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1u64 << 32) | 1)?;
            sink_writer.SetInputMediaType(video_stream_index, &in_video_mt, None)?;
            println!("[RECORDER] Video input media type set");

            video_stream_index
        };

        let audio_stream_index: Option<u32> = if has_audio {
            let af = audio_format.as_ref().unwrap();
            unsafe {
                // AUDIO OUTPUT
                println!("[RECORDER] Creating audio output media type");
                let out_audio_mt = MFCreateMediaType()?;
                out_audio_mt.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
                out_audio_mt.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC)?;
                out_audio_mt.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, af.sample_rate)?;
                out_audio_mt.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, af.channels)?;
                let audio_stream_index = sink_writer.AddStream(&out_audio_mt)?;
                println!("[RECORDER] Audio stream index: {}", audio_stream_index);

                // AUDIO INPUT
                println!("[RECORDER] Creating audio input media type");
                let in_audio_mt = MFCreateMediaType()?;
                in_audio_mt.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
                in_audio_mt.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_Float)?;
                in_audio_mt.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, af.sample_rate)?;
                in_audio_mt.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, af.channels)?;
                in_audio_mt.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, af.bits_per_sample)?;
                let block_alignment = af.channels * (af.bits_per_sample / 8);
                in_audio_mt.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, block_alignment)?;
                let bytes_per_second = af.sample_rate * block_alignment;
                in_audio_mt.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, bytes_per_second)?;
                sink_writer.SetInputMediaType(audio_stream_index, &in_audio_mt, None)?;
                println!("[RECORDER] Audio input media type set");

                Some(audio_stream_index)
            }
        } else {
            println!("[RECORDER] Skipping audio streams");
            None
        };

        println!("[RECORDER] Beginning writing");
        unsafe { sink_writer.BeginWriting()? };

        let mut frames_written = 0;
        let mut audio_samples_written = 0;

        println!("[RECORDER] Starting encoding loop");
        for _ in 0..total_frames_to_capture {
            let (video_data, video_ts) = video_receiver.recv()?;

            // Drain and write any audio that arrived before this video frame's timestamp
            if let Some(audio_idx) = audio_stream_index {
                let af = audio_format.as_ref().unwrap();
                loop {
                    match audio_receiver.try_recv() {
                        Ok((audio_data, audio_ts)) => {
                            if audio_ts < video_ts {
                                unsafe {
                                    let audio_bytes: Vec<u8> =
                                        audio_data.iter().flat_map(|&f| f.to_le_bytes()).collect();
                                    let num_sample_frames =
                                        (audio_data.len() / af.channels as usize) as i64;
                                    let duration =
                                        (10_000_000 * num_sample_frames) / af.sample_rate as i64;
                                    let sample = MFCreateSample()?;
                                    let buffer = MFCreateMemoryBuffer(audio_bytes.len() as u32)?;
                                    let mut data_ptr = std::ptr::null_mut();
                                    buffer.Lock(&mut data_ptr, None, None)?;
                                    std::ptr::copy_nonoverlapping(
                                        audio_bytes.as_ptr(),
                                        data_ptr,
                                        audio_bytes.len(),
                                    );
                                    buffer.Unlock()?;
                                    buffer.SetCurrentLength(audio_bytes.len() as u32)?;
                                    sample.AddBuffer(&buffer)?;
                                    sample.SetSampleTime(audio_ts)?;
                                    sample.SetSampleDuration(duration)?;
                                    sink_writer.WriteSample(audio_idx, &sample)?;
                                    audio_samples_written += 1;
                                }
                            } else {
                                // This audio belongs to the next interval, leave it in the queue
                                break;
                            }
                        }
                        Err(mpsc::TryRecvError::Empty) => {
                            break;
                        }
                        Err(mpsc::TryRecvError::Disconnected) => {
                            println!("[RECORDER] Audio channel disconnected during recording");
                            break;
                        }
                    }
                }
            }

            // Write the video frame for the current timestamp
            unsafe {
                let corrected_frame = flip_frame_vertically(&video_data, width, height);
                let sample = MFCreateSample()?;
                let buffer = MFCreateMemoryBuffer(corrected_frame.len() as u32)?;
                let mut data_ptr = std::ptr::null_mut();
                buffer.Lock(&mut data_ptr, None, None)?;
                std::ptr::copy_nonoverlapping(
                    corrected_frame.as_ptr(),
                    data_ptr,
                    corrected_frame.len(),
                );
                buffer.Unlock()?;
                buffer.SetCurrentLength(corrected_frame.len() as u32)?;
                sample.AddBuffer(&buffer)?;
                sample.SetSampleTime(video_ts)?;
                let video_frame_duration = 10_000_000i64 / frame_rate as i64;
                sample.SetSampleDuration(video_frame_duration)?;
                sink_writer.WriteSample(video_stream_index, &sample)?;
                frames_written += 1;

                if frames_written % 30 == 0 {
                    println!(
                        "[RECORDER] Encoded {}/{} video frames, {} audio samples",
                        frames_written, total_frames_to_capture, audio_samples_written
                    );
                }
            }
        }

        println!("[RECORDER] All frames processed. Stopping capture threads...");
        stop_signal.store(true, Ordering::SeqCst);

        println!("[RECORDER] Finalizing video file...");
        unsafe {
            sink_writer.Finalize()?;
            println!("[RECORDER] Sink writer finalized");
            MFShutdown()?;
            println!("[RECORDER] Media Foundation shutdown");
        }

        println!(
            "[RECORDER] ✅ Recording complete! Written {} video frames, {} audio samples",
            frames_written, audio_samples_written
        );
        println!("[RECORDER] File saved: {}", self.path.display());
        if !has_audio {
            println!("[RECORDER] Note: Recorded without audio");
        }

        // Check file size
        if let Ok(metadata) = fs::metadata(&self.path) {
            let size_mb = metadata.len() as f64 / (1024.0 * 1024.0);
            println!("[RECORDER] File size: {:.2} MB", size_mb);
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
        BitBlt(
            hdc_mem,
            0,
            0,
            width as i32,
            height as i32,
            hdc_screen,
            0,
            0,
            ROP_CODE(0x00CC0020),
        );
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
        GetDIBits(
            hdc_mem,
            hbm_mem,
            0,
            height,
            Some(bits.as_mut_ptr() as *mut std::ffi::c_void),
            &mut bmi,
            DIB_RGB_COLORS,
        );
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
        .unwrap_or_else(|_| PathBuf::from("./data"))
        .join("video_recordings_pending")
}

fn get_today_pending_folder(base_dir: &PathBuf) -> PathBuf {
    let now = Utc::now();
    base_dir.join(format!(
        "{:04}-{:02}-{:02}",
        now.year(),
        now.month(),
        now.day()
    ))
}

fn try_upload_video_file(
    client: &Client,
    filepath: &PathBuf,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    println!("[UPLOAD] Starting upload for: {}", filepath.display());
    let url = "http://192.168.1.26:3000/api/v1/upload-video";
    let filename = filepath
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown.mp4");
    let file_data = fs::read(filepath)?;
    println!("[UPLOAD] Read {} bytes for {}", file_data.len(), filename);

    let form = reqwest::blocking::multipart::Form::new().part(
        "file",
        reqwest::blocking::multipart::Part::bytes(file_data)
            .file_name(filename.to_string())
            .mime_str("video/mp4")?,
    );
    let response = client
        .post(url)
        .multipart(form)
        .timeout(Duration::from_secs(60))
        .send()?;

    if response.status().is_success() {
        println!("[UPLOAD] ✅ Upload successful for {}", filename);
        Ok(())
    } else {
        let error_msg = format!(
            "[UPLOAD] ❌ Upload failed for {}: {} - {}",
            filename,
            response.status(),
            response.text().unwrap_or_default()
        );
        Err(error_msg.into())
    }
}

pub fn retry_all_pending_videos(client: &Client, base_dir: &PathBuf) {
    println!(
        "[RETRY] Checking for pending videos in: {}",
        base_dir.display()
    );
    if let Ok(date_dirs) = fs::read_dir(base_dir) {
        for date_dir_entry in date_dirs.flatten() {
            let dir_path = date_dir_entry.path();
            if !dir_path.is_dir() {
                continue;
            }
            if let Ok(file_entries) = fs::read_dir(&dir_path) {
                for file_entry in file_entries.flatten() {
                    let file_path = file_entry.path();
                    if file_path.extension().and_then(|s| s.to_str()) != Some("mp4") {
                        continue;
                    }
                    println!("[RETRY] Retrying upload for: {}", file_path.display());
                    if try_upload_video_file(client, &file_path).is_ok() {
                        println!(
                            "[RETRY] Upload successful, deleting: {}",
                            file_path.display()
                        );
                        let _ = fs::remove_file(&file_path);
                    }
                }
            }
        }
    }
}

fn handle_video_upload_async(filepath: PathBuf, client: Client) {
    thread::spawn(move || {
        println!(
            "[UPLOAD_THREAD] Starting async upload for: {}",
            filepath.display()
        );
        match try_upload_video_file(&client, &filepath) {
            Ok(()) => {
                println!(
                    "[UPLOAD_THREAD] Upload successful, deleting: {}",
                    filepath.display()
                );
                let _ = fs::remove_file(&filepath);
            }
            Err(e) => {
                eprintln!(
                    "[UPLOAD_THREAD] Upload failed: {}. File saved for retry.",
                    e
                );
            }
        }
    });
}

fn run_video_recorder(app: AppHandle, state: VideoServiceState) {
    let client = Client::new();
    let pending_dir = get_pending_dir(&app);
    let is_running = state.is_running.clone();
    println!("🎥 Starting video recording service loop...");

    let mut recording_count = 0;

    loop {
        if !*is_running.lock().unwrap() {
            println!("[LOOP] `is_running` is false. Stopping video recording thread.");
            break;
        }

        recording_count += 1;
        println!("\n========== RECORDING #{} ==========", recording_count);

        let today_dir = get_today_pending_folder(&pending_dir);
        if let Err(e) = fs::create_dir_all(&today_dir) {
            eprintln!(
                "[LOOP] ERROR: Failed to create video directory {}: {}",
                today_dir.display(),
                e
            );
            thread::sleep(Duration::from_secs(30));
            continue;
        }

        let timestamp = Utc::now().format("%Y-%m-%d_%H%M%S_%3f").to_string();
        let unique_id = Uuid::new_v4();
        let filename = format!("video_{}_{}.mp4", timestamp, unique_id);
        let filepath = today_dir.join(&filename);

        println!("[LOOP] Starting new recording: {}", filename);
        let start_time = Instant::now();

        let recording_succeeded = match Recorder::new(filepath.clone()) {
            Ok(mut recorder) => match recorder.record() {
                Ok(()) => {
                    let elapsed = start_time.elapsed();
                    println!(
                        "[LOOP] ✅ Recording completed successfully in {:.2}s",
                        elapsed.as_secs_f64()
                    );
                    true
                }
                Err(e) => {
                    eprintln!("[LOOP] ❌ Recording failed: {}", e);
                    let _ = fs::remove_file(&filepath);
                    false
                }
            },
            Err(e) => {
                eprintln!("[LOOP] ❌ Failed to init recorder: {}", e);
                false
            }
        };

        if recording_succeeded {
            let upload_client = client.clone();
            handle_video_upload_async(filepath, upload_client);
            println!("[LOOP] Upload started in background. Starting next recording immediately...");
        } else {
            println!("[LOOP] Recording failed, sleeping 30 seconds before retry...");
            thread::sleep(Duration::from_secs(30));
        }
    }

    println!(
        "🛑 Video recording service loop terminated. Total recordings: {}",
        recording_count
    );
}

#[tauri::command]
pub fn start_video_recording_service(app: AppHandle, state: tauri::State<'_, MainAppState>) {
    println!("[COMMAND] start_video_recording_service called");
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

    println!("[COMMAND] Starting video recording thread...");
    let recorder_app = app.clone();
    thread::spawn(move || {
        run_video_recorder(recorder_app, video_state);
    });

    println!("[COMMAND] Starting retry thread...");
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
    println!("[COMMAND] ✅ Video recording service started successfully.");
}

#[tauri::command]
pub fn stop_video_recording_service(state: tauri::State<'_, MainAppState>) {
    println!("[COMMAND] stop_video_recording_service called");
    let mut is_running = state.video_service_state.is_running.lock().unwrap();
    *is_running = false;
    println!("🛑 Video recording service manually stopped. Threads will terminate after their current cycle.");
}
