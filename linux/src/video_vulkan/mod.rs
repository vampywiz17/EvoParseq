mod commands;
mod device;
mod output;
mod parameters;
mod session;

use anyhow::{Context as _, Result, ensure};
pub use device::{Context, create};
use eframe::{egui, egui_wgpu};
use lowlat_decode::h264::{Read, Stream};
use std::sync::Arc;

pub struct Decoder {
    context: Arc<Context>,
    stream: Stream,
    session: Option<session::Session>,
    pipeline: Arc<wgpu::RenderPipeline>,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    frame: Option<Arc<Frame>>,
    frames: Vec<Arc<Frame>>,
    presentation: Option<((u32, u32), [u32; 4], bool)>,
    srgb: bool,
    pub decoded: u64,
    presented: std::cell::Cell<u64>,
    profile: Option<u8>,
}

struct Frame {
    pipeline: Arc<wgpu::RenderPipeline>,
    bindings: wgpu::BindGroup,
    size: (u32, u32),
}

impl Decoder {
    pub fn new(context: Arc<Context>, format: wgpu::TextureFormat) -> Self {
        let device = &context.device;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Original NV12 planes"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("H.264 presentation"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("NV12 to SDR output"),
            source: wgpu::ShaderSource::Wgsl(include_str!("present.wgsl").into()),
        });
        let pipeline = Arc::new(
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Original decoder surface presentation"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vertex"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fragment"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview: None,
                cache: None,
            }),
        );
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Video scaling"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            context,
            stream: Stream::new(),
            session: None,
            pipeline,
            layout,
            sampler,
            frame: None,
            frames: Vec::new(),
            presentation: None,
            srgb: format.is_srgb(),
            decoded: 0,
            presented: std::cell::Cell::new(0),
            profile: None,
        }
    }

    pub fn submit(&mut self, bytes: &[u8]) -> Result<()> {
        let mut read = self
            .stream
            .read(bytes)
            .map_err(|e| anyhow::anyhow!("H.264 parsing: {e:?}"))?;
        if read == Read::FormatChanged {
            self.frame = None;
            self.frames.clear();
            self.presentation = None;
            self.session = None;
            read = self
                .stream
                .read(bytes)
                .map_err(|e| anyhow::anyhow!("H.264 format change: {e:?}"))?;
        }
        if read != Read::Picture {
            return Ok(());
        }
        let job = self.stream.job().context("H.264 picture unavailable")?;
        ensure!(
            job.sps.vui.max_num_reorder_frames.unwrap_or(0) == 0,
            "This low-latency preview requires an H.264 stream without reordered frames"
        );
        if self.session.is_none() {
            self.session = Some(session::Session::new(self.context.clone(), &job.sps)?);
        }
        self.profile = Some(job.sps.profile_idc);
        let session = self.session.as_mut().unwrap();
        session.decode(job, bytes)?;
        let visible = job.sps.visible();
        let full_range = job.sps.vui.video_full_range;
        let crop = job.sps.crop.unwrap_or_default();
        self.stream
            .finish()
            .map_err(|e| anyhow::anyhow!("H.264 reference state: {e:?}"))?;
        let output = output::latest(&mut self.stream.dpb);
        let key = (visible, crop, full_range);
        if self.presentation != Some(key) {
            self.frames.clear();
            for texture in &session.textures {
                let y = texture.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("Decode Y plane"),
                    format: Some(wgpu::TextureFormat::R8Unorm),
                    aspect: wgpu::TextureAspect::Plane0,
                    ..Default::default()
                });
                let uv = texture.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("Decode UV plane"),
                    format: Some(wgpu::TextureFormat::Rg8Unorm),
                    aspect: wgpu::TextureAspect::Plane1,
                    ..Default::default()
                });
                let values = [
                    visible.0 as f32 / session.size.0 as f32,
                    visible.1 as f32 / session.size.1 as f32,
                    (crop[0] * 2) as f32 / session.size.0 as f32,
                    (crop[2] * 2) as f32 / session.size.1 as f32,
                    if full_range { 1.0 } else { 0.0 },
                    if self.srgb { 1.0 } else { 0.0 },
                    0.0,
                    0.0,
                ];
                use wgpu::util::DeviceExt;
                let constants: Vec<u8> = values
                    .iter()
                    .flat_map(|value| value.to_ne_bytes())
                    .collect();
                let uniform =
                    self.context
                        .device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("Video range and crop"),
                            contents: &constants,
                            usage: wgpu::BufferUsages::UNIFORM,
                        });
                let bindings = self
                    .context
                    .device
                    .create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("Original decoder output"),
                        layout: &self.layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(&y),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::TextureView(&uv),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: wgpu::BindingResource::Sampler(&self.sampler),
                            },
                            wgpu::BindGroupEntry {
                                binding: 3,
                                resource: uniform.as_entire_binding(),
                            },
                        ],
                    });
                self.frames.push(Arc::new(Frame {
                    pipeline: self.pipeline.clone(),
                    bindings,
                    size: visible,
                }));
            }
            self.presentation = Some(key);
        }
        if let Some(slot) = output {
            self.frame = Some(
                self.frames
                    .get(slot)
                    .context("Invalid presentation surface")?
                    .clone(),
            );
        }
        self.decoded += 1;
        Ok(())
    }

    /// CPU wall-clock waits, not GPU execution or scanout measurements.
    pub fn timing_summary(&self) -> String {
        self.session.as_ref().map_or_else(
            || format!("{} decoded pictures; decoder not initialized", self.decoded),
            |session| format!("{} decoded pictures; graphics wait last/avg/peak {:.1}/{:.1}/{:.1} ms; decode completion wait {:.1}/{:.1}/{:.1} ms",
                self.decoded, session.graphics_wait.latest_ms, session.graphics_wait.average_ms,
                session.graphics_wait.peak_ms, session.decode_wait.latest_ms,
                session.decode_wait.average_ms, session.decode_wait.peak_ms),
        )
    }

    pub fn snapshot(&self, report: &mut crate::video_output::Snapshot) -> Option<u8> {
        report.enabled = true;
        report.decoder_initialized = self.session.is_some();
        report.frames_decoded = self.decoded;
        report.frames_presented = self.presented.get();
        report.decoder = Some("Vulkan Video H.264");
        report.renderer = Some("Vulkan / NV12 conversion");
        report.native_decoder_surface = Some("Original Vulkan NV12 decoder image");
        report.hardware_decode = None;
        report.gpu_surface_output = self.session.is_some();
        report.decoded_bit_depth = self.session.as_ref().map(|_| 8);
        report.decoded_chroma = self.session.as_ref().map(|_| "4:2:0");
        report.display_rgb_bit_depth = Some(8);
        report.codec = Some(crate::video_stream::Codec::H264);
        report.adapter = Some(self.context.adapter_name.clone());
        report.presentation_mode = Some("Vulkan / compositor-managed (vsync requested)");
        report.vsr_status = Some("Not enabled on Linux");
        if let Some(frame) = &self.frame {
            report.width = Some(frame.size.0);
            report.height = Some(frame.size.1);
        }
        self.profile
    }
    pub fn show(&self, ui: &mut egui::Ui) -> Option<(egui::Rect, (u32, u32))> {
        if let Some(frame) = &self.frame {
            let available = ui.available_size();
            let (canvas, _) = ui.allocate_exact_size(available, egui::Sense::hover());
            let [l, t, r, b] = crate::viewport::Viewport {
                source: frame.size,
                client: (
                    available.x.round().max(1.) as u32,
                    available.y.round().max(1.) as u32,
                ),
            }
            .rect()?;
            let rect = egui::Rect::from_min_max(
                canvas.min + egui::vec2(l as f32, t as f32),
                canvas.min + egui::vec2(r as f32, b as f32),
            );
            self.presented.set(self.presented.get().saturating_add(1));
            ui.painter().add(egui_wgpu::Callback::new_paint_callback(
                rect,
                Present(frame.clone()),
            ));
            Some((rect, frame.size))
        } else {
            None
        }
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        self.frame = None;
        self.frames.clear();
        self.session = None;
    }
}
struct Present(Arc<Frame>);

impl egui_wgpu::CallbackTrait for Present {
    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        _resources: &egui_wgpu::CallbackResources,
    ) {
        pass.set_pipeline(&self.0.pipeline);
        pass.set_bind_group(0, &self.0.bindings, &[]);
        pass.draw(0..3, 0..1);
    }
}
