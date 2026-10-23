//! Native window and terminal grid sizing: the initial window, each terminal's Orbit
//! surface, and whether another workspace column fits.

use crate::Result;
use eon_workspace_protocol::v18::MAX_COLUMNS;
use orbit_protocol::{MAX_CELLS, session::SurfaceSize};
use winit::dpi::PhysicalSize;
use yazelix_venus::{
    CellMetrics, EonBar, SceneRect, ShortcutHintsScene, WorkspaceFocus, WorkspaceScene,
    workspace_column_width,
};

/// Narrowest terminal grid a new workspace column may leave.
pub(super) const MIN_COLUMN_CELLS: u16 = 40;

pub(super) fn terminal_screen(
    workspace: Option<&WorkspaceScene>,
    fallback: PhysicalSize<u32>,
) -> PhysicalSize<u32> {
    workspace.map_or(fallback, |workspace| rect_screen(workspace.terminal))
}

pub(super) fn rect_screen(rect: SceneRect) -> PhysicalSize<u32> {
    PhysicalSize::new(
        rect.width.max(0.0).round() as u32,
        rect.height.max(0.0).round() as u32,
    )
}

pub(super) fn initial_window_size(
    columns: Option<u16>,
    rows: Option<u16>,
    metrics: CellMetrics,
    scale: f64,
    snapshot: Option<&eon_workspace_protocol::v18::Snapshot>,
) -> Result<PhysicalSize<u32>> {
    let (horizontal, vertical) = snapshot.map_or((0.0, 0.0), |snapshot| {
        WorkspaceScene::initial_overhead(snapshot, metrics)
    });
    let mut size = PhysicalSize::new(
        columns.map_or((960.0 * scale).round() as u32, |columns| {
            (f32::from(columns) * metrics.width + metrics.padding * 2.0 + horizontal).ceil() as u32
        }),
        rows.map_or((600.0 * scale).round() as u32, |rows| {
            (f32::from(rows) * metrics.height + metrics.padding * 2.0 + vertical).ceil() as u32
        }),
    );
    let limit = wgpu::Limits::default().max_texture_dimension_2d;
    // Grid admission depends on header heights, not tab text fitting.
    let measure = |size: PhysicalSize<u32>| {
        let workspace = snapshot.map(|snapshot| {
            WorkspaceScene::from_snapshot_with_metadata(
                snapshot,
                EonBar::default(),
                size,
                metrics,
                0.0,
                0.0,
                |_| None,
                |_, _| (String::new(), 0.0),
                Some(ShortcutHintsScene::new(
                    Vec::new(),
                    size,
                    metrics,
                    WorkspaceFocus::Terminal,
                    |_| 0.0,
                )),
            )
        });
        workspace
            .as_ref()
            .filter(|workspace| workspace.panes.is_empty() && workspace.popup_label().is_none())
            .map_or_else(
                || terminal_screen(workspace.as_ref(), size),
                |workspace| {
                    // Admit space for later work without creating or attaching a placeholder.
                    PhysicalSize::new(
                        workspace.pane_viewport.width as u32,
                        workspace.pane_viewport.height as u32,
                    )
                },
            )
    };
    let mut terminal = measure(size);
    // Card gaps yield on tiny windows, so the overhead estimate can miss a tiny requested grid;
    // the window then grows or shrinks by each axis's shortfall until the layout gives the grid.
    for _ in 0..8 {
        let actual = surface_size(terminal, metrics);
        let shortfall = |cells: Option<u16>, actual: Option<u16>, cell: f32, pixels: u32| {
            cells
                .filter(|cells| Some(*cells) != actual)
                .map_or(0, |cells| {
                    (f32::from(cells) * cell + metrics.padding * 2.0).ceil() as i64
                        - i64::from(pixels)
                })
        };
        let width = shortfall(
            columns,
            actual.map(|actual| actual.cols),
            metrics.width,
            terminal.width,
        );
        let height = shortfall(
            rows,
            actual.map(|actual| actual.rows),
            metrics.height,
            terminal.height,
        );
        if width == 0 && height == 0 {
            break;
        }
        size = PhysicalSize::new(
            (i64::from(size.width) + width).max(1) as u32,
            (i64::from(size.height) + height).max(1) as u32,
        );
        terminal = measure(size);
    }
    let admitted = surface_size(terminal, metrics);
    if size.width > limit
        || size.height > limit
        || admitted.is_none_or(|actual| {
            columns.is_some_and(|columns| columns != actual.cols)
                || rows.is_some_and(|rows| rows != actual.rows)
                || ((terminal.width as f32 - metrics.padding * 2.0) / metrics.width).floor()
                    * ((terminal.height as f32 - metrics.padding * 2.0) / metrics.height).floor()
                    > MAX_CELLS as f32
        })
    {
        return Err(
            "initial terminal dimensions exceed the native surface or Orbit grid limits".into(),
        );
    }
    Ok(size)
}

/// Whether a column request beside `columns` existing columns may reach Eon. A
/// tab without columns gets a pane and a full tab gets Eon's capacity refusal;
/// otherwise one more column must leave each `MIN_COLUMN_CELLS` terminal
/// columns wide.
pub(super) fn admits_column(
    window: PhysicalSize<u32>,
    columns: usize,
    metrics: CellMetrics,
) -> bool {
    if columns == 0 || columns >= MAX_COLUMNS {
        return true;
    }
    let width = workspace_column_width(window.width as f32, columns + 1, metrics);
    surface_size(PhysicalSize::new(width as u32, window.height), metrics)
        .is_some_and(|size| size.cols >= MIN_COLUMN_CELLS)
}

pub(super) fn surface_size(screen: PhysicalSize<u32>, metrics: CellMetrics) -> Option<SurfaceSize> {
    if screen.width == 0
        || screen.height == 0
        || screen.width > u32::from(u16::MAX)
        || screen.height > u32::from(u16::MAX)
    {
        return None;
    }
    let cell_width = metrics.width.round() as u32;
    let cell_height = metrics.height.round() as u32;
    let padding = metrics.padding.round() as u32;
    let horizontal_padding = padding.checked_mul(2)?;
    let vertical_padding = padding.checked_mul(2)?;
    if horizontal_padding >= screen.width || vertical_padding >= screen.height {
        return None;
    }
    let columns = (screen.width - horizontal_padding) / cell_width;
    let mut rows = (screen.height - vertical_padding) / cell_height;
    if columns == 0 || rows == 0 {
        return None;
    }
    rows = rows.min(MAX_CELLS as u32 / columns);
    Some(SurfaceSize {
        cols: u16::try_from(columns).ok()?,
        rows: u16::try_from(rows).ok()?,
        screen_width: screen.width,
        screen_height: screen.height,
        cell_width,
        cell_height,
        padding_top: padding,
        padding_bottom: screen.height - padding - rows * cell_height,
        padding_left: padding,
        padding_right: screen.width - padding - columns * cell_width,
    })
}

#[cfg(test)]
mod tests {
    use super::super::tests::show_test_popup;
    use super::super::*;
    use super::*;
    use eon_workspace_protocol::v18::{Pane, Snapshot, Tab};

    #[test]
    fn initial_grid_includes_workspace_chrome_at_each_scale() {
        let mut snapshot = Snapshot {
            active_tab: "t1".into(),
            geometry: eon_workspace_protocol::v18::PopupGeometry {
                side_margin: 8.0,
                vertical_margin: 4.0,
            },
            entries: Vec::new(),
            tabs: vec![Tab {
                pending: false,
                selected_popup: None,
                popups: Vec::new(),
                id: "t1".into(),
                directory: b"/tmp".to_vec(),
                selected_pane: Some("p1".into()),
                columns: vec![eon_workspace_protocol::v18::Column {
                    panes: (1..=3)
                        .map(|i| Pane {
                            id: format!("p{i}"),
                            session: format!("s{i}"),
                            endpoint: format!("/tmp/{i}.sock").into_bytes(),
                            live: true,
                            attention: None,
                        })
                        .collect(),
                    expanded: "p1".into(),
                }],
                attention_rollup: None,
            }],
            codex_quota: None,
            claude_quota: None,
            sidebar: eon_workspace_protocol::v18::Sidebar::Open,
        };
        for popup in [false, true] {
            if popup {
                show_test_popup(&mut snapshot);
            }
            for scale in [1.0, 1.25, 1.5, 2.0] {
                let metrics = CellMetrics::for_scale(scale);
                for snapshot in [None, Some(&snapshot)] {
                    let size =
                        initial_window_size(Some(100), Some(30), metrics, scale, snapshot).unwrap();
                    let workspace = snapshot.map(|s| {
                        WorkspaceScene::from_snapshot_with_metadata(
                            s,
                            EonBar::default(),
                            size,
                            metrics,
                            0.0,
                            0.0,
                            |_| None,
                            |_, _| (String::new(), 0.0),
                            Some(ShortcutHintsScene::new(
                                Vec::new(),
                                size,
                                metrics,
                                WorkspaceFocus::Terminal,
                                |_| 0.0,
                            )),
                        )
                    });
                    let actual =
                        surface_size(terminal_screen(workspace.as_ref(), size), metrics).unwrap();
                    assert_eq!((actual.cols, actual.rows), (100, 30));
                    let size =
                        initial_window_size(None, Some(30), metrics, scale, snapshot).unwrap();
                    assert_eq!(size.width, (960.0 * scale).round() as u32);
                }
            }
        }
        // Card gaps yield on tiny windows; the window still gives a tiny requested grid, here for
        // a popup over two columns.
        snapshot.tabs[0]
            .columns
            .push(eon_workspace_protocol::v18::Column {
                panes: vec![Pane {
                    id: "p4".into(),
                    session: "s4".into(),
                    endpoint: b"/tmp/4.sock".to_vec(),
                    live: true,
                    attention: None,
                }],
                expanded: "p4".into(),
            });
        for (columns, rows) in [(1, 1), (1, 30), (100, 1)] {
            for scale in [1.0, 1.25, 2.0] {
                assert!(
                    initial_window_size(
                        Some(columns),
                        Some(rows),
                        CellMetrics::for_scale(scale),
                        scale,
                        Some(&snapshot)
                    )
                    .is_ok(),
                    "{columns}x{rows} at {scale}"
                );
            }
        }
        let metrics = CellMetrics::for_scale(1.0);
        snapshot.tabs[0].columns.clear();
        snapshot.tabs[0].selected_pane = None;
        snapshot.tabs[0].selected_popup = None;
        assert!(initial_window_size(Some(100), Some(30), metrics, 1.0, Some(&snapshot)).is_ok());
        assert!(initial_window_size(Some(u16::MAX), Some(30), metrics, 1.0, None).is_err());
        assert!(initial_window_size(Some(500), Some(300), metrics, 1.0, None).is_err());
    }

    #[test]
    fn surface_measurements_match_orbit_invariants() {
        let metrics = CellMetrics::for_scale(1.0);
        let size = surface_size(PhysicalSize::new(960, 600), metrics).unwrap();
        assert_eq!((size.cols, size.rows), (93, 32));
        assert!(
            orbit_protocol::session::encode_client_message(&ClientMessage::Resize(size)).is_ok()
        );
        for screen in [PhysicalSize::new(929, 976), PhysicalSize::new(4096, 4096)] {
            let size = surface_size(screen, metrics).unwrap();
            assert_eq!(
                size.padding_left + u32::from(size.cols) * size.cell_width + size.padding_right,
                screen.width
            );
            assert_eq!(
                size.padding_top + u32::from(size.rows) * size.cell_height + size.padding_bottom,
                screen.height
            );
        }
        assert!(surface_size(PhysicalSize::new(1, 1), metrics).is_none());
        assert!(surface_size(PhysicalSize::new(25, 600), metrics).is_none());
        assert!(surface_size(PhysicalSize::new(960, 25), metrics).is_none());
        assert!(surface_size(PhysicalSize::new(u32::from(u16::MAX) + 1, 600), metrics).is_none());
    }

    #[test]
    fn each_column_keeps_forty_terminal_columns() {
        let metrics = CellMetrics::for_scale(1.0);
        let column = u32::from(MIN_COLUMN_CELLS) * metrics.width.round() as u32
            + 2 * metrics.padding.round() as u32;
        // The card gap at default typography.
        let gap = 8;
        for existing in 1..MAX_COLUMNS {
            let count = existing as u32 + 1;
            // Cards keep one gap beside and between them.
            let window = column * count + gap * (count + 1);
            assert!(admits_column(
                PhysicalSize::new(window, 600),
                existing,
                metrics
            ));
            assert!(!admits_column(
                PhysicalSize::new(window - 1, 600),
                existing,
                metrics
            ));
        }
        // An empty tab gets a pane and a full tab gets Eon's capacity refusal.
        let narrow = PhysicalSize::new(column - 1, 600);
        assert!(admits_column(narrow, 0, metrics));
        assert!(admits_column(narrow, MAX_COLUMNS, metrics));
    }
}
