//! Local hardware test: authored Fresco -> Naga IR -> SPIR-V -> unsafe Vulkan passthrough.
//! Run explicitly; this example is never executed by CI.
use ash::vk;
use fresco::driver::native::{self, Options, ResourceBinding, Target};
use std::{borrow::Cow, collections::BTreeMap, sync::mpsc, time::Duration};
use wgpu::util::DeviceExt;
type Vulkan = wgpu::hal::api::Vulkan;
const COUNT: u32 = 64;
const GUARD: u32 = 0xfeed_beef;

fn main() {
    const {
        assert!(
            cfg!(debug_assertions),
            "run in debug mode for the validation canary"
        );
    }
    let (device, queue) = device();
    for compact in [false, true] {
        for alias in [false, true] {
            execute(&device, &queue, compact, alias);
        }
    }
    drop(queue);
    drop(device);
    let errors = wgpu_hal::VALIDATION_CANARY.get_and_reset();
    assert!(errors.is_empty(), "Vulkan validation errors: {errors:#?}");
}

fn execute(device: &wgpu::Device, queue: &wgpu::Queue, compact: bool, alias: bool) {
    let mut artifact = native::compile(
        include_str!("native_pointers.fr"),
        &Options {
            target: Target::NativeSpirV,
            entry_point: "update_one".into(),
            workgroup_size: [64, 1, 1],
            bindings: BTreeMap::from([(
                "roots".into(),
                ResourceBinding {
                    group: 0,
                    binding: 0,
                },
            )]),
        },
    )
    .unwrap();
    if compact {
        artifact.compact().unwrap();
    }
    let words = artifact.spirv().unwrap();
    let path = std::env::temp_dir().join(format!("fresco-native-gpu-{}.spv", std::process::id()));
    std::fs::write(
        &path,
        words
            .iter()
            .flat_map(|w| w.to_le_bytes())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    let validation = std::process::Command::new(
        std::env::var_os("SPIRV_VAL").unwrap_or_else(|| "spirv-val".into()),
    )
    .args(["--target-env", "vulkan1.1"])
    .arg(&path)
    .output()
    .expect("install spirv-val or set SPIRV_VAL");
    std::fs::remove_file(path).unwrap();
    assert!(
        validation.status.success(),
        "{}",
        String::from_utf8_lossy(&validation.stderr)
    );
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let _rejected = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("native pointers must be rejected by safe wgpu"),
        source: wgpu::ShaderSource::Naga(Cow::Owned(artifact.module)),
    });
    assert!(matches!(
        pollster::block_on(scope.pop()),
        Some(wgpu::Error::Validation { .. })
    ));
    // SAFETY: SPIR-V was validated, entry metadata matches, and this test maintains
    // every allocation, address, alignment and resource transition through completion.
    let shader = unsafe {
        device.create_shader_module_passthrough(wgpu::ShaderModuleDescriptorPassthrough {
            label: Some("authored Fresco native pointers"),
            spirv: Some(Cow::Owned(words)),
            entry_points: Cow::Owned(vec![wgpu::PassthroughShaderEntryPoint {
                name: Cow::Borrowed("update_one"),
                workgroup_size: (64, 1, 1),
            }]),
            ..Default::default()
        })
    };
    let bindings = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[Some(&bindings)],
        immediate_size: 0,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: None,
        layout: Some(&layout),
        module: &shader,
        entry_point: Some("update_one"),
        compilation_options: Default::default(),
        cache: None,
    });
    let input: Vec<u32> = std::iter::once(GUARD)
        .chain((0..COUNT).map(|i| 17 + i * 5))
        .chain(std::iter::once(GUARD))
        .collect();
    let (source, source_address) = addressed_buffer(device, &input);
    let (destination, destination_address) = addressed_buffer(device, &input);
    assert_ne!(source_address, destination_address);
    let mut root_bytes = [0_u8; 16];
    root_bytes[..8].copy_from_slice(&(source_address + 4).to_le_bytes());
    root_bytes[8..].copy_from_slice(
        &(if alias {
            source_address + 4
        } else {
            destination_address + 4
        })
        .to_le_bytes(),
    );
    let roots = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("bit-exact device addresses"),
        contents: &root_bytes,
        usage: wgpu::BufferUsages::STORAGE,
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &bindings,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: roots.as_entire_binding(),
        }],
    });
    let size = u64::from(COUNT + 2) * 4;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: size * 2,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.transition_resources(
            [&source, &destination]
                .into_iter()
                .map(|buffer| wgpu::BufferTransition {
                    buffer,
                    state: wgpu::BufferUses::STORAGE_READ_WRITE,
                }),
            std::iter::empty(),
        );
        // The second workgroup is entirely out of bounds and must perform no memory accesses.
        pass.dispatch_workgroups(2, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&source, 0, &readback, 0, size);
    encoder.copy_buffer_to_buffer(&destination, 0, &readback, size, size);
    queue.submit([encoder.finish()]);
    let (send, receive) = mpsc::channel();
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            send.send(result).unwrap();
        });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_secs(30)),
        })
        .unwrap();
    receive
        .recv_timeout(Duration::from_secs(30))
        .unwrap()
        .unwrap();
    let mapped = readback.slice(..).get_mapped_range().unwrap();
    for (buffer_index, bytes) in mapped
        .chunks_exact(usize::try_from(size).unwrap())
        .enumerate()
    {
        let actual: Vec<u32> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .copied()
            .map(u32::from_le_bytes)
            .collect();
        assert_eq!(actual[0], GUARD);
        assert_eq!(*actual.last().unwrap(), GUARD);
        for index in 0..usize::try_from(COUNT).unwrap() {
            let original = input[index + 1];
            let expected = match (alias, buffer_index) {
                (true, 0) => original * 3 + 8,
                (false, 1) => original + 1,
                _ => original,
            };
            assert_eq!(
                actual[index + 1],
                expected,
                "alias={alias}, compact={compact}, buffer={buffer_index}, index={index}"
            );
        }
    }
    drop(mapped);
    readback.unmap();
    println!(
        "Verified alias={alias}, compact={compact}: 128 values, 4 guards, 64 guarded-out invocations"
    );
}
fn device() -> (wgpu::Device, wgpu::Queue) {
    // Layer presence plus explicit instance flags makes missing validation a test failure.
    // SAFETY: Load the system Vulkan loader for the lifetime of this query.
    let entry = unsafe { ash::Entry::load() }.unwrap();
    // SAFETY: The loader is alive and no pointers outlive the enumeration.
    let layers = unsafe { entry.enumerate_instance_layer_properties() }.unwrap();
    assert!(
        layers
            .iter()
            .any(|layer| layer.layer_name_as_c_str().unwrap() == c"VK_LAYER_KHRONOS_validation"),
        "install Vulkan validation layers or set VK_ADD_LAYER_PATH"
    );
    assert!(wgpu_hal::VALIDATION_CANARY.get_and_reset().is_empty());
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        flags: wgpu::InstanceFlags::VALIDATION | wgpu::InstanceFlags::DEBUG,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN));
    let features = wgpu::Features::PASSTHROUGH_SHADERS;
    let adapter = adapters.into_iter().find(|adapter| {
        let info = adapter.get_info();
        println!("Vulkan adapter: {} ({:?}), driver {} {}", info.name, info.device_type, info.driver, info.driver_info);
        if !(matches!(info.device_type, wgpu::DeviceType::DiscreteGpu | wgpu::DeviceType::IntegratedGpu)
                || (std::env::var_os("FRESCO_ALLOW_SOFTWARE_ADAPTER").as_deref() == Some(std::ffi::OsStr::new("1")) && info.device_type == wgpu::DeviceType::Cpu))
            || !adapter.features().contains(features) { return false; }
        // SAFETY: This adapter is Vulkan and the guard keeps its instance alive.
        let hal = unsafe { adapter.as_hal::<Vulkan>() }.unwrap();
        let mut bda = vk::PhysicalDeviceBufferDeviceAddressFeatures::default();
        let mut query = vk::PhysicalDeviceFeatures2::default().push_next(&mut bda);
        // SAFETY: Both feature structures remain alive for the synchronous query.
        unsafe { hal.shared_instance().raw_instance().get_physical_device_features2(hal.raw_physical_device(), &mut query); }
        bda.buffer_device_address == vk::TRUE
    }).expect("a Vulkan adapter supporting bufferDeviceAddress and passthrough is required; CPU adapters require FRESCO_ALLOW_SOFTWARE_ADAPTER=1");
    let limits = wgpu::Limits {
        max_immediate_size: 0,
        ..Default::default()
    };
    let desc = wgpu::DeviceDescriptor {
        label: Some("physical pointer GPU test"),
        required_features: features,
        required_limits: limits,
        ..Default::default()
    };
    // SAFETY: Only Vulkan adapters are enumerated; the guard owns the instance reference.
    let hal = unsafe { adapter.as_hal::<Vulkan>() }.unwrap();
    let mut bda =
        vk::PhysicalDeviceBufferDeviceAddressFeatures::default().buffer_device_address(true);
    // SAFETY: Support was queried above; the callback adds only the supported BDA feature.
    let opened = unsafe {
        hal.open_with_callback(
            features,
            &desc.required_limits,
            &desc.memory_hints,
            Some(Box::new(|args| {
                args.extensions.push(ash::khr::buffer_device_address::NAME);
                *args.create_info = args.create_info.push_next(&mut bda);
            })),
        )
    }
    .unwrap();
    drop(hal);
    // SAFETY: The device was opened on this adapter with this exact descriptor.
    unsafe { adapter.create_device_from_hal::<Vulkan>(opened, &desc) }.unwrap()
}

fn addressed_buffer(device: &wgpu::Device, values: &[u32]) -> (wgpu::Buffer, u64) {
    let byte_count = size_of_val(values);
    let size = u64::try_from(byte_count).unwrap();
    // SAFETY: This test creates only Vulkan devices and keeps the guard alive during import.
    let hal = unsafe { device.as_hal::<Vulkan>() }.unwrap();
    let raw = hal.raw_device().clone();
    let instance = hal.shared_instance().raw_instance();
    // SAFETY: BDA was enabled at device creation; the create info contains no borrowed extension pointers.
    let buffer = unsafe {
        raw.create_buffer(
            &vk::BufferCreateInfo::default()
                .size(size)
                .usage(
                    vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS
                        | vk::BufferUsageFlags::STORAGE_BUFFER
                        | vk::BufferUsageFlags::TRANSFER_SRC,
                )
                .sharing_mode(vk::SharingMode::EXCLUSIVE),
            None,
        )
    }
    .unwrap();
    // SAFETY: The buffer was just created on this live device.
    let requirements = unsafe { raw.get_buffer_memory_requirements(buffer) };
    // SAFETY: The physical device belongs to this live instance.
    let properties =
        unsafe { instance.get_physical_device_memory_properties(hal.raw_physical_device()) };
    let memory_type = (0..properties.memory_type_count)
        .find(|&i| {
            requirements.memory_type_bits & (1 << i) != 0
                && properties.memory_types[usize::try_from(i).unwrap()]
                    .property_flags
                    .contains(
                        vk::MemoryPropertyFlags::HOST_VISIBLE
                            | vk::MemoryPropertyFlags::HOST_COHERENT,
                    )
        })
        .expect("host-visible coherent addressable memory required");
    let mut flags =
        vk::MemoryAllocateFlagsInfo::default().flags(vk::MemoryAllocateFlags::DEVICE_ADDRESS);
    // SAFETY: The memory type satisfies the buffer requirements, and DEVICE_ADDRESS is enabled.
    let memory = unsafe {
        raw.allocate_memory(
            &vk::MemoryAllocateInfo::default()
                .allocation_size(requirements.size)
                .memory_type_index(memory_type)
                .push_next(&mut flags),
            None,
        )
    }
    .unwrap();
    // SAFETY: This allocation matches the queried size/type/alignment requirements; offset zero is aligned.
    unsafe { raw.bind_buffer_memory(buffer, memory, 0) }.unwrap();
    // SAFETY: Memory is host visible, not mapped elsewhere, and size is within the allocation.
    let mapped = unsafe { raw.map_memory(memory, 0, size, vk::MemoryMapFlags::empty()) }.unwrap();
    // SAFETY: Source is byte_count initialized bytes; destination is a distinct mapping of at least that size.
    unsafe {
        std::ptr::copy_nonoverlapping(
            values.as_ptr().cast::<u8>(),
            mapped.cast::<u8>(),
            byte_count,
        );
    }
    // SAFETY: This memory was mapped above and the host write is complete. Coherence makes flushing unnecessary.
    unsafe {
        raw.unmap_memory(memory);
    }
    let bda = ash::khr::buffer_device_address::Device::new(instance, &raw);
    // SAFETY: The buffer has SHADER_DEVICE_ADDRESS usage and bound DEVICE_ADDRESS memory on a BDA-enabled device.
    let address = unsafe {
        bda.get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(buffer))
    };
    assert_ne!(address, 0);
    let keep_device_alive = device.clone();
    let release = Box::new(move || {
        // SAFETY: wgpu invokes the ownership callback after all GPU uses have completed; the device is retained.
        unsafe {
            raw.destroy_buffer(buffer, None);
        }
        // SAFETY: Its only bound buffer has just been destroyed and no mappings remain.
        unsafe {
            raw.free_memory(memory, None);
        }
        drop(keep_device_alive);
    });
    // SAFETY: This is an initialized live Vulkan buffer on the importing device;
    // the ownership callback retains that device and releases the allocation exactly once.
    let imported = unsafe { wgpu::hal::vulkan::Buffer::from_raw_externally_owned(buffer, release) };
    drop(hal);
    // SAFETY: The imported buffer belongs to this device and its size/usages match this descriptor.
    let buffer = unsafe {
        device.create_buffer_from_hal::<Vulkan>(
            imported,
            &wgpu::BufferDescriptor {
                label: Some("addressable data with guards"),
                size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            },
        )
    };
    (buffer, address)
}
