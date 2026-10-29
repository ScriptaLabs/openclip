//! Vendor-neutral H.264 encoding through Media Foundation.
//!
//! Intel, AMD and most other GPUs expose their hardware encoder as a Media Foundation transform
//! (MFT); Microsoft ships a software MFT that works on any machine. Both take NV12 system-memory
//! pictures, so the captured D3D11 frame is copied to a staging texture, converted on the CPU and
//! handed to whichever transform was picked. The output is the same independently playable
//! segments the NVENC backend produces.

use std::collections::VecDeque;
use std::mem::ManuallyDrop;
use std::time::{Duration, Instant};

use windows::Win32::Graphics::Direct3D11::{
    D3D11_CPU_ACCESS_READ, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_TEXTURE2D_DESC,
    D3D11_USAGE_STAGING, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Media::MediaFoundation::{
    CODECAPI_AVEncCommonMaxBitRate, CODECAPI_AVEncCommonMeanBitRate,
    CODECAPI_AVEncCommonRateControlMode, CODECAPI_AVEncMPVDefaultBPictureCount,
    CODECAPI_AVEncMPVGOPSize, CODECAPI_AVEncVideoForceKeyFrame, CODECAPI_AVLowLatencyMode,
    ICodecAPI, IMFActivate, IMFMediaEventGenerator, IMFMediaType, IMFSample, IMFTransform,
    METransformHaveOutput, METransformNeedInput, MF_E_NO_EVENTS_AVAILABLE, MF_E_NOTACCEPTING,
    MF_E_TRANSFORM_NEED_MORE_INPUT, MF_E_TRANSFORM_STREAM_CHANGE, MF_EVENT_FLAG_NO_WAIT,
    MF_MT_AVG_BITRATE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE,
    MF_MT_MPEG_SEQUENCE_HEADER, MF_MT_MPEG2_PROFILE, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE,
    MF_MT_VIDEO_NOMINAL_RANGE, MF_MT_VIDEO_PRIMARIES, MF_MT_YUV_MATRIX, MF_TRANSFORM_ASYNC,
    MF_TRANSFORM_ASYNC_UNLOCK, MF_VERSION, MFCreateMediaType, MFCreateMemoryBuffer, MFCreateSample,
    MFMediaType_Video, MFNominalRange_16_235, MFSTARTUP_FULL, MFShutdown, MFStartup,
    MFT_CATEGORY_VIDEO_ENCODER, MFT_ENUM_FLAG_HARDWARE, MFT_ENUM_FLAG_SORTANDFILTER,
    MFT_ENUM_FLAG_SYNCMFT, MFT_FRIENDLY_NAME_Attribute, MFT_MESSAGE_NOTIFY_BEGIN_STREAMING,
    MFT_MESSAGE_NOTIFY_START_OF_STREAM, MFT_OUTPUT_DATA_BUFFER, MFT_OUTPUT_STREAM_PROVIDES_SAMPLES,
    MFT_REGISTER_TYPE_INFO, MFTEnumEx, MFVideoFormat_H264, MFVideoFormat_NV12,
    MFVideoInterlace_Progressive, MFVideoPrimaries_BT709, MFVideoTransferMatrix_BT709,
    eAVEncCommonRateControlMode_PeakConstrainedVBR, eAVEncH264VProfile_Main,
};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Variant::VARIANT;
use windows::core::Interface;
use windows_capture::frame::Frame;

use crate::config::EncoderConfig;
use crate::h264::{
    BackendError, VideoSegment, annex_b_nals, annex_b_to_length_prefixed, assemble_segment,
    avcc_from_annex_b,
};
use crate::pixel::{Geometry, bgra_to_nv12};

const HUNDRED_NS_PER_SECOND: i64 = 10_000_000;
/// A transform that stops answering must not freeze the capture callback forever.
const STALL_TIMEOUT: Duration = Duration::from_secs(3);
const IDR_NAL_TYPE: u8 = 5;

/// Which class of Media Foundation transform to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// The GPU vendor's encoder (Intel Quick Sync, AMD AMF, NVIDIA and others).
    Hardware,
    /// Microsoft's CPU encoder, available on every Windows install.
    Software,
}

/// Starts Media Foundation for as long as an encoder lives.
struct Runtime;

impl Runtime {
    fn start() -> Result<Self, BackendError> {
        unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) }
            .map_err(|error| format!("could not start Media Foundation: {error}"))?;
        Ok(Self)
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        let _ = unsafe { MFShutdown() };
    }
}

pub(crate) struct MfEncoder {
    transform: IMFTransform,
    events: Option<IMFMediaEventGenerator>,
    codec_api: Option<ICodecAPI>,
    provides_samples: bool,
    output_buffer_size: u32,
    name: String,
    width: u32,
    height: u32,
    frames_per_second: u16,
    segment_frames: usize,
    staging: Option<ID3D11Texture2D>,
    nv12: Vec<u8>,
    /// Pictures the transform has asked for but not yet received (asynchronous transforms).
    input_credits: u32,
    submitted: i64,
    since_key: usize,
    avcc: Option<Vec<u8>>,
    sequence_header: Option<Vec<u8>>,
    current: Vec<Vec<u8>>,
    ready: VecDeque<VideoSegment>,
    // Dropped last: the transform must go before Media Foundation shuts down.
    _runtime: Runtime,
}

// Created and used only on the capture thread, like the NVENC encoder.
unsafe impl Send for MfEncoder {}

impl MfEncoder {
    /// Opens the first transform of `kind` that accepts the configuration.
    pub(crate) fn new(config: EncoderConfig, kind: Kind) -> Result<Self, BackendError> {
        let _enumeration = Runtime::start()?;
        let candidates = enumerate(kind)?;
        if candidates.is_empty() {
            return Err(match kind {
                Kind::Hardware => "no hardware H.264 encoder is registered with Windows",
                Kind::Software => "the Windows software H.264 encoder is unavailable",
            }
            .into());
        }
        let mut failures = Vec::new();
        for activate in candidates {
            let name = friendly_name(&activate);
            match Self::open(&activate, &name, config) {
                Ok(encoder) => return Ok(encoder),
                Err(error) => {
                    tracing::warn!(encoder = %name, %error, "Media Foundation encoder rejected");
                    failures.push(format!("{name}: {error}"));
                }
            }
        }
        Err(failures.join("; ").into())
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    fn open(
        activate: &IMFActivate,
        name: &str,
        config: EncoderConfig,
    ) -> Result<Self, BackendError> {
        let runtime = Runtime::start()?;
        let transform: IMFTransform = unsafe { activate.ActivateObject() }?;
        let width = config.width & !1;
        let height = config.height & !1;
        if width == 0 || height == 0 {
            return Err("the capture size is too small to encode".into());
        }

        let is_async = unsafe { transform.GetAttributes() }.is_ok_and(|attributes| {
            unsafe { attributes.GetUINT32(&MF_TRANSFORM_ASYNC) }.unwrap_or(0) == 1
        });
        if is_async {
            let attributes = unsafe { transform.GetAttributes() }?;
            unsafe { attributes.SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1) }?;
        }

        // Encoders require the output type first, then accept an input type that matches it.
        let output = media_type(width, height, config.frames_per_second)?;
        unsafe {
            output.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
            output.SetUINT32(&MF_MT_AVG_BITRATE, config.bitrate)?;
            output.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_Main.0 as u32)?;
            transform.SetOutputType(0, &output, 0)?;
        }
        let input = media_type(width, height, config.frames_per_second)?;
        unsafe { input.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)? };
        // Colour hints are optional; some transforms refuse them, so fall back to a plain type.
        let colour_ok = unsafe {
            input.SetUINT32(&MF_MT_VIDEO_PRIMARIES, MFVideoPrimaries_BT709.0 as u32)?;
            input.SetUINT32(&MF_MT_YUV_MATRIX, MFVideoTransferMatrix_BT709.0 as u32)?;
            input.SetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE, MFNominalRange_16_235.0 as u32)?;
            transform.SetInputType(0, &input, 0)
        };
        if colour_ok.is_err() {
            let plain = media_type(width, height, config.frames_per_second)?;
            unsafe {
                plain.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
                transform.SetInputType(0, &plain, 0)?;
            }
        }

        let codec_api = transform.cast::<ICodecAPI>().ok();
        let segment_frames = usize::from(config.frames_per_second).saturating_mul(2);
        if let Some(api) = codec_api.as_ref() {
            apply_codec_settings(api, config, segment_frames);
        }

        let stream_info = unsafe { transform.GetOutputStreamInfo(0) }?;
        let provides_samples =
            stream_info.dwFlags & MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32 != 0;
        let events = if is_async {
            Some(transform.cast::<IMFMediaEventGenerator>()?)
        } else {
            None
        };
        unsafe {
            transform.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
            transform.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)?;
        }
        let sequence_header = sequence_header_of(&transform);
        Ok(Self {
            transform,
            events,
            codec_api,
            provides_samples,
            output_buffer_size: stream_info.cbSize.max(1 << 20),
            name: name.to_owned(),
            width,
            height,
            frames_per_second: config.frames_per_second,
            segment_frames,
            staging: None,
            nv12: Vec::new(),
            input_credits: 0,
            submitted: 0,
            since_key: 0,
            avcc: None,
            sequence_header,
            current: Vec::new(),
            ready: VecDeque::new(),
            _runtime: runtime,
        })
    }

    pub(crate) fn encode_frame(
        &mut self,
        frame: &Frame<'_>,
        copies: u32,
    ) -> Result<Option<VideoSegment>, BackendError> {
        if copies > 0 {
            self.convert(frame)?;
        }
        self.encode_converted(copies)
    }

    /// Encodes the picture last converted, `copies` times in a row.
    fn encode_converted(&mut self, copies: u32) -> Result<Option<VideoSegment>, BackendError> {
        for _ in 0..copies {
            self.submit()?;
        }
        self.collect_outputs()?;
        Ok(self.ready.pop_front())
    }

    pub(crate) fn finish_segment(&mut self) -> Result<Option<VideoSegment>, BackendError> {
        if let Some(segment) = self.ready.pop_front() {
            return Ok(Some(segment));
        }
        self.close_segment()?;
        Ok(self.ready.pop_front())
    }

    /// Copies the GPU frame to system memory and converts it to NV12.
    fn convert(&mut self, frame: &Frame<'_>) -> Result<(), BackendError> {
        self.convert_texture(
            frame.device(),
            frame.device_context(),
            frame.as_raw_texture(),
            (frame.width() as usize, frame.height() as usize),
        )
    }

    fn convert_texture(
        &mut self,
        device: &ID3D11Device,
        context: &ID3D11DeviceContext,
        texture: &ID3D11Texture2D,
        (source_width, source_height): (usize, usize),
    ) -> Result<(), BackendError> {
        if self.staging.is_none() {
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            unsafe { texture.GetDesc(&raw mut desc) };
            desc.Usage = D3D11_USAGE_STAGING;
            desc.BindFlags = 0;
            desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
            desc.MiscFlags = 0;
            let mut staging = None;
            unsafe { device.CreateTexture2D(&raw const desc, None, Some(&raw mut staging)) }?;
            self.staging = Some(staging.ok_or("D3D11 did not create a staging texture")?);
        }
        let staging = self.staging.as_ref().ok_or("staging texture missing")?;
        unsafe { context.CopyResource(staging, texture) };
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe { context.Map(staging, 0, D3D11_MAP_READ, 0, Some(&raw mut mapped)) }?;
        let pitch = mapped.RowPitch as usize;
        let bytes =
            unsafe { std::slice::from_raw_parts(mapped.pData.cast::<u8>(), pitch * source_height) };
        bgra_to_nv12(
            bytes,
            Geometry {
                source_width,
                source_height,
                source_pitch: pitch,
                width: self.width as usize,
                height: self.height as usize,
            },
            &mut self.nv12,
        );
        unsafe { context.Unmap(staging, 0) };
        Ok(())
    }

    fn submit(&mut self) -> Result<(), BackendError> {
        if self.since_key >= self.segment_frames {
            self.request_key_frame();
        }
        let sample = self.input_sample()?;
        if let Some(events) = self.events.clone() {
            let deadline = Instant::now() + STALL_TIMEOUT;
            while self.input_credits == 0 {
                self.poll_events(&events)?;
                if self.input_credits == 0 {
                    if Instant::now() > deadline {
                        return Err(format!("{} stopped accepting frames", self.name).into());
                    }
                    std::thread::sleep(Duration::from_micros(500));
                }
            }
            unsafe { self.transform.ProcessInput(0, &sample, 0) }?;
            self.input_credits -= 1;
        } else {
            match unsafe { self.transform.ProcessInput(0, &sample, 0) } {
                Ok(()) => {}
                Err(error) if error.code() == MF_E_NOTACCEPTING => {
                    self.collect_outputs()?;
                    unsafe { self.transform.ProcessInput(0, &sample, 0) }?;
                }
                Err(error) => return Err(error.into()),
            }
        }
        self.submitted += 1;
        self.since_key += 1;
        Ok(())
    }

    fn request_key_frame(&mut self) {
        if let Some(api) = self.codec_api.as_ref() {
            let value = VARIANT::from(1_u32);
            if let Err(error) =
                unsafe { api.SetValue(&CODECAPI_AVEncVideoForceKeyFrame, &raw const value) }
            {
                tracing::debug!(%error, "encoder ignored a key frame request");
            }
        }
        self.since_key = 0;
    }

    fn input_sample(&self) -> Result<IMFSample, BackendError> {
        let length = u32::try_from(self.nv12.len())?;
        let buffer = unsafe { MFCreateMemoryBuffer(length) }?;
        let mut target = std::ptr::null_mut();
        unsafe { buffer.Lock(&raw mut target, None, None) }?;
        unsafe { std::ptr::copy_nonoverlapping(self.nv12.as_ptr(), target, self.nv12.len()) };
        unsafe {
            buffer.Unlock()?;
            buffer.SetCurrentLength(length)?;
        }
        let sample = unsafe { MFCreateSample() }?;
        let duration = HUNDRED_NS_PER_SECOND / i64::from(self.frames_per_second.max(1));
        unsafe {
            sample.AddBuffer(&buffer)?;
            sample.SetSampleTime(self.submitted * duration)?;
            sample.SetSampleDuration(duration)?;
        }
        Ok(sample)
    }

    /// Drains whatever the transform has finished without waiting for more.
    fn collect_outputs(&mut self) -> Result<(), BackendError> {
        if let Some(events) = self.events.clone() {
            self.poll_events(&events)
        } else {
            while let Some(picture) = self.process_output()? {
                self.accept(&picture)?;
            }
            Ok(())
        }
    }

    fn poll_events(&mut self, events: &IMFMediaEventGenerator) -> Result<(), BackendError> {
        loop {
            let event = match unsafe { events.GetEvent(MF_EVENT_FLAG_NO_WAIT) } {
                Ok(event) => event,
                Err(error) if error.code() == MF_E_NO_EVENTS_AVAILABLE => return Ok(()),
                Err(error) => return Err(error.into()),
            };
            let kind = unsafe { event.GetType() }?;
            if kind == METransformNeedInput.0 as u32 {
                self.input_credits += 1;
            } else if kind == METransformHaveOutput.0 as u32 {
                if let Some(picture) = self.process_output()? {
                    self.accept(&picture)?;
                }
            } else if let Ok(status) = unsafe { event.GetStatus() }
                && status.is_err()
            {
                return Err(format!("{} reported an error: {status:?}", self.name).into());
            }
        }
    }

    /// One encoded picture in Annex B form, or `None` when the transform has nothing ready.
    fn process_output(&mut self) -> Result<Option<Vec<u8>>, BackendError> {
        let own_sample = if self.provides_samples {
            None
        } else {
            let buffer = unsafe { MFCreateMemoryBuffer(self.output_buffer_size) }?;
            let sample = unsafe { MFCreateSample() }?;
            unsafe { sample.AddBuffer(&buffer) }?;
            Some(sample)
        };
        let mut output = [MFT_OUTPUT_DATA_BUFFER {
            dwStreamID: 0,
            pSample: ManuallyDrop::new(own_sample),
            dwStatus: 0,
            pEvents: ManuallyDrop::new(None),
        }];
        let mut status = 0_u32;
        let result = unsafe {
            self.transform
                .ProcessOutput(0, &mut output, &raw mut status)
        };
        let sample = unsafe { ManuallyDrop::take(&mut output[0].pSample) };
        drop(unsafe { ManuallyDrop::take(&mut output[0].pEvents) });
        match result {
            Ok(()) => {}
            Err(error) if error.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(None),
            Err(error) if error.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                let negotiated = unsafe { self.transform.GetOutputAvailableType(0, 0) }?;
                unsafe { self.transform.SetOutputType(0, &negotiated, 0) }?;
                self.sequence_header = sequence_header_of(&self.transform);
                return Ok(None);
            }
            Err(error) => return Err(error.into()),
        }
        let Some(sample) = sample else {
            return Ok(None);
        };
        let buffer = unsafe { sample.ConvertToContiguousBuffer() }?;
        let mut data = std::ptr::null_mut();
        let mut length = 0_u32;
        unsafe { buffer.Lock(&raw mut data, None, Some(&raw mut length)) }?;
        let bytes = unsafe { std::slice::from_raw_parts(data, length as usize) }.to_vec();
        unsafe { buffer.Unlock() }?;
        Ok(Some(bytes))
    }

    /// Places an encoded picture in the segment being built, cutting a new segment at each IDR.
    fn accept(&mut self, annex_b: &[u8]) -> Result<(), BackendError> {
        let is_idr = annex_b_nals(annex_b)
            .iter()
            .any(|nal| nal.first().is_some_and(|byte| byte & 0x1f == IDR_NAL_TYPE));
        if is_idr {
            self.close_segment()?;
            if self.avcc.is_none() {
                self.avcc = Some(self.parameter_sets(annex_b)?);
            }
        } else if self.current.is_empty() {
            // Pictures before the first IDR cannot start a segment.
            return Ok(());
        }
        self.current.push(annex_b_to_length_prefixed(annex_b)?);
        if self.current.len() > self.segment_frames.saturating_mul(4) {
            return Err(format!("{} stopped producing key frames", self.name).into());
        }
        Ok(())
    }

    fn parameter_sets(&self, first_idr: &[u8]) -> Result<Vec<u8>, BackendError> {
        avcc_from_annex_b(first_idr).or_else(|inline_error| {
            self.sequence_header
                .as_deref()
                .ok_or(inline_error)
                .and_then(avcc_from_annex_b)
        })
    }

    fn close_segment(&mut self) -> Result<(), BackendError> {
        if self.current.is_empty() {
            return Ok(());
        }
        let avcc = self
            .avcc
            .as_ref()
            .ok_or("the encoder did not emit H.264 parameter sets")?;
        let frames = std::mem::take(&mut self.current);
        self.ready.push_back(assemble_segment(
            avcc,
            (self.width, self.height),
            self.frames_per_second,
            &frames,
        )?);
        Ok(())
    }
}

/// Best-effort tuning: transforms ignore the settings they do not support.
fn apply_codec_settings(api: &ICodecAPI, config: EncoderConfig, segment_frames: usize) {
    let set = |key: &windows::core::GUID, value: VARIANT, label: &str| {
        if let Err(error) = unsafe { api.SetValue(key, &raw const value) } {
            tracing::debug!(%error, setting = label, "encoder ignored a setting");
        }
    };
    set(
        &CODECAPI_AVLowLatencyMode,
        VARIANT::from(true),
        "low latency",
    );
    set(
        &CODECAPI_AVEncCommonRateControlMode,
        VARIANT::from(eAVEncCommonRateControlMode_PeakConstrainedVBR.0 as u32),
        "rate control",
    );
    set(
        &CODECAPI_AVEncCommonMeanBitRate,
        VARIANT::from(config.bitrate),
        "bitrate",
    );
    set(
        &CODECAPI_AVEncCommonMaxBitRate,
        VARIANT::from(config.bitrate.saturating_add(config.bitrate / 2)),
        "peak bitrate",
    );
    set(
        &CODECAPI_AVEncMPVGOPSize,
        VARIANT::from(u32::try_from(segment_frames).unwrap_or(120)),
        "gop size",
    );
    set(
        &CODECAPI_AVEncMPVDefaultBPictureCount,
        VARIANT::from(0_u32),
        "b frames",
    );
}

fn media_type(width: u32, height: u32, fps: u16) -> Result<IMFMediaType, BackendError> {
    let media = unsafe { MFCreateMediaType() }?;
    unsafe {
        media.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        media.SetUINT64(&MF_MT_FRAME_SIZE, pack(width, height))?;
        media.SetUINT64(&MF_MT_FRAME_RATE, pack(u32::from(fps), 1))?;
        media.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1))?;
        media.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
    }
    Ok(media)
}

const fn pack(high: u32, low: u32) -> u64 {
    ((high as u64) << 32) | low as u64
}

fn sequence_header_of(transform: &IMFTransform) -> Option<Vec<u8>> {
    let current = unsafe { transform.GetOutputCurrentType(0) }.ok()?;
    let size = unsafe { current.GetBlobSize(&MF_MT_MPEG_SEQUENCE_HEADER) }.ok()?;
    let mut blob = vec![0_u8; size as usize];
    unsafe { current.GetBlob(&MF_MT_MPEG_SEQUENCE_HEADER, &mut blob, None) }.ok()?;
    Some(blob)
}

fn friendly_name(activate: &IMFActivate) -> String {
    let mut name = windows::core::PWSTR::null();
    let mut length = 0_u32;
    unsafe {
        if activate
            .GetAllocatedString(&MFT_FRIENDLY_NAME_Attribute, &raw mut name, &raw mut length)
            .is_ok()
        {
            let text = name.to_string().unwrap_or_default();
            CoTaskMemFree(Some(name.0.cast_const().cast()));
            if !text.is_empty() {
                return text;
            }
        }
    }
    "H.264 encoder".to_owned()
}

fn enumerate(kind: Kind) -> Result<Vec<IMFActivate>, BackendError> {
    let flags = match kind {
        Kind::Hardware => MFT_ENUM_FLAG_HARDWARE,
        Kind::Software => MFT_ENUM_FLAG_SYNCMFT,
    } | MFT_ENUM_FLAG_SORTANDFILTER;
    let output = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: MFVideoFormat_H264,
    };
    let mut array = std::ptr::null_mut::<Option<IMFActivate>>();
    let mut count = 0_u32;
    unsafe {
        MFTEnumEx(
            MFT_CATEGORY_VIDEO_ENCODER,
            flags,
            None,
            Some(&raw const output),
            &raw mut array,
            &raw mut count,
        )
    }?;
    let mut found = Vec::new();
    if !array.is_null() {
        for index in 0..count as usize {
            if let Some(activate) = unsafe { array.add(index).read() } {
                found.push(activate);
            }
        }
        unsafe { CoTaskMemFree(Some(array.cast_const().cast())) };
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDTH: u32 = 640;
    const HEIGHT: u32 = 360;
    const FPS: u16 = 30;

    fn config() -> EncoderConfig {
        EncoderConfig {
            width: WIDTH,
            height: HEIGHT,
            frames_per_second: FPS,
            bitrate: 4_000_000,
            capture_game_audio: false,
            capture_microphone: false,
        }
    }

    /// A moving gradient, so every picture differs and the encoder has real work to do.
    #[allow(clippy::cast_possible_truncation)]
    fn load_picture(encoder: &mut MfEncoder, index: usize) {
        let (width, height) = (WIDTH as usize, HEIGHT as usize);
        let mut bgra = vec![255_u8; width * height * 4];
        for y in 0..height {
            for x in 0..width {
                let at = (y * width + x) * 4;
                bgra[at] = ((x + index * 4) % 256) as u8;
                bgra[at + 1] = ((y + index * 2) % 256) as u8;
                bgra[at + 2] = ((x + y) % 256) as u8;
            }
        }
        bgra_to_nv12(
            &bgra,
            Geometry {
                source_width: width,
                source_height: height,
                source_pitch: width * 4,
                width,
                height,
            },
            &mut encoder.nv12,
        );
    }

    /// Encodes five seconds and checks that the output is a run of two-second, IDR-led segments.
    fn exercise(kind: Kind) -> Option<Vec<VideoSegment>> {
        let mut encoder = match MfEncoder::new(config(), kind) {
            Ok(encoder) => encoder,
            Err(error) => {
                eprintln!("{kind:?} encoder not available here: {error}");
                return None;
            }
        };
        eprintln!("using {}", encoder.name());
        let mut segments = Vec::new();
        for index in 0..(usize::from(FPS) * 5) {
            load_picture(&mut encoder, index);
            segments.extend(encoder.encode_converted(1).expect("frame encodes"));
        }
        segments.extend(encoder.finish_segment().expect("segment finishes"));
        while let Some(segment) = encoder.ready.pop_front() {
            segments.push(segment);
        }
        Some(segments)
    }

    fn check(segments: &[VideoSegment]) {
        assert!(segments.len() >= 2, "got {} segments", segments.len());
        let frames: u32 = segments.iter().map(|segment| segment.frame_count).sum();
        assert!(
            frames >= u32::from(FPS) * 3,
            "only {frames} frames came out"
        );
        for segment in segments {
            let mp4 = mp4io::Mp4::parse(&segment.bytes).expect("segment is valid MP4");
            let track = mp4.tracks().first().expect("video track");
            let first = track.samples().next().expect("sample").expect("parses");
            assert!(first.is_sync, "every segment must start on an IDR");
        }
        if let Ok(dir) = std::env::var("OPENCLIP_PROBE_DIR") {
            for (index, segment) in segments.iter().enumerate() {
                std::fs::write(format!("{dir}/segment-{index}.mp4"), &segment.bytes)
                    .expect("probe output is writable");
            }
        }
    }

    #[test]
    fn a_gpu_texture_is_read_back_and_converted() {
        use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
        use windows::Win32::Graphics::Direct3D11::{
            D3D11_BIND_SHADER_RESOURCE, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
            D3D11_SUBRESOURCE_DATA, D3D11_USAGE_DEFAULT, D3D11CreateDevice,
        };
        use windows::Win32::Graphics::Dxgi::Common::{
            DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
        };

        let (mut device, mut context) = (None, None);
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                windows::Win32::Foundation::HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&raw mut device),
                None,
                Some(&raw mut context),
            )
        }
        .expect("a Direct3D 11 device");
        let (device, context) = (device.unwrap(), context.unwrap());
        let pixels = vec![255_u8; WIDTH as usize * HEIGHT as usize * 4];
        let desc = D3D11_TEXTURE2D_DESC {
            Width: WIDTH,
            Height: HEIGHT,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            ..Default::default()
        };
        let data = D3D11_SUBRESOURCE_DATA {
            pSysMem: pixels.as_ptr().cast(),
            SysMemPitch: WIDTH * 4,
            SysMemSlicePitch: 0,
        };
        let mut texture = None;
        unsafe {
            device.CreateTexture2D(
                &raw const desc,
                Some(&raw const data),
                Some(&raw mut texture),
            )
        }
        .expect("a texture");
        let texture = texture.unwrap();

        let mut encoder = MfEncoder::new(config(), Kind::Software).expect("software encoder");
        encoder
            .convert_texture(
                &device,
                &context,
                &texture,
                (WIDTH as usize, HEIGHT as usize),
            )
            .expect("texture converts");
        let luma = WIDTH as usize * HEIGHT as usize;
        assert_eq!(encoder.nv12.len(), luma * 3 / 2);
        assert!(
            encoder.nv12[..luma].iter().all(|&y| y == 235),
            "white is limited-range white"
        );
        // A second conversion reuses the staging texture.
        encoder
            .convert_texture(
                &device,
                &context,
                &texture,
                (WIDTH as usize, HEIGHT as usize),
            )
            .expect("texture converts again");
    }

    #[test]
    fn the_software_encoder_makes_independent_segments() {
        let segments =
            exercise(Kind::Software).expect("Windows always ships a software H.264 encoder");
        check(&segments);
    }

    #[test]
    fn a_hardware_encoder_makes_independent_segments_when_the_machine_has_one() {
        if let Some(segments) = exercise(Kind::Hardware) {
            check(&segments);
        }
    }

    /// Timing at 1080p, to size the cost of the CPU-side path: `cargo test --release -- --ignored --nocapture`.
    #[test]
    #[ignore = "benchmark"]
    fn timing_at_1080p() {
        for (kind, fps) in [(Kind::Hardware, 60), (Kind::Software, 30)] {
            let config = EncoderConfig {
                width: 1_920,
                height: 1_080,
                frames_per_second: fps,
                bitrate: 24_000_000,
                capture_game_audio: false,
                capture_microphone: false,
            };
            let Ok(mut encoder) = MfEncoder::new(config, kind) else {
                continue;
            };
            let bgra = vec![128_u8; 1_920 * 1_080 * 4];
            let geometry = Geometry {
                source_width: 1_920,
                source_height: 1_080,
                source_pitch: 1_920 * 4,
                width: 1_920,
                height: 1_080,
            };
            let started = Instant::now();
            for _ in 0..30 {
                bgra_to_nv12(&bgra, geometry, &mut encoder.nv12);
            }
            let convert = started.elapsed() / 30;
            let started = Instant::now();
            for _ in 0..120 {
                encoder.encode_converted(1).expect("frame encodes");
            }
            eprintln!(
                "{kind:?} {} 1080p{fps}: convert {convert:?}/frame, encode {:?}/frame",
                encoder.name(),
                started.elapsed() / 120
            );
        }
    }
}
