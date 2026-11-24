//! Controlled B1-derived Vulkan setup, encoding and pipeline; see README provenance.
use crate::{
    config::{AdapterPolicy, DeviceClass},
    diagnostic,
    engine::*,
    layout::Layout,
    native_contract::{checked_call, checked_release, checked_unmap},
    settlement::*,
};
use std::cell::Cell;
use std::future::Future;
use std::sync::{mpsc::SyncSender, Arc};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};
use wgpu::{Device, Queue, TextureFormat};
const FORMAT: TextureFormat = TextureFormat::Bgra8Unorm;
const USAGE: wgpu::TextureUsages =
    wgpu::TextureUsages::RENDER_ATTACHMENT.union(wgpu::TextureUsages::COPY_SRC);

pub struct NativeOps {
    layout: Layout,
    extent: wgpu::Extent3d,
    buffer: wgpu::Buffer,
    texture: wgpu::Texture,
    target: wgpu::TextureView,
    pipeline: wgpu::RenderPipeline,
    queue: Queue,
    device: Device,
    faults: Arc<Faults>,
}
impl NativeOps {
    fn new(
        policy: AdapterPolicy,
        faults: Arc<Faults>,
        initialization_uncertain: &mut bool,
    ) -> Result<(Self, Arc<Faults>, Telemetry), String> {
        let layout = Layout::fixture().map_err(|e| format!("{e:?}"))?;
        let started = Instant::now();
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            flags: wgpu::InstanceFlags::VALIDATION,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = if policy == AdapterPolicy::Default {
            native_setup(
                "request adapter",
                instance.request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    ..Default::default()
                }),
            )?
            .map_err(|error| format!("Vulkan adapter unavailable: {error}"))?
        } else {
            native_setup(
                "enumerate adapters",
                instance.enumerate_adapters(wgpu::Backends::VULKAN),
            )?
            .into_iter()
            .find(|adapter| {
                let info = adapter.get_info();
                info.backend == wgpu::Backend::Vulkan && policy.accepts(class(info.device_type))
            })
            .ok_or("required Vulkan adapter unavailable")?
        };
        let info = adapter.get_info();
        if info.backend != wgpu::Backend::Vulkan {
            return Err(format!("unexpected adapter backend {:?}", info.backend));
        }
        let formats = adapter.get_texture_format_features(FORMAT);
        if !formats.allowed_usages.contains(USAGE) {
            return Err(format!(
                "{FORMAT:?} requires {USAGE:?}, adapter provides {formats:?}"
            ));
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
            return Err(format!("insufficient Vulkan limits: required {required:?}, available {available:?}, layout {layout:?}"));
        }
        let (device, queue) = native_setup(
            "request device",
            adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("B1 synthetic readback"),
                required_features: wgpu::Features::empty(),
                required_limits: required,
                ..Default::default()
            }),
        )?
        .map_err(|error| format!("request Vulkan device: {error}"))?;
        let device_faults = Arc::clone(&faults);
        device.set_device_lost_callback(move |reason, message| {
            device_faults.observe_loss(
                reason == wgpu::DeviceLostReason::Destroyed,
                &format!("device lost {reason:?}: {message}"),
            );
        });
        let device_faults = Arc::clone(&faults);
        device.on_uncaptured_error(Arc::new(move |error| {
            device_faults.record(&error.to_string());
        }));

        let device_initialization = started.elapsed();
        let pipeline_started = Instant::now();
        let pipeline = create_pipeline(&device);
        let pipeline_initialization = pipeline_started.elapsed();
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
        let telemetry = Telemetry {
            adapter: Some(Adapter {
                name: diagnostic(&info.name),
                device_type: format!("{:?}", info.device_type),
                backend: format!("{:?}", info.backend),
                driver: diagnostic(&format!("{} {}", info.driver, info.driver_info)),
            }),
            initialization: started.elapsed(),
            initialization_complete: true,
            device_initialization,
            pipeline_initialization,
            ..Default::default()
        };
        let ops = Self {
            layout,
            extent,
            buffer,
            texture,
            target,
            pipeline,
            queue,
            device,
            faults: Arc::clone(&faults),
        };
        if let Some(error) = faults.get() {
            // Initialization has not submitted owner work, but a faulted
            // native resource owner must not be silently destroyed/forgotten
            // and later acknowledged as a controlled empty-backend release.
            *initialization_uncertain = true;
            std::mem::forget(ops);
            return Err(error);
        }
        Ok((ops, faults, telemetry))
    }

    fn commands(&self, phase: u32) -> wgpu::CommandBuffer {
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
                buffer: &self.buffer,
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

fn native_setup<T>(operation: &str, future: impl Future<Output = T>) -> Result<T, String> {
    // The pinned native backend returns ready futures for adapter and device
    // requests. Fail explicitly if that contract changes.
    match std::pin::pin!(future).poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(value) => Ok(value),
        Poll::Pending => Err(format!(
            "native Vulkan {operation} returned a pending future"
        )),
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

fn class(value: wgpu::DeviceType) -> DeviceClass {
    match value {
        wgpu::DeviceType::Cpu => DeviceClass::Cpu,
        wgpu::DeviceType::IntegratedGpu => DeviceClass::Integrated,
        wgpu::DeviceType::DiscreteGpu => DeviceClass::Discrete,
        _ => DeviceClass::Other,
    }
}

thread_local! { static IN_NATIVE_POLL: Cell<bool> = const { Cell::new(false) }; }

/// B1 containment with a stronger outer boundary: only a Device::poll panic
/// is caught. Every other panic exits immediately, without native destruction.
pub fn install_containment_hook() {
    std::panic::set_hook(Box::new(|_| {
        if !IN_NATIVE_POLL.get() {
            std::process::exit(101);
        }
    }));
}
fn guarded_poll<T>(faults: &Faults, poll: impl FnOnce() -> T) -> Result<T, String> {
    IN_NATIVE_POLL.with(|flag| {
        flag.set(true);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(poll));
        flag.set(false);
        result.map_err(|_| {
            faults.record("native Vulkan poll panicked; settlement unproven");
            faults.get().unwrap()
        })
    })
}
impl Operations for NativeOps {
    type Submission = wgpu::SubmissionIndex;
    type Commands = wgpu::CommandBuffer;
    type View<'a> = wgpu::BufferView;
    fn encode(&mut self, phase: u32) -> Result<Self::Commands, String> {
        Ok(self.commands(phase))
    }
    fn submit(&mut self, commands: Self::Commands) -> Result<Self::Submission, String> {
        checked_call(&self.faults, || Ok(self.queue.submit([commands])))
    }
    fn wait(&mut self, submission: &Self::Submission, remaining: Duration) -> Result<(), String> {
        checked_call(&self.faults, || {
            match guarded_poll(&self.faults, || {
                self.device.poll(wgpu::PollType::Wait {
                    submission_index: Some(submission.clone()),
                    timeout: Some(remaining),
                })
            })? {
                Ok(status) if status.wait_finished() => Ok(()),
                other => Err(format!("submission-specific wait failed: {other:?}")),
            }
        })
    }
    fn map(&mut self, operation: Operation, sender: SyncSender<Completion>) -> Result<(), String> {
        checked_call(&self.faults, || {
            self.buffer
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    // Operation-local, one-element channel: callbacks never block,
                    // write the socket, or acquire worker/protocol state.
                    let _ = sender.try_send(Completion {
                        operation,
                        result: result.map_err(|e| diagnostic(&e.to_string())),
                    });
                });
            Ok(())
        })
    }
    fn view(&self) -> Self::View<'_> {
        self.buffer.get_mapped_range(..)
    }
    fn unmap(&mut self) -> Result<(), String> {
        checked_unmap(
            &self.faults,
            || {
                self.buffer.unmap();
                Ok(())
            },
            || {
                guarded_poll(&self.faults, || self.device.poll(wgpu::PollType::Poll))?
                    .map(|s| s.is_queue_empty())
                    .map_err(|e| format!("post-unmap poll failed: {e}"))
            },
        )
    }
    fn observe(&mut self) -> Result<(), String> {
        checked_call(&self.faults, || {
            guarded_poll(&self.faults, || self.device.poll(wgpu::PollType::Poll))?
                .map_err(|e| format!("idle Vulkan poll failed: {e}"))
                .and_then(|s| {
                    if s.is_queue_empty() {
                        Ok(())
                    } else {
                        Err("unexpected outstanding native work".into())
                    }
                })
        })
    }
    fn prepare_release(&mut self) -> Result<(), String> {
        let poll = || {
            guarded_poll(&self.faults, || self.device.poll(wgpu::PollType::Poll))?
                .map(|s| s.is_queue_empty())
                .map_err(|e| format!("native teardown poll failed: {e}"))
        };
        checked_release(
            &self.faults,
            poll,
            || {
                self.device.destroy();
                Ok(())
            },
            poll,
        )
    }
}

pub struct NativeBackend {
    policy: AdapterPolicy,
    inner: Option<Persistent<NativeOps>>,
    faults: Arc<Faults>,
    initialized: bool,
    empty_evidence: Option<Evidence>,
    initialization_uncertain: bool,
    startup_telemetry: Telemetry,
}
impl NativeBackend {
    pub fn new(policy: AdapterPolicy) -> Self {
        Self {
            policy,
            inner: None,
            faults: Arc::default(),
            initialized: false,
            empty_evidence: None,
            initialization_uncertain: false,
            startup_telemetry: Telemetry::default(),
        }
    }
    pub fn telemetry(&self) -> Option<&Telemetry> {
        self.inner.as_ref().map(Persistent::telemetry)
    }
}
impl Backend for NativeBackend {
    fn acceptance_telemetry(&self) -> Option<&Telemetry> {
        Some(self.telemetry().unwrap_or(&self.startup_telemetry))
    }
    fn initialize(&mut self) -> Result<(), String> {
        if self.initialized {
            return Err("backend already initialized".into());
        }
        self.initialized = true;
        let started = Instant::now();
        let initialization = NativeOps::new(
            self.policy,
            Arc::clone(&self.faults),
            &mut self.initialization_uncertain,
        );
        self.startup_telemetry.initialization = started.elapsed();
        match initialization {
            Ok((ops, faults, telemetry)) => {
                self.faults = Arc::clone(&faults);
                self.inner = Some(Persistent::new(ops, faults, telemetry));
                Ok(())
            }
            Err(text) => {
                self.faults.record(&text);
                Err(diagnostic(&text))
            }
        }
    }
    fn render(&mut self, operation: Operation) -> Result<Prepared, Failure> {
        match self.inner.as_mut() {
            Some(inner) => inner.render(operation),
            None => Err(Failure {
                diagnostic: "backend unavailable".into(),
                evidence: Evidence::new(operation),
            }),
        }
    }
    fn fault(&mut self) -> Option<String> {
        if let Some(inner) = self.inner.as_mut() {
            inner.fault()
        } else {
            self.faults.get()
        }
    }
    fn settled(&self) -> bool {
        !self.initialization_uncertain && self.inner.as_ref().is_none_or(Backend::settled)
    }
    fn evidence(&self) -> Option<Evidence> {
        self.inner
            .as_ref()
            .and_then(Persistent::evidence)
            .or(self.empty_evidence)
    }
    fn reject(&mut self, operation: Operation) -> Option<Evidence> {
        if self.initialization_uncertain {
            return None;
        }
        if let Some(inner) = self.inner.as_mut() {
            inner.reject(operation)
        } else {
            let evidence = Evidence::new(operation);
            self.empty_evidence = Some(evidence);
            Some(evidence)
        }
    }
    fn release(&mut self) -> bool {
        let released =
            !self.initialization_uncertain && self.inner.as_mut().is_none_or(Backend::release);
        self.startup_telemetry.controlled_shutdown = released;
        self.startup_telemetry.uncertain_shutdown = !released;
        released
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_poll_panic_is_uncertain_without_unwinding_the_resource_owner() {
        let faults = Faults::default();
        assert!(guarded_poll(&faults, || panic!("injected native poll panic")).is_err());
        assert_eq!(
            faults.get().as_deref(),
            Some("native Vulkan poll panicked; settlement unproven")
        );
    }
    #[test]
    fn native_poll_panic_preserves_first_callback_fault() {
        let faults = Faults::default();
        assert_eq!(
            guarded_poll(&faults, || {
                faults.record("first device loss");
                panic!("wgpu panic after callback");
            }),
            Err::<(), _>("first device loss".into())
        );
    }
}
