use super::cells::tests::plain_style;
use super::*;
use crate::{DrawCell, DrawRow, SceneGraphics};
use orbit_protocol::{CellWidth, Screen};

mod device;
mod outlines;
mod padding;
mod shaping;
mod text_clip;

#[test]
fn unset_frame_timings_skip_the_real_clock_and_writer_paths() {
    use crate::transport::{FRAME_CLOCK_READS, frame_elapsed, frame_time, frame_timings_path};
    assert!(
        std::env::var_os("EON_VENUS_FRAME_TIMINGS").is_none(),
        "run through isolated eon-test"
    );
    assert!(frame_timings_path().is_none());
    let reads = FRAME_CLOCK_READS.get();
    assert_eq!(frame_time(), None);
    assert_eq!(frame_elapsed(Some(0)), 0);
    let outcome =
        PresentOutcome::Presented(frame_time().map(|_| panic!("disabled timing row built")));
    let mut log = FrameTimings::from_environment();
    log.record(outcome, Some(1), None, 0);
    assert!(
        log.sender.is_none() && log.writer.is_none(),
        "no writer means no row encoding, allocation or file I/O"
    );
    assert_eq!(
        FRAME_CLOCK_READS.get(),
        reads,
        "disabled probes must not sample the diagnostic clock"
    );
}

#[test]
fn frame_timings_append_successful_presents_and_flush_on_drop() {
    let root = std::env::temp_dir().join(format!("venus-frame-timings-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("frames.jsonl");
    let rendered = PresentOutcome::Presented(Some(RenderTimings {
        present_ns: 100,
        render_ns: 20,
        rebuild_ns: 10,
        prepare_ns: 5,
        acquire_ns: 30,
        submit_ns: 2,
        full_rebuild: true,
    }));
    std::fs::write(&path, "existing\n").unwrap();
    {
        let mut log = FrameTimings::new(Some(&path));
        for outcome in [
            PresentOutcome::Deferred,
            PresentOutcome::Occluded,
            PresentOutcome::Recovered,
            PresentOutcome::Presented(None),
        ] {
            log.record(outcome, Some(7), Some(50), 3);
        }
        log.record(rendered, Some(7), Some(50), 3);
        log.record(
            PresentOutcome::Presented(Some(RenderTimings {
                full_rebuild: false,
                ..RenderTimings::default()
            })),
            Some(7),
            Some(50),
            0,
        );
    }
    let text = std::fs::read_to_string(&path).unwrap();
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some("existing"));
    let rows: Vec<serde_json::Value> = lines
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0],
        serde_json::json!({
            "v": 1, "revision": 7, "receive_ns": 50, "present_ns": 100,
            "scene_ns": 3, "render_ns": 20, "rebuild_ns": 10, "prepare_ns": 5,
            "acquire_ns": 30, "submit_ns": 2, "full_rebuild": true, "dropped": 0,
        })
    );
    assert_eq!(rows[1]["revision"], 7);
    assert_eq!(rows[1]["scene_ns"], 0);
    assert_eq!(rows[1]["full_rebuild"], false);
    let (_, receiver) = mpsc::sync_channel(1);
    assert!(write_frame_rows(&root, receiver).is_err());
    drop(FrameTimings::new(Some(&root)));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn frame_timings_drop_full_queue_rows_without_waiting_and_report_loss() {
    let (sender, receiver) = mpsc::sync_channel(1);
    let mut log = FrameTimings {
        sender: Some(sender),
        writer: None,
        dropped: 0,
    };
    let outcome = PresentOutcome::Presented(Some(RenderTimings::default()));
    log.record(outcome, None, None, 0);
    log.record(outcome, Some(1), Some(10), 0);
    assert_eq!(receiver.recv().unwrap().dropped, 0);
    log.record(outcome, Some(2), Some(20), 0);
    let row = receiver.recv().unwrap();
    assert_eq!(row.revision, Some(2));
    assert_eq!(row.dropped, 1);
    drop(receiver);
    log.record(outcome, None, None, 0);
    assert!(log.sender.is_none());
}

fn painted_rects(batch: &RectangleBatch) -> impl Iterator<Item = SceneRect> + '_ {
    batch
        .bytes
        .chunks_exact(VERTEX_SIZE as usize * 6)
        .map(|quad| {
            let value = |vertex: usize, component: usize| {
                let offset = vertex * VERTEX_SIZE as usize + component * 4;
                f32::from_ne_bytes(quad[offset..offset + 4].try_into().unwrap())
            };
            SceneRect {
                left: (value(0, 0) + 1.0) * batch.width as f32 / 2.0,
                top: (1.0 - value(0, 1)) * batch.height as f32 / 2.0,
                width: (value(2, 0) - value(0, 0)) * batch.width as f32 / 2.0,
                height: (value(0, 1) - value(2, 1)) * batch.height as f32 / 2.0,
            }
        })
}

fn padding_scene() -> Scene {
    Scene {
        revision: 1,
        columns: 3,
        rows: 2,
        screen: Screen::Primary,
        title: String::new(),
        working_directory: String::new(),
        graphics: SceneGraphics::default(),
        background: palette::TERMINAL_FALLBACK,
        foreground: palette::CARD_OUTLINE,
        cursor: None,
        content: (0..2)
            .map(|row| DrawRow {
                wrapped: false,
                wrap_continuation: false,
                cells: (0..3)
                    .map(|col| DrawCell {
                        width: CellWidth::Narrow,
                        text: String::new(),
                        hyperlink: String::new(),
                        style: crate::DrawStyle {
                            background: SceneColor {
                                r: 20 + row * 60 + col * 10,
                                g: 30,
                                b: 40,
                            },
                            background_is_default: false,
                            ..plain_style()
                        },
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn padding_pixel(batch: &RectangleBatch, x: f32, y: f32) -> Option<SceneColor> {
    batch
        .bytes
        .chunks_exact(3 * VERTEX_SIZE as usize)
        .find_map(|triangle| {
            let value = |vertex: usize, component: usize| {
                let offset = vertex * VERTEX_SIZE as usize + component * 4;
                f32::from_ne_bytes(triangle[offset..offset + 4].try_into().unwrap())
            };
            let points = [0, 1, 2].map(|v| {
                (
                    (value(v, 0) + 1.0) * batch.width as f32 / 2.0,
                    (1.0 - value(v, 1)) * batch.height as f32 / 2.0,
                )
            });
            let cross = |a: (f32, f32), b: (f32, f32), p: (f32, f32)| {
                (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0)
            };
            let signs = [
                cross(points[0], points[1], (x, y)),
                cross(points[1], points[2], (x, y)),
                cross(points[2], points[0], (x, y)),
            ];
            (cross(points[0], points[1], points[2]).abs() > 0.0001
                && (signs.iter().all(|v| *v >= 0.0) || signs.iter().all(|v| *v <= 0.0)))
            .then(|| {
                assert_eq!(value(0, 5), 1.0, "explicit backgrounds stay opaque");
                SceneColor {
                    r: (value(0, 2) * 255.0).round() as u8,
                    g: (value(0, 3) * 255.0).round() as u8,
                    b: (value(0, 4) * 255.0).round() as u8,
                }
            })
        })
}

fn clipping_test_fonts() -> FontSetup {
    FontSetup::new(&FontSettings {
        family: Some("DejaVu Sans Mono".into()),
        ..FontSettings::default()
    })
    .unwrap()
}

fn retained_frame_text_clip(size: PhysicalSize<u32>) -> (WorkspaceScene, SceneRect) {
    use eon_workspace_protocol::v45::{Column, Pane, PopupGeometry, Sidebar, Snapshot, Tab};

    let metrics = CellMetrics::for_scale(1.0);
    let snapshot = Snapshot {
        focus: None,
        generation: "g1-00000000000000000000000000000001".into(),
        created_build: "g1-00000000000000000000000000000001".into(),
        active_build: "g1-00000000000000000000000000000001".into(),
        workspace: "s1".into(),
        active_workspace: "s1".into(),
        workspaces: vec![eon_workspace_protocol::v45::WorkspaceInfo {
            id: "s1".into(),
            persistent_id: Some("s1".into()),
            registration_error: None,
            name: None,
            tab_count: 1,
            attention: None,
            recency: 0,
        }],
        active_tab: "t1".into(),
        tabs: vec![Tab {
            id: "t1".into(),
            directory: b"/tmp".to_vec(),
            pending: false,
            selected_pane: Some("p1".into()),
            selected_popup: None,
            columns: vec![Column {
                panes: vec![Pane {
                    label: "p1".into(),
                    pinned_name: None,
                    tag: None,
                    id: "p1".into(),
                    session: "s1".into(),
                    endpoint: b"/tmp/s1.sock".to_vec(),
                    live: true,
                    attention: None,
                }],
                expanded: "p1".into(),
            }],
            popups: vec![],
            attention_rollup: None,
        }],
        geometry: PopupGeometry {
            side_margin: 8.0,
            vertical_margin: 4.0,
        },
        entries: vec![],
        codex_quota: None,
        claude_quota: None,
        google_quota: None,
        google_quota_failure: None,
        codex_quota_failure: None,
        claude_quota_failure: None,
        running: None,
        busy: None,
        sidebar: Sidebar::Closed,
        attention_queue: Vec::new(),
    };
    let workspace = |size| {
        WorkspaceScene::from_snapshot(&snapshot, size, metrics, 0.0, 0.0, |_, text| {
            (text.into(), text.len() as f32 * metrics.width)
        })
    };
    let old = workspace(PhysicalSize::new(2000, 1000));
    let scene = Scene {
        revision: 1,
        columns: ((old.terminal.width - metrics.padding * 2.0) / metrics.width).floor() as u16,
        rows: ((old.terminal.height - metrics.padding * 2.0) / metrics.height).floor() as u16,
        screen: Screen::Primary,
        title: String::new(),
        working_directory: String::new(),
        graphics: SceneGraphics::default(),
        background: palette::TERMINAL_FALLBACK,
        foreground: palette::CARD_OUTLINE,
        cursor: None,
        content: vec![],
    };
    assert_eq!((scene.columns, scene.rows), (196, 47));
    let current = workspace(size);
    let (terminal, _) = terminal_paint_clip(
        &current,
        current.chrome,
        current.terminal,
        metrics,
        true,
        physical_outline_width(2, metrics.scale),
    )
    .unwrap();
    let clip = scene_grid(&scene, current.terminal, metrics)
        .intersection(terminal)
        .unwrap_or(terminal);
    assert_eq!(
        terminal_text_clip(current.terminal, clip, metrics, false),
        clip
    );
    let clip = terminal_text_clip(current.terminal, clip, metrics, true);
    (current, clip)
}

fn fractional_card_workspace(scale: f64, scroll: f32) -> WorkspaceScene {
    use crate::scene::{EonBar, ShortcutHintsScene};
    use eon_workspace_protocol::v45::{Column, Pane, PopupGeometry, Sidebar, Snapshot, Tab};

    let snapshot = Snapshot {
        focus: None,
        generation: "g1-00000000000000000000000000000001".into(),
        created_build: "g1-00000000000000000000000000000001".into(),
        active_build: "g1-00000000000000000000000000000001".into(),
        workspace: "s1".into(),
        active_workspace: "s1".into(),
        workspaces: vec![eon_workspace_protocol::v45::WorkspaceInfo {
            id: "s1".into(),
            persistent_id: Some("s1".into()),
            registration_error: None,
            name: None,
            tab_count: 1,
            attention: None,
            recency: 0,
        }],
        active_tab: "t1".into(),
        tabs: vec![Tab {
            id: "t1".into(),
            directory: b"/tmp".to_vec(),
            pending: false,
            selected_pane: Some("p40".into()),
            selected_popup: None,
            columns: vec![Column {
                panes: (0..80)
                    .map(|index| Pane {
                        label: format!("p{index}"),
                        pinned_name: None,
                        tag: None,
                        id: format!("p{index}"),
                        session: format!("s{index}"),
                        endpoint: format!("/tmp/{index}.sock").into_bytes(),
                        live: true,
                        attention: None,
                    })
                    .collect(),
                expanded: "p40".into(),
            }],
            popups: vec![],
            attention_rollup: None,
        }],
        geometry: PopupGeometry {
            side_margin: 8.0,
            vertical_margin: 4.0,
        },
        entries: vec![],
        codex_quota: None,
        claude_quota: None,
        google_quota: None,
        google_quota_failure: None,
        codex_quota_failure: None,
        claude_quota_failure: None,
        running: None,
        busy: None,
        sidebar: Sidebar::Closed,
        attention_queue: Vec::new(),
    };
    let size = PhysicalSize::new(2000, 900);
    let metrics = CellMetrics::for_scale(scale);
    let hints = ShortcutHintsScene::new(vec![], size, metrics, None, None, |text: &str| {
        text.len() as f32 * metrics.width
    });
    WorkspaceScene::from_snapshot_with_bar(
        &snapshot,
        EonBar::default(),
        size,
        metrics,
        0.0,
        scroll,
        |_, text| (text.into(), text.len() as f32 * metrics.width),
        Some(hints),
    )
}
