use std::future::Future;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use wgpu::{Buffer, Device, Queue, TextureFormat};

use crate::layout::Layout;
use crate::readback::{
    copy_mapped, guard_native_poll, Completion, Deadline, Fault, Faults, Identity, Mapping,
    ReadMap, ReadbackError, Staging, Step,
};

const FORMAT: TextureFormat = TextureFormat::Bgra8Unorm;
pub const WAIT_BUDGET: Duration = Duration::from_secs(2);
const USAGE: wgpu::TextureUsages =
    wgpu::TextureUsages::RENDER_ATTACHMENT.union(wgpu::TextureUsages::COPY_SRC);

pub struct Gpu {
    layout: Layout,
    extent: wgpu::Extent3d,
    staging: Staging<Buffer>,
    texture: wgpu::Texture,
    target: wgpu::TextureView,
    pipeline: wgpu::RenderPipeline,
    queue: Queue,
    device: Device,
    faults: Arc<Faults>,
    pub counters: Counters,
    pub unsettled: bool,
}

#[derive(Default)]
pub struct Counters {
    pub submissions: u64,
    pub map_requests: u64,
    pub mapped: u64,
    pub unmapped: u64,
    pub staging_allocations: u64,
    pub staging_reuses: u64,
    pub staging_retirements: u64,
    pub peak_outstanding: u64,
}

#[derive(Default)]
pub struct TransferTimes {
    pub encode: Duration,
    pub submit: Duration,
    pub gpu_wait: Duration,
    pub map: Duration,
    pub repack: Duration,
}

pub enum SetupError {
    Unavailable(String),
    Failed(String),
}

impl Gpu {
    pub fn new(layout: Layout) -> Result<Self, SetupError> {
        use SetupError::{Failed, Unavailable};

        let started = Instant::now();
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            flags: wgpu::InstanceFlags::VALIDATION,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = native_setup(
            "request adapter",
            instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                ..Default::default()
            }),
        )?
        .map_err(|error| Unavailable(format!("Vulkan adapter unavailable: {error}")))?;
        let info = adapter.get_info();
        let rendering = match info.device_type {
            wgpu::DeviceType::Cpu => "software",
            wgpu::DeviceType::DiscreteGpu | wgpu::DeviceType::IntegratedGpu => "hardware",
            _ => "undetermined",
        };
        println!(
            "adapter name={:?} backend={:?} vendor={:#06x} device={:#06x} device_type={:?} driver={:?} driver_info={:?} rendering={rendering}",
            info.name, info.backend, info.vendor, info.device, info.device_type, info.driver, info.driver_info,
        );
        if info.backend != wgpu::Backend::Vulkan {
            return Err(Unavailable(format!(
                "unexpected adapter backend {:?}",
                info.backend
            )));
        }
        let formats = adapter.get_texture_format_features(FORMAT);
        if !formats.allowed_usages.contains(USAGE) {
            return Err(Unavailable(format!(
                "{FORMAT:?} requires {USAGE:?}, adapter provides {formats:?}"
            )));
        }
        let extent = wgpu::Extent3d {
            width: layout.width as u32,
            height: layout.height as u32,
            depth_or_array_layers: 1,
        };
        let required = wgpu::Limits::downlevel_defaults();
        let available = adapter.limits();
        if extent.width.max(extent.height) > required.max_texture_dimension_2d
            || layout.staging as u64 > required.max_buffer_size
            || !required.check_limits(&available)
            || !layout
                .pitch
                .is_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize)
        {
            return Err(Unavailable(format!("insufficient Vulkan limits: required {required:?}, available {available:?}, layout {layout:?}")));
        }
        println!(
            "capabilities format={FORMAT:?} allowed_usages={:?} format_flags={:?} adapter_max_texture_dimension_2d={} adapter_max_buffer_size={} device_required_features=empty device_required_limits=downlevel_defaults",
            formats.allowed_usages, formats.flags, available.max_texture_dimension_2d, available.max_buffer_size,
        );
        let (device, queue) = native_setup(
            "request device",
            adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("B1 synthetic readback"),
                required_features: wgpu::Features::empty(),
                required_limits: required,
                ..Default::default()
            }),
        )?
        .map_err(|error| Failed(format!("request Vulkan device: {error}")))?;
        let faults = Arc::new(Faults::default());
        let device_faults = Arc::clone(&faults);
        device.set_device_lost_callback(move |reason, message| {
            device_faults.record(Fault::DeviceLost(format!("{reason:?}: {message}")));
        });
        let device_faults = Arc::clone(&faults);
        device.on_uncaptured_error(Arc::new(move |error| {
            let fault = match error {
                wgpu::Error::Validation { .. } => Fault::Validation(error.to_string()),
                _ => Fault::OtherDeviceError(error.to_string()),
            };
            device_faults.record(fault);
        }));
        let device_time = started.elapsed();

        let pipeline_started = Instant::now();
        let pipeline = create_pipeline(&device);
        let pipeline_time = pipeline_started.elapsed();
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("B1 persistent render target"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: USAGE,
            view_formats: &[],
        });
        let target = texture.create_view(&Default::default());
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("B1 readback staging"),
            size: layout.staging as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        faults
            .check()
            .map_err(|error| Failed(format!("GPU initialization: {error:?}")))?;
        println!(
            "initialization device_us={} pipeline_us={} total_us={}",
            device_time.as_micros(),
            pipeline_time.as_micros(),
            started.elapsed().as_micros()
        );
        Ok(Self {
            layout,
            extent,
            staging: Staging::Available {
                buffer,
                reused: false,
            },
            texture,
            target,
            pipeline,
            queue,
            device,
            faults,
            counters: Counters {
                staging_allocations: 1,
                ..Default::default()
            },
            unsettled: false,
        })
    }

    pub fn read(&mut self, key: Identity) -> Result<(Vec<u8>, TransferTimes), ReadbackError> {
        let (buffer, reused) = self.staging.acquire()?;
        self.counters.staging_reuses += u64::from(reused);
        let mut times = TransferTimes::default();
        let result = self.transfer(key, &buffer, &mut times);
        let counters = &mut self.counters;
        let bytes = self.staging.settle(buffer, result, |buffer| {
            counters.staging_retirements += 1;
            // With an unsettled submission, wgpu still owns the in-use resource.
            // The standalone command exits with the queue alive on that path.
            if !self.unsettled {
                buffer.destroy();
            }
        })?;
        Ok((bytes, times))
    }

    fn transfer(
        &mut self,
        key: Identity,
        buffer: &Buffer,
        times: &mut TransferTimes,
    ) -> Result<Vec<u8>, ReadbackError> {
        self.faults.check()?;
        let encoding = Instant::now();
        let commands = self.encode(key.phase as u32, buffer);
        times.encode = encoding.elapsed();
        self.faults.check()?;

        let submitting = Instant::now();
        let deadline = Deadline(submitting + WAIT_BUDGET);
        self.unsettled = true;
        let submission = self.queue.submit([commands]);
        times.submit = submitting.elapsed();
        self.counters.submissions += 1;
        self.counters.peak_outstanding = 1;

        let waiting = Instant::now();
        let gpu_result = self.wait(&submission, deadline, Step::Submission);
        if gpu_result.is_ok() {
            self.unsettled = false;
        }
        times.gpu_wait = waiting.elapsed();
        self.faults.check()?;
        gpu_result?;
        deadline.remaining(Instant::now(), Step::Submission)?;

        let mapping = Instant::now();
        let (completion, sender) = Mapping::new(key);
        self.counters.map_requests += 1;
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(Completion {
                    identity: key,
                    result: result.map_err(|error| error.to_string()),
                });
            });
        let map_wait = self.wait(&submission, deadline, Step::Mapping);
        self.faults.check()?;
        map_wait?;
        completion.complete(deadline, &self.faults)?;
        times.map = mapping.elapsed();

        let repacking = Instant::now();
        self.counters.mapped += 1;
        let packed = copy_mapped(buffer, self.layout);
        self.counters.unmapped += 1;
        times.repack = repacking.elapsed();
        self.faults.check()?;
        packed
    }

    fn wait(
        &mut self,
        submission: &wgpu::SubmissionIndex,
        deadline: Deadline,
        step: Step,
    ) -> Result<(), ReadbackError> {
        let remaining = deadline.remaining(Instant::now(), step)?;
        let poll = guard_native_poll(&self.faults, || {
            self.device.poll(wgpu::PollType::Wait {
                submission_index: Some(submission.clone()),
                timeout: Some(remaining),
            })
        });
        let status = match poll {
            Ok(status) => status,
            Err(error) => {
                // The native poll unwound before proving driver state.
                self.unsettled = true;
                return Err(error);
            }
        };
        match status {
            Ok(status) if status.wait_finished() => Ok(()),
            Ok(status) => Err(ReadbackError::Poll(format!(
                "unexpected wait status: {status:?}"
            ))),
            Err(wgpu::PollError::Timeout) => Err(ReadbackError::Timeout(step)),
            Err(error) => Err(ReadbackError::Poll(error.to_string())),
        }
    }

    fn encode(&self, phase: u32, buffer: &Buffer) -> wgpu::CommandBuffer {
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("B1 render and readback"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("B1 synthetic phase"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.draw(0..3, phase..phase + 1);
        }
        encoder.copy_texture_to_buffer(
            self.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.layout.pitch as u32),
                    rows_per_image: Some(self.extent.height),
                },
            },
            self.extent,
        );
        encoder.finish()
    }
}

impl ReadMap for Buffer {
    type View<'a> = wgpu::BufferView;

    fn view(&self) -> Self::View<'_> {
        self.get_mapped_range(..)
    }

    fn unmap(&self) {
        self.unmap();
    }
}

fn native_setup<T>(operation: &str, future: impl Future<Output = T>) -> Result<T, SetupError> {
    // The pinned native backend returns ready futures for adapter and device
    // requests. Fail explicitly if that contract changes.
    match std::pin::pin!(future).poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(value) => Ok(value),
        Poll::Pending => Err(SetupError::Failed(format!(
            "native Vulkan {operation} returned a pending future"
        ))),
    }
}

fn create_pipeline(device: &Device) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("B1 synthetic fixture"),
        source: wgpu::ShaderSource::Wgsl(include_str!("fixture.wgsl").into()),
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("B1 BGRA8 pipeline"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vertex_main"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fragment_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}
