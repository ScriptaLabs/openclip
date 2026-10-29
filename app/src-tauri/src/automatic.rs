use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use openclip_buffer::{CaptureJob, MomentEngine};
use openclip_capture::RecorderService;
use openclip_core::{GameStateUpdate, MatchRepository, Moment, Settings};
use openclip_storage::SqliteRepository;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AutomaticMomentResult {
    pub moment: Moment,
    /// True whenever a clip exists, including a partial one.
    pub captured: bool,
    pub status: CaptureStatus,
    /// Footage missing from the start of a partial clip.
    pub missing_ms: u64,
    pub error: Option<String>,
}

/// What the user can rely on after a moment closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CaptureStatus {
    /// The full requested window was saved.
    Captured,
    /// A clip exists but starts late or skips footage because the buffer did not cover it.
    Partial,
    /// No clip was saved; the logical moment is still recorded.
    Failed,
}

enum Command {
    Update(GameStateUpdate),
    Settings(Settings),
}

#[derive(Clone)]
pub(crate) struct AutomaticMomentService {
    sender: Sender<Command>,
}

impl AutomaticMomentService {
    pub(crate) fn start(
        repository: Arc<Mutex<SqliteRepository>>,
        recorder: RecorderService,
        settings: Settings,
        notify: Arc<dyn Fn(AutomaticMomentResult) + Send + Sync>,
    ) -> openclip_core::Result<Self> {
        let (sender, receiver) = channel();
        let (finalize_sender, finalize_receiver) = channel();
        let save_repository = repository.clone();
        let save_recorder = recorder.clone();
        thread::Builder::new()
            .name("openclip-highlight-saver".to_owned())
            .spawn(move || {
                save_worker(
                    &finalize_receiver,
                    &save_repository,
                    &save_recorder,
                    notify.as_ref(),
                );
            })
            .map_err(|error| openclip_core::OpenClipError::Unavailable(error.to_string()))?;
        thread::Builder::new()
            .name("openclip-moment-engine".to_owned())
            .spawn(move || moment_worker(&receiver, &finalize_sender, &recorder, &settings))
            .map_err(|error| openclip_core::OpenClipError::Unavailable(error.to_string()))?;
        Ok(Self { sender })
    }

    pub(crate) fn observe(&self, update: &GameStateUpdate) {
        if let Err(error) = self.sender.send(Command::Update(update.clone())) {
            tracing::error!(%error, "automatic moment engine is unavailable");
        }
    }

    pub(crate) fn update_settings(&self, settings: Settings) {
        if let Err(error) = self.sender.send(Command::Settings(settings)) {
            tracing::error!(%error, "automatic moment settings could not be applied");
        }
    }
}

fn moment_worker(
    receiver: &Receiver<Command>,
    finalize_sender: &Sender<CaptureJob>,
    recorder: &RecorderService,
    settings: &Settings,
) {
    let mut engine = MomentEngine::new(settings);
    loop {
        let command = match engine.next_deadline_ms() {
            None => receiver.recv().ok(),
            Some(deadline) => {
                let wait = deadline.saturating_sub(recorder.timeline_now_ms());
                match receiver.recv_timeout(Duration::from_millis(wait)) {
                    Ok(command) => Some(command),
                    Err(RecvTimeoutError::Timeout) => {
                        if let Some(job) = engine.expire(recorder.timeline_now_ms()) {
                            send_finalize(finalize_sender, job);
                        }
                        continue;
                    }
                    Err(RecvTimeoutError::Disconnected) => None,
                }
            }
        };
        let Some(command) = command else {
            break;
        };
        match command {
            Command::Settings(settings) => engine.update_settings(&settings),
            Command::Update(update) => {
                let now_ms = recorder.timeline_now_ms();
                for job in engine.ingest(update.events, now_ms) {
                    send_finalize(finalize_sender, job);
                }
            }
        }
    }
    if let Some(job) = engine.flush() {
        send_finalize(finalize_sender, job);
    }
}

fn send_finalize(sender: &Sender<CaptureJob>, job: CaptureJob) {
    if let Err(error) = sender.send(job) {
        tracing::error!(%error, "automatic highlight save worker is unavailable");
    }
}

fn save_worker(
    receiver: &Receiver<CaptureJob>,
    repository: &Arc<Mutex<SqliteRepository>>,
    recorder: &RecorderService,
    notify: &(dyn Fn(AutomaticMomentResult) + Send + Sync),
) {
    while let Ok(job) = receiver.recv() {
        let mut moment = job.moment;
        let result = recorder.save_range(job.capture_range, moment.match_id, moment.id);
        let (status, missing_ms, error) = match result {
            Ok(saved) => {
                let complete = saved.is_complete();
                let missing_ms = saved.leading_shortfall_ms;
                let clip = saved.clip;
                moment.clip_id = Some(clip.id);
                moment.clip_path = Some(clip.path.clone());
                let persisted = repository
                    .lock()
                    .map_err(|_| "database lock was poisoned".to_owned())
                    .and_then(|mut repository| {
                        repository
                            .save_moment_with_clip(&moment, &clip)
                            .map_err(|error| error.to_string())
                    });
                match persisted {
                    Ok(()) if complete => (CaptureStatus::Captured, 0, None),
                    Ok(()) => (CaptureStatus::Partial, missing_ms, None),
                    Err(error) => (CaptureStatus::Failed, 0, Some(error)),
                }
            }
            Err(capture_error) => {
                let persistence = repository
                    .lock()
                    .map_err(|_| "database lock was poisoned".to_owned())
                    .and_then(|mut repository| {
                        repository
                            .save_moment(&moment)
                            .map_err(|error| error.to_string())
                    });
                let error = persistence.map_or_else(
                    |storage_error| format!("{capture_error}; {storage_error}"),
                    |()| capture_error.to_string(),
                );
                (CaptureStatus::Failed, 0, Some(error))
            }
        };
        let captured = status != CaptureStatus::Failed;
        tracing::info!(
            moment_id = %moment.id,
            classification = ?moment.classification,
            ?status,
            missing_ms,
            error,
            "automatic moment finalized"
        );
        notify(AutomaticMomentResult {
            moment,
            captured,
            status,
            missing_ms,
            error,
        });
    }
}
