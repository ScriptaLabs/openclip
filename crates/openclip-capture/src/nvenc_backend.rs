#![allow(unsafe_code)]

use std::ffi::c_void;
use std::mem::ManuallyDrop;
use std::mem::MaybeUninit;
use std::ptr::NonNull;

use nvenc::sys::enums::{
    NVencBufferFormat, NVencBufferUsage, NVencDeviceType, NVencInputResourceType,
    NVencOutputStatsLevel, NVencParamsRcMode, NVencPicFlags, NVencPicStruct, NVencPicType,
    NVencTuningInfo,
};
use nvenc::sys::function_table::NVencFunctionList;
use nvenc::sys::guids::{NV_ENC_CODEC_H264_GUID, NV_ENC_PRESET_P4_GUID};
use nvenc::sys::structs::{
    NV_ENC_CONFIG_VER, NV_ENC_CREATE_BITSTREAM_BUFFER_VER, NV_ENC_INITIALIZE_PARAMS_VER,
    NV_ENC_LOCK_BITSTREAM_VER, NV_ENC_MAP_INPUT_RESOURCE_VER,
    NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS_VER, NV_ENC_PIC_PARAMS_VER, NV_ENC_PRESET_CONFIG_VER,
    NV_ENC_REGISTER_RESOURCE_VER, NVencCodecPicParams, NVencCreateBitstreamBuffer,
    NVencInitializeParams, NVencInitializeParamsBitfields, NVencLockBitStream,
    NVencMapInputResource, NVencOpenEncodeSessionExParams, NVencPicParams, NVencPresetConfig,
    NVencRegisterResource,
};
use nvenc::sys::version::NVENC_API_VERSION;
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_TEX2D_VPIV, D3D11_TEX2D_VPOV,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
    D3D11_VIDEO_PROCESSOR_CONTENT_DESC, D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC,
    D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC, D3D11_VIDEO_PROCESSOR_STREAM,
    D3D11_VIDEO_USAGE_PLAYBACK_NORMAL, D3D11_VPIV_DIMENSION_TEXTURE2D,
    D3D11_VPOV_DIMENSION_TEXTURE2D, ID3D11Texture2D, ID3D11VideoContext, ID3D11VideoDevice,
    ID3D11VideoProcessor, ID3D11VideoProcessorEnumerator, ID3D11VideoProcessorInputView,
    ID3D11VideoProcessorOutputView,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_RATIONAL, DXGI_SAMPLE_DESC};
use windows::core::Interface;
use windows_capture::frame::Frame;

use crate::config::EncoderConfig;
use crate::h264::{
    BackendError, VideoSegment, annex_b_to_length_prefixed, assemble_segment, avcc_from_annex_b,
};

const HUNDRED_NS_PER_SECOND: u64 = 10_000_000;

fn nvenc_result(result: nvenc::sys::result::NVencResult) -> Result<(), BackendError> {
    result
        .into_error()
        .map_err(|error| format!("NVENC error: {error:?}").into())
}

/// Converts irregular WGC arrival timestamps into a deterministic constant-frame-rate timeline.
/// A late callback repeats its newest GPU surface at the missed presentation slot. This bounds
/// timing jitter without making the capture callback sleep or inventing interpolated motion.
pub(crate) struct FramePacer {
    fps: u32,
    origin: Option<u64>,
    emitted: u64,
}

impl FramePacer {
    pub(crate) const fn new(fps: u16) -> Self {
        Self {
            fps: fps as u32,
            origin: None,
            emitted: 0,
        }
    }

    pub(crate) fn frames_due(&mut self, timestamp_100ns: u64) -> u32 {
        let origin = *self.origin.get_or_insert(timestamp_100ns);
        let elapsed = timestamp_100ns.saturating_sub(origin);
        let timeline_index = elapsed.saturating_mul(u64::from(self.fps)) / HUNDRED_NS_PER_SECOND;
        let desired_total = timeline_index.saturating_add(1);
        // WGC is intentionally requested above the output rate. Early callbacks therefore
        // produce no output; late callbacks repeat the newest surface at up to three slots.
        let due = desired_total.saturating_sub(self.emitted).min(3);
        self.emitted = self.emitted.saturating_add(due);
        u32::try_from(due).unwrap_or(3)
    }
}

pub(crate) struct DirectNvencEncoder {
    handle: NonNull<c_void>,
    functions: NVencFunctionList,
    output: NonNull<c_void>,
    inputs: Vec<RegisteredTexture>,
    scaler: Option<GpuScaler>,
    config: EncoderConfig,
    frame_index: usize,
    segment_frames: Vec<Vec<u8>>,
    avcc: Option<Vec<u8>>,
}

struct RegisteredTexture {
    texture: ID3D11Texture2D,
    registered: NonNull<c_void>,
    mapped: NonNull<c_void>,
}

struct GpuScaler {
    output: ID3D11Texture2D,
    video_device: ID3D11VideoDevice,
    video_context: ID3D11VideoContext,
    enumerator: ID3D11VideoProcessorEnumerator,
    processor: ID3D11VideoProcessor,
    output_view: ID3D11VideoProcessorOutputView,
    input_views: Vec<(ID3D11Texture2D, ID3D11VideoProcessorInputView)>,
}

// The capture framework requires the handler type to be Send before constructing it on its
// dedicated thread. The encoder is initialized only inside that destination thread and never
// leaves it. NVENC's session internals are thread-safe; all encode/lock/drop calls remain serial.
unsafe impl Send for DirectNvencEncoder {}

impl DirectNvencEncoder {
    pub(crate) fn new(frame: &Frame<'_>, config: EncoderConfig) -> Result<Self, BackendError> {
        let scaler = ((frame.width(), frame.height()) != (config.width, config.height))
            .then(|| GpuScaler::new(frame, config))
            .transpose()?;

        let library =
            nvenc::nvenc_init().map_err(|error| format!("could not load NVENC: {error}"))?;
        let driver_api = library
            .get_max_version()
            .map_err(|error| format!("could not query NVENC driver API: {error:?}"))?;
        let driver_major = driver_api & 0xffff;
        let required_major = NVENC_API_VERSION & 0xffff;
        if driver_major < required_major {
            return Err(format!(
                "NVIDIA driver exposes NVENC API {driver_major}, but OpenClip requires {required_major}"
            )
            .into());
        }
        let functions = library
            .create_instance()
            .map_err(|error| format!("could not create NVENC API: {error:?}"))?;
        let mut open: NVencOpenEncodeSessionExParams = unsafe { std::mem::zeroed() };
        open.version = NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS_VER;
        open.device_type = NVencDeviceType::DirectX;
        open.api_version = NVENC_API_VERSION;
        open.device = frame.device().as_raw();
        let mut handle = std::ptr::null_mut();
        nvenc_result(unsafe {
            (functions.nvenc_open_encode_session_ex)(&raw mut open, &raw mut handle)
        })?;
        let handle = NonNull::new(handle).ok_or("NVENC returned a null encoder session")?;

        let mut preset: MaybeUninit<NVencPresetConfig> = MaybeUninit::zeroed();
        unsafe { &mut *preset.as_mut_ptr() }.version = NV_ENC_PRESET_CONFIG_VER;
        unsafe { &mut *preset.as_mut_ptr() }.preset_cfg.version = NV_ENC_CONFIG_VER;
        nvenc_result(unsafe {
            (functions.nvenc_get_encode_preset_config_ex)(
                handle.as_ptr(),
                NV_ENC_CODEC_H264_GUID,
                NV_ENC_PRESET_P4_GUID,
                NVencTuningInfo::LowLatency,
                &raw mut preset,
            )
        })?;
        let mut preset = unsafe { preset.assume_init() };
        preset.preset_cfg.rc_params.rate_control_mode = NVencParamsRcMode::VBR;
        preset.preset_cfg.rc_params.average_bit_rate = config.bitrate;
        // No B-frames: output order equals presentation order, which keeps every segment
        // independently remuxable and removes reordering latency from the capture callback.
        preset.preset_cfg.frame_interval_p = 1;
        preset.preset_cfg.gop_len = u32::from(config.frames_per_second) * 2;

        let mut init: NVencInitializeParams = unsafe { std::mem::zeroed() };
        init.version = NV_ENC_INITIALIZE_PARAMS_VER;
        init.encode_guid = NV_ENC_CODEC_H264_GUID;
        init.preset_guid = NV_ENC_PRESET_P4_GUID;
        init.encode_width = config.width;
        init.encode_height = config.height;
        init.dar_width = config.width;
        init.dar_height = config.height;
        init.frame_rate_num = u32::from(config.frames_per_second);
        init.frame_rate_den = 1;
        init.enable_encode_async = 0;
        init.enable_ptd = 1;
        init.bit_flags = NVencInitializeParamsBitfields::new();
        init.encode_config = &raw mut preset.preset_cfg;
        init.max_encode_width = config.width;
        init.max_encode_height = config.height;
        init.tuning_info = NVencTuningInfo::LowLatency;
        init.buffer_format = NVencBufferFormat::ARGB;
        init.output_stats_level = NVencOutputStatsLevel::None;
        nvenc_result(unsafe {
            (functions.nvenc_initialize_encoder)(handle.as_ptr(), &raw mut init)
        })?;

        let mut output_params: NVencCreateBitstreamBuffer = unsafe { std::mem::zeroed() };
        output_params.version = NV_ENC_CREATE_BITSTREAM_BUFFER_VER;
        nvenc_result(unsafe {
            (functions.nvenc_create_bit_stream_buffer)(handle.as_ptr(), &raw mut output_params)
        })?;
        let output = NonNull::new(output_params.bitstream_buffer)
            .ok_or("NVENC returned a null bitstream buffer")?;
        Ok(Self {
            handle,
            functions,
            output,
            inputs: Vec::with_capacity(4),
            scaler,
            config,
            frame_index: 0,
            segment_frames: Vec::with_capacity(
                usize::from(config.frames_per_second).saturating_mul(2),
            ),
            avcc: None,
        })
    }

    /// Registers the WGC D3D11 texture directly with NVENC. Locking the output before returning
    /// guarantees the borrowed capture surface remains alive until the GPU has consumed it.
    pub(crate) fn encode_frame(
        &mut self,
        frame: &Frame<'_>,
        copies: u32,
    ) -> Result<Option<VideoSegment>, BackendError> {
        let texture = if let Some(scaler) = self.scaler.as_mut() {
            scaler.scale(frame)?.clone()
        } else {
            frame.as_raw_texture().clone()
        };
        let mapped_input =
            self.registered_input(&texture, self.config.width, self.config.height)?;
        let frames_per_segment = usize::from(self.config.frames_per_second).saturating_mul(2);
        for _ in 0..copies {
            let at_boundary = self.segment_frames.is_empty();
            let mut params: MaybeUninit<NVencPicParams> = MaybeUninit::zeroed();
            let params_ref = unsafe { &mut *params.as_mut_ptr() };
            params_ref.version = NV_ENC_PIC_PARAMS_VER;
            params_ref.input_width = self.config.width;
            params_ref.input_height = self.config.height;
            params_ref.input_buffer = mapped_input;
            params_ref.output_bitstream = self.output.as_ptr();
            params_ref.buffer_format = NVencBufferFormat::ARGB;
            params_ref.picture_struct = NVencPicStruct::Frame;
            params_ref.picture_type = NVencPicType::UNKNOWN;
            params_ref.frame_idx = u32::try_from(self.frame_index).unwrap_or(u32::MAX);
            params_ref.input_time_stamp = u64::try_from(self.frame_index).unwrap_or(u64::MAX);
            params_ref.codec_pic_params = NVencCodecPicParams { rsvd: [0; 256] };
            if at_boundary {
                params_ref.encode_pic_flags =
                    NVencPicFlags::ForceIDR as u32 | NVencPicFlags::OutputSpspps as u32;
            }
            nvenc_result(unsafe {
                (self.functions.nvenc_encode_picture)(self.handle.as_ptr(), &raw mut params)
            })?;
            let annex_b = {
                let mut lock: MaybeUninit<NVencLockBitStream> = MaybeUninit::zeroed();
                unsafe { &mut *lock.as_mut_ptr() }.version = NV_ENC_LOCK_BITSTREAM_VER;
                unsafe { &mut *lock.as_mut_ptr() }.output_bit_stream = self.output.as_ptr();
                nvenc_result(unsafe {
                    (self.functions.nvenc_lock_bit_stream)(self.handle.as_ptr(), &raw mut lock)
                })?;
                let lock = unsafe { lock.assume_init() };
                let bytes = unsafe {
                    std::slice::from_raw_parts(
                        lock.bitstream_buffer.cast::<u8>(),
                        usize::try_from(lock.bitstream_size_in_bytes).unwrap_or(usize::MAX),
                    )
                }
                .to_vec();
                nvenc_result(unsafe {
                    (self.functions.nvenc_unlock_bit_stream)(
                        self.handle.as_ptr(),
                        self.output.as_ptr(),
                    )
                })?;
                bytes
            };
            if self.avcc.is_none() {
                self.avcc = Some(avcc_from_annex_b(&annex_b)?);
            }
            self.segment_frames
                .push(annex_b_to_length_prefixed(&annex_b)?);
            self.frame_index = self.frame_index.saturating_add(1);
            if self.segment_frames.len() == frames_per_segment {
                return self.take_segment().map(Some);
            }
        }
        Ok(None)
    }

    fn registered_input(
        &mut self,
        texture: &ID3D11Texture2D,
        width: u32,
        height: u32,
    ) -> Result<*mut c_void, BackendError> {
        let identity = texture.as_raw();
        if let Some(input) = self
            .inputs
            .iter()
            .find(|input| input.texture.as_raw() == identity)
        {
            return Ok(input.mapped.as_ptr());
        }
        let mut registered: NVencRegisterResource = unsafe { std::mem::zeroed() };
        registered.version = NV_ENC_REGISTER_RESOURCE_VER;
        registered.resource_type = NVencInputResourceType::DirectX;
        registered.width = width;
        registered.height = height;
        registered.resource_to_register = texture.as_raw();
        registered.buffer_format = NVencBufferFormat::Undefined;
        registered.buffer_usage = NVencBufferUsage::Image;
        nvenc_result(unsafe {
            (self.functions.nvenc_register_resource)(self.handle.as_ptr(), &raw mut registered)
        })?;
        let registered_ptr = NonNull::new(registered.registered_resource)
            .ok_or("NVENC returned a null registered D3D11 resource")?;
        let mut mapped: NVencMapInputResource = unsafe { std::mem::zeroed() };
        mapped.version = NV_ENC_MAP_INPUT_RESOURCE_VER;
        mapped.registered_resource = registered.registered_resource;
        mapped.mapped_buffer_format = NVencBufferFormat::ARGB;
        if let Err(error) = nvenc_result(unsafe {
            (self.functions.nvenc_map_input_resource)(self.handle.as_ptr(), &raw mut mapped)
        }) {
            unsafe {
                (self.functions.nvenc_unregister_resource)(
                    self.handle.as_ptr(),
                    registered.registered_resource,
                );
            }
            return Err(error);
        }
        let mapped_ptr = NonNull::new(mapped.mapped_resource)
            .ok_or("NVENC returned a null mapped D3D11 resource")?;
        self.inputs.push(RegisteredTexture {
            texture: texture.clone(),
            registered: registered_ptr,
            mapped: mapped_ptr,
        });
        Ok(mapped_ptr.as_ptr())
    }

    pub(crate) fn finish_segment(&mut self) -> Result<Option<VideoSegment>, BackendError> {
        if self.segment_frames.is_empty() {
            Ok(None)
        } else {
            self.take_segment().map(Some)
        }
    }

    fn take_segment(&mut self) -> Result<VideoSegment, BackendError> {
        let avcc = self
            .avcc
            .as_ref()
            .ok_or("NVENC did not emit H.264 parameter sets")?;
        let frames = std::mem::take(&mut self.segment_frames);
        assemble_segment(
            avcc,
            (self.config.width, self.config.height),
            self.config.frames_per_second,
            &frames,
        )
    }
}

impl GpuScaler {
    fn new(frame: &Frame<'_>, config: EncoderConfig) -> Result<Self, BackendError> {
        let video_device: ID3D11VideoDevice = frame.device().cast()?;
        let video_context: ID3D11VideoContext = frame.device_context().cast()?;
        let fps = u32::from(config.frames_per_second);
        let content = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
            InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            InputFrameRate: DXGI_RATIONAL {
                Numerator: fps,
                Denominator: 1,
            },
            InputWidth: frame.width(),
            InputHeight: frame.height(),
            OutputFrameRate: DXGI_RATIONAL {
                Numerator: fps,
                Denominator: 1,
            },
            OutputWidth: config.width,
            OutputHeight: config.height,
            Usage: D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
        };
        let enumerator =
            unsafe { video_device.CreateVideoProcessorEnumerator(&raw const content) }?;
        let processor = unsafe { video_device.CreateVideoProcessor(&enumerator, 0) }?;

        let texture_desc = D3D11_TEXTURE2D_DESC {
            Width: config.width,
            Height: config.height,
            MipLevels: 1,
            ArraySize: 1,
            Format: frame.desc().Format,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut output = None;
        unsafe {
            frame
                .device()
                .CreateTexture2D(&raw const texture_desc, None, Some(&raw mut output))?;
        }
        let output = output.ok_or("D3D11 did not create an NVENC scaling texture")?;
        let output_desc = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
            ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
            Anonymous:
                windows::Win32::Graphics::Direct3D11::D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
                    Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 },
                },
        };
        let mut output_view = None;
        unsafe {
            video_device.CreateVideoProcessorOutputView(
                &output,
                &enumerator,
                &raw const output_desc,
                Some(&raw mut output_view),
            )?;
        }
        let output_view =
            output_view.ok_or("D3D11 did not create a video processor output view")?;
        configure_video_processor(
            &video_context,
            &processor,
            (frame.width(), frame.height()),
            (config.width, config.height),
        )?;
        tracing::info!(
            source_width = frame.width(),
            source_height = frame.height(),
            output_width = config.width,
            output_height = config.height,
            "D3D11 video-processor scaling enabled"
        );
        Ok(Self {
            output,
            video_device,
            video_context,
            enumerator,
            processor,
            output_view,
            input_views: Vec::with_capacity(4),
        })
    }

    fn scale(&mut self, frame: &Frame<'_>) -> Result<&ID3D11Texture2D, BackendError> {
        let identity = frame.as_raw_texture().as_raw();
        let input_view = if let Some((_, view)) = self
            .input_views
            .iter()
            .find(|(texture, _)| texture.as_raw() == identity)
        {
            view.clone()
        } else {
            let input_desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
                FourCC: 0,
                ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
                Anonymous:
                    windows::Win32::Graphics::Direct3D11::D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 {
                        Texture2D: D3D11_TEX2D_VPIV {
                            MipSlice: 0,
                            ArraySlice: 0,
                        },
                    },
            };
            let mut view = None;
            unsafe {
                self.video_device.CreateVideoProcessorInputView(
                    frame.as_raw_texture(),
                    &self.enumerator,
                    &raw const input_desc,
                    Some(&raw mut view),
                )?;
            }
            let view = view.ok_or("D3D11 did not create a video processor input view")?;
            self.input_views
                .push((frame.as_raw_texture().clone(), view.clone()));
            view
        };
        let mut stream = D3D11_VIDEO_PROCESSOR_STREAM {
            Enable: true.into(),
            pInputSurface: ManuallyDrop::new(Some(input_view)),
            ..Default::default()
        };
        let result = unsafe {
            self.video_context.VideoProcessorBlt(
                &self.processor,
                &self.output_view,
                0,
                std::slice::from_ref(&stream),
            )
        };
        unsafe {
            ManuallyDrop::drop(&mut stream.pInputSurface);
        }
        result?;
        Ok(&self.output)
    }
}

fn configure_video_processor(
    context: &ID3D11VideoContext,
    processor: &ID3D11VideoProcessor,
    source: (u32, u32),
    destination: (u32, u32),
) -> Result<(), BackendError> {
    let source_rect = RECT {
        left: 0,
        top: 0,
        right: i32::try_from(source.0)?,
        bottom: i32::try_from(source.1)?,
    };
    let destination_rect = RECT {
        left: 0,
        top: 0,
        right: i32::try_from(destination.0)?,
        bottom: i32::try_from(destination.1)?,
    };
    unsafe {
        context.VideoProcessorSetOutputTargetRect(
            processor,
            true,
            Some(&raw const destination_rect),
        );
        context.VideoProcessorSetStreamFrameFormat(
            processor,
            0,
            D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
        );
        context.VideoProcessorSetStreamSourceRect(processor, 0, true, Some(&raw const source_rect));
        context.VideoProcessorSetStreamDestRect(
            processor,
            0,
            true,
            Some(&raw const destination_rect),
        );
        context.VideoProcessorSetStreamAutoProcessingMode(processor, 0, false);
    }
    Ok(())
}

impl Drop for DirectNvencEncoder {
    fn drop(&mut self) {
        let mut eos: MaybeUninit<NVencPicParams> = MaybeUninit::zeroed();
        unsafe { &mut *eos.as_mut_ptr() }.version = NV_ENC_PIC_PARAMS_VER;
        unsafe { &mut *eos.as_mut_ptr() }.encode_pic_flags = NVencPicFlags::Eos as u32;
        unsafe {
            for input in self.inputs.drain(..) {
                let _ = (self.functions.nvenc_unmap_input_resource)(
                    self.handle.as_ptr(),
                    input.mapped.as_ptr(),
                );
                let _ = (self.functions.nvenc_unregister_resource)(
                    self.handle.as_ptr(),
                    input.registered.as_ptr(),
                );
            }
            let _ = (self.functions.nvenc_encode_picture)(self.handle.as_ptr(), &raw mut eos);
            let _ = (self.functions.nvenc_destory_bit_stream_buffer)(
                self.handle.as_ptr(),
                self.output.as_ptr(),
            );
            let _ = (self.functions.nvenc_destroy_encoder)(self.handle.as_ptr());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pacer_turns_late_arrivals_into_a_bounded_cfr_schedule() {
        let mut pacer = FramePacer::new(60);
        assert_eq!(pacer.frames_due(1_000_000), 1);
        assert_eq!(pacer.frames_due(1_176_000), 1);
        assert_eq!(pacer.frames_due(1_352_000), 1);
        assert_eq!(pacer.frames_due(1_528_000), 1);
        for index in 4..18 {
            assert_eq!(pacer.frames_due(1_000_000 + index * 176_000), 1);
        }
        assert_eq!(pacer.frames_due(1_000_000 + 18 * 176_000), 2);
    }

    #[test]
    fn pacer_holds_a_sixty_fps_timeline_over_a_rough_source_clock() {
        let mut pacer = FramePacer::new(60);
        let mut emitted = 0_u32;
        for frame in 0..568_u64 {
            emitted += pacer.frames_due(1_000_000 + frame * 176_366);
        }
        assert!((598..=602).contains(&emitted));
    }

    #[test]
    fn pacer_discards_early_callbacks_from_a_headroom_capture_clock() {
        let mut pacer = FramePacer::new(60);
        let mut emitted = 0_u32;
        let mut skipped = 0_u32;
        for frame in 0..750_u64 {
            let due = pacer.frames_due(1_000_000 + frame * 133_333);
            emitted += due;
            skipped += u32::from(due == 0);
        }
        assert!((598..=602).contains(&emitted));
        assert!((148..=152).contains(&skipped));
    }
}
