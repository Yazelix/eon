use super::*;

#[test]
fn hosted_frames_prepare_draw_and_resize_without_a_surface() {
    let (device, queue) = wgpu::Device::noop(&DeviceDescriptor::default());
    let mut renderer = Renderer::hosted(
        device.clone(),
        queue.clone(),
        wgpu::TextureFormat::Rgba8Unorm,
        (320, 180),
        1.0,
        clipping_test_fonts(),
    )
    .unwrap();
    let mut scene = padding_scene();
    scene.content[0].cells[0].text = "A".into();
    for (frame, (size, scale)) in [
        (PhysicalSize::new(320, 180), 1.0),
        (PhysicalSize::new(640, 360), 2.0),
    ]
    .into_iter()
    .enumerate()
    {
        if frame == 0 {
            renderer.prepare_hosted(&scene, true, 1).unwrap();
        } else {
            renderer.resize(size, scale);
            let (content_changed, _, _) = renderer
                .prepare_frame(
                    Some(&scene),
                    None,
                    0.0,
                    None,
                    &[],
                    None,
                    WorkspaceFocus::Terminal,
                    "",
                    true,
                    "",
                    1,
                )
                .unwrap();
            assert!(content_changed, "resize must invalidate prepared content");
        }
        assert_eq!(renderer.size(), size);
        assert_eq!(renderer.metrics().scale, scale as f32);
        assert!(renderer.vertex_count > 0);
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: size.width,
                height: size.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = target.create_view(&TextureViewDescriptor::default());
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(wgpu::Color::BLACK),
                        store: StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            renderer.draw_hosted(&mut pass).unwrap();
        }
        queue.submit(Some(encoder.finish()));
        renderer.end_frame_hosted();
    }
    assert!(renderer.windowed.is_none());
    assert_eq!(
        renderer.render(
            Some(&scene),
            None,
            0.0,
            None,
            &[],
            None,
            WorkspaceFocus::Terminal,
            "",
            true,
            "",
            1
        ),
        Err(RenderError(
            "a hosted Venus renderer has no presentation surface".into()
        ))
    );
}

#[test]
fn allocation_hints_favor_memory_only_on_non_cpu_adapters() {
    use wgpu::{DeviceType, MemoryHints};

    for device_type in [
        DeviceType::Other,
        DeviceType::IntegratedGpu,
        DeviceType::DiscreteGpu,
        DeviceType::VirtualGpu,
    ] {
        assert!(matches!(
            device_memory_hints(device_type),
            MemoryHints::MemoryUsage
        ));
    }
    assert!(matches!(
        device_memory_hints(DeviceType::Cpu),
        MemoryHints::Performance
    ));
}

#[test]
fn device_loss_blocks_surface_recovery_but_destroy_does_not() {
    let lost = AtomicBool::new(false);

    assert!(!record_device_loss(&lost, DeviceLostReason::Destroyed));
    assert_eq!(ensure_device_available(&lost), Ok(()));

    assert!(record_device_loss(&lost, DeviceLostReason::Unknown));
    assert_eq!(
        ensure_device_available(&lost),
        Err(RenderError(DEVICE_LOST.into()))
    );
}

#[test]
fn uncaptured_errors_remain_fatal_until_device_loss() {
    let lost = AtomicBool::new(false);
    let out_of_memory = || wgpu::Error::OutOfMemory {
        source: Box::new(std::io::Error::other("test GPU allocation failure")),
    };

    assert!(
        std::panic::catch_unwind(|| handle_uncaptured_gpu_error(&lost, out_of_memory())).is_err()
    );

    assert!(record_device_loss(&lost, DeviceLostReason::Unknown));
    handle_uncaptured_gpu_error(&lost, out_of_memory());
}

#[test]
fn content_cache_separates_generations_and_viewer_state() {
    let key = |generation, dialog| ContentKey {
        generation,
        column_revisions: vec![Some(4), None],
        scroll_offset: 0.0,
        workspace_focus: WorkspaceFocus::Terminal,
        blink_visible: true,
        preedit: String::new(),
        status: String::new(),
        hyperlink: None,
        header: HeaderInteraction::default(),
        scrollback_label: None,
        dialog,
        cpu: None,
        ram: None,
        disk: None,
        clock: None,
        quota_tooltips: Vec::new(),
        images: Vec::new(),
    };

    let dialog = |kind, scroll| {
        Some(DialogScene::new(
            kind,
            vec![crate::ShortcutGroup::new(
                "Host",
                (0..32)
                    .map(|index| crate::ShortcutRow::new("Alt+Z", format!("Entry {index}")))
                    .collect(),
            )],
            PhysicalSize::new(320, 180),
            CellMetrics::for_scale(1.0),
            scroll,
            &crate::CellText(CellMetrics::for_scale(1.0)),
        ))
    };
    let viewer = |scroll| dialog(crate::DialogKind::Shortcuts, scroll);
    assert_ne!(key(1, None), key(2, None));
    assert_ne!(key(1, None), key(1, viewer(0.0)));
    assert_ne!(key(1, viewer(0.0)), key(1, viewer(1.0)));
    // A quota hover's reset wording turns with the local day, without a new generation.
    assert_ne!(
        key(1, None),
        ContentKey {
            quota_tooltips: vec!["Codex 5h window\nResets Thu 00:30".into()],
            ..key(1, None)
        }
    );
    // The prompt replaces the viewer's text at the same scroll.
    assert_ne!(
        key(1, viewer(0.0)),
        key(
            1,
            dialog(
                crate::DialogKind::CloseLastTab {
                    terminals: 32,
                    detach: None,
                    workspace: None,
                },
                0.0,
            ),
        )
    );
    // Another column's frame repaints without a new selected-terminal generation.
    assert_ne!(
        key(1, None),
        ContentKey {
            column_revisions: vec![Some(5), None],
            ..key(1, None)
        }
    );
}

#[test]
fn background_opacity_selects_only_proved_surface_modes() {
    assert_eq!(
        surface_alpha_mode(1.0, wgpu::Backend::Vulkan, &[CompositeAlphaMode::Inherit]).unwrap(),
        CompositeAlphaMode::Auto
    );
    assert_eq!(
        surface_alpha_mode(
            0.88,
            wgpu::Backend::Vulkan,
            &[
                CompositeAlphaMode::Opaque,
                CompositeAlphaMode::PreMultiplied,
            ],
        )
        .unwrap(),
        CompositeAlphaMode::PreMultiplied
    );
    assert_eq!(
        surface_alpha_mode(
            0.80,
            wgpu::Backend::Metal,
            &[
                CompositeAlphaMode::Opaque,
                CompositeAlphaMode::PostMultiplied,
            ],
        )
        .unwrap(),
        CompositeAlphaMode::PostMultiplied
    );
    assert_eq!(
        surface_alpha_mode(
            0.80,
            wgpu::Backend::Metal,
            &[
                CompositeAlphaMode::Opaque,
                CompositeAlphaMode::PreMultiplied,
                CompositeAlphaMode::PostMultiplied,
            ],
        )
        .unwrap(),
        CompositeAlphaMode::PreMultiplied
    );
    for backend in [wgpu::Backend::Metal, wgpu::Backend::Vulkan] {
        assert!(surface_alpha_mode(0.0, backend, &[CompositeAlphaMode::Opaque]).is_err());
    }
    assert!(
        surface_alpha_mode(
            0.5,
            wgpu::Backend::Vulkan,
            &[CompositeAlphaMode::PostMultiplied],
        )
        .is_err()
    );
    assert!(
        surface_alpha_mode(
            0.5,
            wgpu::Backend::Vulkan,
            &[
                CompositeAlphaMode::Opaque,
                CompositeAlphaMode::PostMultiplied,
            ],
        )
        .is_err()
    );
}

#[test]
fn background_opacity_premultiplies_clear_colors() {
    let source = SceneColor {
        r: 128,
        g: 0,
        b: 255,
    };
    let opaque = clear_color(source, 1.0, true);
    let translucent = clear_color(source, 0.5, true);
    let encoded = clear_color(source, 1.0, false);

    assert!((opaque.r - 0.215_860_5).abs() < 0.000_001);
    assert_eq!(opaque.a, 1.0);
    assert!((translucent.r - 0.107_930_25).abs() < 0.000_001);
    assert_eq!(translucent.g, 0.0);
    assert_eq!(translucent.b, 0.5);
    assert_eq!(translucent.a, 0.5);
    assert_eq!(encoded.r, 128.0 / 255.0);
}
