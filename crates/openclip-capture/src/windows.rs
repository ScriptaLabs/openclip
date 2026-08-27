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
use crate::media::mux_video_with_audio;
use crate::nvenc_backend::{DirectNvencEncoder, FramePacer, VideoSegment};
use crate::recorder::{SharedRecorder, lock, read, set_status, waiting_status};

const AUDIO_CHANNEL_CAPACITY: usize = 128;
const AUDIO_CHUNK_FRAMES: usize = 480;
const AUDIO_MIX_JITTER_CHUNKS: usize = 2;

type HandlerError = Box<dyn std::error::Error + Send + Sync>;
type PreparedResult = std::result::Result<PreparedAudioSegment, String>;
type AudioEncoderFactory = (
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
    audio_encoder_request: SyncSender<(u32, u32)>,
    prepared_audio_encoder: Arc<Mutex<Receiver<PreparedResult>>>,
}

struct PreparedAudioSegment {
    encoder: Option<VideoEncoder>,
    path: Option<PathBuf>,
    source_size: (u32, u32),
    config: EncoderConfig,
    init_ms: u128,
}

struct FinalizationJob {
    video: VideoSegment,
    audio_encoder: Option<VideoEncoder>,
    audio_path: Option<PathBuf>,
    partial_path: PathBuf,
    started_at_ms: u64,
    generation: u64,
    generation_clock: Arc<AtomicU64>,
}

struct Cs2CaptureHandler {
    flags: HandlerFlags,
    encoder: Option<DirectNvencEncoder>,
    pacer: Option<FramePacer>,
    audio_segment: Option<PreparedAudioSegment>,
    segment_started_at_ms: u64,
    source_size: Option<(u32, u32)>,
    frames: u64,
    encoded_frames: u64,
    repeated_frames: u64,
    skipped_early_frames: u64,
    encode_micros: u128,
    max_encode_micros: u128,
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
            pacer: None,
            audio_segment: None,
            segment_started_at_ms: 0,
            source_size: None,
            frames: 0,
            encoded_frames: 0,
            repeated_frames: 0,
            skipped_early_frames: 0,
            encode_micros: 0,
            max_encode_micros: 0,
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
        if self.encoder.is_none() || source_changed {
            self.initialize(frame, source_size, source_changed)?;
        }

        if let Some(encoder) = self.encoder.as_mut() {
            loop {
                let audio = lock(&self.flags.audio_receiver).try_recv();
                match audio {
                    Ok(bytes) => {
                        if let Some(audio) = self
                            .audio_segment
                            .as_mut()
                            .and_then(|segment| segment.encoder.as_mut())
                        {
                            audio.send_audio_buffer(&bytes, 0)?;
                        }
                    }
                    Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
                }
            }
            let timestamp = u64::try_from(frame.timestamp()?.Duration).unwrap_or_default();
            let copies = self
                .pacer
                .as_mut()
                .expect("NVENC and pacer are initialized together")
                .frames_due(timestamp);
            let encode_started = Instant::now();
            if let Some(segment) = encoder.encode_frame(frame, copies)? {
                self.queue_segment_finalization(segment)?;
                self.prepare_next_audio(source_size)?;
            }
            let elapsed_micros = encode_started.elapsed().as_micros();
            self.encode_micros = self.encode_micros.saturating_add(elapsed_micros);
            self.max_encode_micros = self.max_encode_micros.max(elapsed_micros);
            self.encoded_frames = self.encoded_frames.saturating_add(u64::from(copies));
            self.repeated_frames = self
                .repeated_frames
                .saturating_add(u64::from(copies.saturating_sub(1)));
            self.skipped_early_frames = self
                .skipped_early_frames
                .saturating_add(u64::from(copies == 0));
            self.frames = self.frames.saturating_add(1);
        } else {
            self.dropped_frames = self.dropped_frames.saturating_add(1);
        }

        if self.diagnostics_started.elapsed() >= Duration::from_secs(10) {
            let seconds = self.diagnostics_started.elapsed().as_secs_f64();
            tracing::info!(
                capture_fps = f64::from(u32::try_from(self.frames).unwrap_or(u32::MAX)) / seconds,
                encoded_fps =
                    f64::from(u32::try_from(self.encoded_frames).unwrap_or(u32::MAX)) / seconds,
                repeated_frames = self.repeated_frames,
                skipped_early_frames = self.skipped_early_frames,
                average_encode_micros = self.encode_micros / u128::from(self.frames.max(1)),
                max_encode_micros = self.max_encode_micros,
                dropped_frames = self.dropped_frames,
                "capture diagnostics"
            );
            self.frames = 0;
            self.encoded_frames = 0;
            self.repeated_frames = 0;
            self.skipped_early_frames = 0;
            self.encode_micros = 0;
            self.max_encode_micros = 0;
            self.dropped_frames = 0;
            self.diagnostics_started = Instant::now();
        }
        Ok(())
    }

    fn on_closed(&mut self) -> std::result::Result<(), Self::Error> {
        if let Some(mut encoder) = self.encoder.take()
            && let Some(segment) = encoder.finish_segment()?
        {
            self.queue_segment_finalization(segment)?;
        }
        tracing::info!("CS2 capture item closed");
        Ok(())
    }
}

impl Cs2CaptureHandler {
    fn initialize(
        &mut self,
        frame: &Frame<'_>,
        source_size: (u32, u32),
        source_changed: bool,
    ) -> std::result::Result<(), HandlerError> {
        // Audio capture is started before WGC so endpoint initialization cannot delay video.
        // Discard those pre-video samples: feeding them to a zero-based AAC stream makes sound
        // lead the first picture by the amount of time WGC/NVENC took to initialize.
        let discarded_audio_chunks = drain_pending_audio(&self.flags.audio_receiver);
        if discarded_audio_chunks > 0 {
            tracing::debug!(
                discarded_audio_ms = discarded_audio_chunks * 10,
                "discarded audio captured before the first video frame"
            );
        }
        if source_changed
            && let Some(mut encoder) = self.encoder.take()
            && let Some(segment) = encoder.finish_segment()?
        {
            self.finalize_segment_now(segment)?;
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
            self.discard_prepared_audio_encoder();
            prepare_audio_segment(&self.flags.shared, &self.flags.sequence, source_size)?
        } else {
            match self.take_prepared_audio_encoder(source_size)? {
                Some(prepared) => prepared,
                None => {
                    prepare_audio_segment(&self.flags.shared, &self.flags.sequence, source_size)?
                }
            }
        };
        let config = prepared.config;
        self.source_size = Some(source_size);
        self.segment_started_at_ms = self.flags.shared.timeline_offset_ms.saturating_add(
            u64::try_from(self.flags.shared.clock_started.elapsed().as_millis())
                .unwrap_or(u64::MAX),
        );
        let initialized = Instant::now();
        self.encoder = Some(DirectNvencEncoder::new(frame, config)?);
        self.pacer = Some(FramePacer::new(config.frames_per_second));
        self.audio_segment = Some(prepared);
        let _ = self.flags.audio_encoder_request.try_send(source_size);

        let audio_error = lock(&self.flags.audio_error).clone();
        let manual_replay_shortcut = read(&self.flags.shared.settings)
            .manual_replay_shortcut
            .clone();
        let mut status = lock(&self.flags.shared.status);
        status.state = RecordingState::Recording;
        status.message = Some(audio_error.map_or_else(
            || format!("Replay active · {manual_replay_shortcut} saves the last 15 seconds"),
            |error| format!("Replay active · audio unavailable: {error}"),
        ));
        status.encoder = Some("NVIDIA NVENC DirectX 11".to_owned());
        status.capture_width = Some(config.width);
        status.capture_height = Some(config.height);
        status.capture_fps = Some(config.frames_per_second);
        tracing::info!(
            width = config.width,
            height = config.height,
            fps = config.frames_per_second,
            bitrate = config.bitrate,
            game_audio = config.capture_game_audio,
            microphone = config.capture_microphone,
            init_ms = initialized.elapsed().as_millis(),
            audio_init_ms = self.audio_segment.as_ref().map_or(0, |audio| audio.init_ms),
            backend = %status.encoder.as_deref().unwrap_or("unknown"),
            "continuous direct NVENC encoder initialized"
        );
        Ok(())
    }

    fn take_prepared_audio_encoder(
        &self,
        source_size: (u32, u32),
    ) -> std::result::Result<Option<PreparedAudioSegment>, HandlerError> {
        match lock(&self.flags.prepared_audio_encoder).try_recv() {
            Ok(prepared) => {
                let prepared = prepared.map_err(capture_error)?;
                if prepared.source_size == source_size {
                    Ok(Some(prepared))
                } else {
                    discard_audio_encoder_async(prepared);
                    Ok(None)
                }
            }
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => Ok(None),
        }
    }

    fn discard_prepared_audio_encoder(&self) {
        if let Ok(prepared) = lock(&self.flags.prepared_audio_encoder).try_recv()
            && let Ok(prepared) = prepared
        {
            discard_audio_encoder_async(prepared);
        }
    }

    fn take_finalization_job(&mut self, video: VideoSegment) -> FinalizationJob {
        let audio = self.audio_segment.take().expect("audio segment is paired");
        let sequence = self.flags.sequence.fetch_add(1, Ordering::Relaxed);
        let partial_path = self.flags.shared.replay_root.join(format!(
            "segment-{:012}-{sequence:06}.mp4.partial",
            self.flags.shared.clock_started.elapsed().as_millis()
        ));
        FinalizationJob {
            video,
            audio_encoder: audio.encoder,
            audio_path: audio.path,
            partial_path,
            started_at_ms: self.segment_started_at_ms,
            generation: self.generation,
            generation_clock: self.flags.generation.clone(),
        }
    }

    fn queue_segment_finalization(
        &mut self,
        video: VideoSegment,
    ) -> std::result::Result<(), HandlerError> {
        let duration_ms = video.duration_ms;
        let job = self.take_finalization_job(video);
        self.flags
            .finalizer
            .send(job)
            .map_err(|_| "replay segment finalizer stopped")?;
        self.segment_started_at_ms = self.segment_started_at_ms.saturating_add(duration_ms);
        Ok(())
    }

    fn finalize_segment_now(
        &mut self,
        video: VideoSegment,
    ) -> std::result::Result<(), HandlerError> {
        let job = self.take_finalization_job(video);
        finalize_segment(&self.flags.shared, job)?;
        Ok(())
    }

    fn prepare_next_audio(
        &mut self,
        source_size: (u32, u32),
    ) -> std::result::Result<(), HandlerError> {
        self.audio_segment = Some(match self.take_prepared_audio_encoder(source_size)? {
            Some(prepared) => prepared,
            None => prepare_audio_segment(&self.flags.shared, &self.flags.sequence, source_size)?,
        });
        let _ = self.flags.audio_encoder_request.try_send(source_size);
        Ok(())
    }
}

fn drain_pending_audio(receiver: &Mutex<Receiver<Vec<u8>>>) -> u64 {
    let receiver = lock(receiver);
    let mut discarded = 0_u64;
    while receiver.try_recv().is_ok() {
        discarded = discarded.saturating_add(1);
    }
    discarded
}

fn finalize_segment(
    shared: &SharedRecorder,
    job: FinalizationJob,
) -> std::result::Result<(), HandlerError> {
    let started = Instant::now();
    if let Some(encoder) = job.audio_encoder {
        encoder.finish()?;
    }
    let media = mux_video_with_audio(
        &job.video.bytes,
        job.audio_path.as_deref(),
        &job.partial_path,
    )?;
    if let Some(audio_path) = job.audio_path
        && let Err(error) = std::fs::remove_file(&audio_path)
    {
        tracing::warn!(%error, path = %audio_path.display(), "could not remove temporary AAC segment");
    }
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
        encoded_frames = job.video.frame_count,
        "finalized replay segment"
    );
    drop(status);
    drop(buffer);
    shared.buffer_finalized.notify_all();
    remove_segments(shared, removed);
    Ok(())
}

fn prepare_audio_segment(
    shared: &SharedRecorder,
    sequence: &AtomicU64,
    source_size: (u32, u32),
) -> std::result::Result<PreparedAudioSegment, HandlerError> {
    let config = encoder_config(&read(&shared.settings), source_size.0, source_size.1);
    if !config.capture_game_audio && !config.capture_microphone {
        return Ok(PreparedAudioSegment {
            encoder: None,
            path: None,
            source_size,
            config,
            init_ms: 0,
        });
    }
    let sequence = sequence.fetch_add(1, Ordering::Relaxed);
    let path = shared
        .replay_root
        .join(format!("audio-{sequence:06}.m4a.partial"));
    let initialized = Instant::now();
    let encoder = VideoEncoder::new(
        VideoSettingsBuilder::new(config.width, config.height).disabled(true),
        AudioSettingsBuilder::new(),
        ContainerSettingsBuilder::new(),
        &path,
    )?;
    Ok(PreparedAudioSegment {
        encoder: Some(encoder),
        path: Some(path),
        source_size,
        config,
        init_ms: initialized.elapsed().as_millis(),
    })
}

fn spawn_audio_encoder_factory(
    shared: &Arc<SharedRecorder>,
    sequence: &Arc<AtomicU64>,
) -> std::io::Result<AudioEncoderFactory> {
    let (request_sender, requests) = sync_channel(1);
    let (prepared_sender, prepared_receiver) = sync_channel(1);
    let shared = shared.clone();
    let sequence = sequence.clone();
    let worker = thread::Builder::new()
        .name("openclip-aac-prewarmer".to_owned())
        .spawn(move || {
            let _ = initialize_mta();
            while let Ok(source_size) = requests.recv() {
                let prepared = prepare_audio_segment(&shared, &sequence, source_size)
                    .map_err(|error| error.to_string());
                if prepared_sender.send(prepared).is_err() {
                    break;
                }
            }
        })?;
    Ok((request_sender, prepared_receiver, worker))
}

fn discard_audio_encoder_async(prepared: PreparedAudioSegment) {
    let result = thread::Builder::new()
        .name("openclip-unused-aac-cleanup".to_owned())
        .spawn(move || {
            let path = prepared.path.clone();
            drop(prepared);
            if let Some(path) = path {
                let _ = std::fs::remove_file(path);
            }
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
    let settings_generation = shared.settings_generation.load(Ordering::Acquire);
    let audio_stop = Arc::new(AtomicBool::new(false));
    let (audio_sender, audio_receiver) = sync_channel(AUDIO_CHANNEL_CAPACITY);
    let audio_error = Arc::new(Mutex::new(None));
    let recording_settings = read(&shared.settings).clone();
    spawn_audio_sources(
        process_id,
        recording_settings.capture_game_audio,
        recording_settings.capture_microphone,
        recording_settings.microphone_device_id.clone(),
        audio_sender,
        audio_stop.clone(),
        audio_error.clone(),
    );
    let fps = recording_settings.frames_per_second.clamp(1, 60);
    let (finalizer, finalizer_thread) = spawn_segment_finalizer(shared)?;
    let (audio_encoder_request, prepared_audio_encoder, audio_encoder_factory) =
        spawn_audio_encoder_factory(shared, sequence)?;
    let flags = HandlerFlags {
        shared: shared.clone(),
        sequence: sequence.clone(),
        audio_receiver: Arc::new(Mutex::new(audio_receiver)),
        audio_error,
        finalizer: finalizer.clone(),
        generation: Arc::new(AtomicU64::new(0)),
        audio_encoder_request,
        prepared_audio_encoder: Arc::new(Mutex::new(prepared_audio_encoder)),
    };
    let settings = CaptureSettings::new(
        target,
        CursorCaptureSettings::WithoutCursor,
        DrawBorderSettings::WithoutBorder,
        SecondaryWindowSettings::Exclude,
        // WGC is sampled above the encoded rate; the deterministic pacer assigns arrivals
        // to exact output slots and direct NVENC keeps the callback free of MF video work.
        MinimumUpdateIntervalSettings::Custom(capture_interval_with_headroom(fps)),
        DirtyRegionSettings::Default,
        ColorFormat::Bgra8,
        flags,
    );
    let capture_result = match Cs2CaptureHandler::start_free_threaded(settings) {
        Ok(control) => {
            tracing::info!(process_id, "CS2 capture target detected");
            while !stop.load(Ordering::Acquire)
                && !control.is_finished()
                && cs2_window_exists()
                && shared.settings_generation.load(Ordering::Acquire) == settings_generation
            {
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
    if audio_encoder_factory.join().is_err() {
        return Err("AAC prewarmer panicked".into());
    }
    if shared.settings_generation.load(Ordering::Acquire) != settings_generation {
        let removed = lock(&shared.buffer).clear();
        remove_segments(shared, removed);
        tracing::info!("capture settings changed; replay window reset");
    }
    capture_result
}

fn capture_interval_with_headroom(encoded_fps: u16) -> Duration {
    let requested_fps = f64::from(encoded_fps).mul_add(0.25, f64::from(encoded_fps));
    Duration::from_secs_f64(1.0 / requested_fps)
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

fn spawn_audio_sources(
    process_id: u32,
    capture_game: bool,
    capture_microphone: bool,
    microphone_device_id: Option<String>,
    sender: SyncSender<Vec<u8>>,
    stop: Arc<AtomicBool>,
    error_slot: Arc<Mutex<Option<String>>>,
) {
    match (capture_game, capture_microphone) {
        (true, true) => {
            let (game_sender, game_receiver) = sync_channel(AUDIO_CHANNEL_CAPACITY / 2);
            let (mic_sender, mic_receiver) = sync_channel(AUDIO_CHANNEL_CAPACITY / 2);
            spawn_game_audio_capture(process_id, game_sender, stop.clone(), error_slot.clone());
            spawn_microphone_capture(microphone_device_id, mic_sender, stop.clone(), error_slot);
            if let Err(error) = thread::Builder::new()
                .name("openclip-audio-mixer".to_owned())
                .spawn(move || mix_audio_sources(&game_receiver, &mic_receiver, &sender, &stop))
            {
                tracing::warn!(%error, "could not start audio mixer thread");
            }
        }
        (true, false) => spawn_game_audio_capture(process_id, sender, stop, error_slot),
        (false, true) => {
            spawn_microphone_capture(microphone_device_id, sender, stop, error_slot);
        }
        (false, false) => {}
    }
}

fn spawn_game_audio_capture(
    process_id: u32,
    sender: SyncSender<Vec<u8>>,
    stop: Arc<AtomicBool>,
    error_slot: Arc<Mutex<Option<String>>>,
) {
    let result = thread::Builder::new()
        .name("openclip-cs2-audio".to_owned())
        .spawn(move || {
            if let Err(error) = capture_game_audio(process_id, &sender, &stop) {
                tracing::warn!(%error, "CS2 process-loopback audio stopped");
                *lock(&error_slot) = Some(error.to_string());
            }
        });
    if let Err(error) = result {
        tracing::warn!(%error, "could not start CS2 audio thread");
    }
}

fn spawn_microphone_capture(
    device_id: Option<String>,
    sender: SyncSender<Vec<u8>>,
    stop: Arc<AtomicBool>,
    error_slot: Arc<Mutex<Option<String>>>,
) {
    let result = thread::Builder::new()
        .name("openclip-microphone-audio".to_owned())
        .spawn(move || {
            if let Err(error) = capture_microphone_audio(device_id.as_deref(), &sender, &stop) {
                tracing::warn!(%error, "microphone capture stopped");
                *lock(&error_slot) = Some(format!("microphone: {error}"));
            }
        });
    if let Err(error) = result {
        tracing::warn!(%error, "could not start microphone thread");
    }
}

fn capture_game_audio(
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

fn capture_microphone_audio(
    device_id: Option<&str>,
    sender: &SyncSender<Vec<u8>>,
    stop: &AtomicBool,
) -> std::result::Result<(), HandlerError> {
    let _ = initialize_mta();
    let enumerator = wasapi::DeviceEnumerator::new()?;
    let device = match device_id {
        Some(id) => enumerator.get_device(id)?,
        None => enumerator.get_default_device(&Direction::Capture)?,
    };
    let mut client = device.get_iaudioclient()?;
    let format = WaveFormat::new(16, 16, &SampleType::Int, 48_000, 2, None);
    let block_align = usize::try_from(format.get_blockalign())?;
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
        capture.read_from_device_to_deque(&mut queue)?;
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

fn mix_audio_sources(
    game: &Receiver<Vec<u8>>,
    microphone: &Receiver<Vec<u8>>,
    output: &SyncSender<Vec<u8>>,
    stop: &AtomicBool,
) {
    let mut game_queue = VecDeque::new();
    let mut microphone_queue = VecDeque::new();
    let mut game_disconnected = false;
    let mut microphone_disconnected = false;
    while !stop.load(Ordering::Acquire) {
        game_disconnected |= drain_audio_receiver(game, &mut game_queue);
        microphone_disconnected |= drain_audio_receiver(microphone, &mut microphone_queue);
        while let Some(chunk) = next_mixed_audio_chunk(
            &mut game_queue,
            &mut microphone_queue,
            game_disconnected,
            microphone_disconnected,
        ) {
            // The video callback must never wait for audio. A full downstream queue drops the
            // oldest ready chunk here instead of allowing an unbounded mixer backlog.
            let _ = output.try_send(chunk);
        }
        thread::sleep(Duration::from_millis(2));
    }
}

fn drain_audio_receiver(receiver: &Receiver<Vec<u8>>, queue: &mut VecDeque<Vec<u8>>) -> bool {
    loop {
        match receiver.try_recv() {
            Ok(chunk) => queue.push_back(chunk),
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => return true,
        }
    }
}

fn next_mixed_audio_chunk(
    game: &mut VecDeque<Vec<u8>>,
    microphone: &mut VecDeque<Vec<u8>>,
    game_disconnected: bool,
    microphone_disconnected: bool,
) -> Option<Vec<u8>> {
    if !game.is_empty() && !microphone.is_empty() {
        return Some(mix_pcm_s16(
            &game.pop_front().expect("queue is non-empty"),
            &microphone.pop_front().expect("queue is non-empty"),
        ));
    }
    if !game.is_empty() && (microphone_disconnected || game.len() > AUDIO_MIX_JITTER_CHUNKS) {
        return game.pop_front();
    }
    if !microphone.is_empty() && (game_disconnected || microphone.len() > AUDIO_MIX_JITTER_CHUNKS) {
        return microphone.pop_front();
    }
    None
}

fn mix_pcm_s16(left: &[u8], right: &[u8]) -> Vec<u8> {
    left.chunks_exact(2)
        .zip(right.chunks_exact(2))
        .flat_map(|(left, right)| {
            let left = i16::from_le_bytes([left[0], left[1]]);
            let right = i16::from_le_bytes([right[0], right[1]]);
            let mixed = i32::from(left)
                .saturating_add(i32::from(right))
                .clamp(i32::from(i16::MIN), i32::from(i16::MAX));
            i16::try_from(mixed)
                .unwrap_or(if mixed < 0 { i16::MIN } else { i16::MAX })
                .to_le_bytes()
        })
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm_mixer_sums_and_saturates_signed_samples() {
        let left = [1_000_i16, 30_000, -30_000]
            .into_iter()
            .flat_map(i16::to_le_bytes)
            .collect::<Vec<_>>();
        let right = [2_000_i16, 10_000, -10_000]
            .into_iter()
            .flat_map(i16::to_le_bytes)
            .collect::<Vec<_>>();
        let mixed = mix_pcm_s16(&left, &right)
            .chunks_exact(2)
            .map(|sample| i16::from_le_bytes([sample[0], sample[1]]))
            .collect::<Vec<_>>();
        assert_eq!(mixed, [3_000, i16::MAX, i16::MIN]);
    }

    #[test]
    fn audio_mixer_bounds_clock_drift_with_a_small_jitter_window() {
        let mut game = VecDeque::from([vec![1], vec![2], vec![3]]);
        let mut microphone = VecDeque::new();
        assert_eq!(
            next_mixed_audio_chunk(&mut game, &mut microphone, false, false),
            Some(vec![1])
        );
        assert_eq!(game.len(), AUDIO_MIX_JITTER_CHUNKS);
        assert!(next_mixed_audio_chunk(&mut game, &mut microphone, false, false).is_none());
    }

    #[test]
    fn audio_mixer_keeps_the_surviving_source_after_endpoint_loss() {
        let mut game = VecDeque::from([vec![1, 0, 2, 0]]);
        let mut microphone = VecDeque::new();
        assert_eq!(
            next_mixed_audio_chunk(&mut game, &mut microphone, false, true),
            Some(vec![1, 0, 2, 0])
        );
    }

    #[test]
    fn stale_audio_is_discarded_before_the_first_video_frame() {
        let (sender, receiver) = sync_channel(4);
        sender.send(vec![1]).unwrap();
        sender.send(vec![2]).unwrap();
        assert_eq!(drain_pending_audio(&Mutex::new(receiver)), 2);
    }
}
