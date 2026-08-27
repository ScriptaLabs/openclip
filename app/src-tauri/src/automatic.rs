use std::ops::RangeInclusive;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use openclip_buffer::WindowedMomentDetector;
use openclip_capture::RecorderService;
use openclip_core::{
    GameEventType, GameStateUpdate, MatchRepository, Moment, MomentDetector, MomentId, Settings,
};
use openclip_storage::SqliteRepository;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AutomaticMomentResult {
    pub moment: Moment,
    pub captured: bool,
    pub error: Option<String>,
}

enum Command {
    Update(GameStateUpdate),
    Settings(Settings),
}

struct FinalizeJob {
    moment: Moment,
    capture_range: RangeInclusive<u64>,
}

#[derive(Debug, Clone, Copy)]
struct ActiveCaptureRange {
    moment_id: MomentId,
    start_ms: u64,
    end_ms: u64,
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
            .spawn(move || moment_worker(&receiver, &finalize_sender, &recorder, settings))
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
    finalize_sender: &Sender<FinalizeJob>,
    recorder: &RecorderService,
    settings: Settings,
) {
    let mut detector = WindowedMomentDetector::from_settings(&settings);
    let mut active_range: Option<ActiveCaptureRange> = None;
    loop {
        let wait = active_range.map_or(Duration::MAX, |range| {
            Duration::from_millis(range.end_ms.saturating_sub(recorder.timeline_now_ms()))
        });
        let command = if wait == Duration::MAX {
            receiver.recv().ok()
        } else {
            match receiver.recv_timeout(wait) {
                Ok(command) => Some(command),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    if let Some(active) = detector.active() {
                        let end = active.end_offset_ms;
                        if let Some(moment) = detector.finalize_expired(end)
                            && let Some(range) = active_range.take()
                        {
                            send_finalize(finalize_sender, moment, range);
                        }
                    }
                    continue;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => None,
            }
        };
        let Some(command) = command else {
            break;
        };
        match command {
            Command::Settings(settings) => detector.update_settings(&settings),
            Command::Update(update) => {
                for event in update.events {
                    let observed_ms = recorder.timeline_now_ms();
                    let before_id = detector.active().map(|moment| moment.id);
                    let before_events = detector.active().map_or(0, |moment| moment.events.len());
                    match detector.ingest(event) {
                        Ok(finalized) => {
                            if let Some(moment) = finalized
                                && let Some(range) = active_range.take()
                            {
                                send_finalize(finalize_sender, moment, range);
                            }
                        }
                        Err(error) => {
                            tracing::error!(%error, "automatic moment event was rejected");
                            continue;
                        }
                    }
                    let Some(active) = detector.active() else {
                        continue;
                    };
                    let after_id = Some(active.id);
                    if before_id != after_id {
                        let event_offset = active
                            .events
                            .last()
                            .and_then(|item| item.match_offset_ms)
                            .unwrap_or(active.start_offset_ms);
                        active_range = Some(ActiveCaptureRange {
                            moment_id: active.id,
                            start_ms: observed_ms.saturating_sub(
                                event_offset.saturating_sub(active.start_offset_ms),
                            ),
                            end_ms: observed_ms
                                .saturating_add(active.end_offset_ms.saturating_sub(event_offset)),
                        });
                    } else if active.events.len() > before_events
                        && matches!(
                            active.events.last().map(|item| item.event_type),
                            Some(GameEventType::Kill | GameEventType::Headshot)
                        )
                        && let Some(range) = active_range.as_mut()
                    {
                        let event_offset = active
                            .events
                            .last()
                            .and_then(|item| item.match_offset_ms)
                            .unwrap_or(active.end_offset_ms);
                        range.end_ms = observed_ms
                            .saturating_add(active.end_offset_ms.saturating_sub(event_offset));
                    }
                }
            }
        }
    }
}

fn send_finalize(sender: &Sender<FinalizeJob>, moment: Moment, range: ActiveCaptureRange) {
    debug_assert_eq!(moment.id, range.moment_id);
    if let Err(error) = sender.send(FinalizeJob {
        moment,
        capture_range: range.start_ms..=range.end_ms,
    }) {
        tracing::error!(%error, "automatic highlight save worker is unavailable");
    }
}

fn save_worker(
    receiver: &Receiver<FinalizeJob>,
    repository: &Arc<Mutex<SqliteRepository>>,
    recorder: &RecorderService,
    notify: &(dyn Fn(AutomaticMomentResult) + Send + Sync),
) {
    while let Ok(job) = receiver.recv() {
        let mut moment = job.moment;
        let result = recorder.save_range(job.capture_range, moment.match_id, moment.id);
        let (captured, error) = match result {
            Ok(clip) => {
                moment.clip_id = Some(clip.id);
                moment.clip_path = Some(clip.path.clone());
                let saved = repository
                    .lock()
                    .map_err(|_| "database lock was poisoned".to_owned())
                    .and_then(|mut repository| {
                        repository
                            .save_moment_with_clip(&moment, &clip)
                            .map_err(|error| error.to_string())
                    });
                match saved {
                    Ok(()) => (true, None),
                    Err(error) => (false, Some(error)),
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
                (false, Some(error))
            }
        };
        tracing::info!(
            moment_id = %moment.id,
            classification = ?moment.classification,
            captured,
            error,
            "automatic moment finalized"
        );
        notify(AutomaticMomentResult {
            moment,
            captured,
            error,
        });
    }
}
