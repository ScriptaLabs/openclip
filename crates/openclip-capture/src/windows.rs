use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, SyncSender, TryRecvError, channel, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use openclip_buffer::ReplaySegment;
use openclip_core::{OpenClipError, RecordingState, Result};
use wasapi::{AudioClient, Direction, SampleType, StreamMode, WaveFormat, initialize_mta};
use windows_capture::capture::{Context, GraphicsCaptureApiHandler};
use windows_capture::encoder::{
    AudioSettingsBuilder, ContainerSettingsBuilder, VideoEncoder, VideoSettingsBuilder,
    VideoSettingsSubType,
};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
    MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings as CaptureSettings,
};
use windows_capture::window::Window;

use crate::AudioDevice;
use crate::config::{EncoderConfig, encoder_config};
use crate::recorder::{SharedRecorder, lock, read, set_status, waiting_status};
use crate::validate_finalized_segment;

const SEGMENT_DURATION: Duration = Duration::from_secs(2);
const AUDIO_CHANNEL_CAPACITY: usize = 128;
const AUDIO_CHUNK_FRAMES: usize = 480;

type HandlerError = Box<dyn std::error::Error + Send + Sync>;
type PreparedResult = std::result::Result<PreparedSegment, String>;
type EncoderFactory = (
    SyncSender<(u32, u32)>,
    Receiver<PreparedResult>,
    thread::JoinHandle<()>,
);

pub(crate) struct CaptureLifecycle {
    running: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
}

impl CaptureLifecycle {
    pub(crate) fn new() -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
            stop: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(crate) fn start(&self, shared: Arc<SharedRecorder>) -> Result<()> {
        if self.running.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        self.stop.store(false, Ordering::Release);
        let running = self.running.clone();
        let stop = self.stop.clone();
        thread::Builder::new()
            .name("openclip-capture-supervisor".to_owned())
            .spawn(move || {
                supervise(&shared, &stop);
                running.store(false, Ordering::Release);
            })
            .map_err(capture_error)?;
        Ok(())
    }

    pub(crate) fn stop(&self) {
        self.stop.store(true, Ordering::Release);
    }
}

#[derive(Clone)]
struct HandlerFlags {
    shared: Arc<SharedRecorder>,
    sequence: Arc<AtomicU64>,
    audio_receiver: Arc<Mutex<Receiver<Vec<u8>>>>,
    audio_error: Arc<Mutex<Option<String>>>,
    finalizer: Sender<FinalizationJob>,
    generation: Arc<AtomicU64>,
    encoder_request: SyncSender<(u32, u32)>,
    prepared_encoder: Arc<Mutex<Receiver<PreparedResult>>>,
}

struct PreparedSegment {
    encoder: VideoEncoder,
    partial_path: PathBuf,
    source_size: (u32, u32),
    config: EncoderConfig,
    init_ms: u128,
}

struct FinalizationJob {
    encoder: VideoEncoder,
    partial_path: PathBuf,
    started_at_ms: u64,
    generation: u64,
    generation_clock: Arc<AtomicU64>,
}

struct Cs2CaptureHandler {
    flags: HandlerFlags,
    encoder: Option<VideoEncoder>,
    partial_path: Option<PathBuf>,
    segment_started: Option<Instant>,
    segment_started_at_ms: u64,
    source_size: Option<(u32, u32)>,
    frames: u64,
    dropped_frames: u64,
    diagnostics_started: Instant,
    generation: u64,
}

impl GraphicsCaptureApiHandler for Cs2CaptureHandler {
    type Flags = HandlerFlags;
    type Error = HandlerError;

    fn new(ctx: Context<Self::Flags>) -> std::result::Result<Self, Self::Error> {
        let generation = ctx.flags.generation.load(Ordering::Acquire);
        Ok(Self {
            flags: ctx.flags,
            encoder: None,
            partial_path: None,
            segment_started: None,
            segment_started_at_ms: 0,
            source_size: None,
            frames: 0,
            dropped_frames: 0,
            diagnostics_started: Instant::now(),
            generation,
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        _capture_control: InternalCaptureControl,
    ) -> std::result::Result<(), Self::Error> {
        let source_size = (frame.width(), frame.height());
        let source_changed = self
            .source_size
            .is_some_and(|previous| previous != source_size);
        let segment_expired = self
            .segment_started
            .is_some_and(|started| started.elapsed() >= SEGMENT_DURATION);
        if self.encoder.is_none() || source_changed || segment_expired {
            self.rotate(source_size, source_changed)?;
        }

        if let Some(encoder) = self.encoder.as_mut() {
            loop {
                let audio = lock(&self.flags.audio_receiver).try_recv();
                match audio {
                    Ok(bytes) => encoder.send_audio_buffer(&bytes, 0)?,
                    Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
                }
            }
            encoder.send_frame(frame)?;
            self.frames = self.frames.saturating_add(1);
        } else {
            self.dropped_frames = self.dropped_frames.saturating_add(1);
        }

        if self.diagnostics_started.elapsed() >= Duration::from_secs(10) {
            let seconds = self.diagnostics_started.elapsed().as_secs_f64();
            tracing::info!(
                capture_fps = f64::from(u32::try_from(self.frames).unwrap_or(u32::MAX)) / seconds,
                dropped_frames = self.dropped_frames,
                "capture diagnostics"
            );
            self.frames = 0;
            self.dropped_frames = 0;
            self.diagnostics_started = Instant::now();
        }
        Ok(())
    }

    fn on_closed(&mut self) -> std::result::Result<(), Self::Error> {
        self.queue_segment_finalization()?;
        tracing::info!("CS2 capture item closed");
        Ok(())
    }
}

impl Cs2CaptureHandler {
    fn rotate(
        &mut self,
        source_size: (u32, u32),
        source_changed: bool,
    ) -> std::result::Result<(), HandlerError> {
        if source_changed {
            self.finalize_segment_now()?;
        } else {
            self.queue_segment_finalization()?;
        }
        if source_changed {
            let mut buffer = lock(&self.flags.shared.buffer);
            self.generation = self.flags.generation.fetch_add(1, Ordering::AcqRel) + 1;
            let removed = buffer.clear();
            drop(buffer);
            remove_segments(&self.flags.shared, removed);
            tracing::info!(
                width = source_size.0,
                height = source_size.1,
                "capture resolution changed; replay window reset"
            );
        }
        let prepared = if source_changed {
            self.discard_prepared_encoder();
            prepare_segment(&self.flags.shared, &self.flags.sequence, source_size)?
        } else {
            match self.take_prepared_encoder(source_size)? {
                Some(prepared) => prepared,
                None => prepare_segment(&self.flags.shared, &self.flags.sequence, source_size)?,
            }
        };
        let config = prepared.config;
        self.source_size = Some(source_size);
        self.segment_started = Some(Instant::now());
        self.segment_started_at_ms = self.flags.shared.timeline_offset_ms.saturating_add(
            u64::try_from(self.flags.shared.clock_started.elapsed().as_millis())
                .unwrap_or(u64::MAX),
        );
        self.partial_path = Some(prepared.partial_path);
        self.encoder = Some(prepared.encoder);
        let _ = self.flags.encoder_request.try_send(source_size);

        let audio_error = lock(&self.flags.audio_error).clone();
        let mut status = lock(&self.flags.shared.status);
        status.state = RecordingState::Recording;
        status.message = Some(audio_error.map_or_else(
            || "Replay active · F8 saves the last 15 seconds".to_owned(),
            |error| format!("Replay active · game audio unavailable: {error}"),
        ));
        status.encoder = Some(if nvidia_driver_available() {
            "NVIDIA H.264 via Windows Media Foundation".to_owned()
        } else {
            "Windows Media Foundation H.264 hardware acceleration".to_owned()
        });
        status.capture_width = Some(config.width);
        status.capture_height = Some(config.height);
        status.capture_fps = Some(config.frames_per_second);
        tracing::info!(
            width = config.width,
            height = config.height,
            fps = config.frames_per_second,
            bitrate = config.bitrate,
            audio = config.capture_game_audio,
            init_ms = prepared.init_ms,
            backend = %status.encoder.as_deref().unwrap_or("unknown"),
            "H.264 hardware-accelerated segment encoder initialized"
        );
        Ok(())
    }

    fn take_prepared_encoder(
        &self,
        source_size: (u32, u32),
    ) -> std::result::Result<Option<PreparedSegment>, HandlerError> {
        match lock(&self.flags.prepared_encoder).try_recv() {
            Ok(prepared) => {
                let prepared = prepared.map_err(capture_error)?;
                if prepared.source_size == source_size {
                    Ok(Some(prepared))
                } else {
                    discard_encoder_async(prepared);
                    Ok(None)
                }
            }
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => Ok(None),
        }
    }

    fn discard_prepared_encoder(&self) {
        if let Ok(prepared) = lock(&self.flags.prepared_encoder).try_recv()
            && let Ok(prepared) = prepared
        {
            discard_encoder_async(prepared);
        }
    }

    fn take_finalization_job(&mut self) -> Option<FinalizationJob> {
        let encoder = self.encoder.take()?;
        Some(FinalizationJob {
            encoder,
            partial_path: self.partial_path.take().expect("encoder path is paired"),
            started_at_ms: self.segment_started_at_ms,
            generation: self.generation,
            generation_clock: self.flags.generation.clone(),
        })
    }

    fn queue_segment_finalization(&mut self) -> std::result::Result<(), HandlerError> {
        if let Some(job) = self.take_finalization_job() {
            self.flags
                .finalizer
                .send(job)
                .map_err(|_| "replay segment finalizer stopped")?;
        }
        Ok(())
    }

    fn finalize_segment_now(&mut self) -> std::result::Result<(), HandlerError> {
        if let Some(job) = self.take_finalization_job() {
            finalize_segment(&self.flags.shared, job)?;
        }
        Ok(())
    }
}

fn finalize_segment(
    shared: &SharedRecorder,
    job: FinalizationJob,
) -> std::result::Result<(), HandlerError> {
    let started = Instant::now();
    job.encoder.finish()?;
    let media = validate_finalized_segment(&job.partial_path)?;
    let final_path = job.partial_path.with_extension("");
    std::fs::rename(&job.partial_path, &final_path)?;
    let size_bytes = std::fs::metadata(&final_path)?.len();
    let segment = ReplaySegment {
        path: final_path,
        started_at_ms: job.started_at_ms,
        ended_at_ms: job.started_at_ms.saturating_add(media.duration_ms),
        size_bytes,
        begins_with_keyframe: true,
    };
    let mut buffer = lock(&shared.buffer);
    if job.generation != job.generation_clock.load(Ordering::Acquire) {
        drop(buffer);
        std::fs::remove_file(&segment.path)?;
        tracing::debug!("discarded a finalized segment from an obsolete capture resolution");
        return Ok(());
    }
    let removed = buffer.push(segment);
    let mut status = lock(&shared.status);
    status.buffered_seconds = u16::try_from(buffer.duration_ms() / 1_000).unwrap_or(u16::MAX);
    tracing::debug!(
        segments = buffer.len(),
        buffer_bytes = buffer.total_bytes(),
        buffered_seconds = status.buffered_seconds,
        finalize_ms = started.elapsed().as_millis(),
        "finalized replay segment"
    );
    drop(status);
    drop(buffer);
    shared.buffer_finalized.notify_all();
    remove_segments(shared, removed);
    Ok(())
}

fn prepare_segment(
    shared: &SharedRecorder,
    sequence: &AtomicU64,
    source_size: (u32, u32),
) -> std::result::Result<PreparedSegment, HandlerError> {
    let config = encoder_config(&read(&shared.settings), source_size.0, source_size.1);
    let sequence = sequence.fetch_add(1, Ordering::Relaxed);
    let partial_path = shared.replay_root.join(format!(
        "segment-{:012}-{sequence:06}.mp4.partial",
        shared.clock_started.elapsed().as_millis()
    ));
    let initialized = Instant::now();
    let encoder = VideoEncoder::new(
        VideoSettingsBuilder::new(config.width, config.height)
            .sub_type(VideoSettingsSubType::H264)
            .bitrate(config.bitrate)
            .frame_rate(u32::from(config.frames_per_second)),
        AudioSettingsBuilder::new().disabled(!config.capture_game_audio),
        ContainerSettingsBuilder::new(),
        &partial_path,
    )?;
    Ok(PreparedSegment {
        encoder,
        partial_path,
        source_size,
        config,
        init_ms: initialized.elapsed().as_millis(),
    })
}

fn spawn_encoder_factory(
    shared: &Arc<SharedRecorder>,
    sequence: &Arc<AtomicU64>,
) -> std::io::Result<EncoderFactory> {
    let (request_sender, requests) = sync_channel(1);
    let (prepared_sender, prepared_receiver) = sync_channel(1);
    let shared = shared.clone();
    let sequence = sequence.clone();
    let worker = thread::Builder::new()
        .name("openclip-encoder-prewarmer".to_owned())
        .spawn(move || {
            let _ = initialize_mta();
            while let Ok(source_size) = requests.recv() {
                let prepared = prepare_segment(&shared, &sequence, source_size)
                    .map_err(|error| error.to_string());
                if prepared_sender.send(prepared).is_err() {
                    break;
                }
            }
        })?;
    Ok((request_sender, prepared_receiver, worker))
}

fn discard_encoder_async(prepared: PreparedSegment) {
    let result = thread::Builder::new()
        .name("openclip-unused-encoder-cleanup".to_owned())
        .spawn(move || {
            let path = prepared.partial_path.clone();
            drop(prepared);
            let _ = std::fs::remove_file(path);
        });
    if let Err(error) = result {
        tracing::warn!(%error, "could not start unused encoder cleanup");
    }
}

fn spawn_segment_finalizer(
    shared: &Arc<SharedRecorder>,
) -> std::io::Result<(Sender<FinalizationJob>, thread::JoinHandle<()>)> {
    let (sender, receiver) = channel();
    let shared = shared.clone();
    let worker = thread::Builder::new()
        .name("openclip-segment-finalizer".to_owned())
        .spawn(move || {
            while let Ok(job) = receiver.recv() {
                if let Err(error) = finalize_segment(&shared, job) {
                    tracing::error!(%error, "could not finalize replay segment");
                }
            }
        })?;
    Ok((sender, worker))
}

fn supervise(shared: &Arc<SharedRecorder>, stop: &Arc<AtomicBool>) {
    let sequence = Arc::new(AtomicU64::new(0));
    while !stop.load(Ordering::Acquire) {
        if !hardware_ready(shared) {
            thread::sleep(Duration::from_secs(5));
            continue;
        }
        let target = match find_cs2_window() {
            Ok(Some(target)) => target,
            Ok(None) => {
                *lock(&shared.status) = waiting_status();
                thread::sleep(Duration::from_millis(750));
                continue;
            }
            Err(error) => {
                set_status(
                    shared,
                    RecordingState::Error,
                    format!("CS2 discovery failed: {error}"),
                );
                thread::sleep(Duration::from_secs(2));
                continue;
            }
        };
        set_status(
            shared,
            RecordingState::Initializing,
            "Initializing CS2 capture…",
        );
        if let Err(error) = run_capture_session(shared, stop, &sequence, target) {
            set_status(
                shared,
                RecordingState::Error,
                format!("Capture session failed: {error}"),
            );
            tracing::error!(%error, "capture session failed");
        }
        if !stop.load(Ordering::Acquire) {
            *lock(&shared.status) = waiting_status();
            thread::sleep(Duration::from_millis(750));
        }
    }
}

fn run_capture_session(
    shared: &Arc<SharedRecorder>,
    stop: &AtomicBool,
    sequence: &Arc<AtomicU64>,
    target: Window,
) -> std::result::Result<(), HandlerError> {
    let process_id = target.process_id()?;
    let audio_stop = Arc::new(AtomicBool::new(false));
    let (audio_sender, audio_receiver) = sync_channel(AUDIO_CHANNEL_CAPACITY);
    let audio_error = Arc::new(Mutex::new(None));
    if read(&shared.settings).capture_game_audio {
        spawn_audio_capture(
            process_id,
            audio_sender,
            audio_stop.clone(),
            audio_error.clone(),
        );
    }
    let fps = read(&shared.settings).frames_per_second.clamp(1, 60);
    let (finalizer, finalizer_thread) = spawn_segment_finalizer(shared)?;
    let (encoder_request, prepared_encoder, encoder_factory) =
        spawn_encoder_factory(shared, sequence)?;
    let flags = HandlerFlags {
        shared: shared.clone(),
        sequence: sequence.clone(),
        audio_receiver: Arc::new(Mutex::new(audio_receiver)),
        audio_error,
        finalizer: finalizer.clone(),
        generation: Arc::new(AtomicU64::new(0)),
        encoder_request,
        prepared_encoder: Arc::new(Mutex::new(prepared_encoder)),
    };
    let settings = CaptureSettings::new(
        target,
        CursorCaptureSettings::WithoutCursor,
        DrawBorderSettings::WithoutBorder,
        SecondaryWindowSettings::Exclude,
        MinimumUpdateIntervalSettings::Custom(Duration::from_secs_f64(1.0 / f64::from(fps))),
        DirtyRegionSettings::Default,
        ColorFormat::Bgra8,
        flags,
    );
    let capture_result = match Cs2CaptureHandler::start_free_threaded(settings) {
        Ok(control) => {
            tracing::info!(process_id, "CS2 capture target detected");
            while !stop.load(Ordering::Acquire) && !control.is_finished() && cs2_window_exists() {
                thread::sleep(Duration::from_millis(500));
            }
            control.stop().map_err(HandlerError::from)
        }
        Err(error) => Err(HandlerError::from(error)),
    };
    audio_stop.store(true, Ordering::Release);
    drop(finalizer);
    if finalizer_thread.join().is_err() {
        return Err("replay segment finalizer panicked".into());
    }
    if encoder_factory.join().is_err() {
        return Err("encoder prewarmer panicked".into());
    }
    capture_result
}

fn hardware_ready(shared: &SharedRecorder) -> bool {
    let available = nvidia_driver_available();
    if !available {
        set_status(
            shared,
            RecordingState::Error,
            "No supported NVIDIA H.264 hardware encoder was detected",
        );
    }
    available
}

fn find_cs2_window() -> std::result::Result<Option<Window>, windows_capture::window::Error> {
    let mut candidates = Window::enumerate()?
        .into_iter()
        .filter(|window| {
            window
                .process_name()
                .is_ok_and(|name| name.eq_ignore_ascii_case("cs2.exe"))
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|window| {
        window
            .width()
            .unwrap_or_default()
            .saturating_mul(window.height().unwrap_or_default())
    });
    Ok(candidates.pop())
}

fn cs2_window_exists() -> bool {
    find_cs2_window().is_ok_and(|target| target.is_some())
}

fn spawn_audio_capture(
    process_id: u32,
    sender: SyncSender<Vec<u8>>,
    stop: Arc<AtomicBool>,
    error_slot: Arc<Mutex<Option<String>>>,
) {
    let result = thread::Builder::new()
        .name("openclip-cs2-audio".to_owned())
        .spawn(move || {
            if let Err(error) = capture_audio(process_id, &sender, &stop) {
                tracing::warn!(%error, "CS2 process-loopback audio stopped");
                *lock(&error_slot) = Some(error.to_string());
            }
        });
    if let Err(error) = result {
        tracing::warn!(%error, "could not start CS2 audio thread");
    }
}

fn capture_audio(
    process_id: u32,
    sender: &SyncSender<Vec<u8>>,
    stop: &AtomicBool,
) -> std::result::Result<(), HandlerError> {
    let _ = initialize_mta();
    let format = WaveFormat::new(16, 16, &SampleType::Int, 48_000, 2, None);
    let block_align = usize::try_from(format.get_blockalign())?;
    let mut client = AudioClient::new_application_loopback_client(process_id, true)?;
    client.initialize_client(
        &format,
        &Direction::Capture,
        &StreamMode::EventsShared {
            autoconvert: true,
            buffer_duration_hns: 0,
        },
    )?;
    let event = client.set_get_eventhandle()?;
    let capture = client.get_audiocaptureclient()?;
    let mut queue = VecDeque::new();
    client.start_stream()?;
    while !stop.load(Ordering::Acquire) {
        let frames = capture.get_next_packet_size()?.unwrap_or(0);
        if frames > 0 {
            capture.read_from_device_to_deque(&mut queue)?;
        }
        let chunk_bytes = block_align * AUDIO_CHUNK_FRAMES;
        while queue.len() >= chunk_bytes {
            let bytes = queue.drain(..chunk_bytes).collect();
            if sender.try_send(bytes).is_err() {
                break;
            }
        }
        let _ = event.wait_for_event(250);
    }
    client.stop_stream()?;
    Ok(())
}

fn nvidia_driver_available() -> bool {
    std::env::var_os("WINDIR").is_some_and(|windows| {
        PathBuf::from(windows)
            .join("System32")
            .join("nvEncodeAPI64.dll")
            .is_file()
    })
}

fn remove_segments(shared: &SharedRecorder, segments: Vec<ReplaySegment>) {
    for segment in segments {
        if lock(&shared.pinned_segments).contains(&segment.path) {
            lock(&shared.deferred_deletes).push(segment.path);
            continue;
        }
        if let Err(error) = std::fs::remove_file(&segment.path) {
            tracing::warn!(%error, path = %segment.path.display(), "could not prune replay segment");
        }
    }
}

fn capture_error(error: impl std::fmt::Display) -> OpenClipError {
    OpenClipError::Capture(error.to_string())
}

pub(crate) fn list_microphone_devices() -> Result<Vec<AudioDevice>> {
    let _ = initialize_mta();
    let enumerator = wasapi::DeviceEnumerator::new().map_err(capture_error)?;
    let collection = enumerator
        .get_device_collection(&Direction::Capture)
        .map_err(capture_error)?;
    let mut devices = Vec::new();
    for device in &collection {
        let device = device.map_err(capture_error)?;
        devices.push(AudioDevice {
            id: device.get_id().map_err(capture_error)?,
            name: device.get_friendlyname().map_err(capture_error)?,
        });
    }
    devices.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(devices)
}
