use super::*;

fn blink_test_renderer() -> Renderer {
    let (device, queue) = wgpu::Device::noop(&DeviceDescriptor::default());
    Renderer::hosted(
        device,
        queue,
        wgpu::TextureFormat::Rgba8Unorm,
        (320, 180),
        1.0,
        clipping_test_fonts(),
    )
    .unwrap()
}

fn blink_test_scene() -> Scene {
    let mut scene = padding_scene();
    scene.content[0].cells[0].text = "A".into();
    scene.cursor = Some(crate::DrawCursor {
        visible: true,
        blinking: true,
        password_input: false,
        shape: orbit_protocol::CursorShape::Block,
        column: 0,
        row: 0,
        at_wide_tail: false,
        color: scene.foreground,
        explicit_color: false,
    });
    scene
}

fn blink_test_workspace(metrics: CellMetrics) -> WorkspaceScene {
    use eon_workspace_protocol::v44::{Column, Pane};
    let mut snapshot = super::outlines::popup_title_snapshot("", 4.0);
    let tab = &mut snapshot.tabs[0];
    tab.selected_popup = None;
    tab.popups.clear();
    tab.selected_pane = Some("p0".into());
    tab.columns = (0..2)
        .map(|index| Column {
            expanded: format!("p{index}"),
            panes: vec![Pane {
                id: format!("p{index}"),
                label: format!("p{index}"),
                session: format!("s{index}"),
                endpoint: format!("/tmp/blink-{index}.sock").into_bytes(),
                pinned_name: None,
                tag: None,
                live: true,
                attention: None,
            }],
        })
        .collect();
    WorkspaceScene::from_snapshot(
        &snapshot,
        PhysicalSize::new(900, 600),
        metrics,
        0.0,
        0.0,
        |_, text| (text.into(), text.len() as f32 * metrics.width),
    )
}

#[test]
fn cursor_blink_reuses_grid_and_glyphon_preparation() {
    let mut renderer = blink_test_renderer();
    let scene = blink_test_scene();
    renderer.prepare_hosted(&scene, true, 1).unwrap();
    let before = (renderer.grid_rebuilds, renderer.glyphon_prepares);
    let text_buffers = renderer
        .text
        .iter()
        .map(|text| std::ptr::from_ref(&*text.buffer))
        .collect::<Vec<_>>();
    let grid_vertices = renderer.vertex_count;
    let phases = [false, true, false, true, false, true];
    let mut rebuilt = Vec::new();
    let mut cursor_visible = Vec::new();
    for visible in phases {
        let (changed, _, _) = renderer
            .prepare_frame(
                Some(&scene),
                None,
                0.0,
                None,
                &[],
                None,
                WorkspaceFocus::Terminal,
                "",
                visible,
                "",
                1,
            )
            .unwrap();
        rebuilt.push(changed);
        cursor_visible.push(renderer.dynamic_vertex_count > 0);
    }
    assert_eq!(
        (
            renderer.grid_rebuilds - before.0,
            renderer.glyphon_prepares - before.1
        ),
        (0, 0),
        "cursor phases must not rebuild the grid or prepare glyphs",
    );
    assert_eq!(rebuilt, vec![false; phases.len()]);
    assert_eq!(cursor_visible, phases);
    assert_eq!(renderer.vertex_count, grid_vertices);
    assert_eq!(
        renderer
            .text
            .iter()
            .map(|text| std::ptr::from_ref(&*text.buffer))
            .collect::<Vec<_>>(),
        text_buffers,
    );
}

#[test]
fn sgr_blink_still_rebuilds_foregrounds_and_prepares_glyphs() {
    let mut renderer = blink_test_renderer();
    let mut scene = padding_scene();
    let cell = &mut scene.content[0].cells[0];
    cell.text = "A".into();
    cell.style.blink = true;
    cell.style.underline = orbit_protocol::Underline::Single;
    renderer.prepare_hosted(&scene, true, 1).unwrap();
    let before = (renderer.grid_rebuilds, renderer.glyphon_prepares);
    let lit_vertices = renderer.vertex_count;
    for visible in [false, true, false, true, false, true] {
        let (changed, _, _) = renderer
            .prepare_frame(
                Some(&scene),
                None,
                0.0,
                None,
                &[],
                None,
                WorkspaceFocus::Terminal,
                "",
                visible,
                "",
                1,
            )
            .unwrap();
        assert!(changed);
        assert_eq!(renderer.text.is_empty(), !visible);
        assert_eq!(renderer.vertex_count == lit_vertices, visible);
    }
    assert_eq!(
        (
            renderer.grid_rebuilds - before.0,
            renderer.glyphon_prepares - before.1
        ),
        (6, 6),
    );
}

#[test]
fn sgr_blink_in_other_columns_and_previews_invalidates_only_visible_foregrounds() {
    use orbit_protocol::session::VerticalDirection;

    let mut renderer = blink_test_renderer();
    renderer.resize(PhysicalSize::new(900, 600), 1.0);
    let workspace = blink_test_workspace(renderer.metrics());
    let scene = padding_scene();
    for (index, invisible) in [false, true].into_iter().enumerate() {
        let mut other = padding_scene();
        other.content[0].cells[0].text = "A".into();
        other.content[0].cells[0].style.blink = true;
        other.content[0].cells[0].style.invisible = invisible;
        let columns = [Some(&other)];
        let generation = index as u64 + 1;
        renderer
            .prepare_frame(
                Some(&scene),
                None,
                0.0,
                Some(&workspace),
                &columns,
                None,
                WorkspaceFocus::Terminal,
                "",
                true,
                "",
                generation,
            )
            .unwrap();
        let before = (renderer.grid_rebuilds, renderer.glyphon_prepares);
        let (changed, _, _) = renderer
            .prepare_frame(
                Some(&scene),
                None,
                0.0,
                Some(&workspace),
                &columns,
                None,
                WorkspaceFocus::Terminal,
                "",
                false,
                "",
                generation,
            )
            .unwrap();
        assert_eq!(changed, !invisible);
        let expected = u64::from(!invisible);
        assert_eq!(
            (
                renderer.grid_rebuilds - before.0,
                renderer.glyphon_prepares - before.1
            ),
            (expected, expected),
        );
    }

    let mut generation = 3;
    for direction in [VerticalDirection::Up, VerticalDirection::Down] {
        let offset = renderer.metrics().height
            * if direction == VerticalDirection::Up {
                0.5
            } else {
                -0.5
            };
        for invisible in [false, true] {
            let mut row = scene.content[0].clone();
            row.cells[0].text = "A".into();
            row.cells[0].style.blink = true;
            row.cells[0].style.invisible = invisible;
            let preview = ScenePreview::Viewport {
                frame_revision: scene.revision,
                direction,
                edge_reached: false,
                rows: vec![row],
            };
            renderer
                .prepare_frame(
                    Some(&scene),
                    Some(&preview),
                    offset,
                    None,
                    &[],
                    None,
                    WorkspaceFocus::Terminal,
                    "",
                    true,
                    "",
                    generation,
                )
                .unwrap();
            let before = (renderer.grid_rebuilds, renderer.glyphon_prepares);
            let (changed, _, _) = renderer
                .prepare_frame(
                    Some(&scene),
                    Some(&preview),
                    offset,
                    None,
                    &[],
                    None,
                    WorkspaceFocus::Terminal,
                    "",
                    false,
                    "",
                    generation,
                )
                .unwrap();
            assert_eq!(changed, !invisible);
            assert!(renderer.text.is_empty());
            let expected = u64::from(!invisible);
            assert_eq!(
                (
                    renderer.grid_rebuilds - before.0,
                    renderer.glyphon_prepares - before.1
                ),
                (expected, expected),
            );
            generation += 1;
        }
        let preview = ScenePreview::TerminalOwned {
            frame_revision: scene.revision,
            direction,
        };
        renderer
            .prepare_frame(
                Some(&scene),
                Some(&preview),
                offset,
                None,
                &[],
                None,
                WorkspaceFocus::Terminal,
                "",
                true,
                "",
                generation,
            )
            .unwrap();
        let before = (renderer.grid_rebuilds, renderer.glyphon_prepares);
        let (changed, _, _) = renderer
            .prepare_frame(
                Some(&scene),
                Some(&preview),
                offset,
                None,
                &[],
                None,
                WorkspaceFocus::Terminal,
                "",
                false,
                "",
                generation,
            )
            .unwrap();
        assert!(!changed);
        assert_eq!((renderer.grid_rebuilds, renderer.glyphon_prepares), before);
        generation += 1;
    }
}

#[test]
fn dynamic_cursors_keep_column_clips_and_selected_scroll_dialog_gates() {
    use orbit_protocol::CursorShape;

    let mut renderer = blink_test_renderer();
    for scale in [1.0, 2.0] {
        renderer.resize(PhysicalSize::new(900, 600), scale);
        let metrics = renderer.metrics();
        let workspace = blink_test_workspace(metrics);
        let mut selected = blink_test_scene();
        let mut other = selected.clone();
        other.cursor.as_mut().unwrap().blinking = false;
        let columns = [Some(&other)];
        let dialog = DialogScene::new(
            crate::DialogKind::Shortcuts,
            vec![],
            PhysicalSize::new(900, 600),
            metrics,
            0.0,
            &crate::CellText(metrics),
        );
        for frames in [false, true] {
            renderer.pane_frames = frames;
            for shape in [
                CursorShape::Block,
                CursorShape::Bar,
                CursorShape::Underline,
                CursorShape::BlockHollow,
            ] {
                selected.cursor.as_mut().unwrap().shape = shape;
                for trail in [false, true] {
                    renderer.cursor_tail = trail.then_some((
                        selected.foreground,
                        cursor_outline(selected.foreground),
                        1.0,
                    ));
                    for (offset, dialog) in [
                        (0.0, None),
                        (metrics.height * 0.25, None),
                        (0.0, Some(&dialog)),
                    ] {
                        for visible in [false, true] {
                            renderer.reset_cursor_animation();
                            let batch = renderer.build_dynamic_cursors(
                                Some(&selected),
                                offset,
                                Some(&workspace),
                                &columns,
                                dialog,
                                visible,
                            );
                            let quads = painted_rects(&batch).collect::<Vec<_>>();
                            let other_area = workspace.other_columns[0].terminal;
                            let has_other = quads
                                .iter()
                                .any(|quad| quad.intersection(other_area).is_some());
                            let has_selected = quads
                                .iter()
                                .any(|quad| quad.intersection(workspace.terminal).is_some());
                            assert!(
                                has_other,
                                "selected trail suppression cannot hide other columns"
                            );
                            assert_eq!(
                                has_selected,
                                visible && (!trail || (offset == 0.0 && dialog.is_none()))
                            );
                            assert!(
                                quads.iter().all(|quad| {
                                    [workspace.terminal, other_area].into_iter().any(|area| {
                                        quad.left >= area.left - 0.001
                                            && quad.top >= area.top - 0.001
                                            && quad.right() <= area.right() + 0.001
                                            && quad.bottom() <= area.bottom() + 0.001
                                    })
                                }),
                                "cursor paint escaped its column"
                            );
                        }
                    }
                }
            }
        }
        renderer.terminal.cursor_handoff = true;
        let batch =
            renderer.build_dynamic_cursors(None, 0.0, Some(&workspace), &columns, None, true);
        assert!(
            !batch.bytes.is_empty(),
            "selected attachment gap cannot hide other columns"
        );
        renderer.rebuild_if_needed(
            None,
            None,
            0.0,
            Some(&workspace),
            &columns,
            None,
            WorkspaceFocus::Terminal,
            "",
            true,
            "",
            1,
        );
        assert_eq!(renderer.cursor_vertex_boundary, renderer.vertex_count);
        assert!(
            renderer.cursor_vertex_boundary > 0,
            "other cursors must draw after the background"
        );
    }
}

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
    let scene = blink_test_scene();
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
        assert!(renderer.dynamic_vertex_count > 0);
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: size.width * 2,
                height: size.height * 2,
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
            pass.set_viewport(
                size.width as f32 / 2.0,
                size.height as f32 / 2.0,
                size.width as f32,
                size.height as f32,
                0.0,
                1.0,
            );
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
        text_blink_visible: true,
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
