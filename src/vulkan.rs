//! Optional Vulkan host for immutable VKF1 kernels, with one in-flight job.
//!
//! `raw` reexports pinned registry-generated ash bindings. Raw calls are unsafe
//! and are not covered by this executor's ownership or resource checks. Native
//! loaders/drivers are trusted; fence wait budgets cannot preempt a driver call.
use crate::cpu_compute::{MemoryBudget, Tensor};
use crate::{
    Error,
    compute::{Kernel, Reflection},
};
pub use ash as raw;
use ash::{Device, Entry, Instance, vk};
use serde::Serialize;
use std::ffi::CStr;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

fn fail(message: impl std::fmt::Display) -> Error {
    Error(message.to_string())
}

#[derive(Clone, Copy, Debug)]
pub struct DispatchLimits {
    pub max_elements: usize,
    /// Sum of actual Vulkan allocation requirements, including padding.
    pub max_allocation_bytes: u64,
    pub timeout: Duration,
}
impl Default for DispatchLimits {
    fn default() -> Self {
        Self {
            max_elements: 1 << 20,
            max_allocation_bytes: 64 << 20,
            timeout: Duration::from_secs(10),
        }
    }
}

/// Opt-in live pipeline reuse within one executor/device. Bounds cover retained
/// exact source/shader identity bytes and object count, not opaque driver memory.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct PipelineCacheLimits {
    pub max_entries: usize,
    pub max_key_bytes: usize,
}
impl Default for PipelineCacheLimits {
    fn default() -> Self {
        Self {
            max_entries: 16,
            max_key_bytes: 1 << 20,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq)]
pub struct PipelineCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub bypasses: u64,
    pub evictions: u64,
    pub pipeline_creations: u64,
    pub resident_entries: usize,
    pub resident_key_bytes: usize,
}

struct PipelineKey {
    source: Vec<u8>,
    words: Vec<u32>,
    reflection: Reflection,
    bytes: usize,
}
impl PipelineKey {
    fn byte_len(kernel: &Kernel) -> Option<usize> {
        let r = kernel.reflection();
        kernel
            .source_bytes()
            .len()
            .checked_add(kernel.words().len().checked_mul(4)?)?
            .checked_add(r.source_sha256.len())?
            .checked_add(r.spirv_sha256.len())?
            .checked_add(r.export.len())?
            .checked_add(
                r.specializations
                    .len()
                    .checked_mul(std::mem::size_of::<crate::compute::Specialization>())?,
            )?
            .checked_add(r.variant_sha256.as_ref().map_or(0, String::len))
    }
    fn new(kernel: &Kernel, bytes: usize) -> Self {
        Self {
            source: kernel.source_bytes().to_vec(),
            words: kernel.words().to_vec(),
            reflection: kernel.reflection().clone(),
            bytes,
        }
    }
    fn matches(&self, kernel: &Kernel) -> bool {
        // Digests describe provenance; actual byte equality decides reuse.
        self.reflection == *kernel.reflection()
            && self.source == kernel.source_bytes()
            && self.words == kernel.words()
    }
}

struct CachedPipeline {
    key: PipelineKey,
    pipeline: Arc<PipelineState>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DeviceInfo {
    pub name: String,
    pub api_version: u32,
    pub driver_version: u32,
    pub vendor_id: u32,
    pub device_id: u32,
    pub device_type: String,
    pub software_device: bool,
    pub loader_api_version: u32,
    pub queue_family: u32,
}

#[derive(Clone, Debug, Serialize)]
pub struct Timings {
    pub preparation_upload_ns: u128,
    pub submit_wait_ns: u128,
    pub download_ns: u128,
    pub allocation_bytes: u64,
    pub elements: usize,
    pub groups: u32,
    pub pipeline_cache_hit: bool,
    pub pipeline_creation_ns: u128,
}

/// Host-resident owned output plus explicit operation provenance. The tensor's
/// FTB1 finite-storage profile is not a claim about the GPU arithmetic policy.
pub struct TensorDispatch {
    pub output: Tensor,
    pub numerical_profile: &'static str,
    pub source_sha256: String,
    pub spirv_sha256: String,
    pub variant_sha256: Option<String>,
    pub device: DeviceInfo,
    pub timings: Timings,
}

/// Queue ownership is exclusive through `&mut self`. A cancelled/timed-out
/// submission stays owned and blocks further work until `reap` sees its fence.
pub struct Executor {
    entry: Option<Entry>,
    instance: Instance,
    device: Device,
    queue: vk::Queue,
    family: u32,
    memory: vk::PhysicalDeviceMemoryProperties,
    properties: vk::PhysicalDeviceProperties,
    info: DeviceInfo,
    pending: Option<Submission>,
    cache_limits: Option<PipelineCacheLimits>,
    pipeline_cache: Vec<CachedPipeline>,
    cache_stats: PipelineCacheStats,
}

impl Executor {
    pub fn new() -> Result<Self, Error> {
        // SAFETY: the process-selected Vulkan loader and ICD are trusted native code.
        unsafe {
            let entry = Entry::load().map_err(fail)?;
            let loader_api_version = entry
                .try_enumerate_instance_version()
                .map_err(fail)?
                .unwrap_or(vk::API_VERSION_1_0);
            let app = vk::ApplicationInfo::default()
                .application_name(c"FluxR VKF1")
                .api_version(vk::API_VERSION_1_0);
            let instance = entry
                .create_instance(
                    &vk::InstanceCreateInfo::default().application_info(&app),
                    None,
                )
                .map_err(fail)?;
            let selected = (|| {
                for physical in instance.enumerate_physical_devices().map_err(fail)? {
                    let families = instance.get_physical_device_queue_family_properties(physical);
                    if let Some((family, _)) = families.iter().enumerate().find(|(_, f)| {
                        f.queue_count > 0 && f.queue_flags.contains(vk::QueueFlags::COMPUTE)
                    }) {
                        let properties = instance.get_physical_device_properties(physical);
                        let memory = instance.get_physical_device_memory_properties(physical);
                        let priorities = [1.0];
                        let queue_info = [vk::DeviceQueueCreateInfo::default()
                            .queue_family_index(family as u32)
                            .queue_priorities(&priorities)];
                        let device = instance
                            .create_device(
                                physical,
                                &vk::DeviceCreateInfo::default().queue_create_infos(&queue_info),
                                None,
                            )
                            .map_err(fail)?;
                        return Ok((device, family as u32, properties, memory));
                    }
                }
                Err(fail("no Vulkan compute device available"))
            })();
            let (device, family, properties, memory) = match selected {
                Ok(value) => value,
                Err(error) => {
                    instance.destroy_instance(None);
                    return Err(error);
                }
            };
            let info = DeviceInfo {
                name: CStr::from_ptr(properties.device_name.as_ptr())
                    .to_string_lossy()
                    .into_owned(),
                api_version: properties.api_version,
                driver_version: properties.driver_version,
                vendor_id: properties.vendor_id,
                device_id: properties.device_id,
                device_type: format!("{:?}", properties.device_type),
                software_device: properties.device_type == vk::PhysicalDeviceType::CPU,
                loader_api_version,
                queue_family: family,
            };
            let queue = device.get_device_queue(family, 0);
            Ok(Self {
                entry: Some(entry),
                instance,
                device,
                queue,
                family,
                memory,
                properties,
                info,
                pending: None,
                cache_limits: None,
                pipeline_cache: vec![],
                cache_stats: PipelineCacheStats::default(),
            })
        }
    }
    /// The released constructor remains uncached. Reuse is an explicit host
    /// policy; no persistent driver blob, portable shader semantic change, or
    /// cross-device handle sharing is introduced.
    pub fn with_pipeline_cache(limits: PipelineCacheLimits) -> Result<Self, Error> {
        if limits.max_entries == 0 || limits.max_key_bytes == 0 {
            return Err(fail("Vulkan pipeline cache bounds must be positive"));
        }
        let mut executor = Self::new()?;
        executor.cache_limits = Some(limits);
        Ok(executor)
    }
    pub fn pipeline_cache_stats(&self) -> PipelineCacheStats {
        self.cache_stats
    }
    /// Refuse while completion is unobserved. A timeout never authorizes
    /// destruction of a pipeline used by an outstanding command buffer.
    pub fn clear_pipeline_cache(&mut self) -> Result<(), Error> {
        if self.pending.is_some() {
            return Err(fail(
                "Vulkan submission pending; reap before clearing pipeline cache",
            ));
        }
        self.pipeline_cache.clear();
        self.cache_stats.resident_entries = 0;
        self.cache_stats.resident_key_bytes = 0;
        Ok(())
    }
    fn cached_pipeline(&mut self, kernel: &Kernel) -> Option<Arc<PipelineState>> {
        self.cache_limits?;
        if let Some(index) = self
            .pipeline_cache
            .iter()
            .position(|entry| entry.key.matches(kernel))
        {
            // Most recently used at the end; the list is bounded by admission.
            let entry = self.pipeline_cache.remove(index);
            let pipeline = Arc::clone(&entry.pipeline);
            self.pipeline_cache.push(entry);
            self.cache_stats.hits = self.cache_stats.hits.saturating_add(1);
            Some(pipeline)
        } else {
            self.cache_stats.misses = self.cache_stats.misses.saturating_add(1);
            None
        }
    }
    fn retain_pipeline(&mut self, kernel: &Kernel, pipeline: &Arc<PipelineState>) {
        let Some(limits) = self.cache_limits else {
            return;
        };
        let Some(bytes) = PipelineKey::byte_len(kernel).filter(|&n| n <= limits.max_key_bytes)
        else {
            self.cache_stats.bypasses = self.cache_stats.bypasses.saturating_add(1);
            return;
        };
        // This method runs only before submission, after the entry gate proved
        // no earlier job pending. Eviction therefore cannot destroy an in-use
        // pipeline. The new job independently owns its Arc through completion.
        while self.pipeline_cache.len() >= limits.max_entries
            || self.cache_stats.resident_key_bytes > limits.max_key_bytes - bytes
        {
            let removed = self.pipeline_cache.remove(0);
            self.cache_stats.resident_key_bytes -= removed.key.bytes;
            self.cache_stats.evictions = self.cache_stats.evictions.saturating_add(1);
        }
        self.pipeline_cache.push(CachedPipeline {
            key: PipelineKey::new(kernel, bytes),
            pipeline: Arc::clone(pipeline),
        });
        self.cache_stats.resident_entries = self.pipeline_cache.len();
        self.cache_stats.resident_key_bytes += bytes;
    }

    pub fn info(&self) -> &DeviceInfo {
        &self.info
    }
    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Share CPU/ML/audio tensor ownership and payload admission across an
    /// explicit host -> Vulkan -> host transfer. No broadcasting or persistent
    /// GPU placement is inferred. Host output is reserved before native work.
    pub fn run_tensors(
        &mut self,
        kernel: &Kernel,
        inputs: &[Tensor],
        output_memory: &MemoryBudget,
        limits: DispatchLimits,
        cancel: &AtomicBool,
    ) -> Result<TensorDispatch, Error> {
        if inputs.len() != kernel.reflection().input_bindings {
            return Err(fail("Vulkan tensor binding count mismatch"));
        }
        let first = inputs
            .first()
            .ok_or_else(|| fail("Vulkan tensor inputs empty"))?;
        if inputs.iter().any(|input| input.shape() != first.shape()) {
            return Err(fail(
                "Vulkan tensor shapes differ; no implicit broadcasting",
            ));
        }
        let views: Vec<_> = inputs.iter().map(Tensor::values).collect();
        let count = kernel.check_inputs(&views, limits.max_elements)?;
        let bytes = count
            .checked_mul(4)
            .ok_or_else(|| fail("Vulkan tensor output overflow"))?;
        let reservation = output_memory.reserve_bytes(bytes)?;
        let (values, timings) = self.run(kernel, &views, limits, cancel)?;
        // This also rejects a nonfinite GPU output and releases its reservation.
        let output = Tensor::from_owned_reserved(first.shape(), values, reservation)?;
        Ok(TensorDispatch {
            output,
            numerical_profile: kernel.reflection().profile,
            source_sha256: kernel.reflection().source_sha256.clone(),
            spirv_sha256: kernel.reflection().spirv_sha256.clone(),
            variant_sha256: kernel.reflection().variant_sha256.clone(),
            device: self.info.clone(),
            timings,
        })
    }

    /// Nonblocking completion poll. Device-loss remains an explicit failure;
    /// no unobserved shard or abandoned submission is called successful.
    pub fn reap(&mut self) -> Result<bool, Error> {
        let Some(pending) = self.pending.as_ref() else {
            return Ok(true);
        };
        // SAFETY: the fence belongs to this live device and is not concurrently used.
        match unsafe { self.device.get_fence_status(pending.fence) } {
            Ok(true) => {
                self.pending.take();
                Ok(true)
            }
            Ok(false) => Ok(false),
            Err(error) => Err(fail(error)),
        }
    }

    pub fn run(
        &mut self,
        kernel: &Kernel,
        inputs: &[&[f32]],
        limits: DispatchLimits,
        cancel: &AtomicBool,
    ) -> Result<(Vec<f32>, Timings), Error> {
        if self.pending.is_some() {
            return Err(fail(
                "Vulkan submission pending; reap before reusing executor",
            ));
        }
        if limits.timeout.is_zero() || cancel.load(Ordering::Acquire) {
            return Err(fail("Vulkan dispatch cancelled or zero wait budget"));
        }
        let started = Instant::now();
        let count = kernel.check_inputs(inputs, limits.max_elements)?;
        let local = kernel.reflection().workgroup[0];
        let groups = (count as u32).div_ceil(local);
        let props = &self.properties.limits;
        if local > props.max_compute_work_group_size[0]
            || local > props.max_compute_work_group_invocations
            || groups > props.max_compute_work_group_count[0]
            || inputs.len() + 1 > props.max_per_stage_descriptor_storage_buffers as usize
            || inputs.len() + 1 > props.max_descriptor_set_storage_buffers as usize
        {
            return Err(fail("Vulkan device compute or descriptor limit exceeded"));
        }
        let byte_len = (count as u64)
            .checked_mul(4)
            .ok_or_else(|| fail("Vulkan byte count overflow"))?;
        if byte_len > props.max_storage_buffer_range as u64 {
            return Err(fail("Vulkan storage buffer range exceeded"));
        }
        let minimum = byte_len
            .checked_mul(inputs.len() as u64 + 1)
            .ok_or_else(|| fail("Vulkan allocation overflow"))?;
        if minimum > limits.max_allocation_bytes {
            return Err(fail("Vulkan allocation budget exceeded"));
        }
        let cached = self.cached_pipeline(kernel);
        let pipeline_cache_hit = cached.is_some();
        // SAFETY: all handles are created by this device, slices are bounded and
        // buffers remain owned until a fence signals. The emitted module fixes
        // descriptor/layout access and guards every lane before storage access.
        let job = unsafe {
            Submission::prepare(
                &self.device,
                self.family,
                &self.memory,
                kernel,
                inputs,
                BufferAdmission {
                    count,
                    budget: limits.max_allocation_bytes,
                },
                cached,
            )?
        };
        if !pipeline_cache_hit {
            self.cache_stats.pipeline_creations =
                self.cache_stats.pipeline_creations.saturating_add(1);
            self.retain_pipeline(kernel, &job.pipeline);
        }
        let pipeline_creation_ns = job.pipeline_creation_ns;
        let preparation_upload_ns = started.elapsed().as_nanos();
        if cancel.load(Ordering::Acquire) || started.elapsed() >= limits.timeout {
            return Err(fail(
                "Vulkan dispatch cancelled or preparation exhausted budget",
            ));
        }
        let submitted = Instant::now();
        let command_buffers = [job.command];
        let submits = [vk::SubmitInfo::default().command_buffers(&command_buffers)];
        let fence = job.fence;
        self.pending = Some(job);
        // A failed native submission (especially device loss) is not proof that
        // the driver has ceased observing its arguments. Keep ownership even on
        // this failure path; a later signalled fence is the only reuse signal.
        unsafe {
            self.device
                .queue_submit(self.queue, &submits, fence)
                .map_err(fail)?;
        }
        loop {
            if cancel.load(Ordering::Acquire) {
                return Err(fail(
                    "Vulkan dispatch cancelled; in-flight resources retained",
                ));
            }
            let remaining = limits.timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Err(fail("Vulkan fence timeout; in-flight resources retained"));
            }
            let wait_ns = remaining.min(Duration::from_millis(2)).as_nanos() as u64;
            let fence = self.pending.as_ref().unwrap().fence;
            match unsafe { self.device.wait_for_fences(&[fence], true, wait_ns) } {
                Ok(()) => break,
                Err(vk::Result::TIMEOUT) => continue,
                Err(error) => {
                    return Err(fail(format!(
                        "Vulkan wait failed: {error}; resources retained"
                    )));
                }
            }
        }
        let submit_wait_ns = submitted.elapsed().as_nanos();
        let job = self.pending.take().unwrap();
        let downloaded = Instant::now();
        let output = unsafe { job.read_output(count)? };
        let timings = Timings {
            preparation_upload_ns,
            submit_wait_ns,
            download_ns: downloaded.elapsed().as_nanos(),
            allocation_bytes: job.allocation_bytes,
            elements: count,
            groups,
            pipeline_cache_hit,
            pipeline_creation_ns,
        };
        Ok((output, timings))
    }
}

impl Drop for Executor {
    fn drop(&mut self) {
        // Never free memory still visible to the device or hide an unbounded
        // device_wait_idle in cancellation. If a final nonblocking fence poll
        // fails, deliberately quarantine the one live context until process exit.
        if self.pending.is_some() && !matches!(self.reap(), Ok(true)) {
            if let Some(pending) = self.pending.take() {
                std::mem::forget(pending);
            }
            // Even idle cached objects belong to the quarantined device. Keep
            // their owners alive too; their Drops must not race pending native work.
            std::mem::forget(std::mem::take(&mut self.pipeline_cache));
            if let Some(entry) = self.entry.take() {
                std::mem::forget(entry);
            }
            return;
        }
        self.pipeline_cache.clear();
        unsafe {
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
        }
    }
}

struct Buffer {
    device: Device,
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
}
impl Drop for Buffer {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_buffer(self.buffer, None);
            if self.memory != vk::DeviceMemory::null() {
                self.device.free_memory(self.memory, None);
            }
        }
    }
}
struct BufferAdmission {
    count: usize,
    budget: u64,
}

struct Submission {
    device: Device,
    buffers: Vec<Buffer>,
    pipeline: Arc<PipelineState>,
    pipeline_creation_ns: u128,
    descriptor_pool: vk::DescriptorPool,
    command_pool: vk::CommandPool,
    command: vk::CommandBuffer,
    fence: vk::Fence,
    allocation_bytes: u64,
}
impl Submission {
    unsafe fn prepare(
        device: &Device,
        family: u32,
        memory: &vk::PhysicalDeviceMemoryProperties,
        kernel: &Kernel,
        inputs: &[&[f32]],
        admission: BufferAdmission,
        cached: Option<Arc<PipelineState>>,
    ) -> Result<Self, Error> {
        let BufferAdmission { count, budget } = admission;
        let reused = cached.is_some();
        let mut job = Self {
            device: device.clone(),
            buffers: vec![],
            pipeline: Arc::new(PipelineState::empty(device)),
            pipeline_creation_ns: 0,
            descriptor_pool: vk::DescriptorPool::null(),
            command_pool: vk::CommandPool::null(),
            command: vk::CommandBuffer::null(),
            fence: vk::Fence::null(),
            allocation_bytes: 0,
        };
        // Every early return drops only resources which have not been submitted.
        unsafe {
            for input in inputs.iter().map(|x| Some(*x)).chain([None]) {
                let buffer = device
                    .create_buffer(
                        &vk::BufferCreateInfo::default()
                            .size(count as u64 * 4)
                            .usage(vk::BufferUsageFlags::STORAGE_BUFFER)
                            .sharing_mode(vk::SharingMode::EXCLUSIVE),
                        None,
                    )
                    .map_err(fail)?;
                job.buffers.push(Buffer {
                    device: device.clone(),
                    buffer,
                    memory: vk::DeviceMemory::null(),
                });
                let requirements = device.get_buffer_memory_requirements(buffer);
                job.allocation_bytes = job
                    .allocation_bytes
                    .checked_add(requirements.size)
                    .ok_or_else(|| fail("Vulkan allocation overflow"))?;
                if job.allocation_bytes > budget {
                    return Err(fail("Vulkan padded allocation budget exceeded"));
                }
                let flags =
                    vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT;
                let index = (0..memory.memory_type_count)
                    .find(|i| {
                        requirements.memory_type_bits & (1 << i) != 0
                            && memory.memory_types[*i as usize]
                                .property_flags
                                .contains(flags)
                    })
                    .ok_or_else(|| fail("Vulkan requires coherent host-visible memory"))?;
                let allocation = device
                    .allocate_memory(
                        &vk::MemoryAllocateInfo::default()
                            .allocation_size(requirements.size)
                            .memory_type_index(index),
                        None,
                    )
                    .map_err(fail)?;
                job.buffers.last_mut().unwrap().memory = allocation;
                device
                    .bind_buffer_memory(buffer, allocation, 0)
                    .map_err(fail)?;
                let mapped = device
                    .map_memory(allocation, 0, count as u64 * 4, vk::MemoryMapFlags::empty())
                    .map_err(fail)?
                    .cast::<f32>();
                if let Some(input) = input {
                    std::ptr::copy_nonoverlapping(input.as_ptr(), mapped, count);
                } else {
                    std::ptr::write_bytes(mapped, 0, count);
                }
                device.unmap_memory(allocation);
            }
            let pipeline_start = Instant::now();
            job.pipeline = match cached {
                Some(pipeline) => pipeline,
                None => Arc::new(PipelineState::prepare(device, kernel)?),
            };
            job.pipeline_creation_ns = if reused {
                0
            } else {
                pipeline_start.elapsed().as_nanos()
            };
            let pool_sizes = [vk::DescriptorPoolSize {
                ty: vk::DescriptorType::STORAGE_BUFFER,
                descriptor_count: job.buffers.len() as u32,
            }];
            job.descriptor_pool = device
                .create_descriptor_pool(
                    &vk::DescriptorPoolCreateInfo::default()
                        .max_sets(1)
                        .pool_sizes(&pool_sizes),
                    None,
                )
                .map_err(fail)?;
            let set_layouts = [job.pipeline.set_layout];
            let set = device
                .allocate_descriptor_sets(
                    &vk::DescriptorSetAllocateInfo::default()
                        .descriptor_pool(job.descriptor_pool)
                        .set_layouts(&set_layouts),
                )
                .map_err(fail)?[0];
            let infos: Vec<_> = job
                .buffers
                .iter()
                .map(|b| {
                    [vk::DescriptorBufferInfo {
                        buffer: b.buffer,
                        offset: 0,
                        range: count as u64 * 4,
                    }]
                })
                .collect();
            let writes: Vec<_> = infos
                .iter()
                .enumerate()
                .map(|(i, b)| {
                    vk::WriteDescriptorSet::default()
                        .dst_set(set)
                        .dst_binding(i as u32)
                        .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                        .buffer_info(b)
                })
                .collect();
            device.update_descriptor_sets(&writes, &[]);
            job.command_pool = device
                .create_command_pool(
                    &vk::CommandPoolCreateInfo::default().queue_family_index(family),
                    None,
                )
                .map_err(fail)?;
            job.command = device
                .allocate_command_buffers(
                    &vk::CommandBufferAllocateInfo::default()
                        .command_pool(job.command_pool)
                        .level(vk::CommandBufferLevel::PRIMARY)
                        .command_buffer_count(1),
                )
                .map_err(fail)?[0];
            device
                .begin_command_buffer(
                    job.command,
                    &vk::CommandBufferBeginInfo::default()
                        .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
                )
                .map_err(fail)?;
            device.cmd_bind_pipeline(
                job.command,
                vk::PipelineBindPoint::COMPUTE,
                job.pipeline.pipeline,
            );
            device.cmd_bind_descriptor_sets(
                job.command,
                vk::PipelineBindPoint::COMPUTE,
                job.pipeline.layout,
                0,
                &[set],
                &[],
            );
            device.cmd_push_constants(
                job.command,
                job.pipeline.layout,
                vk::ShaderStageFlags::COMPUTE,
                0,
                &(count as u32).to_le_bytes(),
            );
            device.cmd_dispatch(
                job.command,
                (count as u32).div_ceil(kernel.reflection().workgroup[0]),
                1,
                1,
            );
            let barrier = [vk::MemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::SHADER_WRITE)
                .dst_access_mask(vk::AccessFlags::HOST_READ)];
            device.cmd_pipeline_barrier(
                job.command,
                vk::PipelineStageFlags::COMPUTE_SHADER,
                vk::PipelineStageFlags::HOST,
                vk::DependencyFlags::empty(),
                &barrier,
                &[],
                &[],
            );
            device.end_command_buffer(job.command).map_err(fail)?;
            job.fence = device
                .create_fence(&vk::FenceCreateInfo::default(), None)
                .map_err(fail)?;
        }
        Ok(job)
    }
    unsafe fn read_output(&self, count: usize) -> Result<Vec<f32>, Error> {
        let mut values = vec![0.0; count];
        let memory = self.buffers.last().unwrap().memory;
        unsafe {
            let mapped = self
                .device
                .map_memory(memory, 0, count as u64 * 4, vk::MemoryMapFlags::empty())
                .map_err(fail)?
                .cast::<f32>();
            std::ptr::copy_nonoverlapping(mapped, values.as_mut_ptr(), count);
            self.device.unmap_memory(memory);
        }
        Ok(values)
    }
}
impl Drop for Submission {
    fn drop(&mut self) {
        // Ownership is released only before submit or after observed completion.
        unsafe {
            self.device.destroy_fence(self.fence, None);
            self.device.destroy_command_pool(self.command_pool, None);
            self.device
                .destroy_descriptor_pool(self.descriptor_pool, None);
        }
    }
}

struct PipelineState {
    device: Device,
    shader: vk::ShaderModule,
    set_layout: vk::DescriptorSetLayout,
    layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,
}
impl PipelineState {
    fn empty(device: &Device) -> Self {
        Self {
            device: device.clone(),
            shader: vk::ShaderModule::null(),
            set_layout: vk::DescriptorSetLayout::null(),
            layout: vk::PipelineLayout::null(),
            pipeline: vk::Pipeline::null(),
        }
    }
    unsafe fn prepare(device: &Device, kernel: &Kernel) -> Result<Self, Error> {
        let mut state = Self::empty(device);
        unsafe {
            state.shader = device
                .create_shader_module(
                    &vk::ShaderModuleCreateInfo::default().code(kernel.words()),
                    None,
                )
                .map_err(fail)?;
            let bindings: Vec<_> = (0..kernel.reflection().input_bindings + 1)
                .map(|binding| {
                    vk::DescriptorSetLayoutBinding::default()
                        .binding(binding as u32)
                        .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                        .descriptor_count(1)
                        .stage_flags(vk::ShaderStageFlags::COMPUTE)
                })
                .collect();
            state.set_layout = device
                .create_descriptor_set_layout(
                    &vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings),
                    None,
                )
                .map_err(fail)?;
            let set_layouts = [state.set_layout];
            let push = [vk::PushConstantRange {
                stage_flags: vk::ShaderStageFlags::COMPUTE,
                offset: 0,
                size: 4,
            }];
            state.layout = device
                .create_pipeline_layout(
                    &vk::PipelineLayoutCreateInfo::default()
                        .set_layouts(&set_layouts)
                        .push_constant_ranges(&push),
                    None,
                )
                .map_err(fail)?;
            // Data and map arrays stay alive for the entire synchronous native
            // creation call. Native byte order matches Vulkan's host ABI.
            let map_entries: Vec<_> = kernel
                .reflection()
                .specializations
                .iter()
                .enumerate()
                .map(|(i, s)| vk::SpecializationMapEntry {
                    constant_id: s.parameter_index,
                    offset: (i * 4) as u32,
                    size: 4,
                })
                .collect();
            let data: Vec<_> = kernel
                .reflection()
                .specializations
                .iter()
                .flat_map(|s| s.value_bits.to_ne_bytes())
                .collect();
            let specialization = vk::SpecializationInfo::default()
                .map_entries(&map_entries)
                .data(&data);
            let mut stage = vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::COMPUTE)
                .module(state.shader)
                .name(c"main");
            if !map_entries.is_empty() {
                stage = stage.specialization_info(&specialization);
            }
            let pipelines = device.create_compute_pipelines(
                vk::PipelineCache::null(),
                &[vk::ComputePipelineCreateInfo::default()
                    .stage(stage)
                    .layout(state.layout)],
                None,
            );
            state.pipeline = match pipelines {
                Ok(p) => p[0],
                Err((partial, error)) => {
                    for p in partial {
                        if p != vk::Pipeline::null() {
                            device.destroy_pipeline(p, None);
                        }
                    }
                    return Err(fail(error));
                }
            };
        }
        Ok(state)
    }
}
impl Drop for PipelineState {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_pipeline(self.pipeline, None);
            self.device.destroy_pipeline_layout(self.layout, None);
            self.device
                .destroy_descriptor_set_layout(self.set_layout, None);
            self.device.destroy_shader_module(self.shader, None);
        }
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;
    use crate::structured::Module;
    fn kernel(local: u32) -> Kernel {
        let m = Module::compile_compact(include_str!("../examples/vkf1-affine.fluxr")).unwrap();
        Kernel::compile(&m, "kernel", local).unwrap()
    }
    #[test]
    fn cache_key_requires_exact_bytes_and_full_profile() {
        let k = kernel(64);
        let bytes = PipelineKey::byte_len(&k).unwrap();
        let mut key = PipelineKey::new(&k, bytes);
        assert!(key.matches(&k));
        assert!(key.matches(&k.clone()));
        assert!(!key.matches(&kernel(32)));
        key.source[0] ^= 1;
        assert!(!key.matches(&k));
        key.source[0] ^= 1;
        key.words[0] ^= 1;
        assert!(!key.matches(&k));
        key.words[0] ^= 1;
        key.reflection.numerical_policy = "different";
        assert!(!key.matches(&k));
    }
    #[test]
    fn cache_counts_owned_identity_payload_exactly() {
        let k = kernel(64);
        let r = k.reflection();
        let expected = k.source_bytes().len()
            + k.words().len() * 4
            + r.source_sha256.len()
            + r.spirv_sha256.len()
            + r.export.len();
        assert_eq!(PipelineKey::byte_len(&k), Some(expected));
        assert_eq!(PipelineKey::new(&k, expected).bytes, expected);
    }
    #[test]
    fn specialized_template_never_reuses_a_different_constant_variant() {
        use crate::compute::Specialization;
        let module =
            Module::compile_compact("v2 (pub kernel (x:f32 y:f32) f32 () (ret (* x y)))").unwrap();
        let a = Kernel::compile_specialized(
            &module,
            "kernel",
            64,
            &[Specialization {
                parameter_index: 1,
                value_bits: 2.0_f32.to_bits(),
            }],
        )
        .unwrap();
        let b = Kernel::compile_specialized(
            &module,
            "kernel",
            64,
            &[Specialization {
                parameter_index: 1,
                value_bits: 3.0_f32.to_bits(),
            }],
        )
        .unwrap();
        assert_eq!(a.words(), b.words());
        let bytes = PipelineKey::byte_len(&a).unwrap();
        let mut key = PipelineKey::new(&a, bytes);
        assert!(key.matches(&a));
        assert!(!key.matches(&b));
        // Even replacing the digest cannot authorize reuse: exact values count.
        key.reflection.variant_sha256 = b.reflection().variant_sha256.clone();
        assert!(!key.matches(&b));
        let r = a.reflection();
        assert_eq!(
            bytes,
            a.source_bytes().len()
                + a.words().len() * 4
                + r.source_sha256.len()
                + r.spirv_sha256.len()
                + r.export.len()
                + std::mem::size_of::<Specialization>()
                + 64
        );
    }
    #[test]
    fn zero_cache_bounds_refuse_before_driver_initialization() {
        for limits in [
            PipelineCacheLimits {
                max_entries: 0,
                max_key_bytes: 1,
            },
            PipelineCacheLimits {
                max_entries: 1,
                max_key_bytes: 0,
            },
        ] {
            assert!(Executor::with_pipeline_cache(limits).is_err());
        }
    }
}
