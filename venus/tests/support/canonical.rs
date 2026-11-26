use eon_workspace_protocol::v51::{self as wire, ALT, SHIFT, Shortcut};
use orbit_protocol::{
    Capabilities, Cell, CellStyle, CellWidth, Colors, Cursor, CursorShape, CursorViewport,
    Dimensions, Frame, ImageLayer, Placement, PlacementState, Rgb, Row, Screen, SourceRect,
    StyleColor, Underline,
    session::{PixelFormat, ResourceAnnounce, ResourceChunk, ServerMessage},
};

pub(crate) fn pane(id: &str) -> wire::Pane {
    wire::Pane {
        label: id.into(),
        pinned_name: None,
        tag: None,
        id: id.into(),
        session: format!("terminal-{id}"),
        endpoint: format!("/run/eon/{id}.sock").into_bytes(),
        live: true,
        attention: None,
    }
}

pub(crate) fn column(id: &str) -> wire::Column {
    wire::Column {
        expanded: id.into(),
        panes: vec![pane(id)],
    }
}

pub(crate) fn workspace_snapshot(tabs: Vec<wire::Tab>) -> wire::Snapshot {
    wire::Snapshot {
        focus: None,
        released_shortcuts: Default::default(),
        input_locked: false,
        input_lock: Shortcut {
            modifiers: ALT | SHIFT,
            key: "KeyI".into(),
        },
        generation: "g1-00000000000000000000000000000001".into(),
        created_build: "g1-00000000000000000000000000000001".into(),
        active_build: "g1-00000000000000000000000000000001".into(),
        workspace: "s1".into(),
        active_workspace: "s1".into(),
        workspaces: vec![eon_workspace_protocol::v51::WorkspaceInfo {
            id: "s1".into(),
            persistent_id: Some("s1".into()),
            registration_error: None,
            name: None,
            tab_count: tabs.len(),
            attention: None,
            recency: 0,
        }],
        active_tab: "t1".into(),
        geometry: wire::PopupGeometry {
            side_margin: 8.0,
            vertical_margin: 4.0,
        },
        entries: Vec::new(),
        tabs,
        codex_quota: None,
        claude_quota: None,
        google_quota: None,
        google_quota_failure: None,
        codex_quota_failure: None,
        claude_quota_failure: None,
        running: None,
        busy: None,
        sidebar: eon_workspace_protocol::v51::Sidebar::Open,
        attention_queue: Vec::new(),
    }
}
pub(crate) fn popup_snapshot() -> wire::Snapshot {
    let mut snapshot = workspace_snapshot(vec![wire::Tab {
        id: "t1".into(),
        directory: b"/work".to_vec(),
        pending: false,
        selected_pane: None,
        selected_popup: Some("u1".into()),
        columns: Vec::new(),
        popups: vec![wire::Popup {
            id: "u1".into(),
            entry: "agent".into(),
            session: "s1".into(),
            endpoint: b"/run/agent.sock".to_vec(),
            attention: None,
        }],
        attention_rollup: None,
    }]);
    snapshot.entries = vec![wire::PopupEntry {
        id: "agent".into(),
        label: "Agent".into(),
        shortcut: wire::PopupKey::Bound(wire::Shortcut {
            modifiers: wire::ALT | wire::SHIFT,
            key: "KeyL".into(),
        }),
    }];
    snapshot
}

pub(crate) fn announce(
    resource: u64,
    format: PixelFormat,
    width: u32,
    height: u32,
) -> ServerMessage {
    ServerMessage::ResourceAnnounce(
        ResourceAnnounce::new(resource, 1, format, width, height).unwrap(),
    )
}

pub(crate) fn chunk(resource: u64, index: u32, bytes: Vec<u8>) -> ServerMessage {
    ServerMessage::ResourceChunk(ResourceChunk {
        resource,
        index,
        bytes,
    })
}

pub(crate) fn placement(resource: u64, col: i32, row: i32, state: PlacementState) -> Placement {
    Placement {
        resource,
        layer: ImageLayer::BelowText,
        z: 0,
        col,
        row,
        grid_cols: 3,
        grid_rows: 2,
        offset_x: 5,
        offset_y: 5,
        width: 30,
        height: 40,
        source: SourceRect {
            x: 1,
            y: 2,
            width: 30,
            height: 40,
        },
        state,
    }
}

pub(crate) fn frame_message(frame: Frame) -> ServerMessage {
    ServerMessage::Frame(Box::new(frame))
}

pub(crate) fn frame(revision: u64, screen: Screen) -> Frame {
    let background = Rgb {
        r: 0x10,
        g: 0x13,
        b: 0x1a,
    };
    let foreground = Rgb {
        r: 0xf0,
        g: 0xf1,
        b: 0xf5,
    };
    let mut palette = [Rgb::BLACK; 256];
    palette[2] = Rgb {
        r: 0x22,
        g: 0xcc,
        b: 0x88,
    };
    Frame {
        revision,
        dimensions: Dimensions { cols: 4, rows: 1 },
        screen,
        scroll_position: orbit_protocol::ScrollPosition::default(),
        title: "Eon ✦".into(),
        working_directory: "/tmp/venus".into(),
        capabilities: Capabilities { hyperlinks: true },
        colors: Colors {
            background,
            foreground,
            cursor: None,
            palette,
        },
        cursor: Cursor {
            visible: true,
            blinking: false,
            password_input: false,
            shape: CursorShape::Block,
            viewport: Some(CursorViewport {
                x: 1,
                y: 0,
                at_wide_tail: false,
            }),
        },
        rows: vec![Row {
            wrapped: false,
            wrap_continuation: false,
            cells: vec![
                cell("e\u{301}", CellWidth::Narrow, StyleColor::None, ""),
                cell(
                    "界",
                    CellWidth::Wide,
                    StyleColor::Palette(2),
                    "https://yazelix.dev",
                ),
                cell("", CellWidth::SpacerTail, StyleColor::None, ""),
                cell("", CellWidth::Narrow, StyleColor::None, ""),
            ],
        }],
        graphics: None,
    }
}

pub(crate) fn cell(text: &str, width: CellWidth, foreground: StyleColor, hyperlink: &str) -> Cell {
    Cell {
        width,
        style: CellStyle {
            foreground,
            background: StyleColor::None,
            underline_color: StyleColor::None,
            bold: false,
            italic: false,
            faint: false,
            blink: false,
            inverse: false,
            invisible: false,
            strikethrough: false,
            overline: false,
            selected: false,
            protected: false,
            underline: Underline::None,
        },
        text: text.into(),
        hyperlink: hyperlink.into(),
    }
}
