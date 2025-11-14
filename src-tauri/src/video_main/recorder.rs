// src/video_main/recorder.rs

use super::avi_writer::{AviSegmentConfig, AviSegmentWriter};
use super::mp4_writer::AudioSource;
use super::mp4_writer::{Mp4SegmentConfig, Mp4SegmentWriter};
#[cfg(feature = "webm")]
use super::webm_writer::{WebmSegmentConfig, WebmSegmentWriter};

use image::{imageops, ImageBuffer, Rgb};
use image::codecs::jpeg::JpegEncoder;
use image::ColorType;
use log::{error, warn};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

// Windows GDI
#[cfg(target_os = "windows")]
use windows::Win32::{
    Foundation::HWND,
    Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC,
        GetDIBits, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
        HGDIOBJ, SRCCOPY,
    },
    UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN},
};

#[derive(Debug, Clone)]
pub struct RecorderConfig {
    pub output_dir: PathBuf,
    pub base_name: String,
    pub segment_duration: Duration,
    pub fps: u32,
    pub container: Container,
    pub display_index: usize,
    pub record_all: bool,
    pub combine_all: bool,
    pub flip_vertical: bool,
    pub flip_horizontal: bool,
    pub video_bitrate_kbps: u32,
    pub scale_max_width: Option<u32>,
    pub include_audio: bool,
    pub audio_bitrate_kbps: u32,
    pub audio_source: AudioSource,
}

pub struct Recorder {
    cfg: RecorderConfig,
    stop: Arc<AtomicBool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    Avi,
    Webm,
    Mp4,
}

// Windows GDI screen capture. Using a negative biHeight gives us a top-down image,
// which is what most encoders expect. No manual flipping is needed.
#[cfg(target_os = "windows")]
fn capture_screen_gdi(width: u32, height: u32) -> Vec<u8> {
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
            SRCCOPY,
        );

        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32), // IMPORTANT: This creates a top-down bitmap
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0 as u32,
                ..Default::default()
            },
            ..Default::default()
        };

        let buffer_size = (width * height * 4) as usize;
        let mut bgra_bits = vec![0u8; buffer_size];

        GetDIBits(
            hdc_mem,
            hbm_mem,
            0,
            height,
            Some(bgra_bits.as_mut_ptr() as *mut _),
            &mut bmi,
            DIB_RGB_COLORS,
        );

        SelectObject(hdc_mem, old_bmp);
        DeleteObject(HGDIOBJ(hbm_mem.0));
        DeleteDC(hdc_mem);
        ReleaseDC(HWND(0), hdc_screen);

        bgra_bits
    }
}

#[cfg(target_os = "windows")]
fn get_screen_dimensions() -> (u32, u32) {
    unsafe {
        (
            GetSystemMetrics(SM_CXSCREEN) as u32,
            GetSystemMetrics(SM_CYSCREEN) as u32,
        )
    }
}

impl Recorder {
    pub fn new(cfg: RecorderConfig) -> Self {
        Self {
            cfg,
            stop: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn stop_flag(&self) -> Arc<AtomicBool> {
        self.stop.clone()
    }

    pub fn run_blocking(&self) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.cfg.output_dir)?;

        #[cfg(target_os = "windows")]
        let (screen_width, screen_height) = get_screen_dimensions();

        #[cfg(not(target_os = "windows"))]
        return Err(anyhow::anyhow!("Only Windows is supported for recording"));

        let mut width = screen_width;
        let mut height = screen_height;

        if let Some(max_w) = self.cfg.scale_max_width {
            if max_w > 0 && width > max_w {
                height = (height as u64 * max_w as u64 / width as u64) as u32;
                width = max_w;
            }
        }

        log::info!(
            "Recording: container={:?}, {}x{} (screen {}x{}), fps={}",
            self.cfg.container, width, height, screen_width, screen_height, self.cfg.fps
        );

        enum WriterKind {
            Avi(AviSegmentWriter),
            #[cfg(feature = "webm")]
            Webm(WebmSegmentWriter),
            Mp4(Mp4SegmentWriter),
        }

        let mut writer = match self.cfg.container {
            Container::Avi => WriterKind::Avi(AviSegmentWriter::create_new(AviSegmentConfig {
                width,
                height,
                fps: self.cfg.fps,
                output_dir: self.cfg.output_dir.join("videos"),
                base_name: self.cfg.base_name.clone(),
            })?),

            Container::Webm => {
                #[cfg(feature = "webm")]
                {
                    WriterKind::Webm(WebmSegmentWriter::create_new(WebmSegmentConfig {
                        width,
                        height,
                        fps: self.cfg.fps,
                        output_dir: self.cfg.output_dir.join("videos"),
                        base_name: self.cfg.base_name.clone(),
                        quantizer: 160,
                    })?)
                }
                #[cfg(not(feature = "webm"))]
                return Err(anyhow::anyhow!("WebM feature is not enabled"));
            }

            Container::Mp4 => WriterKind::Mp4(Mp4SegmentWriter::create_new(Mp4SegmentConfig {
                width,
                height,
                fps: self.cfg.fps,
                output_dir: self.cfg.output_dir.join("videos"),
                base_name: self.cfg.base_name.clone(),
                bitrate_kbps: self.cfg.video_bitrate_kbps,
                include_audio: self.cfg.include_audio,
                audio_bitrate_kbps: self.cfg.audio_bitrate_kbps,
                audio_source: self.cfg.audio_source,
            })?),
        };

        let mut segment_start = Instant::now();
        let expected_frames =
            (self.cfg.fps as u64).saturating_mul(self.cfg.segment_duration.as_secs());
        let mut frames = 0u64;

        let frame_interval = Duration::from_nanos(1_000_000_000 / self.cfg.fps.max(1) as u64);
        let mut next_frame_time = Instant::now();

        log::info!("Starting video recording loop...");
        while !self.stop.load(Ordering::Relaxed) {
            let now = Instant::now();

            if now.duration_since(segment_start) >= self.cfg.segment_duration {
                log::info!("Segment duration reached. Finalizing and starting new segment.");
                match &mut writer {
                    WriterKind::Avi(w) => {
                        let new_writer = AviSegmentWriter::create_new(AviSegmentConfig {
                            width, height, fps: self.cfg.fps,
                            output_dir: self.cfg.output_dir.join("videos"),
                            base_name: self.cfg.base_name.clone(),
                        })?;
                        if let Err(e) = std::mem::replace(w, new_writer).finalize() {
                            error!("Failed to finalize AVI segment: {:?}", e);
                        }
                    }
                    #[cfg(feature = "webm")]
                    WriterKind::Webm(w) => {
                        let new_writer = WebmSegmentWriter::create_new(WebmSegmentConfig {
                             width, height, fps: self.cfg.fps,
                             output_dir: self.cfg.output_dir.join("videos"),
                             base_name: self.cfg.base_name.clone(),
                             quantizer: 160,
                        })?;
                        if let Err(e) = std::mem::replace(w, new_writer).finalize() {
                            error!("Failed to finalize WebM segment: {:?}", e);
                        }
                    }
                    WriterKind::Mp4(w) => {
                        let new_writer = Mp4SegmentWriter::create_new(Mp4SegmentConfig {
                            width, height, fps: self.cfg.fps,
                            output_dir: self.cfg.output_dir.join("videos"),
                            base_name: self.cfg.base_name.clone(),
                            bitrate_kbps: self.cfg.video_bitrate_kbps,
                            include_audio: self.cfg.include_audio,
                            audio_bitrate_kbps: self.cfg.audio_bitrate_kbps,
                            audio_source: self.cfg.audio_source,
                        })?;
                        if let Err(e) = std::mem::replace(w, new_writer).finalize() {
                            error!("Failed to finalize MP4 segment: {:?}", e);
                        }
                    }
                }
                segment_start = now;
                frames = 0;
            }

            if now < next_frame_time {
                 let sleep_duration = next_frame_time - now;
                 if sleep_duration > Duration::from_millis(1) {
                    std::thread::sleep(sleep_duration);
                 }
                continue;
            }
            next_frame_time += frame_interval;

            let bgra = capture_screen_gdi(screen_width, screen_height);

            let mut rgb = Vec::with_capacity((screen_width * screen_height * 3) as usize);
            rgb.extend(bgra.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0]]));

            let mut img = ImageBuffer::<Rgb<u8>, _>::from_raw(screen_width, screen_height, rgb).unwrap();
            if width != screen_width || height != screen_height {
                img = imageops::resize(&img, width, height, imageops::FilterType::Triangle);
            }
            if self.cfg.flip_vertical {
                imageops::flip_vertical_in_place(&mut img);
            }
            if self.cfg.flip_horizontal {
                imageops::flip_horizontal_in_place(&mut img);
            }
            let rgb_buf = img.into_raw();

            match &mut writer {
                WriterKind::Avi(w) => {
                    let mut jpeg = Vec::with_capacity((width * height / 10) as usize);
                    let mut enc = JpegEncoder::new_with_quality(&mut jpeg, 70);
                    if enc.encode(&rgb_buf, width, height, ColorType::Rgb8.into()).is_ok() {
                        if w.write_jpeg_frame(&jpeg).is_ok() {
                            frames += 1;
                        }
                    } else {
                        warn!("JPEG encoding failed.");
                    }
                }
                #[cfg(feature = "webm")]
                WriterKind::Webm(w) => {
                    if w.encode_rgb_frame(&rgb_buf).is_ok() {
                        frames += 1;
                    }
                }
                WriterKind::Mp4(w) => {
                    if w.encode_rgb_frame(&rgb_buf).is_ok() {
                        frames += 1;
                    }
                }
            }
        }

        log::info!("Stop signal received. Finalizing the last segment.");
        match writer {
            WriterKind::Avi(w) => {
                if let Ok(path) = w.finalize() {
                    if frames < expected_frames {
                        log::warn!("Segment incomplete ({} / {} frames). Deleting file: {:?}", frames, expected_frames, path);
                        let _ = std::fs::remove_file(&path);
                    }
                }
            }
            #[cfg(feature = "webm")]
            WriterKind::Webm(w) => {
                if let Ok(path) = w.finalize() {
                    if frames < expected_frames {
                        log::warn!("Segment incomplete ({} / {} frames). Deleting file: {:?}", frames, expected_frames, path);
                        let _ = std::fs::remove_file(&path);
                    }
                }
            }
            WriterKind::Mp4(w) => {
                if let Ok(path) = w.finalize() {
                    if frames < expected_frames {
                        log::warn!("Segment incomplete ({} / {} frames). Deleting file: {:?}", frames, expected_frames, path);
                        let _ = std::fs::remove_file(&path);
                    }
                }
            }
        }

        Ok(())
    }
}