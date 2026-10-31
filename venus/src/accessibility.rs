use crate::{
    CellMetrics, DialogKind, DialogScene, MAX_LINK_BYTES, Scene, SceneRect, WorkspaceFocus,
    WorkspaceHeaderControl, WorkspaceScene,
    scene::{
        AccessiblePosition, AccessibleRow, AccessibleSelection, AccessibleText,
        QuotaWindowIdentity, SIDEBAR_EMPTY, WorkspaceQuota,
    },
};
use accesskit::{
    Action, ActivationHandler, Node, NodeId, Orientation, Rect, Role, TextDirection, TextPosition,
    TextSelection, TreeId, TreeInfo, TreeUpdate,
};
use accesskit_winit::Adapter;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard},
};
use winit::dpi::PhysicalSize;

const WINDOW: NodeId = NodeId(0);
const CONTENT: NodeId = NodeId(1);
const STATUS: NodeId = NodeId(2);
const TAB_LIST: NodeId = NodeId(3);
const PANE_PANEL: NodeId = NodeId(4);
const DIALOG: NodeId = NodeId(5);
const NEW_TAB: NodeId = NodeId(6);
const SHORTCUTS: NodeId = NodeId(7);
const CLOSE_TAB: NodeId = NodeId(8);
const QUOTA: NodeId = NodeId(9);
const SCROLLBACK: NodeId = NodeId(10);
const HINTS: NodeId = NodeId(11);
const VERSION: NodeId = NodeId(12);
const CLAUDE_FIVE_HOUR_QUOTA: NodeId = NodeId(13);
const CLOCK: NodeId = NodeId(14);
const CPU: NodeId = NodeId(15);
const RAM: NodeId = NodeId(16);
const CLAUDE_SEVEN_DAY_QUOTA: NodeId = NodeId(17);
const SIDEBAR: NodeId = NodeId(18);
const SIDEBAR_EMPTY_STATE: NodeId = NodeId(19);
const WORKSPACE_LABEL: NodeId = NodeId(20);
// Scene rows are u16, so this range cannot collide with fixed or text-run nodes.
const WORKSPACE_NODE_START: u64 = 1 << 32;
// Two IDs per u32 duration fit below links; workspace allocation stops before this range.
const CODEX_QUOTA_NODE_START: u64 = 1 << 40;
const LINK_NODE_START: u64 = 1 << 48;
const DIALOG_NODE_START: u64 = 1 << 56;
const DIALOG_GROUP_STRIDE: u64 = 1 << 8;

#[derive(Clone, Debug)]
struct AccessibleLink {
    open: NodeId,
    copy: NodeId,
    row: u16,
    column: u16,
    label: String,
    bounds: SceneRect,
}

#[derive(Clone, Debug)]
struct Snapshot {
    title: String,
    content: AccessibleText,
    status: String,
    scrollback_label: Option<String>,
    scrollback_rect: Option<SceneRect>,
    size: PhysicalSize<u32>,
    metrics: CellMetrics,
    columns: Option<u16>,
    workspace: Option<WorkspaceScene>,
    dialog: Option<DialogScene>,
    workspace_focus: WorkspaceFocus,
    tab_ids: HashMap<String, NodeId>,
    pane_ids: HashMap<String, NodeId>,
    /// Agent sidebar headings by tab id and rows by target; EONW identities never collide.
    sidebar_ids: HashMap<String, NodeId>,
    inbox_ids: HashMap<(String, String), NodeId>,
    next_workspace_id: u64,
    links: Vec<AccessibleLink>,
    link_identity: Option<(u64, u64)>,
    next_link_id: u64,
}

impl Snapshot {
    fn new(size: PhysicalSize<u32>) -> Self {
        Self {
            title: "Venus".into(),
            content: AccessibleText::default(),
            status: "Connecting to Orbit".into(),
            scrollback_label: None,
            scrollback_rect: None,
            size,
            metrics: CellMetrics::for_scale(1.0),
            columns: None,
            workspace: None,
            dialog: None,
            workspace_focus: WorkspaceFocus::Terminal,
            tab_ids: HashMap::new(),
            pane_ids: HashMap::new(),
            sidebar_ids: HashMap::new(),
            inbox_ids: HashMap::new(),
            next_workspace_id: WORKSPACE_NODE_START,
            links: Vec::new(),
            link_identity: None,
            next_link_id: LINK_NODE_START,
        }
    }

    fn set_dialog(&mut self, dialog: Option<&DialogScene>) {
        self.inbox_ids.retain(|key, _| {
            dialog.is_some_and(|dialog| {
                dialog
                    .attention_rows
                    .iter()
                    .any(|row| row.tab == key.0 && row.target == key.1)
            })
        });
        if let Some(dialog) = dialog {
            for row in &dialog.attention_rows {
                let key = (row.tab.clone(), row.target.clone());
                self.inbox_ids.entry(key).or_insert_with(|| {
                    let id = NodeId(self.next_workspace_id);
                    self.next_workspace_id = self
                        .next_workspace_id
                        .checked_add(1)
                        .filter(|next| *next <= CODEX_QUOTA_NODE_START)
                        .expect("AccessKit workspace node IDs exhausted");
                    id
                });
            }
        }
        self.dialog = dialog.cloned();
    }

    fn set_workspace(&mut self, workspace: Option<&WorkspaceScene>) {
        let Some(workspace) = workspace else {
            self.workspace = None;
            self.tab_ids.clear();
            self.pane_ids.clear();
            self.sidebar_ids.clear();
            return;
        };

        self.tab_ids
            .retain(|id, _| workspace.tabs.iter().any(|tab| tab.id == *id));
        self.pane_ids.retain(|id, _| {
            workspace
                .column_panes()
                .any(|(panes, _)| panes.iter().any(|pane| pane.id == *id))
        });
        let sidebar_keys = workspace
            .sidebar_rows()
            .iter()
            .map(|row| row.target.as_str())
            .chain(
                workspace
                    .sidebar_headings
                    .iter()
                    .map(|heading| heading.tab.as_str()),
            )
            .collect::<Vec<_>>();
        self.sidebar_ids
            .retain(|id, _| sidebar_keys.contains(&id.as_str()));
        for key in sidebar_keys {
            allocate_workspace_id(&mut self.sidebar_ids, &mut self.next_workspace_id, key);
        }
        for tab in &workspace.tabs {
            allocate_workspace_id(&mut self.tab_ids, &mut self.next_workspace_id, &tab.id);
        }
        for (panes, _) in workspace.column_panes() {
            for pane in panes {
                allocate_workspace_id(&mut self.pane_ids, &mut self.next_workspace_id, &pane.id);
            }
        }
        self.workspace = Some(workspace.clone());
    }

    fn set_links(&mut self, scene: Option<&Scene>, generation: Option<u64>) {
        let identity = link_identity(scene, generation);
        if self.link_identity == identity {
            return;
        }
        self.links.clear();
        self.link_identity = identity;
        let Some((scene, _)) = scene.zip(generation) else {
            return;
        };
        let terminal = self.workspace.as_ref().map_or(
            SceneRect {
                width: self.size.width as f32,
                height: self.size.height as f32,
                ..SceneRect::default()
            },
            |workspace| workspace.terminal,
        );
        let visible = self
            .workspace
            .as_ref()
            .map_or(Some(terminal), WorkspaceScene::visible_terminal);
        self.links = visible.map_or_else(Vec::new, |visible| {
            scene
                .hyperlinks()
                .filter_map(|link| {
                    let bounds = link.rect(terminal, self.metrics).intersection(visible)?;
                    let open = NodeId(self.next_link_id);
                    let copy = NodeId(
                        self.next_link_id
                            .checked_add(1)
                            .expect("AccessKit link node IDs exhausted"),
                    );
                    self.next_link_id = self
                        .next_link_id
                        .checked_add(2)
                        .expect("AccessKit link node IDs exhausted");
                    Some(AccessibleLink {
                        open,
                        copy,
                        row: link.row,
                        column: link.column,
                        label: accessible_link_label(&link.uri),
                        bounds,
                    })
                })
                .collect()
        });
    }

    fn tree(&self) -> TreeUpdate {
        let terminal = self.columns.is_some();
        let scrollback = self
            .scrollback_label
            .as_deref()
            .zip(self.scrollback_rect)
            .filter(|_| terminal);
        let mut root = Node::new(Role::Window);
        root.set_label(self.title.as_str());
        root.set_bounds(bounds(self.size));
        if let Some(viewer) = &self.dialog {
            root.set_children(vec![DIALOG]);
            let mut dialog = match &viewer.kind {
                DialogKind::AttentionInbox => {
                    let mut dialog = Node::new(Role::Dialog);
                    dialog.set_label("Attention inbox");
                    dialog.set_description(viewer.kind.footer().join(" · "));
                    dialog
                }
                DialogKind::Shortcuts => {
                    let mut dialog = Node::new(Role::Dialog);
                    dialog.set_label("Eon shortcuts");
                    dialog.set_description(
                        "Native Eon surface shortcuts. Use arrows or Page Up and Page Down to scroll; press Escape or Alt+Slash to close.",
                    );
                    dialog
                }
                DialogKind::CloseLastTab {
                    detach, workspace, ..
                } => {
                    let mut dialog = Node::new(Role::AlertDialog);
                    dialog.set_label(viewer.kind.title());
                    let subtitle = viewer.kind.subtitle();
                    let enter = if workspace.is_some() {
                        "close it"
                    } else {
                        "quit"
                    };
                    dialog.set_description(match detach {
                        Some(chord) => format!(
                            "{subtitle} Press Enter to {enter}, Escape to cancel, or {chord} to keep it running instead."
                        ),
                        None => format!("{subtitle} Press Enter to {enter} or Escape to cancel."),
                    });
                    dialog
                }
            };
            dialog.set_bounds(rect(viewer.bounds));
            dialog.set_modal();
            dialog.set_clips_children();
            let mut nodes = vec![(WINDOW, root)];
            let mut group_ids = Vec::with_capacity(viewer.groups.len());
            let mut focus = DIALOG;
            for (index, chip) in viewer.filters.iter().enumerate() {
                let id = NodeId(DIALOG_NODE_START + index as u64);
                let mut node = Node::new(Role::Button);
                node.set_label(format!("{}, {}", chip.filter.label(), chip.count));
                node.set_selected(chip.selected);
                node.set_bounds(rect(chip.rect));
                node.add_action(Action::Click);
                node.add_action(Action::Focus);
                if chip.selected {
                    focus = id;
                }
                group_ids.push(id);
                nodes.push((id, node));
            }
            for row in &viewer.attention_rows {
                let id = self.inbox_ids[&(row.tab.clone(), row.target.clone())];
                let mut node = Node::new(Role::Button);
                node.set_label(row.accessible_label.as_str());
                node.add_action(Action::Click);
                node.add_action(Action::Focus);
                if let Some(bounds) = row.rect.intersection(viewer.content) {
                    node.set_bounds(rect(bounds));
                }
                if row.focused {
                    focus = id;
                }
                group_ids.push(id);
                nodes.push((id, node));
            }
            if let Some(empty) = viewer.empty {
                let id = NodeId(DIALOG_NODE_START + 4);
                let mut node = Node::new(Role::Label);
                node.set_value(empty);
                node.set_bounds(rect(viewer.content));
                group_ids.push(id);
                nodes.push((id, node));
            }
            for (group_index, group) in viewer.groups.iter().enumerate() {
                let group_id = NodeId(DIALOG_NODE_START + group_index as u64 * DIALOG_GROUP_STRIDE);
                let row_ids = (0..group.rows.len())
                    .map(|index| NodeId(group_id.0 + index as u64 + 1))
                    .collect::<Vec<_>>();
                // An untitled group's rows belong to the dialog itself.
                if group.title.is_empty() {
                    group_ids.extend(row_ids.iter().copied());
                } else {
                    group_ids.push(group_id);
                    let mut group_node = Node::new(Role::Group);
                    group_node.set_label(group.title.as_str());
                    if let Some(bounds) = group.heading.intersection(viewer.content) {
                        group_node.set_bounds(rect(bounds));
                    }
                    group_node.set_children(row_ids.clone());
                    nodes.push((group_id, group_node));
                }
                for (row, id) in group.rows.iter().zip(row_ids) {
                    let mut node = Node::new(Role::Label);
                    node.set_value(match viewer.kind {
                        DialogKind::Shortcuts | DialogKind::AttentionInbox => {
                            format!("{} — {}", row.shortcut, row.action)
                        }
                        DialogKind::CloseLastTab { .. } => {
                            format!("{} {}", row.shortcut, row.action)
                        }
                    });
                    if let Some(bounds) = row.rect.intersection(viewer.content) {
                        node.set_bounds(rect(bounds));
                    }
                    nodes.push((id, node));
                }
            }
            dialog.set_children(group_ids);
            nodes.push((DIALOG, dialog));
            return TreeUpdate {
                nodes,
                tree: Some(TreeInfo::new(WINDOW)),
                tree_id: TreeId::ROOT,
                focus,
            };
        }
        let status_alert = terminal && !self.status.is_empty();
        let mut root_children = if let Some(workspace) = &self.workspace {
            let mut children = workspace
                .workspace_label
                .as_ref()
                .map(|_| WORKSPACE_LABEL)
                .into_iter()
                .chain([TAB_LIST, NEW_TAB])
                .collect::<Vec<_>>();
            // Quota chips and labels follow their visual order, which Eon chooses.
            let mut items = workspace
                .quotas
                .iter()
                .map(|quota| (quota.rect.left, quota_id(quota)))
                .chain(
                    [
                        (CPU, &workspace.cpu),
                        (RAM, &workspace.ram),
                        (CLOCK, &workspace.clock),
                        (VERSION, &workspace.version),
                    ]
                    .into_iter()
                    .filter_map(|(id, label)| Some((label.as_ref()?.rect.left, id))),
                )
                .collect::<Vec<_>>();
            items.sort_by(|a, b| a.0.total_cmp(&b.0));
            children.extend(items.into_iter().map(|(_, id)| id));
            if workspace.has_control(WorkspaceHeaderControl::Shortcuts) {
                children.push(SHORTCUTS);
            }
            children.push(CLOSE_TAB);
            if workspace.sidebar().is_some() {
                children.push(SIDEBAR);
            }
            children.push(PANE_PANEL);
            if workspace
                .hints
                .as_ref()
                .is_some_and(|hints| !hints.pieces.is_empty())
            {
                children.push(HINTS);
            }
            if status_alert {
                children.push(STATUS);
            }
            children
        } else if status_alert {
            vec![CONTENT, STATUS]
        } else {
            vec![CONTENT]
        };
        if scrollback.is_some() {
            root_children.push(SCROLLBACK);
        }
        root.set_children(root_children);

        let mut content = Node::new(if terminal {
            Role::Terminal
        } else if self.status.is_empty() {
            Role::GenericContainer
        } else {
            Role::Alert
        });
        content.set_label(if terminal {
            self.title.as_str()
        } else if self.status.is_empty() {
            "Tab content"
        } else {
            "Venus status"
        });
        if terminal {
            content.set_read_only();
            if let Some(label) = &self.scrollback_label {
                content.set_description(format!(
                    "Scrollback: {label} above live output (committed display rows)"
                ));
            }
            content.set_children(
                (0..self.content.rows.len())
                    .map(text_run_id)
                    .chain(self.links.iter().flat_map(|link| [link.open, link.copy]))
                    .collect::<Vec<_>>(),
            );
            if let Some(selection) = self.content.selection {
                content.set_text_selection(text_selection(selection));
            }
        } else {
            content.set_value(self.status.as_str());
        }
        let terminal_layout = self.workspace.as_ref().map_or(
            SceneRect {
                left: 0.0,
                top: 0.0,
                width: self.size.width as f32,
                height: self.size.height as f32,
            },
            |workspace| workspace.terminal,
        );
        let visible_terminal = self
            .workspace
            .as_ref()
            .map_or(Some(terminal_layout), WorkspaceScene::visible_terminal);
        if terminal {
            content.set_clips_children();
        }
        if let Some(bounds) = visible_terminal {
            content.set_bounds(rect(bounds));
        }

        let mut nodes = vec![(WINDOW, root), (CONTENT, content)];
        if let Some((label, pill)) = scrollback {
            let mut button = Node::new(Role::Button);
            button.set_label("Return to live output");
            button.set_description(format!("{label} above live output"));
            button.set_bounds(rect(pill));
            button.add_action(Action::Click);
            nodes.push((SCROLLBACK, button));
        }
        if let Some(workspace) = &self.workspace {
            if let Some(label) = &workspace.workspace_label {
                // Without its popup entry the label is read-only text.
                let mut node = if let Some(tooltip) = &label.tooltip {
                    let mut node = Node::new(Role::Button);
                    node.set_label(label.accessible_label.as_str());
                    node.set_description(tooltip.as_str());
                    node.add_action(Action::Click);
                    node.add_action(Action::Focus);
                    node
                } else {
                    let mut node = Node::new(Role::Label);
                    node.set_value(label.accessible_label.as_str());
                    node
                };
                node.set_bounds(rect(label.rect));
                nodes.push((WORKSPACE_LABEL, node));
            }
            let mut tab_list = Node::new(Role::TabList);
            tab_list.set_label("Eon workspace tabs");
            tab_list.set_orientation(Orientation::Horizontal);
            tab_list.set_bounds(rect(workspace.tab_viewport));
            tab_list.set_clips_children();
            tab_list.set_children(
                workspace
                    .tabs
                    .iter()
                    .map(|tab| self.tab_ids[tab.id.as_str()])
                    .collect::<Vec<_>>(),
            );
            nodes.push((TAB_LIST, tab_list));
            for tab in &workspace.tabs {
                let mut node = Node::new(Role::Tab);
                node.set_label(tab.accessible_label());
                node.set_selected(tab.selected);
                if let Some(bounds) = tab.rect.intersection(workspace.tab_viewport) {
                    node.set_bounds(rect(bounds));
                }
                node.add_action(Action::Click);
                node.add_action(Action::Focus);
                nodes.push((self.tab_ids[tab.id.as_str()], node));
            }
            for quota in &workspace.quotas {
                let mut node = Node::new(Role::Label);
                node.set_value(quota.accessible_label.as_str());
                node.set_description(quota.description.as_str());
                node.set_bounds(rect(quota.rect));
                nodes.push((quota_id(quota), node));
            }
            for (id, label) in [
                (CPU, &workspace.cpu),
                (RAM, &workspace.ram),
                (CLOCK, &workspace.clock),
                (VERSION, &workspace.version),
            ] {
                if let Some(label) = label {
                    let mut node = Node::new(Role::Label);
                    node.set_value(label.description.as_str());
                    node.set_bounds(rect(label.rect));
                    nodes.push((id, node));
                }
            }
            for control in workspace.controls {
                if control.rect.width <= 0.0 {
                    continue;
                }
                let mut node = Node::new(Role::Button);
                node.set_label(control.label);
                if control.kind == WorkspaceHeaderControl::CloseTab
                    && let Some(tab) = workspace.tabs.iter().find(|tab| tab.selected)
                {
                    node.set_description(format!("Active tab {}", tab.id));
                }
                node.set_bounds(rect(control.rect));
                node.add_action(Action::Click);
                node.add_action(Action::Focus);
                nodes.push((control_node_id(control.kind), node));
            }
            if let Some(hints) = &workspace.hints
                && !hints.pieces.is_empty()
            {
                let mut node = Node::new(Role::Label);
                node.set_description("Eon shortcut hints");
                node.set_value(
                    hints
                        .pieces
                        .iter()
                        .map(|hint| hint.accessible_label.as_str())
                        .collect::<Vec<_>>()
                        .join("\n"),
                );
                node.set_bounds(rect(hints.rect));
                nodes.push((HINTS, node));
            }

            if let Some(bounds) = workspace.sidebar() {
                let mut sidebar = Node::new(Role::Complementary);
                sidebar.set_label("Agent sidebar");
                sidebar.set_bounds(rect(bounds));
                if workspace.sidebar_empty() {
                    sidebar.set_children(vec![SIDEBAR_EMPTY_STATE]);
                    let mut empty = Node::new(Role::Label);
                    empty.set_value(SIDEBAR_EMPTY);
                    empty.set_bounds(rect(bounds));
                    nodes.push((SIDEBAR_EMPTY_STATE, empty));
                }
                // Each tab's heading, then its rows, in visual order.
                let mut children = Vec::new();
                for heading in &workspace.sidebar_headings {
                    let id = self.sidebar_ids[heading.tab.as_str()];
                    let mut node = Node::new(Role::Heading);
                    node.set_label(heading.label.as_str());
                    if let Some(bounds) = heading.rect.intersection(workspace.sidebar_list) {
                        node.set_bounds(rect(bounds));
                    }
                    children.push(id);
                    nodes.push((id, node));
                    for row in workspace
                        .sidebar_rows()
                        .iter()
                        .filter(|row| row.tab == heading.tab)
                    {
                        let id = self.sidebar_ids[row.target.as_str()];
                        let mut node = Node::new(Role::Button);
                        node.set_label(row.accessible_label.as_str());
                        if let Some(bounds) = row.rect.intersection(workspace.sidebar_list) {
                            node.set_bounds(rect(bounds));
                        }
                        node.add_action(Action::Click);
                        node.add_action(Action::Focus);
                        children.push(id);
                        nodes.push((id, node));
                    }
                }
                if !children.is_empty() {
                    sidebar.set_children(children);
                }
                nodes.push((SIDEBAR, sidebar));
            }

            let mut panel = Node::new(Role::TabPanel);
            let zoomed = workspace
                .zoomed_pane()
                .map(|pane| format!("Zoomed pane {}", pane.label()));
            panel.set_label(workspace.popup_label().or(zoomed.as_deref()).unwrap_or(
                if workspace.panes.is_empty() {
                    "Empty tab"
                } else {
                    "Active tab panes"
                },
            ));
            panel.set_orientation(Orientation::Vertical);
            panel.set_bounds(rect(workspace.pane_viewport));
            panel.set_clips_children();
            let mut children = Vec::with_capacity(
                workspace
                    .column_panes()
                    .map(|(panes, _)| panes.len())
                    .sum::<usize>()
                    + 1,
            );
            if workspace.panes.is_empty()
                && (terminal || visible_terminal.is_some() || !self.status.is_empty())
            {
                children.push(CONTENT);
            }
            for (panes, selected_column) in workspace.column_panes() {
                for pane in panes {
                    children.push(self.pane_ids[pane.id.as_str()]);
                    if pane.selected && selected_column {
                        children.push(CONTENT);
                    }
                }
            }
            if !children.contains(&CONTENT) {
                // AccessKit rejects a node without a parent. A tab without panes leaves CONTENT
                // out only when it holds no terminal (so no rows or links) and no status.
                nodes.retain(|(id, _)| *id != CONTENT);
            }
            panel.set_children(children);
            nodes.push((PANE_PANEL, panel));
            for (pane, selected_column) in workspace
                .column_panes()
                .flat_map(|(panes, selected)| panes.iter().map(move |pane| (pane, selected)))
            {
                let mut node = Node::new(Role::Button);
                node.set_label(pane.label());
                node.set_selected(pane.selected && selected_column);
                node.set_expanded(pane.selected);
                if let Some(bounds) = pane.rect.intersection(workspace.pane_viewport) {
                    node.set_bounds(rect(bounds));
                }
                node.add_action(Action::Click);
                node.add_action(Action::Focus);
                nodes.push((self.pane_ids[pane.id.as_str()], node));
            }
        }
        if let Some(columns) = self.columns {
            for (index, row) in self.content.rows.iter().enumerate() {
                let row_bounds = SceneRect {
                    left: terminal_layout.left + self.metrics.padding,
                    top: terminal_layout.top
                        + self.metrics.padding
                        + index as f32 * self.metrics.height,
                    width: f32::from(columns) * self.metrics.width,
                    height: self.metrics.height,
                };
                nodes.push((
                    text_run_id(index),
                    text_run(row, row_bounds, visible_terminal, self.metrics.width),
                ));
            }
        }
        for link in &self.links {
            let mut open = Node::new(Role::Link);
            open.set_label(link.label.as_str());
            open.set_bounds(rect(link.bounds));
            open.add_action(Action::Click);
            nodes.push((link.open, open));

            // The locked AT-SPI adapter exposes Click but not CustomAction.
            let mut copy = Node::new(Role::Button);
            copy.set_label(format!("Copy {}", link.label));
            copy.add_action(Action::Click);
            nodes.push((link.copy, copy));
        }
        if status_alert {
            let mut status = Node::new(Role::Alert);
            status.set_label(self.status.as_str());
            status.set_live(accesskit::Live::Assertive);
            nodes.push((STATUS, status));
        }

        TreeUpdate {
            nodes,
            tree: Some(TreeInfo::new(WINDOW)),
            tree_id: TreeId::ROOT,
            focus: self.workspace.as_ref().map_or(CONTENT, |workspace| {
                let selected_tab = workspace
                    .tabs
                    .iter()
                    .find(|tab| tab.selected)
                    .and_then(|tab| self.tab_ids.get(&tab.id).copied())
                    .unwrap_or(CONTENT);
                let content = visible_terminal.map_or(selected_tab, |_| CONTENT);
                match self.workspace_focus {
                    WorkspaceFocus::Terminal => content,
                    WorkspaceFocus::Panes => workspace
                        .panes
                        .iter()
                        .find(|pane| pane.selected)
                        .and_then(|pane| self.pane_ids.get(&pane.id).copied())
                        .unwrap_or(content),
                    WorkspaceFocus::Tabs => selected_tab,
                    WorkspaceFocus::Header(control) => control_node_id(control),
                    WorkspaceFocus::Sidebar => workspace
                        .sidebar_rows()
                        .iter()
                        .find(|row| row.focused)
                        .and_then(|row| self.sidebar_ids.get(&row.target).copied())
                        .unwrap_or(content),
                }
            }),
        }
    }

    fn action_target(&self, target: NodeId, action: Action) -> Option<AccessibilityTarget> {
        if let Some(dialog) = &self.dialog {
            if dialog.kind == DialogKind::AttentionInbox
                && matches!(action, Action::Click | Action::Focus)
            {
                if let Some(chip) = target
                    .0
                    .checked_sub(DIALOG_NODE_START)
                    .and_then(|index| usize::try_from(index).ok())
                    .and_then(|index| dialog.filters.get(index))
                {
                    return Some(AccessibilityTarget::InboxFilter(chip.filter));
                }
                if let Some((key, _)) = self.inbox_ids.iter().find(|(_, id)| **id == target) {
                    return Some(AccessibilityTarget::InboxRow {
                        tab: key.0.clone(),
                        target: key.1.clone(),
                        activate: action == Action::Click,
                    });
                }
            }
            return None;
        }
        if target == SCROLLBACK && action == Action::Click {
            return self
                .scrollback_label
                .as_ref()
                .zip(self.scrollback_rect)
                .map(|_| AccessibilityTarget::ReturnToLive);
        }
        if let Some(link) = self
            .links
            .iter()
            .find(|link| link.open == target || link.copy == target)
        {
            if action != Action::Click {
                return None;
            }
            return Some(AccessibilityTarget::Link {
                row: link.row,
                column: link.column,
                copy: link.copy == target,
            });
        }
        if !matches!(action, Action::Click | Action::Focus) {
            return None;
        }
        let workspace = self.workspace.as_ref()?;
        if let Some(control) = header_control_for_node(target) {
            return workspace
                .has_control(control)
                .then_some(AccessibilityTarget::HeaderControl {
                    control,
                    activate: action == Action::Click,
                });
        }
        if target == CONTENT {
            return workspace
                .visible_terminal()
                .map(|_| AccessibilityTarget::Terminal);
        }
        if let Some((id, _)) = self.tab_ids.iter().find(|(_, node)| **node == target) {
            return Some(AccessibilityTarget::Tab(id.clone()));
        }
        if let Some(row) = self
            .sidebar_ids
            .iter()
            .find(|(_, node)| **node == target)
            .and_then(|(id, _)| workspace.sidebar_row(id))
        {
            return Some(AccessibilityTarget::SidebarRow {
                target: row.target.clone(),
                activate: action == Action::Click,
            });
        }
        self.pane_ids
            .iter()
            .find(|(_, node)| **node == target)
            .map(|(id, _)| AccessibilityTarget::Pane(id.clone()))
    }
}

/// Direct activation callback that always returns the latest derived scene input.
pub struct Activation {
    snapshot: Arc<Mutex<Snapshot>>,
}

impl ActivationHandler for Activation {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        Some(lock(&self.snapshot).tree())
    }
}

/// Latest accessibility input derived from the accepted scene, shared only for activation.
pub struct Accessibility {
    snapshot: Arc<Mutex<Snapshot>>,
}

impl Accessibility {
    #[must_use]
    pub fn new(size: PhysicalSize<u32>) -> Self {
        Self {
            snapshot: Arc::new(Mutex::new(Snapshot::new(size))),
        }
    }

    #[must_use]
    pub fn activation(&self) -> Activation {
        Activation {
            snapshot: Arc::clone(&self.snapshot),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &self,
        adapter: &mut Adapter,
        scene: Option<&Scene>,
        workspace: Option<&WorkspaceScene>,
        dialog: Option<&DialogScene>,
        workspace_focus: WorkspaceFocus,
        status: &str,
        scrollback_label: Option<&str>,
        scrollback_rect: Option<SceneRect>,
        size: PhysicalSize<u32>,
        metrics: CellMetrics,
        link_generation: Option<u64>,
    ) {
        {
            let mut snapshot = lock(&self.snapshot);
            snapshot.size = size;
            snapshot.metrics = metrics;
            snapshot.status = status.to_owned();
            snapshot.scrollback_label = scrollback_label.map(str::to_owned);
            snapshot.scrollback_rect = scrollback_rect;
            snapshot.set_workspace(workspace);
            snapshot.set_dialog(dialog);
            snapshot.workspace_focus = workspace_focus;
            if let Some(scene) = scene {
                snapshot.title = if scene.title.is_empty() {
                    "Venus".into()
                } else {
                    scene.title.clone()
                };
                snapshot.content = scene.accessible_content();
                snapshot.columns = Some(scene.columns);
            } else {
                snapshot.title = "Venus".into();
                snapshot.content = AccessibleText::default();
                snapshot.columns = None;
            }
            snapshot.set_links(scene, link_generation);
        }
        adapter.update_if_active(|| lock(&self.snapshot).tree());
    }

    pub fn present_links(
        &self,
        adapter: &mut Adapter,
        scene: Option<&Scene>,
        generation: Option<u64>,
    ) {
        if lock(&self.snapshot).link_identity == link_identity(scene, generation) {
            return;
        }
        adapter.update_if_active(|| {
            let mut snapshot = lock(&self.snapshot);
            snapshot.set_links(scene, generation);
            snapshot.tree()
        });
    }

    #[must_use]
    pub fn action_target(&self, target: NodeId, action: Action) -> Option<AccessibilityTarget> {
        lock(&self.snapshot).action_target(target, action)
    }
}

fn link_identity(scene: Option<&Scene>, generation: Option<u64>) -> Option<(u64, u64)> {
    scene
        .zip(generation)
        .map(|(scene, generation)| (generation, scene.revision))
}

fn accessible_link_label(uri: &str) -> String {
    if uri.len() > MAX_LINK_BYTES {
        format!("Link target exceeds {MAX_LINK_BYTES} bytes")
    } else {
        uri.escape_debug().to_string()
    }
}

/// Accessible action routed back through Venus and Eon semantics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccessibilityTarget {
    InboxFilter(crate::AttentionFilter),
    InboxRow {
        tab: String,
        target: String,
        activate: bool,
    },
    Terminal,
    ReturnToLive,
    Tab(String),
    Pane(String),
    /// An agent sidebar row by its target; `activate` jumps to it, otherwise it takes focus.
    SidebarRow {
        target: String,
        activate: bool,
    },
    HeaderControl {
        control: WorkspaceHeaderControl,
        activate: bool,
    },
    Link {
        row: u16,
        column: u16,
        copy: bool,
    },
}

fn control_node_id(control: WorkspaceHeaderControl) -> NodeId {
    match control {
        WorkspaceHeaderControl::Workspaces => WORKSPACE_LABEL,
        WorkspaceHeaderControl::NewTab => NEW_TAB,
        WorkspaceHeaderControl::Shortcuts => SHORTCUTS,
        WorkspaceHeaderControl::CloseTab => CLOSE_TAB,
    }
}

fn header_control_for_node(node: NodeId) -> Option<WorkspaceHeaderControl> {
    match node {
        WORKSPACE_LABEL => Some(WorkspaceHeaderControl::Workspaces),
        NEW_TAB => Some(WorkspaceHeaderControl::NewTab),
        SHORTCUTS => Some(WorkspaceHeaderControl::Shortcuts),
        CLOSE_TAB => Some(WorkspaceHeaderControl::CloseTab),
        _ => None,
    }
}

fn bounds(size: PhysicalSize<u32>) -> Rect {
    Rect {
        x0: 0.0,
        y0: 0.0,
        x1: f64::from(size.width),
        y1: f64::from(size.height),
    }
}

fn rect(rect: SceneRect) -> Rect {
    Rect {
        x0: f64::from(rect.left),
        y0: f64::from(rect.top),
        x1: f64::from(rect.right()),
        y1: f64::from(rect.bottom()),
    }
}

fn allocate_workspace_id(ids: &mut HashMap<String, NodeId>, next_id: &mut u64, identity: &str) {
    if ids.contains_key(identity) {
        return;
    }
    let following_id = next_id
        .checked_add(1)
        .filter(|next| *next <= CODEX_QUOTA_NODE_START)
        .expect("AccessKit workspace node IDs exhausted");
    ids.insert(identity.to_owned(), NodeId(*next_id));
    *next_id = following_id;
}

fn quota_id(quota: &WorkspaceQuota) -> NodeId {
    match quota.window {
        Some(QuotaWindowIdentity::Codex {
            duration,
            occurrence,
        }) => NodeId(CODEX_QUOTA_NODE_START + 2 * u64::from(duration) + u64::from(occurrence)),
        Some(QuotaWindowIdentity::Claude(10080)) => CLAUDE_SEVEN_DAY_QUOTA,
        Some(QuotaWindowIdentity::Claude(_)) => CLAUDE_FIVE_HOUR_QUOTA,
        None => QUOTA,
    }
}

fn text_run_id(index: usize) -> NodeId {
    NodeId(10_000 + index as u64)
}

fn text_position(position: AccessiblePosition) -> TextPosition {
    TextPosition {
        node: text_run_id(position.row),
        character_index: position.character_index,
    }
}

fn text_selection(selection: AccessibleSelection) -> TextSelection {
    TextSelection {
        anchor: text_position(selection.anchor),
        focus: text_position(selection.focus),
    }
}

fn text_run(
    row: &AccessibleRow,
    row_bounds: SceneRect,
    visible_terminal: Option<SceneRect>,
    cell_width: f32,
) -> Node {
    let mut node = Node::new(Role::TextRun);
    node.set_value(row.value.as_str());
    node.set_character_lengths(row.character_lengths.clone());
    node.set_text_direction(TextDirection::LeftToRight);
    if let Some(bounds) = visible_terminal.and_then(|visible| row_bounds.intersection(visible)) {
        node.set_bounds(rect(bounds));
        let (positions, widths): (Vec<_>, Vec<_>) = row
            .column_spans
            .iter()
            .map(|span| {
                let start = (row_bounds.left + span.start as f32 * cell_width)
                    .clamp(bounds.left, bounds.right());
                let end = (row_bounds.left + span.end as f32 * cell_width)
                    .clamp(bounds.left, bounds.right());
                (start - bounds.left, end - start)
            })
            .unzip();
        node.set_character_positions(positions);
        node.set_character_widths(widths);
    }
    node
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::QuotaProvider;
    use crate::{
        Color, DrawCell, DrawRow, DrawStyle, EonBar, SceneGraphics, ShortcutGroup, ShortcutRow,
    };
    use eon_workspace_protocol::v26::{
        ClaudeQuota, ClaudeQuotaState, CodexQuota, CodexQuotaState, CodexQuotaWindow, Pane, Popup,
        PopupEntry, PopupGeometry, PopupKey, Shortcut, Snapshot as WorkspaceSnapshot, Tab,
    };
    use orbit_protocol::{CellWidth, Screen, Underline};

    fn workspace_tab(id: &str, panes: &[&str], selected_pane: &str) -> Tab {
        Tab {
            pending: false,
            selected_popup: None,
            popups: Vec::new(),
            id: id.into(),
            directory: format!("/tmp/{id}").into_bytes(),
            selected_pane: Some(selected_pane.into()),
            columns: vec![eon_workspace_protocol::v26::Column {
                panes: panes
                    .iter()
                    .map(|id| Pane {
                        label: format!("{id}  unavailable"),
                        pinned_name: None,
                        id: (*id).into(),
                        session: format!("session-{id}"),
                        endpoint: format!("/run/eon/{id}.sock").into_bytes(),
                        live: true,
                        attention: None,
                    })
                    .collect(),
                expanded: selected_pane.into(),
            }],
            attention_rollup: None,
        }
    }

    fn workspace_scene(active_tab: &str, tabs: Vec<Tab>) -> WorkspaceScene {
        WorkspaceScene::from_snapshot(
            &WorkspaceSnapshot {
                workspace: "w1".into(),
                active_workspace: "w1".into(),
                workspaces: vec![eon_workspace_protocol::v26::WorkspaceInfo {
                    id: "w1".into(),
                    name: None,
                    tab_count: tabs.len(),
                    attention: None,
                    recency: 0,
                }],
                active_tab: active_tab.into(),
                tabs,
                geometry: eon_workspace_protocol::v26::PopupGeometry {
                    side_margin: 8.0,
                    vertical_margin: 4.0,
                },
                entries: Vec::new(),
                codex_quota: None,
                claude_quota: None,
                sidebar: eon_workspace_protocol::v26::Sidebar::Open,
                attention_queue: Vec::new(),
            },
            PhysicalSize::new(800, 600),
            CellMetrics::for_scale(1.0),
            0.0,
            0.0,
            |_, text| (text.to_owned(), text.chars().count() as f32 * 10.0),
        )
    }

    fn set_workspace(
        accessibility: &Accessibility,
        workspace: WorkspaceScene,
        focus: WorkspaceFocus,
    ) {
        let mut snapshot = lock(&accessibility.snapshot);
        snapshot.set_workspace(Some(&workspace));
        snapshot.workspace_focus = focus;
    }

    fn tree_node_id(update: &TreeUpdate, label: &str) -> NodeId {
        update
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some(label))
            .map(|(id, _)| *id)
            .unwrap()
    }

    struct IgnoreChanges;

    impl accesskit_consumer::TreeChangeHandler for IgnoreChanges {
        fn node_added(&mut self, _: &accesskit_consumer::NodeRef) {}
        fn node_updated(
            &mut self,
            _: &accesskit_consumer::NodeRef,
            _: &accesskit_consumer::NodeRef,
        ) {
        }
        fn focus_moved(
            &mut self,
            _: Option<&accesskit_consumer::NodeRef>,
            _: Option<&accesskit_consumer::NodeRef>,
        ) {
        }
        fn node_removed(&mut self, _: &accesskit_consumer::NodeRef) {}
    }

    fn node(update: &TreeUpdate, id: NodeId) -> &Node {
        &update
            .nodes
            .iter()
            .find(|(node_id, _)| *node_id == id)
            .unwrap()
            .1
    }

    #[test]
    fn version_label_uses_only_spare_space_and_has_no_actions() {
        for scale in [1.0, 1.25, 2.0] {
            let metrics = CellMetrics::for_scale(scale);
            for logical_width in [100, 320, 1100, 3000] {
                for tab_count in [1, 10] {
                    for providers in 0..4 {
                        let snapshot = WorkspaceSnapshot {
                            workspace: "w1".into(),
                            active_workspace: "w1".into(),
                            workspaces: vec![eon_workspace_protocol::v26::WorkspaceInfo {
                                id: "w1".into(),
                                name: None,
                                tab_count,
                                attention: None,
                                recency: 0,
                            }],
                            active_tab: "t1".into(),
                            geometry: PopupGeometry {
                                side_margin: 8.0,
                                vertical_margin: 4.0,
                            },
                            entries: Vec::new(),
                            tabs: (1..=tab_count)
                                .map(|n| workspace_tab(&format!("t{n}"), &["p1"], "p1"))
                                .collect(),
                            codex_quota: (providers & 1 != 0).then_some(CodexQuota {
                                state: CodexQuotaState::Fresh,
                                observed_at: 1,
                                windows: vec![CodexQuotaWindow {
                                    duration_minutes: 300,
                                    remaining_percent: 74,
                                    resets_at: None,
                                }],
                            }),
                            claude_quota: (providers & 2 != 0).then_some(ClaudeQuota {
                                state: ClaudeQuotaState::Fresh,
                                observed_at: 1,
                                windows: vec![CodexQuotaWindow {
                                    duration_minutes: 300,
                                    remaining_percent: 74,
                                    resets_at: None,
                                }],
                            }),
                            sidebar: eon_workspace_protocol::v26::Sidebar::Open,
                            attention_queue: Vec::new(),
                        };
                        let size =
                            PhysicalSize::new((logical_width as f32 * metrics.scale) as u32, 600);
                        for hide_help in [false, true] {
                            let hints = hide_help.then(|| {
                                crate::ShortcutHintsScene::new(
                                    vec![crate::ShortcutHint {
                                        modifiers: "Alt".into(),
                                        key: "/".into(),
                                        label: "keys".into(),
                                        accessible_label: "Alt+/ — Show shortcuts".into(),
                                        priority: 0,
                                    }],
                                    size,
                                    metrics,
                                    WorkspaceFocus::Terminal,
                                    |text| text.len() as f32 * metrics.width,
                                )
                            });
                            let project = |version| {
                                WorkspaceScene::from_snapshot_with_bar(
                                    &snapshot,
                                    EonBar {
                                        version,
                                        ..EonBar::default()
                                    },
                                    size,
                                    metrics,
                                    0.0,
                                    0.0,
                                    |_, text| {
                                        (
                                            text.to_owned(),
                                            text.chars().count() as f32 * 10.0 * metrics.scale,
                                        )
                                    },
                                    hints.clone(),
                                )
                            };
                            let baseline = project(None);
                            let scene = project(Some("9.8.7-alpha.2+proof"));
                            assert_eq!(scene.tabs, baseline.tabs);
                            assert_eq!(scene.tab_viewport, baseline.tab_viewport);
                            assert_eq!(scene.controls, baseline.controls);
                            assert_eq!(scene.terminal, baseline.terminal);
                            assert_eq!(scene.panes, baseline.panes);
                            assert_eq!(scene.quotas.len(), baseline.quotas.len());
                            assert_eq!(
                                scene.version.is_some(),
                                tab_count == 1 && logical_width >= 1100 || logical_width == 3000
                            );
                            let mut accessible = Snapshot::new(size);
                            accessible.set_workspace(Some(&scene));
                            let tree = accessible.tree();
                            assert_eq!(
                                tree.nodes.iter().any(|(id, _)| *id == VERSION),
                                scene.version.is_some()
                            );
                            if let Some(version) = &scene.version {
                                assert_eq!(version.label, "EON 9.8.7-alpha.2+proof");
                                assert_eq!(
                                    version.rect.width,
                                    version.label.len() as f32 * metrics.width
                                        + metrics.padding * 2.0
                                );
                                let center = (
                                    version.rect.left + version.rect.width / 2.0,
                                    version.rect.height / 2.0,
                                );
                                assert_eq!(
                                    scene.hit_test(center.0, center.1),
                                    Some(crate::WorkspaceHit::Version)
                                );
                                assert!(scene.drag_region.width >= metrics.font_size * 4.0);
                                assert!(scene.drag_region.right() <= version.rect.left);
                                assert_eq!(version.rect.right(), scene.controls[1].rect.left);
                                assert_eq!(
                                    scene.controls[1].rect.right(),
                                    scene.controls[2].rect.left
                                );
                                let children = node(&tree, WINDOW).children();
                                let version_index =
                                    children.iter().position(|id| *id == VERSION).unwrap();
                                if scene.has_control(WorkspaceHeaderControl::Shortcuts) {
                                    assert_eq!(children[version_index + 1], SHORTCUTS);
                                    assert_eq!(children[version_index + 2], CLOSE_TAB);
                                    let rect = scene.controls[1].rect;
                                    assert_eq!(
                                        scene.hit_test(
                                            rect.left + rect.width / 2.0,
                                            rect.height / 2.0
                                        ),
                                        Some(crate::WorkspaceHit::Control(
                                            WorkspaceHeaderControl::Shortcuts
                                        ))
                                    );
                                } else {
                                    assert_eq!(children[version_index + 1], CLOSE_TAB);
                                }
                                if let Some(quota) = scene.quotas.first() {
                                    assert_eq!(scene.drag_region.right(), quota.rect.left);
                                }
                                if let Some(quota) = scene.quotas.last() {
                                    assert_eq!(quota.rect.right(), version.rect.left);
                                }
                                let node = node(&tree, VERSION);
                                assert_eq!(node.role(), Role::Label);
                                assert_eq!(
                                    node.value(),
                                    Some("Running Eon version 9.8.7-alpha.2+proof")
                                );
                                assert!(!node.supports_action(Action::Click));
                                assert!(!node.supports_action(Action::Focus));
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn local_clock_names_minute_date_and_offset() {
        let at = |hour, minute, offset| {
            crate::LocalClock::new(
                &jiff::civil::date(2026, 10, 21)
                    .at(hour, minute, 59, 999_999_999)
                    .to_zoned(jiff::tz::TimeZone::fixed(offset))
                    .unwrap(),
            )
        };
        for (clock, label, description) in [
            (
                at(14, 5, jiff::tz::offset(-3)),
                "14:05",
                "Local time 14:05, Wednesday 21 October 2026, UTC\u{2212}03:00",
            ),
            (
                at(0, 0, jiff::tz::Offset::UTC),
                "00:00",
                "Local time 00:00, Wednesday 21 October 2026, UTC+00:00",
            ),
            (
                at(
                    23,
                    59,
                    jiff::tz::Offset::from_seconds(5 * 3600 + 45 * 60).unwrap(),
                ),
                "23:59",
                "Local time 23:59, Wednesday 21 October 2026, UTC+05:45",
            ),
        ] {
            let workspace = WorkspaceScene::from_snapshot_with_bar(
                &WorkspaceSnapshot {
                    workspace: "w1".into(),
                    active_workspace: "w1".into(),
                    workspaces: vec![eon_workspace_protocol::v26::WorkspaceInfo {
                        id: "w1".into(),
                        name: None,
                        tab_count: 1,
                        attention: None,
                        recency: 0,
                    }],
                    active_tab: "t1".into(),
                    geometry: PopupGeometry {
                        side_margin: 8.0,
                        vertical_margin: 4.0,
                    },
                    entries: Vec::new(),
                    tabs: vec![workspace_tab("t1", &["p1"], "p1")],
                    codex_quota: None,
                    claude_quota: None,
                    sidebar: eon_workspace_protocol::v26::Sidebar::Open,
                    attention_queue: Vec::new(),
                },
                EonBar {
                    clock: Some(&clock),
                    ..EonBar::default()
                },
                PhysicalSize::new(1100, 600),
                CellMetrics::for_scale(1.0),
                0.0,
                0.0,
                |_, text| (text.to_owned(), text.chars().count() as f32 * 10.0),
                None,
            );
            let shown = workspace.clock.unwrap();
            assert_eq!(shown.label, label);
            assert_eq!(shown.description, description);
        }
    }

    #[test]
    fn local_clock_yields_first_and_has_no_actions() {
        let clock = crate::LocalClock::new(
            &jiff::civil::date(2026, 10, 21)
                .at(14, 5, 0, 0)
                .to_zoned(jiff::tz::TimeZone::fixed(jiff::tz::offset(-3)))
                .unwrap(),
        );
        for scale in [1.0, 1.25, 2.0] {
            let metrics = CellMetrics::for_scale(scale);
            for logical_width in [100, 320, 700, 1100, 3000] {
                for tab_count in [1, 10] {
                    for providers in 0..4 {
                        let window = || {
                            vec![CodexQuotaWindow {
                                duration_minutes: 300,
                                remaining_percent: 74,
                                resets_at: None,
                            }]
                        };
                        let snapshot = WorkspaceSnapshot {
                            workspace: "w1".into(),
                            active_workspace: "w1".into(),
                            workspaces: vec![eon_workspace_protocol::v26::WorkspaceInfo {
                                id: "w1".into(),
                                name: None,
                                tab_count,
                                attention: None,
                                recency: 0,
                            }],
                            active_tab: "t1".into(),
                            geometry: PopupGeometry {
                                side_margin: 8.0,
                                vertical_margin: 4.0,
                            },
                            entries: Vec::new(),
                            tabs: (1..=tab_count)
                                .map(|n| workspace_tab(&format!("t{n}"), &["p1"], "p1"))
                                .collect(),
                            codex_quota: (providers & 1 != 0).then(|| CodexQuota {
                                state: CodexQuotaState::Fresh,
                                observed_at: 1,
                                windows: window(),
                            }),
                            claude_quota: (providers & 2 != 0).then(|| ClaudeQuota {
                                state: ClaudeQuotaState::Fresh,
                                observed_at: 1,
                                windows: window(),
                            }),
                            sidebar: eon_workspace_protocol::v26::Sidebar::Open,
                            attention_queue: Vec::new(),
                        };
                        let project = |logical_width: u32, version, clock| {
                            WorkspaceScene::from_snapshot_with_bar(
                                &snapshot,
                                EonBar {
                                    version,
                                    clock,
                                    ..EonBar::default()
                                },
                                PhysicalSize::new(
                                    (logical_width as f32 * metrics.scale) as u32,
                                    600,
                                ),
                                metrics,
                                0.0,
                                0.0,
                                |_, text| {
                                    (
                                        text.to_owned(),
                                        text.chars().count() as f32 * 10.0 * metrics.scale,
                                    )
                                },
                                None,
                            )
                        };
                        let full_quotas = project(3000, None, None)
                            .quotas
                            .into_iter()
                            .map(|quota| quota.label)
                            .collect::<Vec<_>>();
                        for version in [None, Some("9.8.7")] {
                            let baseline = project(logical_width, version, None);
                            let scene = project(logical_width, version, Some(&clock));
                            assert_eq!(scene.tabs, baseline.tabs);
                            assert_eq!(scene.tab_viewport, baseline.tab_viewport);
                            assert_eq!(scene.controls, baseline.controls);
                            assert_eq!(scene.terminal, baseline.terminal);
                            assert_eq!(scene.panes, baseline.panes);
                            assert_eq!(
                                scene
                                    .quotas
                                    .iter()
                                    .map(|quota| &quota.label)
                                    .collect::<Vec<_>>(),
                                baseline
                                    .quotas
                                    .iter()
                                    .map(|quota| &quota.label)
                                    .collect::<Vec<_>>()
                            );
                            assert_eq!(scene.version.is_some(), baseline.version.is_some());
                            let mut accessible = Snapshot::new(PhysicalSize::new(
                                (logical_width as f32 * metrics.scale) as u32,
                                600,
                            ));
                            accessible.set_workspace(Some(&scene));
                            let tree = accessible.tree();
                            assert_eq!(
                                tree.nodes.iter().any(|(id, _)| *id == CLOCK),
                                scene.clock.is_some()
                            );
                            let Some(shown) = &scene.clock else {
                                assert!(logical_width < 3000, "a wide bar shows the clock");
                                continue;
                            };
                            // The clock yields before quota chips and the version label, which
                            // follows it at the edge.
                            assert_eq!(
                                scene
                                    .quotas
                                    .iter()
                                    .map(|quota| &quota.label)
                                    .collect::<Vec<_>>(),
                                full_quotas.iter().collect::<Vec<_>>()
                            );
                            assert_eq!(version.is_some(), scene.version.is_some());
                            assert_eq!(
                                shown.rect.width,
                                5.0 * metrics.width + metrics.padding * 2.0
                            );
                            assert!(scene.drag_region.width >= metrics.font_size * 4.0);
                            assert_eq!(
                                shown.rect.right(),
                                scene
                                    .version
                                    .as_ref()
                                    .map_or(scene.controls[1].rect.left, |version| version
                                        .rect
                                        .left)
                            );
                            assert_eq!(
                                scene
                                    .quotas
                                    .last()
                                    .map_or(scene.drag_region.right(), |quota| quota.rect.right()),
                                shown.rect.left
                            );
                            if let Some(quota) = scene.quotas.first() {
                                assert_eq!(scene.drag_region.right(), quota.rect.left);
                            }
                            assert_eq!(
                                scene.hit_test(
                                    shown.rect.left + shown.rect.width / 2.0,
                                    shown.rect.height / 2.0
                                ),
                                Some(crate::WorkspaceHit::Clock)
                            );
                            let children = node(&tree, WINDOW).children();
                            let index = children.iter().position(|id| *id == CLOCK).unwrap();
                            assert_eq!(
                                children[index - 1],
                                scene.quotas.last().map_or(NEW_TAB, quota_id)
                            );
                            assert_eq!(children[index + 1] == VERSION, version.is_some());
                            let node = node(&tree, CLOCK);
                            assert_eq!(node.role(), Role::Label);
                            assert_eq!(
                                node.value(),
                                Some(
                                    "Local time 14:05, Wednesday 21 October 2026, UTC\u{2212}03:00"
                                )
                            );
                            assert!(!node.supports_action(Action::Click));
                            assert!(!node.supports_action(Action::Focus));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn readings_yield_after_the_clock_and_have_no_actions() {
        let cpu = crate::CpuUse::between(
            crate::CpuTimes::parse(
                "cpu 0 0 0 0 0 0 0 0\ncpu0 0 0 0 0 0 0 0 0\ncpu1 0 0 0 0 0 0 0 0",
            )
            .unwrap(),
            crate::CpuTimes::parse(
                "cpu 12 0 0 88 0 0 0 0\ncpu0 6 0 0 44 0 0 0 0\ncpu1 6 0 0 44 0 0 0 0",
            )
            .unwrap(),
        )
        .unwrap();
        let ram =
            crate::RamUse::parse("MemTotal: 32800000 kB\nMemAvailable: 18677760 kB\n").unwrap();
        let clock = crate::LocalClock::new(
            &jiff::civil::date(2026, 10, 21)
                .at(14, 5, 0, 0)
                .to_zoned(jiff::tz::TimeZone::fixed(jiff::tz::offset(-3)))
                .unwrap(),
        );
        let metrics = CellMetrics::for_scale(1.25);
        let snapshot = WorkspaceSnapshot {
            workspace: "w1".into(),
            active_workspace: "w1".into(),
            workspaces: vec![eon_workspace_protocol::v26::WorkspaceInfo {
                id: "w1".into(),
                name: None,
                tab_count: 1,
                attention: None,
                recency: 0,
            }],
            active_tab: "t1".into(),
            geometry: PopupGeometry {
                side_margin: 8.0,
                vertical_margin: 4.0,
            },
            entries: Vec::new(),
            tabs: vec![workspace_tab("t1", &["p1"], "p1")],
            codex_quota: Some(CodexQuota {
                state: CodexQuotaState::Fresh,
                observed_at: 1,
                windows: vec![CodexQuotaWindow {
                    duration_minutes: 300,
                    remaining_percent: 74,
                    resets_at: None,
                }],
            }),
            claude_quota: None,
            sidebar: eon_workspace_protocol::v26::Sidebar::Open,
            attention_queue: Vec::new(),
        };
        let project = |logical_width: u32, version, cpu, ram, clock| {
            WorkspaceScene::from_snapshot_with_bar(
                &snapshot,
                EonBar {
                    version,
                    cpu,
                    ram,
                    clock,
                    ..EonBar::default()
                },
                PhysicalSize::new((logical_width as f32 * metrics.scale) as u32, 600),
                metrics,
                0.0,
                0.0,
                |_, text| {
                    (
                        text.to_owned(),
                        text.chars().count() as f32 * 10.0 * metrics.scale,
                    )
                },
                None,
            )
        };
        let full_quotas = project(3000, None, None, None, None).quotas;
        // Widths that show CPU use without RAM, and RAM without the clock.
        let mut partial = [false; 2];
        for version in [None, Some("9.8.7")] {
            for logical_width in (100..=1600).step_by(5) {
                let baseline = project(logical_width, version, None, None, None);
                let scene = project(logical_width, version, Some(&cpu), Some(&ram), Some(&clock));
                assert_eq!(scene.tabs, baseline.tabs);
                assert_eq!(scene.controls, baseline.controls);
                assert_eq!(scene.terminal, baseline.terminal);
                assert_eq!(scene.version.is_some(), baseline.version.is_some());
                // The clock yields first, then RAM, then CPU use, before the version label and
                // quotas.
                assert!(scene.clock.is_none() || scene.ram.is_some());
                assert!(scene.ram.is_none() || scene.cpu.is_some());
                let Some(shown_cpu) = &scene.cpu else {
                    assert!(logical_width < 1600, "a wide bar shows CPU use");
                    continue;
                };
                partial[0] |= scene.ram.is_none();
                partial[1] |= scene.ram.is_some() && scene.clock.is_none();
                assert_eq!(scene.quotas.len(), full_quotas.len());
                assert_eq!(scene.quotas[0].label, full_quotas[0].label);
                assert_eq!(version.is_some(), scene.version.is_some());
                assert_eq!(shown_cpu.label, "CPU 12%");
                if let Some(shown_ram) = &scene.ram {
                    assert_eq!(shown_ram.label, "RAM 43%");
                }
                // The readings follow the quota chips in order, and the version label sits at
                // the edge before Shortcuts and Close.
                let labels = [
                    (CPU, Some(shown_cpu)),
                    (RAM, scene.ram.as_ref()),
                    (CLOCK, scene.clock.as_ref()),
                    (VERSION, scene.version.as_ref()),
                ]
                .into_iter()
                .filter_map(|(id, label)| Some((id, label?)))
                .collect::<Vec<_>>();
                let mut left = scene.quotas[0].rect.right();
                for (id, label) in &labels {
                    assert_eq!(label.rect.left, left);
                    left = label.rect.right();
                    // CPU and RAM keep the width of `CPU 100%`; the clock fits its text.
                    let cells = match *id {
                        CPU | RAM => "CPU 100%".len(),
                        CLOCK => label.label.len(),
                        _ => continue,
                    };
                    assert_eq!(
                        label.rect.width,
                        cells as f32 * metrics.width + metrics.padding * 2.0
                    );
                }
                assert_eq!(left, scene.controls[1].rect.left);
                assert!(scene.drag_region.width >= metrics.font_size * 4.0);
                assert_eq!(scene.drag_region.right(), scene.quotas[0].rect.left);
                for (label, hit) in [
                    (Some(shown_cpu), crate::WorkspaceHit::Cpu),
                    (scene.ram.as_ref(), crate::WorkspaceHit::Ram),
                ] {
                    let Some(label) = label else { continue };
                    assert_eq!(
                        scene.hit_test(
                            label.rect.left + label.rect.width / 2.0,
                            label.rect.height / 2.0
                        ),
                        Some(hit)
                    );
                }
                let mut accessible = Snapshot::new(PhysicalSize::new(
                    (logical_width as f32 * metrics.scale) as u32,
                    600,
                ));
                accessible.set_workspace(Some(&scene));
                let tree = accessible.tree();
                let children = node(&tree, WINDOW).children();
                let index = children.iter().position(|id| *id == CPU).unwrap();
                assert_eq!(children[index - 1], quota_id(&scene.quotas[0]));
                assert_eq!(
                    children[index..index + labels.len()],
                    labels.iter().map(|(id, _)| *id).collect::<Vec<_>>()[..]
                );
                for (id, value) in [
                    (
                        CPU,
                        "CPU use 12% across 2 logical CPUs, over the last 2 seconds",
                    ),
                    (RAM, "RAM use 43% of 31.3 GiB"),
                ] {
                    if id == RAM && scene.ram.is_none() {
                        continue;
                    }
                    let node = node(&tree, id);
                    assert_eq!(node.role(), Role::Label);
                    assert_eq!(node.value(), Some(value));
                    assert!(!node.supports_action(Action::Click));
                    assert!(!node.supports_action(Action::Focus));
                }
            }
        }
        assert_eq!(partial, [true; 2]);
    }

    #[test]
    fn eon_orders_and_omits_bar_items() {
        let cpu = crate::CpuUse::between(
            crate::CpuTimes::parse(
                "cpu 0 0 0 0 0 0 0 0\ncpu0 0 0 0 0 0 0 0 0\ncpu1 0 0 0 0 0 0 0 0",
            )
            .unwrap(),
            crate::CpuTimes::parse(
                "cpu 12 0 0 88 0 0 0 0\ncpu0 6 0 0 44 0 0 0 0\ncpu1 6 0 0 44 0 0 0 0",
            )
            .unwrap(),
        )
        .unwrap();
        let ram =
            crate::RamUse::parse("MemTotal: 32800000 kB\nMemAvailable: 18677760 kB\n").unwrap();
        let clock = crate::LocalClock::new(
            &jiff::civil::date(2026, 10, 21)
                .at(14, 5, 0, 0)
                .to_zoned(jiff::tz::TimeZone::fixed(jiff::tz::offset(-3)))
                .unwrap(),
        );
        let windows = || {
            vec![CodexQuotaWindow {
                duration_minutes: 300,
                remaining_percent: 74,
                resets_at: None,
            }]
        };
        let snapshot = WorkspaceSnapshot {
            workspace: "w1".into(),
            active_workspace: "w1".into(),
            workspaces: vec![eon_workspace_protocol::v26::WorkspaceInfo {
                id: "w1".into(),
                name: None,
                tab_count: 1,
                attention: None,
                recency: 0,
            }],
            active_tab: "t1".into(),
            geometry: PopupGeometry {
                side_margin: 8.0,
                vertical_margin: 4.0,
            },
            entries: Vec::new(),
            tabs: vec![workspace_tab("t1", &["p1"], "p1")],
            codex_quota: Some(CodexQuota {
                state: CodexQuotaState::Fresh,
                observed_at: 1,
                windows: windows(),
            }),
            claude_quota: Some(ClaudeQuota {
                state: ClaudeQuotaState::Fresh,
                observed_at: 1,
                windows: windows(),
            }),
            sidebar: eon_workspace_protocol::v26::Sidebar::Open,
            attention_queue: Vec::new(),
        };
        let metrics = CellMetrics::for_scale(1.0);
        let size = PhysicalSize::new(3000, 600);
        let project = |items: &[crate::BarItem]| {
            WorkspaceScene::from_snapshot_with_bar(
                &snapshot,
                EonBar {
                    items,
                    version: Some("9.8.7"),
                    cpu: Some(&cpu),
                    ram: Some(&ram),
                    clock: Some(&clock),
                    workspace_picker: None,
                },
                size,
                metrics,
                0.0,
                0.0,
                |_, text| (text.to_owned(), text.chars().count() as f32 * 10.0),
                None,
            )
        };
        let full = project(&crate::BarItem::DEFAULT);
        let scene = project(&[
            crate::BarItem::Clock,
            crate::BarItem::ClaudeQuota,
            crate::BarItem::Version,
            crate::BarItem::Cpu,
            crate::BarItem::CodexQuota,
        ]);
        assert_eq!(scene.tabs, full.tabs);
        assert_eq!(scene.controls, full.controls);
        assert_eq!(scene.terminal, full.terminal);
        // The left-out RAM label has no chip, hit target or accessible node.
        assert!(scene.ram.is_none());
        let [claude, codex] = &scene.quotas[..] else {
            panic!("both quota chips show");
        };
        assert_eq!(claude.provider.name(), "Claude");
        let rects = [
            scene.clock.as_ref().unwrap().rect,
            claude.rect,
            scene.version.as_ref().unwrap().rect,
            scene.cpu.as_ref().unwrap().rect,
            codex.rect,
        ];
        assert_eq!(scene.drag_region.right(), rects[0].left);
        for pair in rects.windows(2) {
            assert_eq!(pair[0].right(), pair[1].left);
        }
        assert_eq!(rects[4].right(), scene.controls[1].rect.left);
        assert_eq!(codex.logo.left, codex.rect.left + metrics.padding);
        let mut accessible = Snapshot::new(size);
        accessible.set_workspace(Some(&scene));
        let tree = accessible.tree();
        let children = node(&tree, WINDOW).children();
        let start = children.iter().position(|id| *id == NEW_TAB).unwrap() + 1;
        assert_eq!(
            children[start..start + 6],
            [
                CLOCK,
                quota_id(claude),
                VERSION,
                CPU,
                quota_id(codex),
                SHORTCUTS
            ]
        );
        assert!(tree.nodes.iter().all(|(id, _)| *id != RAM));
        // With no items, the drag region reaches Shortcuts.
        let bare = project(&[]);
        assert!(bare.quotas.is_empty() && bare.cpu.is_none() && bare.version.is_none());
        assert_eq!(bare.drag_region.right(), bare.controls[1].rect.left);
    }

    #[test]
    #[should_panic(expected = "AccessKit workspace node IDs exhausted")]
    fn workspace_node_ids_cannot_enter_codex_quota_range() {
        let mut ids = HashMap::new();
        let mut next = CODEX_QUOTA_NODE_START - 1;
        allocate_workspace_id(&mut ids, &mut next, "last");
        assert_eq!(ids["last"], NodeId(CODEX_QUOTA_NODE_START - 1));
        allocate_workspace_id(&mut ids, &mut next, "overflow");
    }

    #[test]
    fn quota_chip_preserves_truth_layout_and_static_accessibility() {
        let windows = || {
            vec![
                CodexQuotaWindow {
                    duration_minutes: 5 * 60,
                    remaining_percent: 74,
                    resets_at: Some(1_789_752_461),
                },
                CodexQuotaWindow {
                    duration_minutes: 7 * 24 * 60,
                    remaining_percent: 42,
                    resets_at: Some(1_789_814_936),
                },
            ]
        };
        let quota = |state, windows| {
            Some(CodexQuota {
                state,
                observed_at: 1_789_745_261,
                windows,
            })
        };
        let five_hour = (
            "3h/5h 74%",
            "5h window: 74% remaining; observed 3h into 5h.",
            "7d",
        );
        let seven_day = (
            "6d4h/7d 42%",
            "7d window: 42% remaining; observed 6d4h into 7d.",
            "5h",
        );
        let mut reversed = windows();
        reversed.reverse();
        let cases = [
            (
                "fresh",
                quota(CodexQuotaState::Fresh, reversed),
                vec![five_hour, seven_day],
            ),
            (
                "stale",
                quota(CodexQuotaState::Stale, windows()),
                vec![
                    ("3h/5h 74% old", five_hour.1, five_hour.2),
                    ("6d4h/7d 42% old", seven_day.1, seven_day.2),
                ],
            ),
            (
                "one fresh window",
                quota(CodexQuotaState::Fresh, vec![windows()[0].clone()]),
                vec![five_hour],
            ),
            (
                "one stale window",
                quota(CodexQuotaState::Stale, vec![windows()[0].clone()]),
                vec![("3h/5h 74% old", five_hour.1, five_hour.2)],
            ),
            (
                "unusual durations",
                quota(
                    CodexQuotaState::Fresh,
                    vec![
                        windows()[0].clone(),
                        CodexQuotaWindow {
                            duration_minutes: 90,
                            remaining_percent: 50,
                            resets_at: Some(1_789_750_631),
                        },
                    ],
                ),
                vec![
                    (
                        "0m/90m 50%",
                        "90m window: 50% remaining; observed 0m into 90m.",
                        "5h",
                    ),
                    (five_hour.0, five_hour.1, "90m"),
                ],
            ),
            (
                "equal durations preserve order",
                quota(
                    CodexQuotaState::Fresh,
                    vec![
                        CodexQuotaWindow {
                            remaining_percent: 42,
                            ..windows()[0].clone()
                        },
                        windows()[0].clone(),
                    ],
                ),
                vec![
                    (
                        "3h/5h 42%",
                        "5h window: 42% remaining; observed 3h into 5h.",
                        "74%",
                    ),
                    (five_hour.0, five_hour.1, "42%"),
                ],
            ),
            (
                "reset unavailable",
                quota(
                    CodexQuotaState::Fresh,
                    vec![CodexQuotaWindow {
                        resets_at: None,
                        ..windows()[0].clone()
                    }],
                ),
                vec![("5h 74%", "reset time unavailable", "7d")],
            ),
            (
                "blocked",
                quota(CodexQuotaState::Blocked, Vec::new()),
                vec![("blocked", "permission: blocked", "window:")],
            ),
            (
                "unknown",
                quota(CodexQuotaState::Unknown, Vec::new()),
                vec![("unknown", "permission: unknown", "window:")],
            ),
            ("absent", None, vec![]),
        ];

        for (name, codex_quota, expected) in cases {
            let workspace_snapshot = WorkspaceSnapshot {
                workspace: "w1".into(),
                active_workspace: "w1".into(),
                workspaces: vec![eon_workspace_protocol::v26::WorkspaceInfo {
                    id: "w1".into(),
                    name: None,
                    tab_count: 1,
                    attention: None,
                    recency: 0,
                }],
                active_tab: "t1".into(),
                geometry: PopupGeometry {
                    side_margin: 8.0,
                    vertical_margin: 4.0,
                },
                entries: Vec::new(),
                tabs: vec![workspace_tab("t1", &["p1"], "p1")],
                codex_quota,
                claude_quota: None,
                sidebar: eon_workspace_protocol::v26::Sidebar::Open,
                attention_queue: Vec::new(),
            };
            let metrics = CellMetrics::for_scale(1.0);
            let project = |snapshot: &WorkspaceSnapshot, size| {
                WorkspaceScene::from_snapshot(snapshot, size, metrics, 0.0, 0.0, |_, text| {
                    (text.to_owned(), text.chars().count() as f32 * 10.0)
                })
            };
            let mut without_quota = workspace_snapshot.clone();
            without_quota.codex_quota = None;
            let mut sorted_windows = workspace_snapshot
                .codex_quota
                .as_ref()
                .map_or_else(Vec::new, |quota| quota.windows.clone());
            sorted_windows.sort_by_key(|window| window.duration_minutes);
            if sorted_windows.len() == 2
                && sorted_windows[0].duration_minutes != sorted_windows[1].duration_minutes
            {
                let size = PhysicalSize::new(800, 600);
                let mut accessible = Snapshot::new(size);
                let mut snapshot = workspace_snapshot.clone();
                let observed = [
                    sorted_windows.clone(),
                    vec![sorted_windows[1].clone()],
                    sorted_windows,
                ]
                .map(|windows| {
                    snapshot.codex_quota.as_mut().unwrap().windows = windows;
                    let workspace = project(&snapshot, size);
                    accessible.set_workspace(Some(&workspace));
                    let update = accessible.tree();
                    [0, 1].map(|index| {
                        let label = format!("Codex {}", expected[index].0);
                        update.nodes.iter().find_map(|(id, node)| {
                            (node.value() == Some(label.as_str())).then_some(*id)
                        })
                    })
                });
                assert!(observed[0].iter().all(Option::is_some));
                assert_eq!(observed[1][0], None, "{name}: the shorter window expired");
                assert_eq!(
                    observed[0][1], observed[1][1],
                    "{name}: the longer window keeps its identity after expiry"
                );
                assert_eq!(
                    observed[0], observed[2],
                    "{name}: identities survive reappearance"
                );
            }
            let full = project(&workspace_snapshot, PhysicalSize::new(800, 600));
            let ids = full.quotas.iter().map(quota_id).collect::<Vec<_>>();
            assert!(
                ids.windows(2).all(|pair| pair[0] != pair[1]),
                "{name}: distinct chip IDs"
            );
            for (quota, id) in full.quotas.iter().zip(&ids) {
                if quota.window.is_some() {
                    assert!((CODEX_QUOTA_NODE_START..LINK_NODE_START).contains(&id.0));
                } else {
                    assert_eq!(*id, QUOTA);
                }
            }
            let mut seen = [false; 3];
            let mut previous = expected.len();
            let mut accessible = Snapshot::new(PhysicalSize::new(800, 600));
            for width in (100..=800).rev().step_by(10) {
                let size = PhysicalSize::new(width, 600);
                let workspace = project(&workspace_snapshot, size);
                let baseline = project(&without_quota, size);
                let count = workspace.quotas.len();
                if width == 800 {
                    assert_eq!(count, expected.len(), "{name}: both windows fit");
                }
                assert!(count <= previous, "{name}: narrowing cannot restore a chip");
                previous = count;
                seen[count] = true;
                // Only the suffix survives: the chip closest to the tabs falls first.
                let shown = &expected[expected.len() - count..];
                let shown_ids = &ids[expected.len() - count..expected.len()];
                assert_eq!(workspace.terminal, baseline.terminal, "{name}");
                accessible.set_workspace(Some(&workspace));
                let update = accessible.tree();
                let mut children = vec![TAB_LIST, NEW_TAB];
                children.extend_from_slice(shown_ids);
                children.extend([SHORTCUTS, CLOSE_TAB, PANE_PANEL]);
                assert_eq!(node(&update, WINDOW).children(), children, "{name}/{width}");
                for id in &ids {
                    assert_eq!(
                        update
                            .nodes
                            .iter()
                            .filter(|(node_id, _)| node_id == id)
                            .count(),
                        usize::from(shown_ids.contains(id)),
                        "{name}/{width}: each visible chip has its own stable node"
                    );
                }
                for (index, (quota, (label, detail, other))) in
                    workspace.quotas.iter().zip(shown).enumerate()
                {
                    assert_eq!(quota.label, *label, "{name}/{width}");
                    assert_eq!(quota.accessible_label, format!("Codex {label}"), "{name}");
                    assert!(quota.description.starts_with("Codex quota "), "{name}");
                    assert!(quota.description.contains(detail), "{name}");
                    assert!(!quota.description.contains(other), "{name}");
                    assert!(!quota.description.contains("Unix time"), "{name}");
                    if name.contains("stale") {
                        assert!(quota.description.contains("state: stale"));
                        assert!(quota.pace.is_none());
                    }
                    assert_eq!(quota.logo.width, metrics.font_size, "{name}");
                    assert_eq!(quota.logo.height, metrics.font_size, "{name}");
                    assert!(
                        quota.rect.contains(quota.logo.left, quota.logo.top),
                        "{name}"
                    );
                    assert!(
                        quota.logo.right() <= quota.rect.right()
                            && quota.logo.bottom() <= quota.rect.bottom(),
                        "{name}"
                    );
                    assert_eq!(
                        workspace.hit_test(
                            quota.rect.left + quota.rect.width / 2.0,
                            quota.rect.height / 2.0
                        ),
                        Some(crate::WorkspaceHit::Quota(index)),
                        "{name}"
                    );
                    let quota_node = node(&update, shown_ids[index]);
                    assert_eq!(quota_node.role(), Role::Label, "{name}");
                    assert_eq!(
                        quota_node.value(),
                        Some(quota.accessible_label.as_str()),
                        "{name}"
                    );
                    assert_eq!(
                        quota_node.description(),
                        Some(quota.description.as_str()),
                        "{name}"
                    );
                    assert_eq!(quota_node.bounds(), Some(rect(quota.rect)), "{name}");
                    assert!(!quota_node.supports_action(Action::Click), "{name}");
                    assert!(!quota_node.supports_action(Action::Focus), "{name}");
                }
                for pair in workspace.quotas.windows(2) {
                    assert_eq!(pair[0].rect.right(), pair[1].rect.left, "{name}");
                }
                if let Some(first) = workspace.quotas.first() {
                    assert_eq!(workspace.drag_region.right(), first.rect.left, "{name}");
                    assert!(
                        (workspace.quotas.last().unwrap().rect.right()
                            - workspace.controls[1].rect.left)
                            .abs()
                            < 0.01,
                        "{name}"
                    );
                } else {
                    assert_eq!(workspace.tab_viewport, baseline.tab_viewport, "{name}");
                    assert_eq!(workspace.drag_region, baseline.drag_region, "{name}");
                    assert_eq!(workspace.controls, baseline.controls, "{name}");
                }
            }
            assert!(
                seen[..=expected.len()].iter().all(|seen| *seen),
                "{name}: each chip falls separately"
            );
        }
    }

    #[test]
    fn two_provider_chips_share_layout_without_losing_identity_or_freshness() {
        use eon_workspace_protocol::v26::{ClaudeQuota, ClaudeQuotaState};
        let five_hour = CodexQuotaWindow {
            duration_minutes: 300,
            remaining_percent: 98,
            resets_at: Some(4000),
        };
        let seven_day = CodexQuotaWindow {
            duration_minutes: 10080,
            remaining_percent: 100,
            resets_at: Some(100000),
        };
        // EONW may carry Claude's windows in either order, or only one of them.
        let claude_windows = [vec![seven_day.clone(), five_hour], vec![seven_day]];
        for state in [ClaudeQuotaState::Fresh, ClaudeQuotaState::Stale] {
            // Only fresh windows are judged for pace.
            let (suffix, freshness, pace) = match state {
                ClaudeQuotaState::Fresh => ("", "fresh", " At this pace it lasts until reset."),
                ClaudeQuotaState::Stale => (" old", "stale", ""),
            };
            for windows in &claude_windows {
                for providers in 0..4 {
                    for scale in [1.0, 1.25, 2.0] {
                        let metrics = CellMetrics::for_scale(scale);
                        let snapshot = WorkspaceSnapshot {
                            workspace: "w1".into(),
                            active_workspace: "w1".into(),
                            workspaces: vec![eon_workspace_protocol::v26::WorkspaceInfo {
                                id: "w1".into(),
                                name: None,
                                tab_count: 1,
                                attention: None,
                                recency: 0,
                            }],
                            active_tab: "t1".into(),
                            geometry: PopupGeometry {
                                side_margin: 8.0,
                                vertical_margin: 4.0,
                            },
                            entries: vec![],
                            tabs: vec![workspace_tab("t1", &["p1"], "p1")],
                            codex_quota: (providers & 1 != 0).then_some(CodexQuota {
                                state: CodexQuotaState::Fresh,
                                observed_at: 1000,
                                windows: vec![
                                    CodexQuotaWindow {
                                        duration_minutes: 300,
                                        remaining_percent: 74,
                                        resets_at: Some(2000),
                                    },
                                    CodexQuotaWindow {
                                        duration_minutes: 10080,
                                        remaining_percent: 42,
                                        resets_at: Some(20000),
                                    },
                                ],
                            }),
                            claude_quota: (providers & 2 != 0).then(|| ClaudeQuota {
                                state,
                                observed_at: 1000,
                                windows: windows.clone(),
                            }),
                            sidebar: eon_workspace_protocol::v26::Sidebar::Closed,
                            attention_queue: Vec::new(),
                        };
                        let project = |snapshot: &WorkspaceSnapshot, width| {
                            WorkspaceScene::from_snapshot(
                                snapshot,
                                PhysicalSize::new(width, 600),
                                metrics,
                                0.0,
                                0.0,
                                |_, text| {
                                    (
                                        text.to_owned(),
                                        text.chars().count() as f32 * 10.0 * scale as f32,
                                    )
                                },
                            )
                        };
                        let identities = |workspace: &WorkspaceScene| {
                            workspace
                                .quotas
                                .iter()
                                .map(|quota| {
                                    (
                                        quota.provider,
                                        quota.window.map(|window| match window {
                                            QuotaWindowIdentity::Codex { duration, .. }
                                            | QuotaWindowIdentity::Claude(duration) => duration,
                                        }),
                                    )
                                })
                                .collect::<Vec<_>>()
                        };
                        // Both providers have one chip per window, five-hour first.
                        let mut expected = Vec::new();
                        if providers & 1 != 0 {
                            expected.extend([
                                (QuotaProvider::Codex, Some(300)),
                                (QuotaProvider::Codex, Some(10080)),
                            ]);
                        }
                        if providers & 2 != 0 {
                            for minutes in [300, 10080] {
                                if windows
                                    .iter()
                                    .any(|window| window.duration_minutes == minutes)
                                {
                                    expected.push((QuotaProvider::Claude, Some(minutes)));
                                }
                            }
                        }
                        assert_eq!(identities(&project(&snapshot, 3600)), expected);
                        for width in [3600, 1800, 800, 700, 450, 180, 100] {
                            let size = PhysicalSize::new(width, 600);
                            let workspace = project(&snapshot, width);
                            let mut absent = snapshot.clone();
                            absent.codex_quota = None;
                            absent.claude_quota = None;
                            assert_eq!(
                                workspace.terminal,
                                project(&absent, width).terminal,
                                "quota does not resize the terminal"
                            );
                            // Chips fall one by one, the one closest to the tabs first.
                            let shown = identities(&workspace);
                            assert_eq!(shown, expected[expected.len() - shown.len()..]);
                            if width <= 180 || providers == 0 {
                                assert!(shown.is_empty());
                            }
                            let mut accessible = Snapshot::new(size);
                            accessible.set_workspace(Some(&workspace));
                            let update = accessible.tree();
                            let mut children = vec![TAB_LIST, NEW_TAB];
                            for (index, quota) in workspace.quotas.iter().enumerate() {
                                let id = quota_id(quota);
                                children.push(id);
                                let node = node(&update, id);
                                assert_eq!(node.value(), Some(quota.accessible_label.as_str()));
                                assert_eq!(node.bounds(), Some(rect(quota.rect)));
                                assert!(!node.supports_action(Action::Click));
                                assert!(!node.supports_action(Action::Focus));
                                assert_eq!(
                                    workspace.hit_test(
                                        quota.rect.left + quota.rect.width / 2.0,
                                        quota.rect.height / 2.0
                                    ),
                                    Some(crate::WorkspaceHit::Quota(index))
                                );
                                assert!(quota.rect.left >= workspace.drag_region.right());
                                assert!(
                                    quota.rect.right() <= workspace.controls[1].rect.left + 0.01
                                );
                                if let Some(next) = workspace.quotas.get(index + 1) {
                                    assert!(quota.rect.right() <= next.rect.left + 0.01);
                                }
                                if quota.provider != QuotaProvider::Claude {
                                    continue;
                                }
                                // A Claude chip shows and describes only its own window.
                                let (label, detail, other) =
                                    if quota.window == Some(QuotaWindowIdentity::Claude(300)) {
                                        (
                                            "4h10m/5h 98%",
                                            "5h window: 98% remaining; observed 4h10m into 5h.",
                                            "7d",
                                        )
                                    } else {
                                        (
                                            "5d20h/7d 100%",
                                            "7d window: 100% remaining; observed 5d20h into 7d.",
                                            "5h",
                                        )
                                    };
                                assert_eq!(quota.label, format!("{label}{suffix}"));
                                assert_eq!(
                                    quota.accessible_label,
                                    format!("Claude {label}{suffix}")
                                );
                                assert_eq!(
                                    quota.description,
                                    format!("Claude quota state: {freshness}. {detail}{pace}")
                                );
                                assert!(!quota.description.contains(other));
                            }
                            children.extend([SHORTCUTS, CLOSE_TAB, PANE_PANEL]);
                            assert_eq!(node(&update, WINDOW).children(), children);
                            assert!(workspace.tab_viewport.width > 0.0);
                            assert!(
                                workspace
                                    .controls
                                    .iter()
                                    .all(|control| control.rect.right() <= width as f32 + 0.01)
                            );
                            // Chips fall in order: Codex 5h, Codex 7d, Claude 5h, Claude 7d.
                            if providers == 3
                                && windows.len() == 2
                                && scale == 1.0
                                && state == ClaudeQuotaState::Fresh
                            {
                                let labels = workspace
                                    .quotas
                                    .iter()
                                    .map(|quota| quota.label.as_str())
                                    .collect::<Vec<_>>();
                                match width {
                                    1800 => assert_eq!(
                                        labels,
                                        [
                                            "4h43m/5h 74%",
                                            "6d18h/7d 42%",
                                            "4h10m/5h 98%",
                                            "5d20h/7d 100%"
                                        ]
                                    ),
                                    800 => assert_eq!(
                                        labels,
                                        ["6d18h/7d 42%", "4h10m/5h 98%", "5d20h/7d 100%"]
                                    ),
                                    700 => assert_eq!(labels, ["4h10m/5h 98%", "5d20h/7d 100%"]),
                                    450 => assert_eq!(labels, ["5d20h/7d 100%"]),
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    fn linked_scene(revision: u64, uri: &str) -> Scene {
        Scene {
            revision,
            columns: 1,
            rows: 1,
            screen: Screen::Primary,
            title: "shell".into(),
            working_directory: String::new(),
            graphics: SceneGraphics::default(),
            background: Color::default(),
            foreground: Color::default(),
            cursor: None,
            content: vec![DrawRow {
                wrapped: false,
                wrap_continuation: false,
                cells: vec![DrawCell {
                    width: CellWidth::Narrow,
                    text: "x".into(),
                    hyperlink: uri.into(),
                    style: DrawStyle {
                        foreground: Color::default(),
                        background: Color::default(),
                        underline_color: Color::default(),
                        bold: false,
                        italic: false,
                        faint: false,
                        blink: false,
                        invisible: false,
                        strikethrough: false,
                        overline: false,
                        selected: false,
                        background_is_default: true,
                        protected: false,
                        underline: Underline::None,
                    },
                }],
            }],
        }
    }

    #[test]
    fn attention_inbox_names_filters_rows_and_highlight() {
        use crate::AttentionFilter;
        use eon_workspace_protocol::v26::{
            Attention, AttentionRecord, AttentionState, QueueEntry, Sidebar, WorkspaceInfo,
        };
        let size = PhysicalSize::new(1400, 700);
        let mut workspace = WorkspaceSnapshot {
            workspace: "w1".into(),
            active_workspace: "w1".into(),
            active_tab: "t3".into(),
            tabs: vec![
                workspace_tab("t3", &["p3"], "p3"),
                workspace_tab("t1", &["p1"], "p1"),
            ],
            workspaces: vec![
                WorkspaceInfo {
                    id: "w1".into(),
                    name: None,
                    tab_count: 2,
                    attention: None,
                    recency: 1,
                },
                WorkspaceInfo {
                    id: "w2".into(),
                    name: Some("Review".into()),
                    tab_count: 1,
                    attention: None,
                    recency: 0,
                },
            ],
            geometry: PopupGeometry {
                side_margin: 8.0,
                vertical_margin: 4.0,
            },
            entries: Vec::new(),
            codex_quota: None,
            claude_quota: None,
            sidebar: Sidebar::Open,
            attention_queue: vec![QueueEntry {
                workspace: "w2".into(),
                tab: "t2".into(),
                target: "p2".into(),
                attention: Attention {
                    state: AttentionState::NeedsInput,
                    record: Some(AttentionRecord {
                        kind: None,
                        progress: None,
                        app: "codex".into(),
                        name: String::new(),
                        title: "Check API".into(),
                        message: "Approve?".into(),
                        changed_at: 0,
                        stale: false,
                        others: 0,
                    }),
                },
            }],
        };
        for (tab, target, state) in [
            ("t1", "p1", AttentionState::Error),
            ("t3", "p3", AttentionState::Finished),
        ] {
            let mut entry = workspace.attention_queue[0].clone();
            entry.workspace = "w1".into();
            entry.tab = tab.into();
            entry.target = target.into();
            entry.attention.state = state;
            entry.attention.record.as_mut().unwrap().stale = true;
            let source = workspace
                .tabs
                .iter_mut()
                .find(|item| item.id == tab)
                .unwrap();
            source.columns[0].panes[0].attention = Some(entry.attention.clone());
            source.attention_rollup = Some(state);
            workspace.attention_queue.push(entry);
        }
        let viewer = DialogScene::attention_inbox(
            &workspace,
            AttentionFilter::All,
            Some(("t2", "p2")),
            size,
            CellMetrics::for_scale(1.0),
            0.0,
            0,
        );
        let mut snapshot = Snapshot::new(size);
        snapshot.set_dialog(Some(&viewer));
        let update = snapshot.tree();
        assert_eq!(node(&update, DIALOG).label(), Some("Attention inbox"));
        assert_eq!(node(&update, DIALOG).role(), Role::Dialog);
        for (index, name) in ["All, 3", "Needs input, 1", "Error, 1", "Finished, 1"]
            .iter()
            .enumerate()
        {
            let id = tree_node_id(&update, name);
            assert_eq!(node(&update, id).role(), Role::Button);
            assert_eq!(node(&update, id).is_selected(), Some(index == 0));
        }
        let row = &viewer.attention_rows[0];
        let id = tree_node_id(&update, &row.accessible_label);
        assert!(row.accessible_label.contains(", in workspace Review"));
        assert_eq!(update.focus, id);
        assert_eq!(snapshot.action_target(CONTENT, Action::Focus), None);
        assert_eq!(
            viewer
                .attention_rows
                .iter()
                .map(|row| row.target.as_str())
                .collect::<Vec<_>>(),
            ["p2", "p1", "p3"]
        );
        assert_eq!(row.detail, "now · Review");
        assert_eq!(row.message.as_deref(), Some("Approve?"));
        assert_eq!(row.accessible_label.matches("workspace Review").count(), 1);
        let sidebar = WorkspaceScene::from_snapshot(
            &workspace,
            size,
            CellMetrics::for_scale(1.0),
            0.0,
            0.0,
            |_, text| (text.to_owned(), text.len() as f32 * 10.0),
        );
        let local = &viewer.attention_rows[1];
        assert!(local.detail.contains("2  t1 · p1"));
        assert!(local.glyph.dimmed);
        assert_eq!(local.message, None);
        assert_eq!(
            local.accessible_label,
            sidebar.sidebar_row("p1").unwrap().accessible_label
        );
        assert_eq!(
            snapshot.action_target(id, Action::Click),
            Some(AccessibilityTarget::InboxRow {
                tab: "t2".into(),
                target: "p2".into(),
                activate: true
            })
        );
        let filter = tree_node_id(&update, "Error, 1");
        assert_eq!(
            snapshot.action_target(filter, Action::Click),
            Some(AccessibilityTarget::InboxFilter(AttentionFilter::Error))
        );
        let empty = DialogScene::attention_inbox(
            &workspace,
            AttentionFilter::All,
            None,
            size,
            CellMetrics::for_scale(1.0),
            0.0,
            0,
        );
        snapshot.set_dialog(Some(&empty));
        assert_eq!(
            snapshot.tree().focus,
            tree_node_id(&snapshot.tree(), "All, 3")
        );
        let original = workspace.attention_queue[0].clone();
        workspace.attention_queue = (0..256)
            .map(|index| {
                let mut entry = original.clone();
                entry.target = format!("p{}", index + 4);
                entry
            })
            .collect();
        let short = PhysicalSize::new(1000, 200);
        let first = DialogScene::attention_inbox(
            &workspace,
            AttentionFilter::All,
            Some(("t2", "p4")),
            short,
            CellMetrics::for_scale(1.0),
            0.0,
            0,
        );
        assert!(first.attention_rows[0].rect.height > first.content.height);
        for scroll in [
            first.content.height,
            CellMetrics::for_scale(1.0).height * 3.0,
        ] {
            let paged = DialogScene::attention_inbox(
                &workspace,
                AttentionFilter::All,
                Some(("t2", "p4")),
                short,
                CellMetrics::for_scale(1.0),
                scroll,
                0,
            );
            assert_eq!(
                paged.attention_highlight_scroll(),
                scroll,
                "a partly visible oversized row must not reset wheel or page scrolling"
            );
        }
        for scale in [1.0, 1.3, 2.0] {
            for size in [PhysicalSize::new(1, 1), PhysicalSize::new(240, 160), size] {
                let metrics = CellMetrics::for_scale(scale);
                let dialog = DialogScene::attention_inbox(
                    &workspace,
                    AttentionFilter::All,
                    Some(("t2", "p259")),
                    size,
                    metrics,
                    0.0,
                    0,
                );
                if dialog.content.height > dialog.attention_rows[0].rect.height {
                    let target = dialog.attention_scrolled_target().unwrap();
                    assert_ne!(
                        target.1, "p259",
                        "paging keeps the highlight on visible rows"
                    );
                }
                let scroll = dialog.attention_highlight_scroll();
                let dialog = DialogScene::attention_inbox(
                    &workspace,
                    AttentionFilter::All,
                    Some(("t2", "p259")),
                    size,
                    metrics,
                    scroll,
                    0,
                );
                assert!(dialog.max_scroll.is_finite());
                assert!(
                    dialog.bounds.right() <= size.width as f32
                        && dialog.bounds.bottom() <= size.height as f32
                );
                assert!(
                    dialog.attention_rows.last().unwrap().rect.top >= dialog.content.top - 0.01
                );
                assert!(
                    dialog.attention_rows.last().unwrap().rect.bottom()
                        <= dialog.content.bottom() + 0.01
                        || dialog.content.height
                            < dialog.attention_rows.last().unwrap().rect.height
                );
            }
        }
        workspace.attention_queue.clear();
        for filter in AttentionFilter::ALL {
            let dialog = DialogScene::attention_inbox(
                &workspace,
                filter,
                None,
                size,
                CellMetrics::for_scale(1.0),
                0.0,
                0,
            );
            snapshot.set_dialog(Some(&dialog));
            let tree = snapshot.tree();
            let selected = tree_node_id(&tree, &format!("{}, 0", filter.label()));
            assert_eq!(tree.focus, selected);
            assert_eq!(node(&tree, selected).is_selected(), Some(true));
            assert_eq!(
                dialog.empty,
                Some(match filter {
                    AttentionFilter::All => "No agent needs attention",
                    AttentionFilter::NeedsInput => "No agent in needs input",
                    AttentionFilter::Error => "No agent in error",
                    AttentionFilter::Finished => "No agent in finished",
                })
            );
        }
    }

    #[test]
    fn shortcut_viewer_replaces_terminal_tree_with_a_focused_dialog() {
        let size = PhysicalSize::new(320, 180);
        let viewer = DialogScene::new(
            DialogKind::Shortcuts,
            vec![ShortcutGroup::new(
                "Host",
                vec![ShortcutRow::new("Alt+/", "Show or close shortcuts")],
            )],
            size,
            CellMetrics::for_scale(1.0),
            0.0,
        );
        let mut snapshot = Snapshot::new(size);
        snapshot.dialog = Some(viewer);
        let update = snapshot.tree();

        assert_eq!(node(&update, WINDOW).children(), &[DIALOG]);
        assert_eq!(node(&update, DIALOG).role(), Role::Dialog);
        assert!(node(&update, DIALOG).is_modal());
        assert_eq!(update.focus, DIALOG);
        tree_node_id(&update, "Host");
        assert!(
            update
                .nodes
                .iter()
                .any(|(_, node)| { node.value() == Some("Alt+/ — Show or close shortcuts") })
        );
        assert_eq!(snapshot.action_target(CONTENT, Action::Focus), None);

        // The final-close prompt is an alert whose rows name what stops.
        snapshot.dialog = Some(DialogScene::new(
            DialogKind::CloseLastTab {
                terminals: 2,
                detach: Some("Alt+Shift+D".into()),
                workspace: None,
            },
            vec![ShortcutGroup::new(
                "",
                vec![
                    ShortcutRow::new("Pane", "p1"),
                    ShortcutRow::new("Popup", "Agent"),
                ],
            )],
            size,
            CellMetrics::for_scale(1.0),
            0.0,
        ));
        let update = snapshot.tree();
        let prompt = node(&update, DIALOG);
        assert_eq!(prompt.role(), Role::AlertDialog);
        assert!(prompt.is_modal());
        assert_eq!(prompt.label(), Some("Close the last tab?"));
        assert_eq!(
            prompt.description(),
            Some(
                "This quits the window and stops 2 terminals. Press Enter to quit, \
                 Escape to cancel, or Alt+Shift+D to keep it running instead."
            )
        );
        assert_eq!(update.focus, DIALOG);
        let rows = prompt
            .children()
            .iter()
            .map(|id| node(&update, *id).value())
            .collect::<Vec<_>>();
        assert_eq!(rows, [Some("Pane p1"), Some("Popup Agent")]);
        // While other workspaces remain, Enter closes the shown one instead.
        snapshot.dialog = Some(DialogScene::new(
            DialogKind::CloseLastTab {
                terminals: 1,
                detach: None,
                workspace: Some("w2".into()),
            },
            vec![ShortcutGroup::new("", vec![ShortcutRow::new("Pane", "p1")])],
            size,
            CellMetrics::for_scale(1.0),
            0.0,
        ));
        assert_eq!(
            node(&snapshot.tree(), DIALOG).description(),
            Some(
                "This closes workspace w2 and stops 1 terminal. Press Enter to close it \
                 or Escape to cancel."
            )
        );
    }

    #[test]
    fn visible_links_offer_only_open_and_copy_and_retire_with_the_scene() {
        let mut snapshot = Snapshot::new(PhysicalSize::new(800, 600));
        let first_scene = linked_scene(1, "https://example.com/one");
        snapshot.content = first_scene.accessible_content();
        snapshot.columns = Some(first_scene.columns);
        snapshot.status.clear();
        snapshot.set_links(Some(&first_scene), Some(7));

        let first = snapshot.tree();
        let first_link = tree_node_id(&first, "https://example.com/one");
        let first_copy = tree_node_id(&first, "Copy https://example.com/one");
        assert_eq!(node(&first, first_link).role(), Role::Link);
        assert!(node(&first, first_link).supports_action(Action::Click));
        assert!(!node(&first, first_link).supports_action(Action::CustomAction));
        assert!(!node(&first, first_link).supports_action(Action::Focus));
        assert_eq!(node(&first, first_copy).role(), Role::Button);
        assert!(node(&first, first_copy).supports_action(Action::Click));
        assert!(!node(&first, first_copy).supports_action(Action::CustomAction));
        assert!(!node(&first, first_copy).supports_action(Action::Focus));
        assert_eq!(
            snapshot.action_target(first_link, Action::Click),
            Some(AccessibilityTarget::Link {
                row: 0,
                column: 0,
                copy: false,
            })
        );
        assert_eq!(
            snapshot.action_target(first_copy, Action::Click),
            Some(AccessibilityTarget::Link {
                row: 0,
                column: 0,
                copy: true,
            })
        );
        assert_eq!(snapshot.action_target(first_copy, Action::Focus), None);

        snapshot.set_links(Some(&first_scene), None);
        assert_eq!(snapshot.action_target(first_link, Action::Click), None);
        assert_eq!(snapshot.action_target(first_copy, Action::Click), None);

        let second_scene = linked_scene(1, "https://example.com/two");
        snapshot.content = second_scene.accessible_content();
        snapshot.columns = Some(second_scene.columns);
        snapshot.set_links(Some(&second_scene), Some(8));
        let second = snapshot.tree();
        let second_link = tree_node_id(&second, "https://example.com/two");
        assert_ne!(first_link, second_link);
        assert_eq!(snapshot.action_target(first_link, Action::Click), None);

        let oversized = "x".repeat(MAX_LINK_BYTES + 1);
        let oversized_scene = linked_scene(2, &oversized);
        snapshot.set_links(Some(&oversized_scene), Some(9));
        let oversized = snapshot.tree();
        let failure = format!("Link target exceeds {MAX_LINK_BYTES} bytes");
        tree_node_id(&oversized, &failure);
        tree_node_id(&oversized, &format!("Copy {failure}"));
    }

    #[test]
    fn scene_notice_preserves_terminal_content_and_adds_an_alert() {
        let update = Snapshot {
            title: "shell".into(),
            content: AccessibleText {
                rows: vec![AccessibleRow {
                    value: "terminal content".into(),
                    character_lengths: "terminal content"
                        .chars()
                        .map(|character| character.len_utf8() as u8)
                        .collect(),
                    column_spans: (0..16).map(|column| column..column + 1).collect(),
                }],
                selection: None,
            },
            status: "renderer failure".into(),
            columns: Some(16),
            ..Snapshot::new(PhysicalSize::new(800, 600))
        }
        .tree();
        assert_eq!(node(&update, CONTENT).role(), Role::Terminal);
        assert_eq!(
            node(&update, text_run_id(0)).value(),
            Some("terminal content")
        );
        assert_eq!(node(&update, STATUS).role(), Role::Alert);
        assert_eq!(node(&update, STATUS).label(), Some("renderer failure"));
        assert_eq!(
            node(&update, STATUS).live(),
            Some(accesskit::Live::Assertive)
        );
        assert_eq!(node(&update, WINDOW).children(), &[CONTENT, STATUS]);
    }

    #[test]
    fn authoritative_selection_uses_accesskit_text_positions() {
        let update = Snapshot {
            title: "shell".into(),
            content: AccessibleText {
                rows: vec![
                    AccessibleRow {
                        value: "one\n".into(),
                        character_lengths: vec![1, 1, 1, 1],
                        column_spans: vec![0..1, 1..2, 2..3, 3..3],
                    },
                    AccessibleRow {
                        value: "界".into(),
                        character_lengths: vec![3],
                        column_spans: std::iter::once(0..2).collect(),
                    },
                ],
                selection: Some(AccessibleSelection {
                    anchor: AccessiblePosition {
                        row: 0,
                        character_index: 1,
                    },
                    focus: AccessiblePosition {
                        row: 1,
                        character_index: 1,
                    },
                }),
            },
            status: String::new(),
            columns: Some(4),
            ..Snapshot::new(PhysicalSize::new(800, 600))
        }
        .tree();
        assert_eq!(
            node(&update, CONTENT).children(),
            &[text_run_id(0), text_run_id(1)]
        );
        assert_eq!(node(&update, text_run_id(0)).role(), Role::TextRun);
        assert_eq!(node(&update, text_run_id(0)).value(), Some("one\n"));
        assert_eq!(
            node(&update, CONTENT).text_selection(),
            Some(&TextSelection {
                anchor: TextPosition {
                    node: text_run_id(0),
                    character_index: 1,
                },
                focus: TextPosition {
                    node: text_run_id(1),
                    character_index: 1,
                },
            })
        );
    }

    #[test]
    fn text_geometry_preserves_atomic_cell_spans() {
        let mut scene = linked_scene(1, "");
        let cell = scene.content[0].cells[0].clone();
        let oversized = format!("e{}", "\u{301}".repeat(128));
        scene.columns = 7;
        scene.rows = 2;
        scene.content[0].cells = [
            ("A", CellWidth::Narrow, false),
            ("e\u{301}", CellWidth::Narrow, true),
            ("界", CellWidth::Wide, true),
            ("", CellWidth::SpacerTail, true),
            ("", CellWidth::Narrow, true),
            (oversized.as_str(), CellWidth::Wide, false),
            ("", CellWidth::SpacerTail, false),
        ]
        .into_iter()
        .map(|(text, width, selected)| {
            let mut cell = cell.clone();
            cell.text = text.into();
            cell.width = width;
            cell.style.selected = selected;
            cell
        })
        .collect();
        let mut blank = cell;
        blank.text.clear();
        scene.content.push(DrawRow {
            cells: vec![blank; 7],
            ..scene.content[0].clone()
        });
        let content = scene.accessible_content();

        for scale in [1.0, 1.25, 1.5, 2.0] {
            let metrics = CellMetrics::for_scale(scale);
            let update = Snapshot {
                content: content.clone(),
                columns: Some(scene.columns),
                metrics,
                ..Snapshot::new(PhysicalSize::new(800, 600))
            }
            .tree();
            let row = node(&update, text_run_id(0));
            assert_eq!(row.value(), Some("Ae\u{301}界 \u{fffd}\n"));
            assert_eq!(row.character_lengths(), [1, 3, 3, 1, 3, 1]);
            assert_eq!(
                row.character_positions(),
                Some(
                    [0.0, 1.0, 2.0, 4.0, 5.0, 7.0]
                        .map(|x| x * metrics.width)
                        .as_slice()
                ),
                "scale {scale}"
            );
            assert_eq!(
                row.character_widths(),
                Some(
                    [1.0, 1.0, 2.0, 1.0, 2.0, 0.0]
                        .map(|x| x * metrics.width)
                        .as_slice()
                ),
                "scale {scale}"
            );

            let row_bounds = SceneRect {
                left: metrics.padding,
                top: metrics.padding,
                width: 7.0 * metrics.width,
                height: metrics.height,
            };
            let clip = SceneRect {
                left: metrics.padding + 1.5 * metrics.width,
                top: metrics.padding + 0.25 * metrics.height,
                width: 4.5 * metrics.width,
                height: 0.5 * metrics.height,
            };
            let clipped = text_run(&content.rows[0], row_bounds, Some(clip), metrics.width);
            assert_eq!(clipped.bounds(), Some(rect(clip)));
            assert_eq!(
                clipped.character_positions(),
                Some(
                    [0.0, 0.0, 0.5, 2.5, 3.5, 4.5]
                        .map(|x| x * metrics.width)
                        .as_slice()
                )
            );
            assert_eq!(
                clipped.character_widths(),
                Some(
                    [0.0, 0.5, 2.0, 1.0, 1.0, 0.0]
                        .map(|x| x * metrics.width)
                        .as_slice()
                )
            );
            let hidden = text_run(&content.rows[0], row_bounds, None, metrics.width);
            assert_eq!(hidden.bounds(), None);
            assert_eq!(hidden.character_positions(), None);
            assert_eq!(hidden.value(), row.value());
            assert_eq!(hidden.character_lengths(), row.character_lengths());
        }
    }

    #[test]
    fn text_runs_follow_the_rendered_cell_grid() {
        let row = AccessibleRow {
            value: String::new(),
            character_lengths: Vec::new(),
            column_spans: Vec::new(),
        };
        let update = Snapshot {
            content: AccessibleText {
                rows: vec![row.clone(); 32],
                selection: None,
            },
            status: String::new(),
            columns: Some(93),
            ..Snapshot::new(PhysicalSize::new(960, 600))
        }
        .tree();
        assert!(node(&update, CONTENT).clips_children());
        assert_eq!(
            node(&update, text_run_id(0)).bounds(),
            Some(Rect {
                x0: 12.0,
                y0: 12.0,
                x1: 942.0,
                y1: 30.0,
            })
        );
        assert_eq!(
            node(&update, text_run_id(31)).bounds(),
            Some(Rect {
                x0: 12.0,
                y0: 570.0,
                x1: 942.0,
                y1: 588.0,
            })
        );

        let fonts = crate::FontSetup::new(&crate::FontSettings {
            family: Some("DejaVu Sans Mono".into()),
            size: 20.0,
            line_height: 1.5,
            ..Default::default()
        })
        .unwrap();
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let metrics = fonts.metrics(scale);
            let update = Snapshot {
                content: AccessibleText {
                    rows: vec![row.clone(); 2],
                    selection: None,
                },
                status: String::new(),
                metrics,
                columns: Some(3),
                ..Snapshot::new(PhysicalSize::new(400, 300))
            }
            .tree();
            let second_row = node(&update, text_run_id(1)).bounds();

            assert_eq!(
                second_row,
                Some(rect(SceneRect {
                    left: metrics.padding,
                    top: metrics.padding + metrics.height,
                    width: metrics.width * 3.0,
                    height: metrics.height,
                })),
                "scale {scale}"
            );
        }
    }

    #[test]
    fn workspace_accessibility_uses_visible_intersections() {
        let columns = 4;
        let workspace_snapshot = WorkspaceSnapshot {
            workspace: "w1".into(),
            active_workspace: "w1".into(),
            workspaces: vec![eon_workspace_protocol::v26::WorkspaceInfo {
                id: "w1".into(),
                name: None,
                tab_count: 1,
                attention: None,
                recency: 0,
            }],
            active_tab: "t1".into(),
            tabs: vec![
                workspace_tab(
                    "t1",
                    &["pane-1", "pane-2", "pane-3", "pane-4", "pane-5", "pane-6"],
                    "pane-1",
                ),
                workspace_tab("t2", &["pane-7"], "pane-7"),
                workspace_tab("t3", &["pane-8"], "pane-8"),
            ],
            geometry: eon_workspace_protocol::v26::PopupGeometry {
                side_margin: 8.0,
                vertical_margin: 4.0,
            },
            entries: Vec::new(),
            codex_quota: None,
            claude_quota: None,
            sidebar: eon_workspace_protocol::v26::Sidebar::Open,
            attention_queue: Vec::new(),
        };
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let metrics = CellMetrics::for_scale(scale);
            let size = PhysicalSize::new(100, (140.0 * scale) as u32);
            let workspace = WorkspaceScene::from_snapshot(
                &workspace_snapshot,
                size,
                metrics,
                20.0,
                20.0,
                |_, text| (text.to_owned(), text.chars().count() as f32 * 10.0),
            );
            let mut snapshot = Snapshot {
                content: AccessibleText {
                    rows: vec![
                        AccessibleRow {
                            value: "one\n".into(),
                            character_lengths: vec![1; 4],
                            column_spans: vec![0..1, 1..2, 2..3, 3..3],
                        },
                        AccessibleRow {
                            value: "two".into(),
                            character_lengths: vec![1; 3],
                            column_spans: vec![0..1, 1..2, 2..3],
                        },
                    ],
                    selection: None,
                },
                status: String::new(),
                metrics,
                columns: Some(columns),
                workspace_focus: WorkspaceFocus::Panes,
                ..Snapshot::new(size)
            };
            snapshot.set_workspace(Some(&workspace));
            let tab = snapshot.tab_ids["t1"];
            let pane = snapshot.pane_ids["pane-1"];
            let update = snapshot.tree();
            assert!(node(&update, TAB_LIST).clips_children());
            assert!(node(&update, PANE_PANEL).clips_children());
            assert!(node(&update, CONTENT).clips_children());
            assert_eq!(
                node(&update, tab).bounds(),
                workspace.tabs[0]
                    .rect
                    .intersection(workspace.tab_viewport)
                    .map(rect)
            );
            assert_eq!(
                node(&update, pane).bounds(),
                workspace.panes[0]
                    .rect
                    .intersection(workspace.pane_viewport)
                    .map(rect)
            );
            for pane in &workspace.panes {
                let button = node(&update, snapshot.pane_ids[&pane.id]);
                assert_eq!(
                    button.bounds(),
                    pane.rect.intersection(workspace.pane_viewport).map(rect)
                );
                assert_eq!(button.role(), Role::Button);
                assert!(button.supports_action(Action::Click));
                assert!(button.supports_action(Action::Focus));
            }
            assert_eq!(
                node(&update, CONTENT).bounds(),
                workspace.visible_terminal().map(rect)
            );
            let second_row = SceneRect {
                left: workspace.terminal.left + metrics.padding,
                top: workspace.terminal.top + metrics.padding + metrics.height,
                width: f32::from(columns) * metrics.width,
                height: metrics.height,
            }
            .intersection(workspace.visible_terminal().unwrap())
            .unwrap();
            assert_eq!(
                node(&update, text_run_id(1)).bounds(),
                Some(rect(second_row))
            );

            let fully_clipped = WorkspaceScene::from_snapshot(
                &workspace_snapshot,
                size,
                metrics,
                workspace.tab_scroll_limit(),
                workspace.pane_scroll_limit(),
                |_, text| (text.to_owned(), text.chars().count() as f32 * 10.0),
            );
            assert_eq!(fully_clipped.visible_terminal(), None);
            snapshot.set_workspace(Some(&fully_clipped));
            let update = snapshot.tree();
            assert_eq!(node(&update, tab).bounds(), None);
            assert_eq!(node(&update, pane).bounds(), None);
            assert_eq!(node(&update, CONTENT).bounds(), None);
            assert_eq!(node(&update, text_run_id(0)).bounds(), None);
            assert!(node(&update, tab).supports_action(Action::Click));
            assert!(node(&update, tab).supports_action(Action::Focus));
            assert!(node(&update, pane).supports_action(Action::Click));
            assert!(node(&update, pane).supports_action(Action::Focus));
            assert_eq!(node(&update, text_run_id(0)).value(), Some("one\n"));
            assert_eq!(node(&update, PANE_PANEL).children().first(), Some(&pane));
            assert_eq!(update.focus, pane);
        }
    }

    #[test]
    fn agent_sidebar_region_follows_the_header_and_names_its_empty_state() {
        use eon_workspace_protocol::v26::{AttentionState, Sidebar};

        let mut workspace = WorkspaceSnapshot {
            workspace: "w1".into(),
            active_workspace: "w1".into(),
            workspaces: vec![eon_workspace_protocol::v26::WorkspaceInfo {
                id: "w1".into(),
                name: None,
                tab_count: 1,
                attention: None,
                recency: 0,
            }],
            active_tab: "t1".into(),
            tabs: vec![workspace_tab("t1", &["p1"], "p1")],
            geometry: PopupGeometry {
                side_margin: 8.0,
                vertical_margin: 4.0,
            },
            entries: Vec::new(),
            codex_quota: None,
            claude_quota: None,
            sidebar: Sidebar::Open,
            attention_queue: Vec::new(),
        };
        let size = PhysicalSize::new(1400, 600);
        let tree = |workspace: &WorkspaceSnapshot| {
            let scene = WorkspaceScene::from_snapshot(
                workspace,
                size,
                CellMetrics::for_scale(1.0),
                0.0,
                0.0,
                |_, text| (text.to_owned(), text.len() as f32 * 10.0),
            );
            let mut snapshot = Snapshot::new(size);
            snapshot.set_workspace(Some(&scene));
            (scene, snapshot.tree())
        };

        let (scene, update) = tree(&workspace);
        assert_eq!(
            node(&update, WINDOW).children(),
            [TAB_LIST, NEW_TAB, SHORTCUTS, CLOSE_TAB, SIDEBAR, PANE_PANEL]
        );
        let sidebar = node(&update, SIDEBAR);
        assert_eq!(sidebar.role(), Role::Complementary);
        assert_eq!(sidebar.label(), Some("Agent sidebar"));
        assert_eq!(sidebar.bounds(), Some(rect(scene.sidebar().unwrap())));
        assert_eq!(sidebar.children(), [SIDEBAR_EMPTY_STATE]);
        assert_eq!(
            node(&update, SIDEBAR_EMPTY_STATE).value(),
            Some(SIDEBAR_EMPTY)
        );
        assert!(!sidebar.supports_action(Action::Click) && !sidebar.supports_action(Action::Focus));

        // Attention replaces the empty state with each tab's heading and its rows, which are
        // named buttons that take focus and jump.
        workspace.tabs[0].columns[0].panes[0].attention =
            Some(eon_workspace_protocol::v26::Attention {
                state: AttentionState::NeedsInput,
                record: Some(eon_workspace_protocol::v26::AttentionRecord {
                    kind: None,
                    progress: None,
                    app: "claude-code".into(),
                    name: String::new(),
                    title: "Review".into(),
                    message: "Allow edits?".into(),
                    changed_at: 0,
                    stale: false,
                    others: 0,
                }),
            });
        workspace.tabs[0].attention_rollup = Some(AttentionState::NeedsInput);
        let scene = WorkspaceScene::from_snapshot_with_zoom(
            &workspace,
            EonBar::default(),
            size,
            CellMetrics::for_scale(1.0),
            0.0,
            0.0,
            |_, text| (text.to_owned(), text.len() as f32 * 10.0),
            None,
            false,
            crate::SidebarView {
                now: 0,
                focused: Some("p1"),
            },
        );
        let mut snapshot = Snapshot::new(size);
        snapshot.set_workspace(Some(&scene));
        let update = snapshot.tree();
        let (heading, row) = (snapshot.sidebar_ids["t1"], snapshot.sidebar_ids["p1"]);
        assert_eq!(node(&update, SIDEBAR).children(), [heading, row]);
        assert!(
            update
                .nodes
                .iter()
                .all(|(id, _)| *id != SIDEBAR_EMPTY_STATE)
        );
        assert_eq!(node(&update, heading).role(), Role::Heading);
        assert_eq!(node(&update, heading).label(), Some("1  t1"));
        let button = node(&update, row);
        assert_eq!(button.role(), Role::Button);
        assert_eq!(
            button.label(),
            Some(
                "Needs input: Claude in tab 1, t1, p1, current, Review, changed just now: \
                 Allow edits?"
            )
        );
        assert!(button.supports_action(Action::Click) && button.supports_action(Action::Focus));
        for (action, activate) in [(Action::Click, true), (Action::Focus, false)] {
            assert_eq!(
                snapshot.action_target(row, action),
                Some(AccessibilityTarget::SidebarRow {
                    target: "p1".into(),
                    activate,
                })
            );
        }
        assert_eq!(snapshot.action_target(heading, Action::Click), None);
        // The focused row is the tree's focus while the sidebar holds keyboard focus.
        assert_eq!(update.focus, CONTENT);
        snapshot.workspace_focus = WorkspaceFocus::Sidebar;
        assert_eq!(snapshot.tree().focus, row);

        // Closed or hidden for width, the region leaves the tree.
        for (sidebar, width) in [(Sidebar::Closed, 1400), (Sidebar::Open, 1000)] {
            workspace.sidebar = sidebar;
            let scene = WorkspaceScene::from_snapshot(
                &workspace,
                PhysicalSize::new(width, 600),
                CellMetrics::for_scale(1.0),
                0.0,
                0.0,
                |_, text| (text.to_owned(), text.len() as f32 * 10.0),
            );
            let mut snapshot = Snapshot::new(PhysicalSize::new(width, 600));
            snapshot.set_workspace(Some(&scene));
            let update = snapshot.tree();
            assert!(!node(&update, WINDOW).children().contains(&SIDEBAR));
            assert!(update.nodes.iter().all(|(id, _)| *id != SIDEBAR));
        }
    }

    #[test]
    fn shortcut_hints_are_read_only_and_retire_hidden_help_actions() {
        let size = PhysicalSize::new(800, 600);
        let metrics = CellMetrics::for_scale(1.0);
        let workspace = WorkspaceSnapshot {
            workspace: "w1".into(),
            active_workspace: "w1".into(),
            workspaces: vec![eon_workspace_protocol::v26::WorkspaceInfo {
                id: "w1".into(),
                name: None,
                tab_count: 1,
                attention: None,
                recency: 0,
            }],
            active_tab: "t1".into(),
            tabs: vec![workspace_tab("t1", &["p1"], "p1")],
            geometry: PopupGeometry {
                side_margin: 8.0,
                vertical_margin: 4.0,
            },
            entries: Vec::new(),
            codex_quota: None,
            claude_quota: None,
            sidebar: eon_workspace_protocol::v26::Sidebar::Open,
            attention_queue: Vec::new(),
        };
        let hint = crate::ShortcutHint {
            modifiers: "Alt".into(),
            key: "/".into(),
            label: "all keys".into(),
            accessible_label: "Alt+/ — Show shortcuts".into(),
            priority: 0,
        };
        let footer = crate::ShortcutHintsScene::new(
            vec![hint.clone()],
            size,
            metrics,
            WorkspaceFocus::Terminal,
            |text| text.len() as f32 * 10.0,
        );
        let project = |footer| {
            WorkspaceScene::from_snapshot_with_bar(
                &workspace,
                EonBar::default(),
                size,
                metrics,
                0.0,
                0.0,
                |_, text| (text.to_owned(), text.len() as f32 * 10.0),
                footer,
            )
        };
        let mut snapshot = Snapshot::new(size);
        snapshot.set_workspace(Some(&project(Some(footer))));
        let update = snapshot.tree();
        let hints = node(&update, HINTS);
        assert_eq!(hints.role(), Role::Label);
        assert_eq!(hints.value(), Some(hint.accessible_label.as_str()));
        assert!(!hints.supports_action(Action::Focus));
        assert!(!hints.supports_action(Action::Click));
        assert!(!node(&update, WINDOW).children().contains(&SHORTCUTS));
        assert!(!update.nodes.iter().any(|(id, _)| *id == SHORTCUTS));
        assert_eq!(snapshot.action_target(SHORTCUTS, Action::Click), None);
        snapshot.set_workspace(Some(&project(None)));
        let update = snapshot.tree();
        assert!(!update.nodes.iter().any(|(id, _)| *id == HINTS));
        assert!(node(&update, WINDOW).children().contains(&SHORTCUTS));
        assert!(snapshot.action_target(SHORTCUTS, Action::Focus).is_some());
    }

    #[test]
    fn eon_bar_buttons_share_scene_bounds_and_separate_focus_from_activation() {
        let workspace = workspace_scene(
            "t2",
            vec![
                workspace_tab("t1", &["pane-a"], "pane-a"),
                workspace_tab("t2", &["pane-b"], "pane-b"),
            ],
        );
        let mut snapshot = Snapshot::new(PhysicalSize::new(800, 600));
        snapshot.set_workspace(Some(&workspace));

        for (id, control, label) in [
            (
                NEW_TAB,
                WorkspaceHeaderControl::NewTab,
                "New tab — Alt+Shift+T",
            ),
            (
                SHORTCUTS,
                WorkspaceHeaderControl::Shortcuts,
                "Keyboard shortcuts — Alt+/",
            ),
            (
                CLOSE_TAB,
                WorkspaceHeaderControl::CloseTab,
                "Close tab — Alt+Shift+W",
            ),
        ] {
            let update = snapshot.tree();
            let control_node = node(&update, id);
            assert_eq!(control_node.role(), Role::Button);
            assert_eq!(control_node.label(), Some(label));
            assert_eq!(
                control_node.bounds(),
                Some(rect(
                    workspace
                        .controls
                        .into_iter()
                        .find(|candidate| candidate.kind == control)
                        .unwrap()
                        .rect,
                ))
            );
            assert!(control_node.supports_action(Action::Focus));
            assert!(control_node.supports_action(Action::Click));
            assert_eq!(
                snapshot.action_target(id, Action::Focus),
                Some(AccessibilityTarget::HeaderControl {
                    control,
                    activate: false,
                })
            );
            assert_eq!(
                snapshot.action_target(id, Action::Click),
                Some(AccessibilityTarget::HeaderControl {
                    control,
                    activate: true,
                })
            );
            snapshot.workspace_focus = WorkspaceFocus::Header(control);
            assert_eq!(snapshot.tree().focus, id);
        }

        assert_eq!(
            node(&snapshot.tree(), CLOSE_TAB).description(),
            Some("Active tab t2")
        );

        // A configured popup on Alt+Shift+T takes New tab's key, so its name drops the chord.
        let taken = WorkspaceScene::from_snapshot(
            &WorkspaceSnapshot {
                workspace: "w1".into(),
                active_workspace: "w1".into(),
                workspaces: vec![eon_workspace_protocol::v26::WorkspaceInfo {
                    id: "w1".into(),
                    name: None,
                    tab_count: 1,
                    attention: None,
                    recency: 0,
                }],
                active_tab: "t1".into(),
                tabs: vec![workspace_tab("t1", &["pane-a"], "pane-a")],
                geometry: PopupGeometry {
                    side_margin: 8.0,
                    vertical_margin: 4.0,
                },
                entries: vec![PopupEntry {
                    id: "files".into(),
                    label: "Files".into(),
                    shortcut: PopupKey::Bound(Shortcut {
                        modifiers: eon_workspace_protocol::v26::ALT
                            | eon_workspace_protocol::v26::SHIFT,
                        key: "KeyT".into(),
                    }),
                }],
                codex_quota: None,
                claude_quota: None,
                sidebar: eon_workspace_protocol::v26::Sidebar::Open,
                attention_queue: Vec::new(),
            },
            PhysicalSize::new(800, 600),
            CellMetrics::for_scale(1.0),
            0.0,
            0.0,
            |_, text| (text.to_owned(), text.chars().count() as f32 * 10.0),
        );
        snapshot.set_workspace(Some(&taken));
        let update = snapshot.tree();
        assert_eq!(node(&update, NEW_TAB).label(), Some("New tab"));
        assert_eq!(
            node(&update, CLOSE_TAB).label(),
            Some("Close tab — Alt+Shift+W")
        );
    }

    #[test]
    fn workspace_label_leads_the_tabs_and_opens_the_picker() {
        use crate::{HeaderText, WorkspaceHit};

        let metrics = CellMetrics::for_scale(1.0);
        let tabs = (1..=6)
            .map(|n| {
                let pane = format!("p{n}");
                workspace_tab(&format!("t{n}"), &[&pane], &pane)
            })
            .collect::<Vec<_>>();
        let snapshot = |names: &[Option<&str>], shown: &str| WorkspaceSnapshot {
            workspace: shown.into(),
            active_workspace: "w1".into(),
            workspaces: names
                .iter()
                .enumerate()
                .map(|(index, name)| eon_workspace_protocol::v26::WorkspaceInfo {
                    id: format!("w{}", index + 1),
                    name: name.map(Into::into),
                    tab_count: 1,
                    attention: None,
                    recency: 0,
                })
                .collect(),
            active_tab: "t1".into(),
            tabs: tabs.clone(),
            geometry: PopupGeometry {
                side_margin: 8.0,
                vertical_margin: 4.0,
            },
            entries: Vec::new(),
            codex_quota: None,
            claude_quota: None,
            sidebar: eon_workspace_protocol::v26::Sidebar::Open,
            attention_queue: Vec::new(),
        };
        let picker = "Workspaces — Alt+Shift+S";
        let project = |snapshot: &WorkspaceSnapshot, width, picker, tab_scroll| {
            WorkspaceScene::from_snapshot_with_bar(
                snapshot,
                EonBar {
                    workspace_picker: picker,
                    ..EonBar::default()
                },
                PhysicalSize::new(width, 600),
                metrics,
                tab_scroll,
                0.0,
                // Ten pixels a character, fitted by whole characters.
                |kind, text| {
                    let kept = match kind {
                        HeaderText::Fit(width) => (width / 10.0) as usize,
                        _ => usize::MAX,
                    };
                    let text = text.chars().take(kept).collect::<String>();
                    let width = text.chars().count() as f32 * 10.0;
                    (text, width)
                },
                None,
            )
        };
        fn label_hit(scene: &WorkspaceScene) -> Option<WorkspaceHit<'_>> {
            let rect = scene.workspace_label.as_ref().unwrap().rect;
            scene.hit_test(rect.left + rect.width / 2.0, rect.top + rect.height / 2.0)
        }
        let control = WorkspaceHeaderControl::Workspaces;

        // One unnamed workspace leaves the bar as it is without the label.
        let plain = project(&snapshot(&[None], "w1"), 1200, Some(picker), 0.0);
        assert!(plain.workspace_label.is_none() && !plain.has_control(control));
        assert_eq!(plain, project(&snapshot(&[None], "w1"), 1200, None, 0.0));
        assert_eq!(plain.tab_viewport.left, 0.0);
        let mut accessible = Snapshot::new(PhysicalSize::new(1200, 600));
        accessible.set_workspace(Some(&plain));
        let tree = accessible.tree();
        assert_eq!(node(&tree, WINDOW).children()[0], TAB_LIST);
        assert!(tree.nodes.iter().all(|(id, _)| *id != WORKSPACE_LABEL));

        // Two workspaces: the shown one's name and position lead the tabs and open the picker.
        let two = snapshot(&[None, Some("notes")], "w2");
        let scene = project(&two, 1200, Some(picker), 0.0);
        let label = scene.workspace_label.as_ref().unwrap();
        assert_eq!(label.label, "notes 2/2");
        assert_eq!(label.rect.left, plain.tabs[0].rect.left);
        assert!(label.rect.right() < scene.tab_viewport.left);
        assert_eq!(scene.tabs[0].rect.left, scene.tab_viewport.left);
        assert_eq!(label_hit(&scene), Some(WorkspaceHit::Control(control)));
        accessible.set_workspace(Some(&scene));
        let tree = accessible.tree();
        assert_eq!(
            node(&tree, WINDOW).children()[..2],
            [WORKSPACE_LABEL, TAB_LIST]
        );
        let button = node(&tree, WORKSPACE_LABEL);
        assert_eq!(button.role(), Role::Button);
        assert_eq!(button.label(), Some("Workspace notes, 2 of 2"));
        assert_eq!(button.description(), Some(picker));
        assert_eq!(button.bounds(), Some(rect(label.rect)));
        for (action, activate) in [(Action::Focus, false), (Action::Click, true)] {
            assert_eq!(
                accessible.action_target(WORKSPACE_LABEL, action),
                Some(AccessibilityTarget::HeaderControl { control, activate })
            );
        }
        accessible.workspace_focus = WorkspaceFocus::Header(control);
        assert_eq!(accessible.tree().focus, WORKSPACE_LABEL);
        // Tabs scrolled to their end stay inside their viewport, after the label.
        let scrolled = project(&two, 500, Some(picker), f32::MAX);
        assert!(scrolled.tab_scroll() > 0.0);
        assert_eq!(label_hit(&scrolled), Some(WorkspaceHit::Control(control)));

        // Another workspace that needs input badges the label's corner, with the sidebar open,
        // and moves neither the label nor the tabs; the shown one's own attention badges nothing.
        let mut own = two.clone();
        own.workspaces[1].attention = Some(eon_workspace_protocol::v26::AttentionState::NeedsInput);
        assert_eq!(project(&own, 1200, Some(picker), 0.0), scene);
        let mut waiting = two.clone();
        waiting.workspaces[0].attention =
            Some(eon_workspace_protocol::v26::AttentionState::NeedsInput);
        let badged = project(&waiting, 1200, Some(picker), 0.0);
        assert!(badged.sidebar().is_some());
        let badged_label = badged.workspace_label.as_ref().unwrap();
        let badge = badged_label.badge.unwrap();
        assert_eq!(
            badge.state,
            eon_workspace_protocol::v26::AttentionState::NeedsInput
        );
        assert!(badge.rect.right() > badged_label.rect.right() - badge.rect.width);
        assert_eq!(badged_label.rect, label.rect);
        assert_eq!(badged.tabs, scene.tabs);
        accessible.set_workspace(Some(&badged));
        assert_eq!(
            node(&accessible.tree(), WORKSPACE_LABEL).label(),
            Some("Workspace notes, 2 of 2, another workspace needs input")
        );

        // A long name is clipped to a tab's widest text, and the position stays whole.
        let long = "x".repeat(200);
        let clipped = project(
            &snapshot(&[Some(&long), None], "w1"),
            1200,
            Some(picker),
            0.0,
        );
        let label = clipped.workspace_label.as_ref().unwrap();
        assert!(label.label.ends_with("x 1/2"));
        assert!(
            label.rect.width
                <= WorkspaceScene::tab_text_width(PhysicalSize::new(1200, 600), metrics)
                    + metrics.padding * 2.0
        );
        assert_eq!(label.accessible_label, format!("Workspace {long}, 1 of 2"));

        // One named workspace shows its name alone.
        let named = project(&snapshot(&[Some("notes")], "w1"), 1200, Some(picker), 0.0);
        let label = named.workspace_label.as_ref().unwrap();
        assert_eq!(
            (label.label.as_str(), label.accessible_label.as_str()),
            ("notes", "Workspace notes, 1 of 1")
        );

        // Without the picker's entry the label is inert text.
        let inert = project(&two, 1200, None, 0.0);
        assert!(!inert.has_control(control));
        assert_eq!(label_hit(&inert), None);
        accessible.set_workspace(Some(&inert));
        let tree = accessible.tree();
        let text = node(&tree, WORKSPACE_LABEL);
        assert_eq!(text.role(), Role::Label);
        assert_eq!(text.value(), Some("Workspace notes, 2 of 2"));
        assert!(!text.supports_action(Action::Click) && !text.supports_action(Action::Focus));
        assert_eq!(
            accessible.action_target(WORKSPACE_LABEL, Action::Click),
            None
        );

        // Under width pressure quota chips yield first and the label next, before it would
        // shrink the tabs below their minimum; nothing overlaps.
        let mut quota = two.clone();
        quota.codex_quota = Some(CodexQuota {
            state: CodexQuotaState::Fresh,
            observed_at: 1,
            windows: vec![CodexQuotaWindow {
                duration_minutes: 300,
                remaining_percent: 74,
                resets_at: None,
            }],
        });
        let (mut dropped, mut kept_over_quota) = (false, false);
        for width in (40..=1600).step_by(8) {
            let scene = project(&quota, width, Some(picker), 0.0);
            assert!(scene.quotas.is_empty() || scene.workspace_label.is_some());
            match &scene.workspace_label {
                Some(label) => {
                    assert!(label.rect.right() < scene.tab_viewport.left, "{width}");
                    assert_eq!(label_hit(&scene), Some(WorkspaceHit::Control(control)));
                    kept_over_quota |= scene.quotas.is_empty();
                }
                None => {
                    assert_eq!(scene.tab_viewport.left, 0.0, "{width}");
                    dropped = true;
                }
            }
            assert!(scene.tab_viewport.right() <= scene.controls[0].rect.left + 0.01);
            assert!(scene.controls[0].rect.right() <= scene.drag_region.left + 0.01);
            assert!(scene.drag_region.right() <= scene.controls[1].rect.left + 0.01);
        }
        assert!(dropped && kept_over_quota);
    }

    #[test]
    fn workspace_accessibility_order_matches_tabs_then_one_expanded_pane() {
        let workspace = WorkspaceScene::from_snapshot_with_bar(
            &WorkspaceSnapshot {
                workspace: "w1".into(),
                active_workspace: "w1".into(),
                workspaces: vec![eon_workspace_protocol::v26::WorkspaceInfo {
                    id: "w1".into(),
                    name: None,
                    tab_count: 1,
                    attention: None,
                    recency: 0,
                }],
                active_tab: "t1".into(),
                tabs: vec![
                    Tab {
                        pending: false,
                        selected_popup: None,
                        popups: Vec::new(),
                        id: "t1".into(),
                        directory: b"/tmp/eon".to_vec(),
                        selected_pane: Some("p2".into()),
                        columns: vec![eon_workspace_protocol::v26::Column {
                            panes: vec![
                                Pane {
                                    label: "p1  offline".into(),
                                    pinned_name: None,
                                    id: "p1".into(),
                                    session: "session-1".into(),
                                    endpoint: b"/run/eon/one.sock".to_vec(),
                                    live: false,
                                    attention: None,
                                },
                                Pane {
                                    label: "p2  Review · ../repo".into(),
                                    pinned_name: None,
                                    id: "p2".into(),
                                    session: "session-2".into(),
                                    endpoint: b"/run/eon/two.sock".to_vec(),
                                    live: true,
                                    attention: None,
                                },
                            ],
                            expanded: "p2".into(),
                        }],
                        attention_rollup: None,
                    },
                    Tab {
                        pending: false,
                        selected_popup: None,
                        popups: Vec::new(),
                        id: "t2".into(),
                        directory: b"/tmp/nova".to_vec(),
                        selected_pane: Some("p3".into()),
                        columns: vec![eon_workspace_protocol::v26::Column {
                            panes: vec![Pane {
                                label: "p3".into(),
                                pinned_name: None,
                                id: "p3".into(),
                                session: "session-3".into(),
                                endpoint: b"/run/eon/three.sock".to_vec(),
                                live: false,
                                attention: Some(eon_workspace_protocol::v26::Attention {
                                    state: eon_workspace_protocol::v26::AttentionState::Finished,
                                    record: Some(eon_workspace_protocol::v26::AttentionRecord {
                                        kind: None,
                                        progress: None,
                                        app: "claude-code".into(),
                                        name: String::new(),
                                        title: String::new(),
                                        message: String::new(),
                                        changed_at: 1,
                                        stale: false,
                                        others: 0,
                                    }),
                                }),
                            }],
                            expanded: "p3".into(),
                        }],
                        attention_rollup: Some(
                            eon_workspace_protocol::v26::AttentionState::Finished,
                        ),
                    },
                ],
                geometry: eon_workspace_protocol::v26::PopupGeometry {
                    side_margin: 8.0,
                    vertical_margin: 4.0,
                },
                entries: Vec::new(),
                codex_quota: None,
                claude_quota: None,
                sidebar: eon_workspace_protocol::v26::Sidebar::Open,
                attention_queue: Vec::new(),
            },
            EonBar::default(),
            PhysicalSize::new(800, 600),
            CellMetrics::for_scale(1.0),
            0.0,
            0.0,
            |_, text| (text.to_owned(), text.chars().count() as f32 * 10.0),
            None,
        );
        let terminal_bounds = rect(workspace.visible_terminal().unwrap());
        let mut snapshot = Snapshot {
            title: "shell".into(),
            content: AccessibleText {
                rows: vec![AccessibleRow {
                    value: "terminal".into(),
                    character_lengths: vec![1; 8],
                    column_spans: (0..8).map(|column| column..column + 1).collect(),
                }],
                selection: None,
            },
            status: String::new(),
            columns: Some(8),
            workspace_focus: WorkspaceFocus::Panes,
            ..Snapshot::new(PhysicalSize::new(800, 600))
        };
        snapshot.set_workspace(Some(&workspace));
        let tab_1 = snapshot.tab_ids["t1"];
        let tab_2 = snapshot.tab_ids["t2"];
        let pane_1 = snapshot.pane_ids["p1"];
        let pane_2 = snapshot.pane_ids["p2"];
        let update = snapshot.tree();
        assert_eq!(
            node(&update, WINDOW).children(),
            &[TAB_LIST, NEW_TAB, SHORTCUTS, CLOSE_TAB, PANE_PANEL]
        );
        assert_eq!(node(&update, TAB_LIST).children(), &[tab_1, tab_2]);
        assert_eq!(
            node(&update, PANE_PANEL).children(),
            &[pane_1, pane_2, CONTENT]
        );
        assert_eq!(node(&update, tab_1).role(), Role::Tab);
        assert_eq!(node(&update, tab_1).label(), Some("Tab 1 of 2  /tmp/eon"));
        assert_eq!(
            node(&update, tab_2).label(),
            Some("Tab 2 of 2  /tmp/nova, finished")
        );
        assert_eq!(node(&update, pane_1).label(), Some("p1  offline"));
        assert_eq!(workspace.panes[1].label(), "p2  Review · ../repo");
        assert_eq!(
            node(&update, pane_2).label(),
            Some(workspace.panes[1].label())
        );
        assert_eq!(node(&update, pane_2).role(), Role::Button);
        assert_eq!(node(&update, pane_2).is_expanded(), Some(true));
        assert_eq!(node(&update, CONTENT).bounds(), Some(terminal_bounds));
        assert_eq!(update.focus, pane_2);
    }

    #[test]
    fn zoomed_tab_names_the_zoom_and_lists_only_its_pane() {
        let tab = workspace_tab("t1", &["p1", "p2", "p3"], "p2");
        let state = WorkspaceSnapshot {
            workspace: "w1".into(),
            active_workspace: "w1".into(),
            workspaces: vec![eon_workspace_protocol::v26::WorkspaceInfo {
                id: "w1".into(),
                name: None,
                tab_count: 1,
                attention: None,
                recency: 0,
            }],
            active_tab: "t1".into(),
            tabs: vec![tab],
            geometry: eon_workspace_protocol::v26::PopupGeometry {
                side_margin: 8.0,
                vertical_margin: 4.0,
            },
            entries: Vec::new(),
            codex_quota: None,
            claude_quota: None,
            sidebar: eon_workspace_protocol::v26::Sidebar::Open,
            attention_queue: Vec::new(),
        };
        let project = |zoom| {
            WorkspaceScene::from_snapshot_with_zoom(
                &state,
                EonBar::default(),
                PhysicalSize::new(800, 600),
                CellMetrics::for_scale(1.0),
                0.0,
                0.0,
                |_, text| (text.to_owned(), text.chars().count() as f32 * 10.0),
                None,
                zoom,
                crate::SidebarView::default(),
            )
        };
        let normal = project(false);
        let workspace = project(true);
        let mut snapshot = Snapshot::new(PhysicalSize::new(800, 600));
        snapshot.set_workspace(Some(&normal));
        assert_eq!(snapshot.pane_ids.len(), 3);
        snapshot.set_workspace(Some(&workspace));
        let update = snapshot.tree();
        let panel = node(&update, PANE_PANEL);
        assert_eq!(panel.label(), Some("Zoomed pane p2  unavailable"));
        assert!(workspace.tabs[0].label().ends_with(" \u{f00e}"));
        assert!(!workspace.tabs[0].label().contains(" · zoom"));
        assert_eq!(panel.children(), &[snapshot.pane_ids["p2"], CONTENT]);
        assert!(!snapshot.pane_ids.contains_key("p1"));
        assert!(!snapshot.pane_ids.contains_key("p3"));
        assert_eq!(
            node(&update, snapshot.tab_ids["t1"]).label(),
            Some("Tab 1 of 1  /tmp/t1, zoomed")
        );
        snapshot.set_workspace(Some(&normal));
        assert_eq!(snapshot.pane_ids.len(), 3);
        assert_eq!(
            node(&snapshot.tree(), PANE_PANEL).label(),
            Some("Active tab panes")
        );
    }

    #[test]
    fn popup_and_empty_body_expose_only_visible_accessibility_targets() {
        let size = PhysicalSize::new(800, 600);
        let mut state = WorkspaceSnapshot {
            workspace: "w1".into(),
            active_workspace: "w1".into(),
            workspaces: vec![eon_workspace_protocol::v26::WorkspaceInfo {
                id: "w1".into(),
                name: None,
                tab_count: 1,
                attention: None,
                recency: 0,
            }],
            active_tab: "t1".into(),
            geometry: PopupGeometry {
                side_margin: 8.0,
                vertical_margin: 4.0,
            },
            entries: vec![PopupEntry {
                id: "project".into(),
                label: "Project".into(),
                shortcut: PopupKey::Bound(Shortcut {
                    modifiers: eon_workspace_protocol::v26::ALT,
                    key: "KeyZ".into(),
                }),
            }],
            tabs: vec![Tab {
                id: "t1".into(),
                directory: b"/tmp/t1".to_vec(),
                pending: true,
                selected_pane: None,
                columns: Vec::new(),
                selected_popup: Some("u1".into()),
                popups: vec![Popup {
                    id: "u1".into(),
                    entry: "project".into(),
                    session: "s1".into(),
                    endpoint: b"/run/popup.sock".to_vec(),
                    attention: None,
                }],
                attention_rollup: None,
            }],
            codex_quota: None,
            claude_quota: None,
            sidebar: eon_workspace_protocol::v26::Sidebar::Open,
            attention_queue: Vec::new(),
        };
        let workspace = WorkspaceScene::from_snapshot(
            &state,
            size,
            CellMetrics::for_scale(1.0),
            0.0,
            0.0,
            |_, text| (text.into(), text.len() as f32 * 10.0),
        );
        let mut snapshot = Snapshot {
            content: AccessibleText {
                rows: vec![AccessibleRow {
                    value: "popup".into(),
                    character_lengths: vec![1; 5],
                    column_spans: (0..5).map(|column| column..column + 1).collect(),
                }],
                selection: None,
            },
            columns: Some(6),
            workspace_focus: WorkspaceFocus::Panes,
            ..Snapshot::new(size)
        };
        snapshot.set_workspace(Some(&workspace));
        let tab = snapshot.tab_ids["t1"];
        let update = snapshot.tree();
        let mut reader = accesskit_consumer::Tree::new(update.clone(), true);
        // A popup card with no area still holds its terminal rows, so the panel keeps CONTENT.
        let tiny_size = PhysicalSize::new(800, 20);
        let mut tiny = Snapshot {
            size: tiny_size,
            status: String::new(),
            ..snapshot.clone()
        };
        tiny.set_workspace(Some(&WorkspaceScene::from_snapshot(
            &state,
            tiny_size,
            CellMetrics::for_scale(1.0),
            0.0,
            0.0,
            |_, text| (text.into(), text.len() as f32 * 10.0),
        )));
        let tiny = tiny.tree();
        accesskit_consumer::Tree::new(tiny.clone(), true);
        assert_eq!(node(&tiny, CONTENT).bounds(), None);
        assert_eq!(node(&tiny, PANE_PANEL).children(), &[CONTENT]);

        assert_eq!(node(&update, PANE_PANEL).label(), Some("Project"));
        assert_eq!(node(&update, PANE_PANEL).children(), &[CONTENT]);
        assert_eq!(update.focus, CONTENT);
        assert!(node(&update, tab).supports_action(Action::Click));
        assert!(node(&update, tab).supports_action(Action::Focus));
        assert_eq!(
            snapshot.action_target(tab, Action::Click),
            Some(AccessibilityTarget::Tab("t1".into()))
        );
        snapshot.workspace_focus = WorkspaceFocus::Tabs;
        assert_eq!(snapshot.tree().focus, tab);
        state.tabs[0].pending = false;
        state.tabs[0].selected_popup = None;
        snapshot.columns = None;
        snapshot.content = AccessibleText::default();
        snapshot.status.clear();
        snapshot.set_workspace(Some(&WorkspaceScene::from_snapshot(
            &state,
            size,
            CellMetrics::for_scale(1.0),
            0.0,
            0.0,
            |_, text| (text.into(), text.len() as f32 * 10.0),
        )));
        snapshot.workspace_focus = WorkspaceFocus::Terminal;
        let empty = snapshot.tree();
        // AccessKit panics on a node that has no parent, both as the first tree and as an update.
        accesskit_consumer::Tree::new(empty.clone(), true);
        reader.update_and_process_changes(empty.clone(), &mut IgnoreChanges);
        assert!(empty.nodes.iter().all(|(id, _)| *id != CONTENT));
        assert!(node(&empty, PANE_PANEL).children().is_empty());
        assert_eq!(node(&empty, PANE_PANEL).label(), Some("Empty tab"));
        assert_eq!(empty.focus, tab);
        assert!(node(&empty, tab).supports_action(Action::Click));
        snapshot.workspace_focus = WorkspaceFocus::Panes;
        assert_eq!(snapshot.action_target(CONTENT, Action::Click), None);
        snapshot.status = "popup action rejected".into();
        let failed = snapshot.tree();
        reader.update_and_process_changes(failed.clone(), &mut IgnoreChanges);
        assert_eq!(node(&failed, PANE_PANEL).children(), &[CONTENT]);
        assert_eq!(node(&failed, CONTENT).role(), Role::Alert);
        assert_eq!(
            node(&failed, CONTENT).value(),
            Some("popup action rejected")
        );
        assert_eq!(failed.focus, tab);
        snapshot.status.clear();

        state.tabs.push(workspace_tab("t2", &["p2"], "p2"));
        state.workspaces[0].tab_count = state.tabs.len();
        state.active_tab = "t2".into();
        let inactive_popup = WorkspaceScene::from_snapshot(
            &state,
            size,
            CellMetrics::for_scale(1.0),
            0.0,
            0.0,
            |_, text| (text.into(), text.len() as f32 * 10.0),
        );
        snapshot.set_workspace(Some(&inactive_popup));
        let active_tab = snapshot.tab_ids["t2"];
        let pane = snapshot.pane_ids["p2"];
        let update = snapshot.tree();

        assert_eq!(node(&update, PANE_PANEL).label(), Some("Active tab panes"));
        assert_eq!(node(&update, PANE_PANEL).children(), &[pane, CONTENT]);
        assert_eq!(update.focus, pane);
        assert!(node(&update, active_tab).supports_action(Action::Focus));
        assert_eq!(
            snapshot.action_target(active_tab, Action::Click),
            Some(AccessibilityTarget::Tab("t2".into()))
        );
    }

    #[test]
    fn workspace_nodes_keep_identity_and_retire_removed_actions() {
        let accessibility = Accessibility::new(PhysicalSize::new(800, 600));
        let mut activation = accessibility.activation();
        set_workspace(
            &accessibility,
            workspace_scene(
                "t2",
                vec![
                    workspace_tab("t1", &["pane-x"], "pane-x"),
                    workspace_tab("t2", &["pane-a", "pane-b"], "pane-b"),
                ],
            ),
            WorkspaceFocus::Panes,
        );
        let first = activation.request_initial_tree().unwrap();
        let tab_a = tree_node_id(&first, "Tab 1 of 2  /tmp/t1");
        let tab_b = tree_node_id(&first, "Tab 2 of 2  /tmp/t2");
        let pane_a = tree_node_id(&first, "pane-a  unavailable");
        let pane_b = tree_node_id(&first, "pane-b  unavailable");
        assert_ne!(tab_a, pane_a);

        set_workspace(
            &accessibility,
            workspace_scene("t2", vec![workspace_tab("t2", &["pane-b"], "pane-b")]),
            WorkspaceFocus::Panes,
        );
        let second = activation.request_initial_tree().unwrap();

        assert_eq!(tree_node_id(&second, "Tab 1 of 1  /tmp/t2"), tab_b);
        assert_eq!(tree_node_id(&second, "pane-b  unavailable"), pane_b);
        assert_eq!(second.focus, pane_b);
        assert_eq!(
            accessibility.action_target(tab_b, Action::Click),
            Some(AccessibilityTarget::Tab("t2".into()))
        );
        assert_eq!(
            accessibility.action_target(pane_b, Action::Click),
            Some(AccessibilityTarget::Pane("pane-b".into()))
        );
        assert_eq!(accessibility.action_target(tab_a, Action::Click), None);
        assert_eq!(accessibility.action_target(pane_a, Action::Click), None);
        assert_eq!(
            accessibility.action_target(NodeId(u64::MAX), Action::Click),
            None
        );

        set_workspace(
            &accessibility,
            workspace_scene(
                "t2",
                vec![
                    workspace_tab("t3", &["pane-z"], "pane-z"),
                    workspace_tab("t2", &["pane-c", "pane-b"], "pane-b"),
                ],
            ),
            WorkspaceFocus::Panes,
        );
        let third = activation.request_initial_tree().unwrap();

        assert_eq!(tree_node_id(&third, "Tab 2 of 2  /tmp/t2"), tab_b);
        assert_eq!(tree_node_id(&third, "pane-b  unavailable"), pane_b);
        assert_ne!(tree_node_id(&third, "Tab 1 of 2  /tmp/t3"), tab_a);
        assert_ne!(tree_node_id(&third, "pane-c  unavailable"), pane_a);
        assert_eq!(third.focus, pane_b);
    }

    #[test]
    fn scrollback_is_a_terminal_description_without_live_alerts() {
        let mut snapshot = Snapshot::new(PhysicalSize::new(800, 600));
        snapshot.columns = Some(80);
        snapshot.status.clear();
        for label in [Some("↓ 240 rows"), Some("↓ 1 row"), None] {
            snapshot.scrollback_label = label.map(str::to_owned);
            snapshot.scrollback_rect = label.map(|_| SceneRect {
                left: 650.0,
                top: 8.0,
                width: 140.0,
                height: 24.0,
            });
            let update = snapshot.tree();
            assert_eq!(node(&update, CONTENT).role(), Role::Terminal);
            assert_eq!(
                snapshot.action_target(SCROLLBACK, Action::Click),
                label.map(|_| AccessibilityTarget::ReturnToLive)
            );
            if label.is_some() {
                let button = node(&update, SCROLLBACK);
                assert_eq!(button.role(), Role::Button);
                assert_eq!(button.label(), Some("Return to live output"));
                assert!(button.supports_action(Action::Click));
            } else {
                assert!(update.nodes.iter().all(|(id, _)| *id != SCROLLBACK));
            }
            assert_eq!(
                node(&update, CONTENT).description(),
                label
                    .map(|label| format!(
                        "Scrollback: {label} above live output (committed display rows)"
                    ))
                    .as_deref()
            );
            assert!(
                update
                    .nodes
                    .iter()
                    .all(|(_, node)| node.role() != Role::Alert)
            );
        }
    }
}
