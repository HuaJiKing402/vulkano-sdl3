//! Vulkano + SDL3 渲染三角形的起始模板。
//!
//! SDL3 负责窗口和事件，vulkano 负责 Vulkan 初始化、交换链、图形管线与绘制。

use std::{sync::Arc, time::Duration};

use sdl3::event::Event;
use sdl3::keyboard::Keycode;
use vulkano::{
    buffer::{Buffer, BufferContents, BufferCreateInfo, BufferUsage},
    command_buffer::{
        allocator::StandardCommandBufferAllocator, AutoCommandBufferBuilder, CommandBufferUsage,
        RenderPassBeginInfo, SubpassBeginInfo, SubpassContents, SubpassEndInfo,
    },
    device::{
        physical::PhysicalDeviceType, Device, DeviceCreateInfo, DeviceExtensions, Queue,
        QueueCreateInfo, QueueFlags,
    },
    image::{view::ImageView, ImageUsage},
    instance::{Instance, InstanceCreateFlags, InstanceCreateInfo},
    memory::allocator::{AllocationCreateInfo, MemoryTypeFilter, StandardMemoryAllocator},
    pipeline::{
        graphics::{
            color_blend::{ColorBlendAttachmentState, ColorBlendState},
            input_assembly::InputAssemblyState,
            multisample::MultisampleState,
            rasterization::RasterizationState,
            vertex_input::{Vertex, VertexDefinition},
            viewport::{Viewport, ViewportState},
            GraphicsPipelineCreateInfo,
        },
        DynamicState, GraphicsPipeline, PipelineLayout, PipelineShaderStageCreateInfo,
    },
    pipeline::layout::PipelineDescriptorSetLayoutCreateInfo,
    render_pass::{Framebuffer, FramebufferCreateInfo, RenderPass, Subpass},
    swapchain::{
        acquire_next_image, Surface, Swapchain, SwapchainCreateInfo, SwapchainPresentInfo,
    },
    sync::{self, GpuFuture},
    Validated, VulkanError, VulkanLibrary,
};

mod vs {
    vulkano_shaders::shader! {
        bytes: "shaders/tri.vert.spv",
    }
}

mod fs {
    vulkano_shaders::shader! {
        bytes: "shaders/tri.frag.spv",
    }
}

#[derive(BufferContents, Vertex, Clone, Copy)]
#[repr(C)]
struct MyVertex {
    #[format(R32G32_SFLOAT)]
    position: [f32; 2],
}

struct SwapchainBundle {
    swapchain: Arc<Swapchain>,
    framebuffers: Vec<Arc<Framebuffer>>,
    viewport: Viewport,
}

fn create_swapchain_bundle(
    device: &Arc<Device>,
    physical: &Arc<vulkano::device::physical::PhysicalDevice>,
    surface: &Arc<Surface>,
    render_pass: &Arc<RenderPass>,
    extent: [u32; 2],
) -> anyhow::Result<SwapchainBundle> {
    let capabilities = physical.surface_capabilities(surface, Default::default())?;
    let format = physical.surface_formats(surface, Default::default())?[0].0;

    let (swapchain, images) = Swapchain::new(
        device.clone(),
        surface.clone(),
        SwapchainCreateInfo {
            min_image_count: capabilities.min_image_count,
            image_format: format,
            image_extent: extent,
            image_usage: ImageUsage::COLOR_ATTACHMENT,
            composite_alpha: capabilities
                .supported_composite_alpha
                .into_iter()
                .next()
                .unwrap(),
            ..Default::default()
        },
    )?;

    let framebuffers = images
        .iter()
        .map(|image| {
            let view = ImageView::new_default(image.clone())?;
            Ok(Framebuffer::new(
                render_pass.clone(),
                FramebufferCreateInfo {
                    attachments: vec![view],
                    ..Default::default()
                },
            )?)
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    Ok(SwapchainBundle {
        swapchain,
        framebuffers,
        viewport: Viewport {
            offset: [0.0, 0.0],
            extent: [extent[0] as f32, extent[1] as f32],
            depth_range: 0.0..=1.0,
        },
    })
}

fn main() -> anyhow::Result<()> {
    // 1. SDL3：初始化、创建支持 Vulkan 的窗口、准备事件泵
    let sdl = sdl3::init().map_err(|e| anyhow::anyhow!("SDL init failed: {e}"))?;
    let video = sdl.video().map_err(|e| anyhow::anyhow!("SDL video failed: {e}"))?;
    let window = Arc::new(
        video
            .window("Vulkano + SDL3 triangle", 800, 600)
            .vulkan()
            .position_centered()
            .resizable()
            .build()
            .map_err(|e| anyhow::anyhow!("SDL window failed: {e}"))?,
    );
    let mut events = sdl
        .event_pump()
        .map_err(|e| anyhow::anyhow!("SDL event pump failed: {e}"))?;

    // 2. Vulkan Instance：扩展列表从 SDL 窗口查询
    let library = VulkanLibrary::new()?;
    let required_extensions = Surface::required_extensions(&*window)?;
    let instance = Instance::new(
        library,
        InstanceCreateInfo {
            flags: InstanceCreateFlags::ENUMERATE_PORTABILITY,
            enabled_extensions: required_extensions,
            ..Default::default()
        },
    )?;

    // SDL3 的 Window 不是 Send + Sync，因此用 from_window_ref；window 的生命周期覆盖了 surface
    let surface = unsafe { Surface::from_window_ref(instance.clone(), &*window) }?;

    // 3. 选设备与队列：要求支持 swapchain，且队列能向该 surface 呈现
    let device_extensions = DeviceExtensions {
        khr_swapchain: true,
        ..DeviceExtensions::empty()
    };
    let (physical, queue_family_index) = instance
        .enumerate_physical_devices()?
        .into_iter()
        .filter(|p| p.supported_extensions().contains(&device_extensions))
        .filter_map(|p| {
            let family = p
                .queue_family_properties()
                .iter()
                .enumerate()
                .find(|(i, q)| {
                    q.queue_flags.intersects(QueueFlags::GRAPHICS)
                        && p.presentation_support(*i as u32, &*window).unwrap_or(false)
                })
                .map(|(i, _)| i as u32);
            family.map(|i| (p, i))
        })
        .min_by_key(|(p, _)| match p.properties().device_type {
            PhysicalDeviceType::DiscreteGpu => 0,
            PhysicalDeviceType::IntegratedGpu => 1,
            PhysicalDeviceType::VirtualGpu => 2,
            PhysicalDeviceType::Cpu => 3,
            PhysicalDeviceType::Other => 4,
            _ => 5,
        })
        .ok_or_else(|| anyhow::anyhow!("no suitable physical device"))?;

    println!(
        "Using device: {} ({:?})",
        physical.properties().device_name,
        physical.properties().device_type
    );

    let (device, queues) = Device::new(
        physical.clone(),
        DeviceCreateInfo {
            enabled_extensions: device_extensions,
            queue_create_infos: vec![QueueCreateInfo {
                queue_family_index,
                ..Default::default()
            }],
            ..Default::default()
        },
    )?;
    let queue: Arc<Queue> = queues.into_iter().next().unwrap();

    // 4. 渲染通道与交换链
    let render_pass = vulkano::single_pass_renderpass!(
        device.clone(),
        attachments: {
            color: {
                format: physical.surface_formats(&surface, Default::default())?[0].0,
                samples: 1,
                load_op: Clear,
                store_op: Store,
            }
        },
        pass: {
            color: [color],
            depth_stencil: {}
        }
    )?;

    let window_size = window.size();
    let mut bundle = create_swapchain_bundle(
        &device,
        &physical,
        &surface,
        &render_pass,
        [window_size.0, window_size.1],
    )?;

    // 5. 顶点缓冲
    let memory_allocator = Arc::new(StandardMemoryAllocator::new_default(device.clone()));
    let vertices = [
        MyVertex { position: [-0.5, -0.25] },
        MyVertex { position: [0.0, 0.5] },
        MyVertex { position: [0.25, -0.1] },
    ];
    let vertex_buffer = Buffer::from_iter(
        memory_allocator,
        BufferCreateInfo {
            usage: BufferUsage::VERTEX_BUFFER,
            ..Default::default()
        },
        AllocationCreateInfo {
            memory_type_filter: MemoryTypeFilter::PREFER_DEVICE
                | MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
            ..Default::default()
        },
        vertices,
    )?;

    // 6. 图形管线
    let vs = vs::load(device.clone())?
        .entry_point("main")
        .ok_or_else(|| anyhow::anyhow!("vertex shader entry point not found"))?;
    let fs = fs::load(device.clone())?
        .entry_point("main")
        .ok_or_else(|| anyhow::anyhow!("fragment shader entry point not found"))?;
    let vertex_input_state = MyVertex::per_vertex()
        .definition(&vs)
        .map_err(|e| anyhow::anyhow!("vertex definition error: {e}"))?;
    let stages = [
        PipelineShaderStageCreateInfo::new(vs.clone()),
        PipelineShaderStageCreateInfo::new(fs.clone()),
    ];
    let layout = PipelineLayout::new(
        device.clone(),
        PipelineDescriptorSetLayoutCreateInfo::from_stages(&stages)
            .into_pipeline_layout_create_info(device.clone())?,
    )?;
    let subpass = Subpass::from(render_pass.clone(), 0)
        .ok_or_else(|| anyhow::anyhow!("subpass 0 not found"))?;
    let pipeline = GraphicsPipeline::new(
        device.clone(),
        None,
        GraphicsPipelineCreateInfo {
            stages: stages.into_iter().collect(),
            vertex_input_state: Some(vertex_input_state),
            input_assembly_state: Some(InputAssemblyState::default()),
            viewport_state: Some(ViewportState::default()),
            rasterization_state: Some(RasterizationState::default()),
            multisample_state: Some(MultisampleState::default()),
            color_blend_state: Some(ColorBlendState {
                attachments: vec![ColorBlendAttachmentState::default()],
                ..Default::default()
            }),
            dynamic_state: [DynamicState::Viewport].into_iter().collect(),
            subpass: Some(subpass.into()),
            ..GraphicsPipelineCreateInfo::layout(layout)
        },
    )?;

    // 7. 帧循环
    let command_allocator = Arc::new(StandardCommandBufferAllocator::new(
        device.clone(),
        Default::default(),
    ));
    let mut previous_frame_end: Option<Box<dyn GpuFuture>> =
        Some(sync::now(device.clone()).boxed());
    let mut recreate_swapchain = false;
    let mut last_extent = window_size;
    let max_frames: u32 = std::env::var("TRIANGLE_MAX_FRAMES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let mut frame_count = 0u32;

    'running: loop {
        for event in events.poll_iter() {
            match event {
                Event::Quit { .. }
                | Event::KeyDown {
                    keycode: Some(Keycode::Escape),
                    ..
                } => break 'running,
                _ => {}
            }
        }

        let current = window.size();
        if current != last_extent {
            recreate_swapchain = true;
            last_extent = current;
        }

        previous_frame_end.as_mut().unwrap().cleanup_finished();
        if current.0 == 0 || current.1 == 0 {
            std::thread::sleep(Duration::from_millis(16));
            continue;
        }

        if recreate_swapchain {
            bundle = create_swapchain_bundle(
                &device,
                &physical,
                &surface,
                &render_pass,
                [current.0, current.1],
            )?;
            recreate_swapchain = false;
        }

        let (image_index, suboptimal, acquire_future) =
            match acquire_next_image(bundle.swapchain.clone(), None) {
                Ok(result) => result,
                Err(Validated::Error(VulkanError::OutOfDate)) => {
                    recreate_swapchain = true;
                    continue;
                }
                Err(e) => return Err(anyhow::anyhow!("acquire_next_image failed: {e}")),
            };
        if suboptimal {
            recreate_swapchain = true;
        }

        let mut builder = AutoCommandBufferBuilder::primary(
            command_allocator.clone(),
            queue.queue_family_index(),
            CommandBufferUsage::OneTimeSubmit,
        )?;
        unsafe {
            builder
                .begin_render_pass(
                    RenderPassBeginInfo {
                        clear_values: vec![Some([0.02, 0.02, 0.06, 1.0].into())],
                        ..RenderPassBeginInfo::framebuffer(
                            bundle.framebuffers[image_index as usize].clone(),
                        )
                    },
                    SubpassBeginInfo {
                        contents: SubpassContents::Inline,
                        ..Default::default()
                    },
                )?
                .set_viewport(0, [bundle.viewport.clone()].into_iter().collect())?
                .bind_pipeline_graphics(pipeline.clone())?
                .bind_vertex_buffers(0, vertex_buffer.clone())?
                .draw(3, 1, 0, 0)?
                .end_render_pass(SubpassEndInfo::default())?;
        }
        let command_buffer = builder.build()?;

        let future = previous_frame_end
            .take()
            .unwrap()
            .join(acquire_future)
            .then_execute(queue.clone(), command_buffer)?
            .then_swapchain_present(
                queue.clone(),
                SwapchainPresentInfo::swapchain_image_index(
                    bundle.swapchain.clone(),
                    image_index,
                ),
            )
            .then_signal_fence_and_flush();

        match future {
            Ok(future) => previous_frame_end = Some(future.boxed()),
            Err(Validated::Error(VulkanError::OutOfDate)) => {
                recreate_swapchain = true;
                previous_frame_end = Some(sync::now(device.clone()).boxed());
            }
            Err(e) => return Err(anyhow::anyhow!("present failed: {e}")),
        }

        frame_count += 1;
        if max_frames > 0 && frame_count >= max_frames {
            println!("rendered {frame_count} frames, exiting");
            break;
        }
        std::thread::sleep(Duration::from_millis(16));
    }

    // 丢弃 future 即等待 GPU 完成
    drop(previous_frame_end.take());
    println!("done");
    Ok(())
}
