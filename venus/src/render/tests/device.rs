use super::*;

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
        clock: None,
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
        ))
    };
    let viewer = |scroll| dialog(crate::DialogKind::Shortcuts, scroll);
    assert_ne!(key(1, None), key(2, None));
    assert_ne!(key(1, None), key(1, viewer(0.0)));
    assert_ne!(key(1, viewer(0.0)), key(1, viewer(1.0)));
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
