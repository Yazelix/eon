//! The Runtime side of workspace events (EON-C39): the window's event
//! projection, its difference as ordered events, the supervisor's event
//! counter and retained events, and subscription filters. The supervisor
//! publishes and serves subscribers; nothing here touches a connection.

use super::{supervisor::request_id, workspace::resolve_selector};
use eon_workspace_protocol::v37::{
    self as eonw, AttentionState as State, Failure, Snapshot, TargetId, TerminalTarget,
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
};

/// How many of the latest events a supervisor keeps for `--after`.
const RETAINED_EVENTS: usize = 1024;
/// The most the kept events may weigh as EONW frames (EON-C39): a `tab-*`
/// event carries its whole tab, so a count alone would let a few large tabs
/// flipping hold hundreds of MB.
const RETAINED_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Workspace,
    Tab,
    Pane,
    Popup,
    Attention,
    Queue,
    Busy,
}

/// A workspace as `eon session ls --json` lists it, plus its active tab.
/// Its `tabs` count, `attention` and `recency` are derived: no event names
/// them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct WorkspaceItem {
    pub(super) id: String,
    pub(super) name: Option<String>,
    pub(super) shown: bool,
    pub(super) tabs: usize,
    pub(super) attention: Option<State>,
    pub(super) recency: u8,
    pub(super) active_tab: String,
}

/// A tab as `eon session --json` writes it, plus its workspace; tab
/// creations, moves and closes also carry `tabs`, the workspace's tab IDs in
/// order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TabItem {
    pub(super) workspace: String,
    pub(super) tab: eonw::Tab,
    pub(super) tabs: Option<Vec<String>>,
}

/// One column of a tab, as pane events carry the tab's layout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ColumnOrder {
    pub(super) panes: Vec<String>,
    pub(super) expanded: String,
}

/// A pane with its workspace, tab, the column and expanded flag that
/// `eon session --json` writes, and its tab's columns in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PaneItem {
    pub(super) workspace: String,
    pub(super) tab: String,
    pub(super) pane: eonw::Pane,
    /// From 1.
    pub(super) column: usize,
    pub(super) expanded: bool,
    /// Empty once the tab is gone.
    pub(super) columns: Vec<ColumnOrder>,
}

/// A popup with its workspace, tab, and whether it is the tab's chosen one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PopupItem {
    pub(super) workspace: String,
    pub(super) tab: String,
    pub(super) popup: eonw::Popup,
    pub(super) chosen: bool,
}

/// One attention queue entry, as snapshots and `queue` events carry it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct QueueItem {
    pub(super) workspace: String,
    pub(super) tab: String,
    pub(super) target: String,
    pub(super) state: State,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum WorkspaceChange {
    Created,
    Renamed,
    Closed,
    Shown {
        /// The workspace it hid, which filters read and the wire leaves out.
        hidden: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TabChange {
    Created,
    Moved,
    Directory,
    Closed,
    Activated,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PaneChange {
    Created,
    /// Its column, its place among its tab's panes or its expanded flag.
    Moved,
    Renamed,
    Ended,
    Closed,
    Selected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PopupChange {
    Created,
    Shown,
    Hidden,
    Closed,
}

/// One EON-C39 event, carrying the objects as they are after it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Event {
    Workspace(WorkspaceChange, WorkspaceItem),
    Tab(TabChange, TabItem),
    Pane(PaneChange, PaneItem),
    Popup(PopupChange, PopupItem),
    Attention {
        target: TerminalTarget,
        previous: Option<State>,
        attention: Option<eonw::Attention>,
    },
    Queue {
        queue: Vec<QueueItem>,
        /// The queue before, which filters read and the wire leaves out.
        previous: Vec<QueueItem>,
    },
    Busy {
        target: TerminalTarget,
        busy: bool,
        /// Its EON-C11 name while busy.
        name: Option<String>,
    },
}

impl Event {
    pub(super) fn kind(&self) -> Kind {
        match self {
            Self::Workspace(..) => Kind::Workspace,
            Self::Tab(..) => Kind::Tab,
            Self::Pane(..) => Kind::Pane,
            Self::Popup(..) => Kind::Popup,
            Self::Attention { .. } => Kind::Attention,
            Self::Queue { .. } => Kind::Queue,
            Self::Busy { .. } => Kind::Busy,
        }
    }

    /// The event's EON-C39 name, which tests describe events by.
    #[cfg(test)]
    pub(super) fn name(&self) -> &'static str {
        match self {
            Self::Workspace(change, _) => match change {
                WorkspaceChange::Created => "session-created",
                WorkspaceChange::Renamed => "session-renamed",
                WorkspaceChange::Closed => "session-closed",
                WorkspaceChange::Shown { .. } => "session-shown",
            },
            Self::Tab(change, _) => match change {
                TabChange::Created => "tab-created",
                TabChange::Moved => "tab-moved",
                TabChange::Directory => "tab-directory",
                TabChange::Closed => "tab-closed",
                TabChange::Activated => "tab-activated",
            },
            Self::Pane(change, _) => match change {
                PaneChange::Created => "pane-created",
                PaneChange::Moved => "pane-moved",
                PaneChange::Renamed => "pane-renamed",
                PaneChange::Ended => "pane-ended",
                PaneChange::Closed => "pane-closed",
                PaneChange::Selected => "pane-selected",
            },
            Self::Popup(change, _) => match change {
                PopupChange::Created => "popup-created",
                PopupChange::Shown => "popup-shown",
                PopupChange::Hidden => "popup-hidden",
                PopupChange::Closed => "popup-closed",
            },
            Self::Attention { .. } => "attention",
            Self::Queue { .. } => "queue",
            Self::Busy { .. } => "busy",
        }
    }

    /// The workspaces, tabs, terminals and attention states the event is
    /// about, as filters match them. Showing a workspace is also about the
    /// one it hid, a tab event about its tab's terminals, and a queue event
    /// about its entries' workspaces and states, before and after.
    fn values(&self) -> Values<'_> {
        let mut values = Values::default();
        match self {
            Self::Workspace(change, item) => {
                values.workspaces.push(&item.id);
                if let WorkspaceChange::Shown {
                    hidden: Some(hidden),
                } = change
                {
                    values.workspaces.push(hidden);
                }
            }
            Self::Tab(_, item) => {
                values.workspaces.push(&item.workspace);
                values.tabs.push(&item.tab.id);
                values
                    .terminals
                    .extend(holders(&item.tab).map(Holder::session));
            }
            Self::Pane(_, item) => {
                values.workspaces.push(&item.workspace);
                values.tabs.push(&item.tab);
                values.terminals.push(&item.pane.session);
            }
            Self::Popup(_, item) => {
                values.workspaces.push(&item.workspace);
                values.tabs.push(&item.tab);
                values.terminals.push(&item.popup.session);
            }
            Self::Attention {
                target,
                previous,
                attention,
            } => {
                values.workspaces.push(&target.workspace);
                values.tabs.push(&target.tab);
                values.terminals.push(&target.terminal);
                values.states.extend(*previous);
                values
                    .states
                    .extend(attention.as_ref().map(|attention| attention.state));
            }
            Self::Busy { target, .. } => {
                values.workspaces.push(&target.workspace);
                values.tabs.push(&target.tab);
                values.terminals.push(&target.terminal);
            }
            Self::Queue { queue, previous } => {
                for entry in queue.iter().chain(previous) {
                    values.workspaces.push(&entry.workspace);
                    values.states.push(entry.state);
                }
            }
        }
        values
    }
}

#[derive(Default)]
struct Values<'a> {
    workspaces: Vec<&'a str>,
    tabs: Vec<&'a str>,
    terminals: Vec<&'a str>,
    states: Vec<State>,
}

/// The window state events describe: every workspace with its tabs, panes,
/// popups and attention, the attention queue and every busy terminal.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Projection {
    workspaces: Vec<WorkspaceState>,
    queue: Vec<QueueItem>,
    /// Every busy terminal in window order, unbounded.
    busy: Vec<eonw::BusyTerminal>,
}

#[derive(Clone, Debug, PartialEq)]
struct WorkspaceState {
    item: WorkspaceItem,
    tabs: Vec<eonw::Tab>,
}

/// Where a pane or popup terminal stands in a projection.
#[derive(Clone, Copy)]
struct Terminal<'a> {
    workspace: &'a str,
    tab: &'a eonw::Tab,
    holder: Holder<'a>,
}

#[derive(Clone, Copy)]
enum Holder<'a> {
    Pane {
        pane: &'a eonw::Pane,
        column: usize,
        expanded: bool,
    },
    Popup(&'a eonw::Popup),
}

impl Projection {
    /// The projection of every workspace's snapshot, in window order, each
    /// as an inspection presents it with attention (`snapshot_in` then
    /// `Attention::present`), and of `busy`, every busy terminal
    /// (`Attention::busy_terminals`). The first snapshot's workspace list
    /// gives the window order, names, rollups and recency.
    pub(super) fn new(snapshots: &[Snapshot], busy: Vec<eonw::BusyTerminal>) -> Self {
        let first = snapshots.first().expect("a window has a workspace");
        let workspaces = first
            .workspaces
            .iter()
            .map(|info| {
                let snapshot = snapshots
                    .iter()
                    .find(|snapshot| snapshot.workspace == info.id)
                    .expect("every workspace has its snapshot");
                WorkspaceState {
                    item: WorkspaceItem {
                        id: info.id.clone(),
                        name: info.name.clone(),
                        shown: info.id == first.active_workspace,
                        tabs: info.tab_count,
                        attention: info.attention,
                        recency: info.recency,
                        active_tab: snapshot.active_tab.clone(),
                    },
                    tabs: snapshot.tabs.clone(),
                }
            })
            .collect();
        let queue = first
            .attention_queue
            .iter()
            .map(|entry| QueueItem {
                workspace: entry.workspace.clone(),
                tab: entry.tab.clone(),
                target: entry.target.clone(),
                state: entry.attention.state,
            })
            .collect();
        Self {
            workspaces,
            queue,
            busy,
        }
    }

    fn workspace(&self, id: &str) -> Option<&WorkspaceState> {
        self.workspaces
            .iter()
            .find(|workspace| workspace.item.id == id)
    }

    /// Every tab by its ID, with its workspace.
    fn tabs(&self) -> HashMap<&str, (&WorkspaceState, &eonw::Tab)> {
        self.workspaces
            .iter()
            .flat_map(|workspace| {
                workspace
                    .tabs
                    .iter()
                    .map(move |tab| (tab.id.as_str(), (workspace, tab)))
            })
            .collect()
    }

    /// The pane or popup that runs `session`, as `SUBSCRIBED` names its
    /// caller.
    pub(super) fn target(&self, session: &str) -> Option<TerminalTarget> {
        self.terminals()
            .find(|terminal| terminal.session() == session)
            .map(|terminal| terminal.target())
    }

    /// Every pane and popup terminal in window order: workspace by
    /// workspace, tab by tab, panes column by column, then popups.
    fn terminals(&self) -> impl Iterator<Item = Terminal<'_>> {
        self.workspaces.iter().flat_map(|workspace| {
            workspace.tabs.iter().flat_map(move |tab| {
                holders(tab).map(move |holder| Terminal {
                    workspace: &workspace.item.id,
                    tab,
                    holder,
                })
            })
        })
    }

    /// Whether a live pane or a popup still runs terminal `session`.
    fn holds(&self, session: &str) -> bool {
        self.terminals().any(|terminal| {
            terminal.session() == session
                && match terminal.holder {
                    Holder::Pane { pane, .. } => pane.live,
                    Holder::Popup(_) => true,
                }
        })
    }

    /// Workspace `id`'s tab IDs in order; none once it is gone.
    fn tab_order(&self, id: &str) -> Vec<String> {
        self.workspace(id)
            .map(|workspace| workspace.tabs.iter().map(|tab| tab.id.clone()).collect())
            .unwrap_or_default()
    }

    /// Tab `id`'s columns in order; none once it is gone.
    fn column_order(&self, id: &str) -> Vec<ColumnOrder> {
        self.workspaces
            .iter()
            .flat_map(|workspace| &workspace.tabs)
            .find(|tab| tab.id == id)
            .map(|tab| {
                tab.columns
                    .iter()
                    .map(|column| ColumnOrder {
                        panes: column.panes.iter().map(|pane| pane.id.clone()).collect(),
                        expanded: column.expanded.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn tab_item(&self, workspace: &str, tab: &eonw::Tab, ordered: bool) -> TabItem {
        TabItem {
            workspace: workspace.into(),
            tab: tab.clone(),
            tabs: ordered.then(|| self.tab_order(workspace)),
        }
    }

    /// The pane event about `terminal`, with its tab's columns as this
    /// projection orders them.
    fn pane_event(&self, change: PaneChange, terminal: Terminal<'_>) -> Event {
        let Holder::Pane {
            pane,
            column,
            expanded,
        } = terminal.holder
        else {
            unreachable!("a pane event names a pane");
        };
        Event::Pane(
            change,
            PaneItem {
                workspace: terminal.workspace.into(),
                tab: terminal.tab.id.clone(),
                pane: pane.clone(),
                column,
                expanded,
                columns: self.column_order(&terminal.tab.id),
            },
        )
    }

    fn popup_event(change: PopupChange, terminal: Terminal<'_>) -> Event {
        let Holder::Popup(popup) = terminal.holder else {
            unreachable!("a popup event names a popup");
        };
        Event::Popup(
            change,
            PopupItem {
                workspace: terminal.workspace.into(),
                tab: terminal.tab.id.clone(),
                popup: popup.clone(),
                chosen: terminal.tab.selected_popup.as_ref() == Some(&popup.id),
            },
        )
    }

    /// The events that turn `self` into `next`, at most one attention and
    /// one busy event per terminal, in this order: creations and changes,
    /// workspace by workspace and parents first; attention and busy,
    /// terminal by terminal; closes, children first; selections; the queue.
    /// Each event carries its objects and order as `next` has them.
    pub(super) fn diff(&self, next: &Self) -> Vec<Event> {
        if self == next {
            return Vec::new();
        }
        let mut events = Vec::new();
        let old_tabs = self.tabs();
        let new_tabs = next.tabs();
        let old_terminals: HashMap<_, _> = self
            .terminals()
            .map(|terminal| (terminal.key(), terminal))
            .collect();
        let new_terminals: HashMap<_, _> = next
            .terminals()
            .map(|terminal| (terminal.key(), terminal))
            .collect();

        for workspace in &next.workspaces {
            let id = &workspace.item.id;
            let old_workspace = self.workspace(id);
            match old_workspace {
                None => events.push(Event::Workspace(
                    WorkspaceChange::Created,
                    workspace.item.clone(),
                )),
                Some(old) if old.item.name != workspace.item.name => events.push(Event::Workspace(
                    WorkspaceChange::Renamed,
                    workspace.item.clone(),
                )),
                Some(_) => {}
            }
            let moved_tabs = old_workspace.map_or_else(HashSet::new, |old| {
                reordered(
                    old.tabs.iter().map(|tab| tab.id.as_str()),
                    workspace.tabs.iter().map(|tab| tab.id.as_str()),
                )
            });
            for tab in &workspace.tabs {
                let old_tab = old_tabs.get(tab.id.as_str());
                match old_tab {
                    None => {
                        events.push(Event::Tab(TabChange::Created, next.tab_item(id, tab, true)))
                    }
                    Some((old_workspace, old_tab)) => {
                        if old_workspace.item.id != *id || moved_tabs.contains(tab.id.as_str()) {
                            events.push(Event::Tab(TabChange::Moved, next.tab_item(id, tab, true)));
                        }
                        if old_tab.directory != tab.directory || old_tab.pending != tab.pending {
                            events.push(Event::Tab(
                                TabChange::Directory,
                                next.tab_item(id, tab, false),
                            ));
                        }
                    }
                }
                let moved_panes = old_tab.map_or_else(HashSet::new, |(_, old_tab)| {
                    reordered(
                        old_tab.panes().map(|pane| pane.id.as_str()),
                        tab.panes().map(|pane| pane.id.as_str()),
                    )
                });
                for holder in holders(tab) {
                    let terminal = new_terminals[&holder.key()];
                    let Holder::Pane {
                        pane,
                        column,
                        expanded,
                    } = holder
                    else {
                        if !old_terminals.contains_key(&holder.key()) {
                            events.push(Self::popup_event(PopupChange::Created, terminal));
                        }
                        continue;
                    };
                    let Some(old) = old_terminals.get(&holder.key()) else {
                        events.push(next.pane_event(PaneChange::Created, terminal));
                        continue;
                    };
                    let Holder::Pane {
                        pane: old_pane,
                        column: old_column,
                        expanded: old_expanded,
                    } = old.holder
                    else {
                        unreachable!("pane keys name panes");
                    };
                    if old.tab.id != tab.id
                        || old_column != column
                        || old_expanded != expanded
                        || moved_panes.contains(pane.id.as_str())
                    {
                        events.push(next.pane_event(PaneChange::Moved, terminal));
                    }
                    if old_pane.label != pane.label || old_pane.pinned_name != pane.pinned_name {
                        events.push(next.pane_event(PaneChange::Renamed, terminal));
                    }
                    if old_pane.live && !pane.live {
                        events.push(next.pane_event(PaneChange::Ended, terminal));
                    }
                }
            }
        }

        // Attention and busy follow the terminal, also into a pane its popup
        // was parked as.
        let mut old_sessions: HashMap<&str, Terminal<'_>> = self
            .terminals()
            .map(|terminal| (terminal.session(), terminal))
            .collect();
        let (old_busy, new_busy) = (by_terminal(&self.busy), by_terminal(&next.busy));
        for terminal in next.terminals() {
            let session = terminal.session();
            let old = old_sessions.remove(session);
            let previous = old.and_then(Terminal::attention);
            let moved = old.is_some_and(|old| !old.same_place(terminal));
            if previous != terminal.attention() || (moved && previous.is_some()) {
                events.push(Event::Attention {
                    target: terminal.target(),
                    previous: previous.map(|attention| attention.state),
                    attention: terminal.attention().cloned(),
                });
            }
            let now = new_busy.get(session);
            if old_busy.get(session) != now {
                events.push(Event::Busy {
                    target: terminal.target(),
                    busy: now.is_some(),
                    name: now.map(|terminal| terminal.name.clone()),
                });
            }
        }
        // Terminals no pane or popup runs any longer leave both.
        for terminal in self.terminals() {
            if !old_sessions.contains_key(terminal.session()) {
                continue;
            }
            if let Some(attention) = terminal.attention() {
                events.push(Event::Attention {
                    target: terminal.target(),
                    previous: Some(attention.state),
                    attention: None,
                });
            }
            if old_busy.contains_key(terminal.session()) {
                events.push(Event::Busy {
                    target: terminal.target(),
                    busy: false,
                    name: None,
                });
            }
        }

        for workspace in &self.workspaces {
            for tab in &workspace.tabs {
                for holder in holders(tab) {
                    if new_terminals.contains_key(&holder.key()) {
                        continue;
                    }
                    let terminal = old_terminals[&holder.key()];
                    events.push(match holder {
                        // The pane as it was, in its tab's order after it.
                        Holder::Pane { .. } => next.pane_event(PaneChange::Closed, terminal),
                        Holder::Popup(_) => Self::popup_event(PopupChange::Closed, terminal),
                    });
                }
                if !new_tabs.contains_key(tab.id.as_str()) {
                    events.push(Event::Tab(
                        TabChange::Closed,
                        TabItem {
                            workspace: workspace.item.id.clone(),
                            tab: tab.clone(),
                            tabs: Some(next.tab_order(&workspace.item.id)),
                        },
                    ));
                }
            }
            if next.workspace(&workspace.item.id).is_none() {
                events.push(Event::Workspace(
                    WorkspaceChange::Closed,
                    workspace.item.clone(),
                ));
            }
        }

        for workspace in &next.workspaces {
            let id = &workspace.item.id;
            let old = self.workspace(id);
            for tab in &workspace.tabs {
                let old_tab = old_tabs.get(tab.id.as_str()).map(|(_, tab)| *tab);
                if let Some(selected) = &tab.selected_pane
                    && old_tab.and_then(|tab| tab.selected_pane.as_ref()) != Some(selected)
                    && let Some(&terminal) = new_terminals.get(&(false, selected.as_str()))
                {
                    events.push(next.pane_event(PaneChange::Selected, terminal));
                }
                let old_popup = old_tab.and_then(|tab| tab.selected_popup.as_ref());
                if old_popup != tab.selected_popup.as_ref() {
                    // A popup that closed is never hidden.
                    if let Some(hidden) = old_popup
                        && let Some(&terminal) = new_terminals.get(&(true, hidden.as_str()))
                    {
                        events.push(Self::popup_event(PopupChange::Hidden, terminal));
                    }
                    if let Some(shown) = &tab.selected_popup
                        && let Some(&terminal) = new_terminals.get(&(true, shown.as_str()))
                    {
                        events.push(Self::popup_event(PopupChange::Shown, terminal));
                    }
                }
            }
            if old.map(|old| &old.item.active_tab) != Some(&workspace.item.active_tab)
                && let Some((_, tab)) = new_tabs.get(workspace.item.active_tab.as_str())
            {
                events.push(Event::Tab(
                    TabChange::Activated,
                    next.tab_item(id, tab, false),
                ));
            }
            if workspace.item.shown && !old.is_some_and(|old| old.item.shown) {
                let hidden = self
                    .workspaces
                    .iter()
                    .find(|workspace| workspace.item.shown)
                    .map(|workspace| workspace.item.id.clone());
                events.push(Event::Workspace(
                    WorkspaceChange::Shown { hidden },
                    workspace.item.clone(),
                ));
            }
        }

        if self.queue != next.queue {
            events.push(Event::Queue {
                queue: next.queue.clone(),
                previous: self.queue.clone(),
            });
        }
        events
    }
}

fn by_terminal(busy: &[eonw::BusyTerminal]) -> HashMap<&str, &eonw::BusyTerminal> {
    busy.iter()
        .map(|terminal| (terminal.target.terminal.as_str(), terminal))
        .collect()
}

/// A tab's terminals: panes column by column, then popups.
fn holders(tab: &eonw::Tab) -> impl Iterator<Item = Holder<'_>> {
    tab.columns
        .iter()
        .enumerate()
        .flat_map(|(index, column)| {
            column.panes.iter().map(move |pane| Holder::Pane {
                pane,
                column: index + 1,
                expanded: pane.id == column.expanded,
            })
        })
        .chain(tab.popups.iter().map(Holder::Popup))
}

/// The IDs whose place among the IDs both orders hold differs, so a
/// neighbor's arrival or departure moves nothing.
fn reordered<'a>(
    old: impl Iterator<Item = &'a str>,
    new: impl Iterator<Item = &'a str>,
) -> HashSet<&'a str> {
    let (old, new): (Vec<_>, Vec<_>) = (old.collect(), new.collect());
    let (in_old, in_new): (HashSet<_>, HashSet<_>) =
        (old.iter().copied().collect(), new.iter().copied().collect());
    old.iter()
        .filter(|id| in_new.contains(*id))
        .zip(new.iter().filter(|id| in_old.contains(*id)))
        .filter(|(old, new)| old != new)
        .flat_map(|(old, new)| [*old, *new])
        .collect()
}

impl<'a> Holder<'a> {
    fn session(self) -> &'a str {
        match self {
            Self::Pane { pane, .. } => &pane.session,
            Self::Popup(popup) => &popup.session,
        }
    }

    /// Whether it is a popup, and its pane or popup ID.
    fn key(self) -> (bool, &'a str) {
        match self {
            Self::Pane { pane, .. } => (false, &pane.id),
            Self::Popup(popup) => (true, &popup.id),
        }
    }
}

impl<'a> Terminal<'a> {
    fn key(self) -> (bool, &'a str) {
        self.holder.key()
    }

    fn session(self) -> &'a str {
        self.holder.session()
    }

    fn attention(self) -> Option<&'a eonw::Attention> {
        match self.holder {
            Holder::Pane { pane, .. } => pane.attention.as_ref(),
            Holder::Popup(popup) => popup.attention.as_ref(),
        }
    }

    /// Whether `other` holds it in the same workspace, tab, pane or popup.
    fn same_place(self, other: Terminal<'_>) -> bool {
        self.workspace == other.workspace
            && self.tab.id == other.tab.id
            && self.key() == other.key()
    }

    fn target(self) -> TerminalTarget {
        TerminalTarget {
            workspace: self.workspace.into(),
            tab: self.tab.id.clone(),
            id: match self.holder {
                Holder::Pane { pane, .. } => TargetId::Pane(pane.id.clone()),
                Holder::Popup(popup) => TargetId::Popup(popup.id.clone()),
            },
            terminal: self.session().into(),
        }
    }
}

/// What a subscriber asked for (EON-C39 Filters), before resolving.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct FilterSpec {
    /// Workspace selectors, resolved once as EON-C33 resolves them.
    pub(super) workspaces: Vec<String>,
    pub(super) tabs: Vec<String>,
    /// Pane or popup IDs; each follows the terminal it names.
    pub(super) targets: Vec<String>,
    pub(super) kinds: Vec<Kind>,
    pub(super) states: Vec<State>,
}

/// A subscription's filter, resolved when subscribing. Different flags
/// combine with AND and repeats of one with OR. An event passes a flag only
/// when it is about a listed value, except that workspace and queue events
/// pass `--tab` and `--target` when `kinds` names their kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Filter {
    workspaces: Vec<String>,
    tabs: Vec<String>,
    /// The terminals the named targets ran when subscribing.
    terminals: Vec<String>,
    kinds: Vec<Kind>,
    states: Vec<State>,
    /// The calling terminal, never reported busy (EON-C11).
    caller: Option<String>,
}

/// A subscription's snapshot: the workspaces and tabs its filter admits, the
/// window's queue, and the admitted busy terminals without the caller and
/// without EONW's 32-terminal bound.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SnapshotState {
    pub(super) workspaces: Vec<WorkspaceItem>,
    /// Each workspace with admitted tabs, in window order, so each group
    /// travels as one frame.
    pub(super) tabs: Vec<WorkspaceTabs>,
    pub(super) attention_queue: Vec<QueueItem>,
    pub(super) busy: Vec<eonw::BusyTerminal>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct WorkspaceTabs {
    pub(super) workspace: String,
    pub(super) tabs: Vec<eonw::Tab>,
}

impl Filter {
    /// Resolves `spec` over `projection` for the terminal `caller`: a
    /// missing workspace, tab or target fails as EON-C10 names it.
    pub(super) fn resolve(
        spec: &FilterSpec,
        caller: Option<&str>,
        projection: &Projection,
    ) -> Result<Self, Failure> {
        let missing = |code: &str, detail: String| Failure {
            code: code.into(),
            detail,
        };
        let workspaces = spec
            .workspaces
            .iter()
            .map(|selector| {
                let index = resolve_selector(
                    projection.workspaces.iter().map(|workspace| {
                        (workspace.item.id.as_str(), workspace.item.name.as_deref())
                    }),
                    selector,
                )?;
                Ok(projection.workspaces[index].item.id.clone())
            })
            .collect::<Result<_, Failure>>()?;
        let tabs = projection.tabs();
        if let Some(tab) = spec
            .tabs
            .iter()
            .find(|tab| !tabs.contains_key(tab.as_str()))
        {
            return Err(missing("unknown-tab", format!("tab {tab} is not live")));
        }
        let terminals = spec
            .targets
            .iter()
            .map(|id| {
                projection
                    .terminals()
                    .find(|terminal| terminal.key().1 == id)
                    .map(|terminal| terminal.session().to_owned())
                    .ok_or_else(|| {
                        missing(
                            "unknown-target",
                            format!("no pane or popup {id} is live in this window"),
                        )
                    })
            })
            .collect::<Result<_, Failure>>()?;
        Ok(Self {
            workspaces,
            tabs: spec.tabs.clone(),
            terminals,
            kinds: spec.kinds.clone(),
            states: spec.states.clone(),
            caller: caller.map(Into::into),
        })
    }

    pub(super) fn admits(&self, event: &Event) -> bool {
        let kind = event.kind();
        if !self.kinds.is_empty() && !self.kinds.contains(&kind) {
            return false;
        }
        if let Event::Busy { target, .. } = event
            && self.caller.as_ref() == Some(&target.terminal)
        {
            return false;
        }
        let window_wide =
            matches!(kind, Kind::Workspace | Kind::Queue) && self.kinds.contains(&kind);
        let values = event.values();
        listed(&self.workspaces, &values.workspaces)
            && (window_wide || listed(&self.tabs, &values.tabs))
            && (window_wide || listed(&self.terminals, &values.terminals))
            && (self.states.is_empty()
                || values
                    .states
                    .iter()
                    .any(|state| self.states.contains(state)))
    }

    fn admits_workspace(&self, workspace: &str) -> bool {
        self.workspaces.is_empty() || self.workspaces.iter().any(|id| id == workspace)
    }

    fn admits_tab(&self, tab: &eonw::Tab) -> bool {
        (self.tabs.is_empty() || self.tabs.contains(&tab.id))
            && (self.terminals.is_empty()
                || holders(tab)
                    .any(|holder| self.terminals.iter().any(|id| id == holder.session())))
    }

    pub(super) fn snapshot(&self, projection: &Projection) -> SnapshotState {
        let workspaces: Vec<_> = projection
            .workspaces
            .iter()
            .filter(|workspace| self.admits_workspace(&workspace.item.id))
            .collect();
        let tabs = workspaces
            .iter()
            .map(|workspace| WorkspaceTabs {
                workspace: workspace.item.id.clone(),
                tabs: workspace
                    .tabs
                    .iter()
                    .filter(|tab| self.admits_tab(tab))
                    .cloned()
                    .collect(),
            })
            .filter(|group| !group.tabs.is_empty())
            .collect();
        let busy = projection
            .busy
            .iter()
            .filter(|terminal| {
                let target = &terminal.target;
                self.caller.as_ref() != Some(&target.terminal)
                    && self.admits_workspace(&target.workspace)
                    && (self.tabs.is_empty() || self.tabs.contains(&target.tab))
                    && (self.terminals.is_empty() || self.terminals.contains(&target.terminal))
            })
            .cloned()
            .collect();
        SnapshotState {
            workspaces: workspaces
                .into_iter()
                .map(|workspace| workspace.item.clone())
                .collect(),
            tabs,
            attention_queue: projection.queue.clone(),
            busy,
        }
    }

    /// Whether the filter names a workspace, tab or target and every one is
    /// gone: a target once its terminal ends or no pane or popup holds it.
    pub(super) fn gone(&self, projection: &Projection) -> bool {
        let tabs = projection.tabs();
        (!self.workspaces.is_empty() || !self.tabs.is_empty() || !self.terminals.is_empty())
            && self
                .workspaces
                .iter()
                .all(|id| projection.workspace(id).is_none())
            && self.tabs.iter().all(|id| !tabs.contains_key(id.as_str()))
            && self
                .terminals
                .iter()
                .all(|session| !projection.holds(session))
    }
}

/// Whether `values` hold one of the `listed` filter values, or nothing is
/// listed.
fn listed(listed: &[String], values: &[&str]) -> bool {
    listed.is_empty()
        || values
            .iter()
            .any(|value| listed.iter().any(|id| id == value))
}

/// An event and its number. The retained log and every subscriber share the
/// event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Numbered {
    pub(super) seq: u64,
    pub(super) event: Arc<Event>,
}

/// The supervisor's event source: the last published projection, the event
/// counter and the latest events with their weights.
pub(super) struct Events {
    stream: String,
    seq: u64,
    retained: VecDeque<(Numbered, usize)>,
    retained_bytes: usize,
    published: Option<Projection>,
}

impl Events {
    /// The event source of one supervisor process: its stream joins the
    /// generation with a token no other process or source shares, so a
    /// supervisor recovered under the same generation (EON-C15) starts a new
    /// stream.
    pub(super) fn new(generation: &str) -> Self {
        Self {
            stream: format!("{generation}-{}", request_id()),
            seq: 0,
            retained: VecDeque::new(),
            retained_bytes: 0,
            published: None,
        }
    }

    pub(super) fn stream(&self) -> &str {
        &self.stream
    }

    /// The last event's number, 0 before any.
    pub(super) fn seq(&self) -> u64 {
        self.seq
    }

    pub(super) fn projection(&self) -> Option<&Projection> {
        self.published.as_ref()
    }

    /// Publishes `next` and numbers the events that turn the last published
    /// projection into it; the first is the baseline and has none. The
    /// supervisor publishes every loop turn while it has subscribers. Each
    /// event is kept at the weight `weigh` gives it, its EONW frame size, and
    /// the oldest go while more than 1024 are kept or they weigh more than
    /// 8 MiB.
    pub(super) fn publish(
        &mut self,
        next: Projection,
        mut weigh: impl FnMut(&Numbered) -> usize,
    ) -> Vec<Numbered> {
        let Some(published) = &self.published else {
            self.published = Some(next);
            return Vec::new();
        };
        let events = published.diff(&next);
        self.published = Some(next);
        let numbered: Vec<_> = events
            .into_iter()
            .map(|event| {
                self.seq += 1;
                Numbered {
                    seq: self.seq,
                    event: Arc::new(event),
                }
            })
            .collect();
        for event in &numbered {
            let weight = weigh(event);
            self.retained_bytes += weight;
            self.retained.push_back((event.clone(), weight));
        }
        while self.retained.len() > RETAINED_EVENTS || self.retained_bytes > RETAINED_BYTES {
            let Some((_, weight)) = self.retained.pop_front() else {
                break;
            };
            self.retained_bytes -= weight;
        }
        numbered
    }

    /// Stops publishing, as the supervisor does when its last subscriber
    /// leaves. The turns until the next publish, a new baseline, go
    /// unobserved, so no cursor from before may resume across them: one
    /// number is skipped and the retained events dropped.
    pub(super) fn pause(&mut self) {
        if self.published.take().is_some() {
            self.seq += 1;
            self.retained.clear();
            self.retained_bytes = 0;
        }
    }

    /// The events after `after` when `--after STREAM:SEQ` resumes: STREAM is
    /// this stream, SEQ is not above the counter, and the retained events
    /// still hold every event after it.
    pub(super) fn resume(
        &self,
        stream: &str,
        after: u64,
    ) -> Option<impl Iterator<Item = &Numbered>> {
        let oldest = self.seq + 1 - self.retained.len() as u64;
        (stream == self.stream && after <= self.seq && after + 1 >= oldest).then(|| {
            self.retained
                .iter()
                .map(|(event, _)| event)
                .filter(move |event| event.seq > after)
        })
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use eon_workspace_protocol::v37::{
        AttentionRecord, BusyTerminal, Column, PopupGeometry, QueueEntry, Sidebar, WorkspaceInfo,
    };
    use std::collections::BTreeMap;

    pub(crate) fn pane(id: &str, session: &str) -> eonw::Pane {
        eonw::Pane {
            id: id.into(),
            label: id.into(),
            pinned_name: None,
            session: session.into(),
            endpoint: format!("/runtime/{session}.sock").into_bytes(),
            live: true,
            attention: None,
        }
    }

    pub(crate) fn popup(id: &str, entry: &str, session: &str) -> eonw::Popup {
        eonw::Popup {
            id: id.into(),
            entry: entry.into(),
            session: session.into(),
            endpoint: format!("/runtime/{session}.sock").into_bytes(),
            attention: None,
        }
    }

    /// A tab whose columns expand their first pane and whose first column's
    /// is selected.
    pub(crate) fn tab(
        id: &str,
        columns: Vec<Vec<eonw::Pane>>,
        popups: Vec<eonw::Popup>,
    ) -> eonw::Tab {
        eonw::Tab {
            id: id.into(),
            directory: b"/home".to_vec(),
            pending: false,
            selected_pane: columns.first().map(|panes| panes[0].id.clone()),
            selected_popup: None,
            columns: columns
                .into_iter()
                .map(|panes| Column {
                    expanded: panes[0].id.clone(),
                    panes,
                })
                .collect(),
            popups,
            attention_rollup: None,
        }
    }

    pub(crate) fn attention(state: State, stale: bool) -> eonw::Attention {
        eonw::Attention {
            state,
            record: Some(AttentionRecord {
                kind: None,
                progress: None,
                app: "claude-code".into(),
                name: String::new(),
                title: String::new(),
                message: String::new(),
                changed_at: 1,
                stale,
                others: 0,
            }),
        }
    }

    /// Each workspace's snapshot, as `Projection::new` takes them, with
    /// `shown` shown and each workspace's first tab active.
    fn window(
        shown: &str,
        workspaces: Vec<(&str, Option<&str>, Vec<eonw::Tab>)>,
        queue: Vec<QueueEntry>,
    ) -> Vec<Snapshot> {
        let infos: Vec<_> = workspaces
            .iter()
            .enumerate()
            .map(|(index, (id, name, tabs))| WorkspaceInfo {
                id: (*id).into(),
                name: name.map(Into::into),
                tab_count: tabs.len(),
                attention: (index == 0).then_some(State::Working),
                recency: index as u8,
            })
            .collect();
        workspaces
            .into_iter()
            .map(|(id, _, tabs)| Snapshot {
                workspace: id.into(),
                active_workspace: shown.into(),
                workspaces: infos.clone(),
                active_tab: tabs[0].id.clone(),
                geometry: PopupGeometry {
                    side_margin: 0.0,
                    vertical_margin: 0.0,
                },
                entries: Vec::new(),
                tabs,
                codex_quota: None,
                claude_quota: None,
                codex_quota_failure: None,
                claude_quota_failure: None,
                sidebar: Sidebar::Off,
                attention_queue: queue.clone(),
                running: None,
                busy: None,
            })
            .collect()
    }

    /// s1 "main" (shown): t1 with p1 and p2 stacked and the agent popup u1,
    /// and t2 with p3; s2 "side": t3 with p4. Terminal sN runs each.
    pub(crate) fn base() -> Projection {
        let snapshots = window(
            "s1",
            vec![
                (
                    "s1",
                    Some("main"),
                    vec![
                        tab(
                            "t1",
                            vec![vec![pane("p1", "s1"), pane("p2", "s2")]],
                            vec![popup("u1", "agent", "s3")],
                        ),
                        tab("t2", vec![vec![pane("p3", "s4")]], Vec::new()),
                    ],
                ),
                (
                    "s2",
                    Some("side"),
                    vec![tab("t3", vec![vec![pane("p4", "s5")]], Vec::new())],
                ),
            ],
            Vec::new(),
        );
        Projection::new(&snapshots, Vec::new())
    }

    /// Makes `tab` the active tab of `workspace`.
    pub(crate) fn activate(projection: &mut Projection, workspace: &str, tab: &str) {
        workspace_mut(projection, workspace).item.active_tab = tab.into();
    }

    fn workspace_mut<'a>(projection: &'a mut Projection, id: &str) -> &'a mut WorkspaceState {
        projection
            .workspaces
            .iter_mut()
            .find(|workspace| workspace.item.id == id)
            .unwrap()
    }

    pub(crate) fn tab_mut<'a>(projection: &'a mut Projection, id: &str) -> &'a mut eonw::Tab {
        projection
            .workspaces
            .iter_mut()
            .flat_map(|workspace| &mut workspace.tabs)
            .find(|tab| tab.id == id)
            .unwrap()
    }

    pub(crate) fn pane_mut<'a>(projection: &'a mut Projection, id: &str) -> &'a mut eonw::Pane {
        projection
            .workspaces
            .iter_mut()
            .flat_map(|workspace| &mut workspace.tabs)
            .flat_map(|tab| &mut tab.columns)
            .flat_map(|column| &mut column.panes)
            .find(|pane| pane.id == id)
            .unwrap()
    }

    fn busy_terminal(workspace: &str, tab: &str, pane: &str, session: &str) -> BusyTerminal {
        BusyTerminal {
            target: TerminalTarget {
                workspace: workspace.into(),
                tab: tab.into(),
                id: TargetId::Pane(pane.into()),
                terminal: session.into(),
            },
            name: pane.into(),
        }
    }

    fn columns(columns: &[(&[&str], &str)]) -> Vec<ColumnOrder> {
        columns
            .iter()
            .map(|(panes, expanded)| ColumnOrder {
                panes: panes.iter().map(|&pane| pane.into()).collect(),
                expanded: (*expanded).into(),
            })
            .collect()
    }

    fn queue_item(workspace: &str, tab: &str, target: &str, state: State) -> QueueItem {
        QueueItem {
            workspace: workspace.into(),
            tab: tab.into(),
            target: target.into(),
            state,
        }
    }

    /// Each event's name and the ID it is about.
    fn described<'a>(events: impl IntoIterator<Item = &'a Event>) -> Vec<String> {
        events
            .into_iter()
            .map(|event| {
                let id = match event {
                    Event::Workspace(_, item) => item.id.clone(),
                    Event::Tab(_, item) => item.tab.id.clone(),
                    Event::Pane(_, item) => item.pane.id.clone(),
                    Event::Popup(_, item) => item.popup.id.clone(),
                    Event::Attention { target, .. } | Event::Busy { target, .. } => {
                        target.terminal.clone()
                    }
                    Event::Queue { .. } => String::new(),
                };
                format!("{} {id}", event.name()).trim_end().to_owned()
            })
            .collect()
    }

    fn admitted(filter: &Filter, events: &[Event]) -> Vec<String> {
        described(events.iter().filter(|event| filter.admits(event)))
    }

    fn resolve(spec: FilterSpec, projection: &Projection) -> Filter {
        Filter::resolve(&spec, None, projection).unwrap()
    }

    fn unfiltered(projection: &Projection) -> Filter {
        resolve(FilterSpec::default(), projection)
    }

    /// The group and index of tab `id`.
    fn tab_of(state: &SnapshotState, id: &str) -> Option<(usize, usize)> {
        state.tabs.iter().enumerate().find_map(|(group, tabs)| {
            tabs.tabs
                .iter()
                .position(|tab| tab.id == id)
                .map(|index| (group, index))
        })
    }

    /// Tab `id`, or an empty one in `workspace` for a consumer that admits
    /// a tab's panes or popups but not the tab.
    fn known_tab<'a>(state: &'a mut SnapshotState, workspace: &str, id: &str) -> &'a mut eonw::Tab {
        if tab_of(state, id).is_none() {
            let placeholder = tab(id, Vec::new(), Vec::new());
            match state
                .tabs
                .iter_mut()
                .find(|group| group.workspace == workspace)
            {
                Some(group) => group.tabs.push(placeholder),
                None => state.tabs.push(WorkspaceTabs {
                    workspace: workspace.into(),
                    tabs: vec![placeholder],
                }),
            }
        }
        let (group, index) = tab_of(state, id).unwrap();
        &mut state.tabs[group].tabs[index]
    }

    /// Applies `event` as a consumer does: objects replace what they name,
    /// `tabs` and `columns` give the order, a close also clears its
    /// selection, and busy terminals keep window order.
    fn apply(state: &mut SnapshotState, event: &Event) {
        match event {
            Event::Workspace(change, item) => {
                let index = state.workspaces.iter().position(|w| w.id == item.id);
                match (change, index) {
                    (WorkspaceChange::Closed, Some(index)) => {
                        state.workspaces.remove(index);
                    }
                    (WorkspaceChange::Created, None) => state.workspaces.push(item.clone()),
                    (_, Some(index)) => state.workspaces[index] = item.clone(),
                    (_, None) => {}
                }
                if let WorkspaceChange::Shown { .. } = change {
                    for workspace in &mut state.workspaces {
                        workspace.shown = workspace.id == item.id;
                    }
                }
            }
            Event::Tab(change, item) => {
                let found = tab_of(state, &item.tab.id);
                if matches!(change, TabChange::Directory | TabChange::Activated) {
                    if let Some((group, index)) = found {
                        state.tabs[group].tabs[index] = item.tab.clone();
                    }
                } else {
                    if let Some((group, index)) = found {
                        state.tabs[group].tabs.remove(index);
                    }
                    if *change != TabChange::Closed {
                        let group = match state
                            .tabs
                            .iter()
                            .position(|group| group.workspace == item.workspace)
                        {
                            Some(group) => group,
                            None => {
                                state.tabs.push(WorkspaceTabs {
                                    workspace: item.workspace.clone(),
                                    tabs: Vec::new(),
                                });
                                state.tabs.len() - 1
                            }
                        };
                        state.tabs[group].tabs.push(item.tab.clone());
                    }
                    if let Some(order) = &item.tabs
                        && let Some(group) = state
                            .tabs
                            .iter_mut()
                            .find(|group| group.workspace == item.workspace)
                    {
                        group
                            .tabs
                            .sort_by_key(|tab| order.iter().position(|id| *id == tab.id));
                    }
                    state.tabs.retain(|group| !group.tabs.is_empty());
                    let workspaces = state.workspaces.clone();
                    state.tabs.sort_by_key(|group| {
                        workspaces.iter().position(|w| w.id == group.workspace)
                    });
                }
                if *change == TabChange::Activated
                    && let Some(workspace) =
                        state.workspaces.iter_mut().find(|w| w.id == item.workspace)
                {
                    workspace.active_tab = item.tab.id.clone();
                }
            }
            Event::Pane(change, item) => {
                let tab = known_tab(state, &item.workspace, &item.tab);
                let mut panes: HashMap<String, eonw::Pane> = tab
                    .panes()
                    .map(|pane| (pane.id.clone(), pane.clone()))
                    .collect();
                if *change == PaneChange::Closed {
                    panes.remove(&item.pane.id);
                    if tab.selected_pane.as_ref() == Some(&item.pane.id) {
                        tab.selected_pane = None;
                    }
                } else {
                    panes.insert(item.pane.id.clone(), item.pane.clone());
                }
                tab.columns = item
                    .columns
                    .iter()
                    .map(|column| Column {
                        panes: column
                            .panes
                            .iter()
                            .filter_map(|id| panes.get(id).cloned())
                            .collect(),
                        expanded: column.expanded.clone(),
                    })
                    .collect();
                if *change == PaneChange::Selected {
                    tab.selected_pane = Some(item.pane.id.clone());
                }
            }
            Event::Popup(change, item) => {
                let tab = known_tab(state, &item.workspace, &item.tab);
                let position = tab.popups.iter().position(|p| p.id == item.popup.id);
                match (change, position) {
                    (PopupChange::Closed, Some(position)) => {
                        tab.popups.remove(position);
                        if tab.selected_popup.as_ref() == Some(&item.popup.id) {
                            tab.selected_popup = None;
                        }
                    }
                    (PopupChange::Closed, None) => {}
                    (_, Some(position)) => tab.popups[position] = item.popup.clone(),
                    (_, None) => tab.popups.push(item.popup.clone()),
                }
                match change {
                    PopupChange::Shown => tab.selected_popup = Some(item.popup.id.clone()),
                    PopupChange::Hidden => tab.selected_popup = None,
                    _ => {}
                }
            }
            Event::Attention {
                target, attention, ..
            } => {
                let Some((group, index)) = tab_of(state, &target.tab) else {
                    return;
                };
                let tab = &mut state.tabs[group].tabs[index];
                match &target.id {
                    TargetId::Pane(id) => {
                        for pane in tab.columns.iter_mut().flat_map(|c| &mut c.panes) {
                            if pane.id == *id {
                                pane.attention = attention.clone();
                            }
                        }
                    }
                    TargetId::Popup(id) => {
                        for popup in &mut tab.popups {
                            if popup.id == *id {
                                popup.attention = attention.clone();
                            }
                        }
                    }
                }
            }
            Event::Queue { queue, .. } => state.attention_queue = queue.clone(),
            Event::Busy { target, busy, name } => {
                state.busy.retain(|t| t.target.terminal != target.terminal);
                if *busy {
                    state.busy.push(BusyTerminal {
                        target: target.clone(),
                        name: name.clone().unwrap(),
                    });
                }
            }
        }
        // Busy terminals keep window order, which tab and pane order give.
        let order: Vec<String> = state
            .tabs
            .iter()
            .flat_map(|group| &group.tabs)
            .flat_map(|tab| holders(tab).map(|holder| holder.session().to_owned()))
            .collect();
        state
            .busy
            .sort_by_key(|t| order.iter().position(|s| *s == t.target.terminal));
    }

    /// The snapshot without EON-C39's derived values: workspace `tabs`,
    /// `attention` and `recency`, and tab `attention_rollup`.
    fn underived(mut state: SnapshotState) -> SnapshotState {
        for workspace in &mut state.workspaces {
            (workspace.tabs, workspace.attention, workspace.recency) = (0, None, 0);
        }
        for tab in state.tabs.iter_mut().flat_map(|group| &mut group.tabs) {
            tab.attention_rollup = None;
        }
        state
    }

    /// Applying the events `filter` admits from `before` to `after` to the
    /// snapshot of `before` gives the snapshot of `after`.
    fn assert_replays(filter: &Filter, before: &Projection, after: &Projection) {
        let mut state = filter.snapshot(before);
        let events = before.diff(after);
        for event in events.iter().filter(|event| filter.admits(event)) {
            apply(&mut state, event);
        }
        assert_eq!(
            underived(state),
            underived(filter.snapshot(after)),
            "events {:?}",
            described(&events)
        );
    }

    /// From base: s3 "new" opens with t4 and p5 and is shown; s1 is renamed;
    /// t2 closes with p3; p2 is renamed and selected, so it expands; p1
    /// needs input and joins the queue.
    fn busy_turn() -> (Projection, Projection) {
        let before = base();
        let mut after = before.clone();
        workspace_mut(&mut after, "s1").item.shown = false;
        workspace_mut(&mut after, "s1").item.name = Some("renamed".into());
        workspace_mut(&mut after, "s1")
            .tabs
            .retain(|tab| tab.id != "t2");
        workspace_mut(&mut after, "s1").item.tabs = 1;
        let t1 = tab_mut(&mut after, "t1");
        t1.selected_pane = Some("p2".into());
        t1.columns[0].expanded = "p2".into();
        pane_mut(&mut after, "p2").label = "build".into();
        pane_mut(&mut after, "p1").attention = Some(attention(State::NeedsInput, false));
        after.queue = vec![queue_item("s1", "t1", "p1", State::NeedsInput)];
        after.workspaces.push(WorkspaceState {
            item: WorkspaceItem {
                id: "s3".into(),
                name: Some("new".into()),
                shown: true,
                tabs: 1,
                attention: None,
                recency: 0,
                active_tab: "t4".into(),
            },
            tabs: vec![tab("t4", vec![vec![pane("p5", "s6")]], Vec::new())],
        });
        (before, after)
    }

    #[test]
    fn the_projection_takes_each_workspace_its_active_tab_queue_and_busy() {
        let mut snapshots = window(
            "s2",
            vec![
                (
                    "s1",
                    None,
                    vec![tab("t1", vec![vec![pane("p1", "s1")]], Vec::new())],
                ),
                ("s2", Some("side"), {
                    let mut tabs = vec![
                        tab("t2", vec![vec![pane("p2", "s2")]], Vec::new()),
                        tab("t3", vec![vec![pane("p3", "s3")]], Vec::new()),
                    ];
                    tabs[1].columns[0].panes[0].attention = Some(attention(State::Error, false));
                    tabs
                }),
            ],
            vec![QueueEntry {
                workspace: "s2".into(),
                tab: "t3".into(),
                target: "p3".into(),
                attention: attention(State::Error, false),
            }],
        );
        snapshots[1].active_tab = "t3".into();
        let busy = vec![busy_terminal("s2", "t3", "p3", "s3")];
        let projection = Projection::new(&snapshots, busy.clone());

        let items: Vec<_> = projection.workspaces.iter().map(|w| &w.item).collect();
        assert_eq!(
            items,
            [
                &WorkspaceItem {
                    id: "s1".into(),
                    name: None,
                    shown: false,
                    tabs: 1,
                    attention: Some(State::Working),
                    recency: 0,
                    active_tab: "t1".into(),
                },
                &WorkspaceItem {
                    id: "s2".into(),
                    name: Some("side".into()),
                    shown: true,
                    tabs: 2,
                    attention: None,
                    recency: 1,
                    active_tab: "t3".into(),
                },
            ]
        );
        assert_eq!(projection.workspaces[1].tabs, snapshots[1].tabs);
        assert_eq!(
            projection.queue,
            [queue_item("s2", "t3", "p3", State::Error)]
        );
        assert_eq!(projection.busy, busy);
    }

    #[test]
    fn a_turn_orders_creations_attention_closes_selections_then_the_queue() {
        let (before, after) = busy_turn();
        assert_eq!(
            described(&before.diff(&after)),
            [
                "session-renamed s1",
                "pane-moved p1",
                "pane-moved p2",
                "pane-renamed p2",
                "session-created s3",
                "tab-created t4",
                "pane-created p5",
                "attention s1",
                "pane-closed p3",
                "tab-closed t2",
                "pane-selected p2",
                "pane-selected p5",
                "tab-activated t4",
                "session-shown s3",
                "queue",
            ]
        );
        assert!(after.diff(&after).is_empty());
        assert_replays(&unfiltered(&before), &before, &after);

        // Events carry the objects as they are after them.
        let events = before.diff(&after);
        let Event::Pane(PaneChange::Selected, selected) = &events[10] else {
            panic!("{:?}", events[10]);
        };
        assert_eq!(
            (selected.workspace.as_str(), selected.tab.as_str()),
            ("s1", "t1")
        );
        assert_eq!((selected.column, selected.expanded), (1, true));
        assert_eq!(selected.pane.label, "build");
        assert_eq!(selected.columns, columns(&[(&["p1", "p2"], "p2")]));
        let Event::Pane(PaneChange::Renamed, renamed) = &events[3] else {
            panic!("{:?}", events[3]);
        };
        assert_eq!(renamed.columns, columns(&[(&["p1", "p2"], "p2")]));
        let Event::Tab(TabChange::Closed, closed) = &events[9] else {
            panic!("{:?}", events[9]);
        };
        assert_eq!(closed.workspace, "s1");
        assert_eq!(closed.tab.columns[0].panes[0].id, "p3");
        // Creations, moves and closes carry the order after them.
        assert_eq!(closed.tabs, Some(vec!["t1".to_owned()]));
        let Event::Pane(PaneChange::Closed, closed) = &events[8] else {
            panic!("{:?}", events[8]);
        };
        assert!(closed.columns.is_empty());
        let Event::Tab(TabChange::Created, created) = &events[5] else {
            panic!("{:?}", events[5]);
        };
        assert_eq!(created.tabs, Some(vec!["t4".to_owned()]));
        let Event::Tab(TabChange::Activated, activated) = &events[12] else {
            panic!("{:?}", events[12]);
        };
        assert_eq!(activated.tabs, None);
        let Event::Queue { queue, previous } = &events[14] else {
            panic!("{:?}", events[14]);
        };
        assert_eq!((queue, previous), (&after.queue, &before.queue));
    }

    #[test]
    fn attention_is_one_event_per_terminal_per_turn_with_its_previous_state() {
        let before = base();
        let mut after = before.clone();
        // A poll reply and the stale threshold in one turn coalesce.
        pane_mut(&mut after, "p1").attention = Some(attention(State::Working, true));
        pane_mut(&mut after, "p4").attention = Some(attention(State::Finished, false));
        let events = before.diff(&after);
        assert_eq!(described(&events), ["attention s1", "attention s5"]);
        assert_eq!(
            events[0],
            Event::Attention {
                target: TerminalTarget {
                    workspace: "s1".into(),
                    tab: "t1".into(),
                    id: TargetId::Pane("p1".into()),
                    terminal: "s1".into(),
                },
                previous: None,
                attention: Some(attention(State::Working, true)),
            }
        );

        // Staleness alone changes a field, and so does leaving attention.
        let mut later = after.clone();
        pane_mut(&mut later, "p1").attention = Some(attention(State::Working, false));
        pane_mut(&mut later, "p4").attention = None;
        let events = after.diff(&later);
        assert_eq!(described(&events), ["attention s1", "attention s5"]);
        assert!(matches!(
            &events[1],
            Event::Attention {
                previous: Some(State::Finished),
                attention: None,
                ..
            }
        ));
        assert_replays(&unfiltered(&after), &after, &later);
    }

    #[test]
    fn busy_reports_a_terminal_becoming_busy_renamed_or_idle_with_its_name() {
        let before = base();
        let mut busy = before.clone();
        busy.busy = vec![busy_terminal("s1", "t1", "p1", "s1")];
        let events = before.diff(&busy);
        assert_eq!(
            events,
            [Event::Busy {
                target: busy.busy[0].target.clone(),
                busy: true,
                name: Some("p1".into()),
            }]
        );
        // Its name is the pane's label, so a rename reports it again.
        let mut renamed = busy.clone();
        pane_mut(&mut renamed, "p1").label = "build".into();
        renamed.busy[0].name = "build".into();
        assert_eq!(
            described(&busy.diff(&renamed)),
            ["pane-renamed p1", "busy s1"]
        );
        let mut idle = renamed.clone();
        idle.busy.clear();
        assert_eq!(
            renamed.diff(&idle),
            [Event::Busy {
                target: busy.busy[0].target.clone(),
                busy: false,
                name: None,
            }]
        );
        for (from, to) in [(&before, &busy), (&busy, &renamed), (&renamed, &idle)] {
            assert_replays(&unfiltered(from), from, to);
        }
    }

    #[test]
    fn closes_cascade_and_vanished_terminals_drop_attention_and_busy() {
        let mut before = base();
        pane_mut(&mut before, "p4").attention = Some(attention(State::NeedsInput, false));
        before.busy = vec![busy_terminal("s2", "t3", "p4", "s5")];
        before.queue = vec![queue_item("s2", "t3", "p4", State::NeedsInput)];
        let mut after = before.clone();
        after
            .workspaces
            .retain(|workspace| workspace.item.id != "s2");
        after.busy.clear();
        after.queue.clear();
        // p3's terminal ends; its pane stays until closed.
        pane_mut(&mut after, "p3").live = false;
        // u1 is shown; a popup that later closes is never hidden.
        tab_mut(&mut after, "t1").selected_popup = Some("u1".into());
        let events = before.diff(&after);
        assert_eq!(
            described(&events),
            [
                "pane-ended p3",
                "attention s5",
                "busy s5",
                "pane-closed p4",
                "tab-closed t3",
                "session-closed s2",
                "popup-shown u1",
                "queue",
            ]
        );
        assert!(matches!(
            &events[2],
            Event::Busy {
                busy: false,
                name: None,
                ..
            }
        ));
        assert_replays(&unfiltered(&before), &before, &after);

        let mut closed = after.clone();
        tab_mut(&mut closed, "t1").popups.clear();
        tab_mut(&mut closed, "t1").selected_popup = None;
        assert_eq!(described(&after.diff(&closed)), ["popup-closed u1"]);
        assert_replays(&unfiltered(&after), &after, &closed);
        let mut hidden = after.clone();
        tab_mut(&mut hidden, "t1").selected_popup = None;
        assert_eq!(described(&after.diff(&hidden)), ["popup-hidden u1"]);
        let mut opened = hidden.clone();
        tab_mut(&mut opened, "t2")
            .popups
            .push(popup("u3", "git", "s8"));
        assert_eq!(described(&hidden.diff(&opened)), ["popup-created u3"]);
        assert_replays(&unfiltered(&hidden), &hidden, &opened);
    }

    #[test]
    fn a_parked_popup_moves_its_attention_and_busy_to_its_pane() {
        let mut before = base();
        let u1 = &mut tab_mut(&mut before, "t1").popups[0];
        u1.attention = Some(attention(State::Working, false));
        before.busy = vec![BusyTerminal {
            target: TerminalTarget {
                workspace: "s1".into(),
                tab: "t1".into(),
                id: TargetId::Popup("u1".into()),
                terminal: "s3".into(),
            },
            name: "Agent".into(),
        }];
        let mut parked = before.clone();
        let t1 = tab_mut(&mut parked, "t1");
        t1.popups.clear();
        let mut p6 = pane("p6", "s3");
        p6.attention = Some(attention(State::Working, false));
        t1.columns.push(Column {
            expanded: "p6".into(),
            panes: vec![p6],
        });
        parked.busy = vec![busy_terminal("s1", "t1", "p6", "s3")];
        let events = before.diff(&parked);
        assert_eq!(
            described(&events),
            [
                "pane-created p6",
                "attention s3",
                "busy s3",
                "popup-closed u1"
            ]
        );
        assert!(matches!(
            &events[1],
            Event::Attention {
                target: TerminalTarget { id: TargetId::Pane(id), .. },
                previous: Some(State::Working),
                ..
            } if id == "p6"
        ));
        assert_replays(&unfiltered(&before), &before, &parked);
    }

    #[test]
    fn moves_report_a_pane_column_expansion_or_place_and_a_tab_order() {
        let before = base();
        // p2 leaves p1's column for a new second column.
        let mut after = before.clone();
        let t1 = tab_mut(&mut after, "t1");
        let p2 = t1.columns[0].panes.remove(1);
        t1.columns.push(Column {
            expanded: p2.id.clone(),
            panes: vec![p2],
        });
        assert_eq!(described(&before.diff(&after)), ["pane-moved p2"]);
        let events = before.diff(&after);
        let Event::Pane(_, moved) = &events[0] else {
            panic!("{events:?}");
        };
        assert_eq!((moved.column, moved.expanded), (2, true));
        assert_eq!(moved.columns, columns(&[(&["p1"], "p1"), (&["p2"], "p2")]));
        assert_replays(&unfiltered(&before), &before, &after);

        // A new column at the left edge moves every pane of the tab.
        let mut left = before.clone();
        let t1 = tab_mut(&mut left, "t1");
        let p2 = t1.columns[0].panes.remove(1);
        t1.columns.insert(
            0,
            Column {
                expanded: p2.id.clone(),
                panes: vec![p2],
            },
        );
        let events = before.diff(&left);
        assert_eq!(described(&events), ["pane-moved p2", "pane-moved p1"]);
        let Event::Pane(_, moved) = &events[1] else {
            panic!("{events:?}");
        };
        assert_eq!((moved.column, moved.expanded), (2, true));
        assert_eq!(moved.columns, columns(&[(&["p2"], "p2"), (&["p1"], "p1")]));
        assert_replays(&unfiltered(&before), &before, &left);

        // Closing p1's column renumbers p2's, a move, though p2 keeps its
        // place among the surviving panes.
        let mut closed = after.clone();
        tab_mut(&mut closed, "t1").columns.remove(0);
        tab_mut(&mut closed, "t1").selected_pane = Some("p2".into());
        assert_eq!(
            described(&after.diff(&closed)),
            ["pane-moved p2", "pane-closed p1", "pane-selected p2"]
        );
        assert_replays(&unfiltered(&after), &after, &closed);

        // Tabs reorder within a workspace; a closed neighbor moves none, as
        // the busy turn shows. A directory change also reports the end of a
        // pending tab.
        let mut reordered = before.clone();
        workspace_mut(&mut reordered, "s1").tabs.reverse();
        tab_mut(&mut reordered, "t2").directory = b"/work".to_vec();
        let events = before.diff(&reordered);
        assert_eq!(
            described(&events),
            ["tab-moved t2", "tab-directory t2", "tab-moved t1"]
        );
        let Event::Tab(_, moved) = &events[0] else {
            panic!("{events:?}");
        };
        assert_eq!(moved.tabs, Some(vec!["t2".to_owned(), "t1".to_owned()]));
        let Event::Tab(_, directory) = &events[1] else {
            panic!("{events:?}");
        };
        assert_eq!(directory.tabs, None);
        assert_replays(&unfiltered(&before), &before, &reordered);
        let mut pending = before.clone();
        tab_mut(&mut pending, "t3").pending = true;
        assert_eq!(described(&pending.diff(&before)), ["tab-directory t3"]);

        // A tab that changes workspace moves into the other's order.
        let mut crossed = before.clone();
        let t2 = workspace_mut(&mut crossed, "s1").tabs.remove(1);
        workspace_mut(&mut crossed, "s2").tabs.insert(0, t2);
        let events = before.diff(&crossed);
        assert_eq!(described(&events), ["tab-moved t2"]);
        let Event::Tab(_, moved) = &events[0] else {
            panic!("{events:?}");
        };
        assert_eq!(
            (moved.workspace.as_str(), moved.tabs.clone()),
            ("s2", Some(vec!["t2".to_owned(), "t3".to_owned()]))
        );
        assert_replays(&unfiltered(&before), &before, &crossed);
    }

    #[test]
    fn showing_and_activating_existing_workspaces_and_tabs() {
        let before = base();
        let mut switched = before.clone();
        workspace_mut(&mut switched, "s1").item.shown = false;
        workspace_mut(&mut switched, "s2").item.shown = true;
        workspace_mut(&mut switched, "s1").item.active_tab = "t2".into();
        let events = before.diff(&switched);
        assert_eq!(described(&events), ["tab-activated t2", "session-shown s2"]);
        assert!(matches!(
            &events[1],
            Event::Workspace(WorkspaceChange::Shown { hidden: Some(hidden) }, _) if hidden == "s1"
        ));
        assert_replays(&unfiltered(&before), &before, &switched);
        // A stream of s1 alone, a change feed for s1, learns that s1 is
        // hidden.
        let s1 = resolve(
            FilterSpec {
                workspaces: vec!["s1".into()],
                ..FilterSpec::default()
            },
            &before,
        );
        assert_eq!(
            admitted(&s1, &events),
            ["tab-activated t2", "session-shown s2"]
        );
    }

    #[test]
    fn the_log_numbers_events_and_resumes_only_within_its_last_1024() {
        let (before, after) = busy_turn();
        let mut events = Events::new("g1-0123");
        let stream = events.stream().to_owned();
        assert!(stream.starts_with("g1-0123-"));
        assert!(events.publish(before.clone(), |_| 0).is_empty());
        assert_eq!(events.seq(), 0);
        assert_eq!(events.resume(&stream, 0).unwrap().count(), 0);

        let published = events.publish(after.clone(), |_| 0);
        assert_eq!(published.len(), 15);
        assert_eq!(
            published.iter().map(|event| event.seq).collect::<Vec<_>>(),
            (1..=15).collect::<Vec<_>>()
        );
        assert_eq!(events.projection(), Some(&after));
        assert_eq!(events.seq(), 15);
        let resumed: Vec<_> = events.resume(&stream, 13).unwrap().cloned().collect();
        assert_eq!(resumed, published[13..]);
        // The log and the subscribers share each event.
        assert!(Arc::ptr_eq(&resumed[0].event, &published[13].event));
        assert!(events.resume(&format!("{stream}0"), 13).is_none());
        assert!(events.resume(&stream, 16).is_none());

        // An unchanged turn publishes nothing.
        assert!(events.publish(after.clone(), |_| 0).is_empty());
        // Each toggle of p1's attention is one event.
        let mut toggled = after.clone();
        pane_mut(&mut toggled, "p1").attention = None;
        for turn in 0..1100 {
            let next = if turn % 2 == 0 { &toggled } else { &after };
            assert_eq!(events.publish(next.clone(), |_| 0).len(), 1);
        }
        assert_eq!(events.seq(), 1115);
        let oldest = events.resume(&stream, 1115 - 1024).unwrap();
        assert_eq!(oldest.map(|event| event.seq).next(), Some(1115 - 1023));
        assert!(events.resume(&stream, 1115 - 1025).is_none());
        assert!(events.resume(&stream, 13).is_none());
    }

    #[test]
    fn the_log_keeps_at_most_8_mib_of_events_and_resumes_only_within_them() {
        let (_, after) = busy_turn();
        let mut toggled = after.clone();
        pane_mut(&mut toggled, "p1").attention = None;
        let mut events = Events::new("g1-0123");
        let stream = events.stream().to_owned();
        events.publish(after.clone(), |_| 0);
        // Events of 1 MiB each: the log keeps the last 8.
        for turn in 0..20 {
            let next = if turn % 2 == 0 { &toggled } else { &after };
            assert_eq!(events.publish(next.clone(), |_| 1024 * 1024).len(), 1);
        }
        assert_eq!(events.seq(), 20);
        assert_eq!(events.resume(&stream, 12).unwrap().count(), 8);
        assert!(events.resume(&stream, 11).is_none());

        // One event of 3 MiB pushes out the three oldest; small ones then
        // fill toward the count bound without passing the byte bound.
        events.publish(toggled.clone(), |_| 3 * 1024 * 1024);
        assert_eq!(events.resume(&stream, 15).unwrap().count(), 6);
        assert!(events.resume(&stream, 14).is_none());
        for turn in 0..2000 {
            let next = if turn % 2 == 0 { &after } else { &toggled };
            events.publish(next.clone(), |_| 100);
        }
        assert_eq!(events.seq(), 2021);
        assert_eq!(events.resume(&stream, 2021 - 1024).unwrap().count(), 1024);
        assert!(events.resume(&stream, 2021 - 1025).is_none());

        // A pause forgets the weights with the events.
        events.pause();
        events.publish(after.clone(), |_| 0);
        for turn in 0..8 {
            let next = if turn % 2 == 0 { &toggled } else { &after };
            events.publish(next.clone(), |_| 1024 * 1024);
        }
        assert_eq!(events.resume(&stream, 2022).unwrap().count(), 8);
    }

    #[test]
    fn each_event_source_is_its_own_stream() {
        // Two supervisors of one generation, even started together.
        let (first, second) = (Events::new("g1-0123"), Events::new("g1-0123"));
        assert_ne!(first.stream(), second.stream());
        let mut first = first;
        first.publish(base(), |_| 0);
        assert!(second.resume(first.stream(), 0).is_none());
    }

    #[test]
    fn a_pause_ends_every_resume_that_spans_it() {
        let (before, after) = busy_turn();
        let mut events = Events::new("g1-0123");
        let stream = events.stream().to_owned();
        events.publish(before.clone(), |_| 0);
        events.publish(after.clone(), |_| 0);
        let cursor = events.seq();
        // The last subscriber leaves; p1 asks and is answered unobserved.
        events.pause();
        assert_eq!(events.projection(), None);
        assert!(events.resume(&stream, cursor).is_none());
        // The window ends where it was, so a diff would find nothing.
        assert!(events.publish(after.clone(), |_| 0).is_empty());
        assert!(events.resume(&stream, cursor).is_none());
        let fresh = events.seq();
        assert_eq!(fresh, cursor + 1);
        assert_eq!(events.resume(&stream, fresh).unwrap().count(), 0);
        let mut later = after.clone();
        pane_mut(&mut later, "p4").label = "tests".into();
        assert_eq!(events.publish(later, |_| 0).len(), 1);
        assert_eq!(events.resume(&stream, fresh).unwrap().count(), 1);

        // Pausing before any publish, or again before the next, changes
        // nothing more.
        let mut idle = Events::new("g1-0123");
        idle.pause();
        idle.publish(before, |_| 0);
        assert_eq!(idle.seq(), 0);
        idle.pause();
        idle.pause();
        assert_eq!(idle.seq(), 1);
    }

    #[test]
    fn filters_and_across_flags_or_within_and_need_a_listed_value() {
        let (before, after) = busy_turn();
        let events = before.diff(&after);
        let all = described(&events);
        assert_eq!(admitted(&unfiltered(&before), &events), all);
        let spec = |f: fn(&mut FilterSpec)| {
            let mut spec = FilterSpec::default();
            f(&mut spec);
            resolve(spec, &before)
        };

        let panes = spec(|s| s.kinds = vec![Kind::Pane, Kind::Popup]);
        assert_eq!(
            admitted(&panes, &events),
            [
                "pane-moved p1",
                "pane-moved p2",
                "pane-renamed p2",
                "pane-created p5",
                "pane-closed p3",
                "pane-selected p2",
                "pane-selected p5"
            ]
        );
        // p5 is in s3, which did not exist when subscribing. Showing s3 hid
        // s1, and the queue is about s1, where p1 entered it.
        let s1 = spec(|s| s.workspaces = vec!["main".into(), "s2".into()]);
        assert_eq!(
            admitted(&s1, &events),
            [
                "session-renamed s1",
                "pane-moved p1",
                "pane-moved p2",
                "pane-renamed p2",
                "attention s1",
                "pane-closed p3",
                "tab-closed t2",
                "pane-selected p2",
                "session-shown s3",
                "queue",
            ]
        );
        let queue = spec(|s| {
            s.workspaces = vec!["s1".into()];
            s.kinds = vec![Kind::Queue];
        });
        assert_eq!(admitted(&queue, &events), ["queue"]);
        let other_queue = spec(|s| {
            s.workspaces = vec!["s2".into()];
            s.kinds = vec![Kind::Queue];
        });
        assert!(admitted(&other_queue, &events).is_empty());
        let t1 = spec(|s| s.tabs = vec!["t1".into()]);
        assert_eq!(
            admitted(&t1, &events),
            [
                "pane-moved p1",
                "pane-moved p2",
                "pane-renamed p2",
                "attention s1",
                "pane-selected p2"
            ]
        );
        // Workspace and queue events pass --tab and --target by --kind.
        let t1_workspaces = spec(|s| {
            s.tabs = vec!["t1".into()];
            s.kinds = vec![Kind::Workspace, Kind::Attention, Kind::Queue];
        });
        assert_eq!(
            admitted(&t1_workspaces, &events),
            [
                "session-renamed s1",
                "session-created s3",
                "attention s1",
                "session-shown s3",
                "queue"
            ]
        );
        // A tab event is about its tab's terminals.
        let p3 = spec(|s| s.targets = vec!["p3".into()]);
        assert_eq!(admitted(&p3, &events), ["pane-closed p3", "tab-closed t2"]);

        // --state admits attention by its new or previous state and a queue
        // by its entries' states; --kind never lifts it.
        let mut answered = after.clone();
        pane_mut(&mut answered, "p1").attention = Some(attention(State::Working, false));
        pane_mut(&mut answered, "p4").attention = Some(attention(State::Idle, false));
        let answered = after.diff(&answered);
        let needs_input = spec(|s| s.states = vec![State::NeedsInput, State::Error]);
        assert_eq!(admitted(&needs_input, &events), ["attention s1", "queue"]);
        assert_eq!(admitted(&needs_input, &answered), ["attention s1"]);
        let with_panes = spec(|s| {
            s.states = vec![State::Idle];
            s.kinds = vec![Kind::Attention, Kind::Pane];
        });
        assert!(admitted(&with_panes, &events).is_empty());
        assert_eq!(admitted(&with_panes, &answered), ["attention s5"]);
    }

    #[test]
    fn subscribing_resolves_workspaces_once_and_targets_by_terminal() {
        let mut projection = base();
        workspace_mut(&mut projection, "s2").item.name = Some("s1".into());
        let resolved = |projection: &Projection, selector: &str| {
            Filter::resolve(
                &FilterSpec {
                    workspaces: vec![selector.into()],
                    ..FilterSpec::default()
                },
                None,
                projection,
            )
            .map(|filter| filter.workspaces)
        };
        // An exact sN first, then an exact name.
        assert_eq!(resolved(&projection, "s1"), Ok(vec!["s1".to_owned()]));
        assert_eq!(resolved(&projection, "main"), Ok(vec!["s1".to_owned()]));
        assert_eq!(
            resolved(&projection, "s9").unwrap_err().code,
            "unknown-session"
        );
        workspace_mut(&mut projection, "s1").item.name = Some("twin".into());
        workspace_mut(&mut projection, "s2").item.name = Some("twin".into());
        assert_eq!(
            resolved(&projection, "twin").unwrap_err().code,
            "ambiguous-session"
        );
        let missing =
            |spec: FilterSpec| Filter::resolve(&spec, None, &projection).unwrap_err().code;
        assert_eq!(
            missing(FilterSpec {
                tabs: vec!["t9".into()],
                ..FilterSpec::default()
            }),
            "unknown-tab"
        );
        assert_eq!(
            missing(FilterSpec {
                targets: vec!["p9".into()],
                ..FilterSpec::default()
            }),
            "unknown-target"
        );

        // A renamed workspace stays admitted.
        let before = base();
        let main = resolve(
            FilterSpec {
                workspaces: vec!["main".into()],
                ..FilterSpec::default()
            },
            &before,
        );
        let mut renamed = before.clone();
        workspace_mut(&mut renamed, "s1").item.name = Some("main2".into());
        workspace_mut(&mut renamed, "s2").item.name = Some("main".into());
        assert_eq!(
            admitted(&main, &before.diff(&renamed)),
            ["session-renamed s1"]
        );

        // The agent popup u1 parked as pane p6 stays admitted as that pane.
        let agent = resolve(
            FilterSpec {
                targets: vec!["u1".into()],
                ..FilterSpec::default()
            },
            &before,
        );
        let mut parked = before.clone();
        let t1 = tab_mut(&mut parked, "t1");
        t1.popups.clear();
        t1.columns[0].panes.push(pane("p6", "s3"));
        pane_mut(&mut parked, "p6").attention = Some(attention(State::Idle, false));
        assert_eq!(
            admitted(&agent, &before.diff(&parked)),
            ["pane-created p6", "attention s3", "popup-closed u1"]
        );
        assert!(!agent.gone(&parked));
        assert_eq!(
            agent.snapshot(&parked).tabs,
            [WorkspaceTabs {
                workspace: "s1".into(),
                tabs: vec![parked.workspaces[0].tabs[0].clone()],
            }]
        );
        let mut ended = parked.clone();
        pane_mut(&mut ended, "p6").live = false;
        assert!(agent.gone(&ended));
    }

    #[test]
    fn the_stream_ends_once_every_named_workspace_tab_and_target_is_gone() {
        let before = base();
        let named = resolve(
            FilterSpec {
                workspaces: vec!["side".into()],
                tabs: vec!["t1".into()],
                ..FilterSpec::default()
            },
            &before,
        );
        assert!(!named.gone(&before));
        let mut closed = before.clone();
        closed
            .workspaces
            .retain(|workspace| workspace.item.id != "s2");
        assert!(!named.gone(&closed));
        workspace_mut(&mut closed, "s1")
            .tabs
            .retain(|tab| tab.id != "t1");
        assert!(named.gone(&closed));

        let popup = resolve(
            FilterSpec {
                targets: vec!["u1".into(), "p3".into()],
                ..FilterSpec::default()
            },
            &before,
        );
        let mut dismissed = before.clone();
        tab_mut(&mut dismissed, "t1").popups.clear();
        assert!(!popup.gone(&dismissed));
        workspace_mut(&mut dismissed, "s1")
            .tabs
            .retain(|tab| tab.id != "t2");
        assert!(popup.gone(&dismissed));

        let unnamed = resolve(
            FilterSpec {
                kinds: vec![Kind::Attention],
                ..FilterSpec::default()
            },
            &before,
        );
        assert!(!unnamed.gone(&closed));
    }

    #[test]
    fn snapshots_name_workspaces_and_tabs_and_admitted_busy_without_the_caller() {
        let mut projection = base();
        projection.queue = vec![queue_item("s2", "t3", "p4", State::Finished)];
        projection.busy = vec![
            busy_terminal("s1", "t1", "p1", "s1"),
            busy_terminal("s1", "t1", "p2", "s2"),
            busy_terminal("s2", "t3", "p4", "s5"),
        ];
        let filter = Filter::resolve(
            &FilterSpec {
                workspaces: vec!["s1".into()],
                ..FilterSpec::default()
            },
            Some("s1"),
            &projection,
        )
        .unwrap();
        let snapshot = filter.snapshot(&projection);
        assert_eq!(snapshot.workspaces, [projection.workspaces[0].item.clone()]);
        assert_eq!(snapshot.workspaces[0].active_tab, "t1");
        assert_eq!(
            snapshot.tabs,
            [WorkspaceTabs {
                workspace: "s1".into(),
                tabs: projection.workspaces[0].tabs.clone(),
            }]
        );
        // Without a workspace filter, every workspace is its own group.
        let unscoped = unfiltered(&projection).snapshot(&projection);
        assert_eq!(
            unscoped
                .tabs
                .iter()
                .map(|group| (group.workspace.as_str(), group.tabs.len()))
                .collect::<Vec<_>>(),
            [("s1", 2), ("s2", 1)]
        );
        // The window's queue; the admitted busy terminals.
        assert_eq!(snapshot.attention_queue, projection.queue);
        assert_eq!(snapshot.busy, [busy_terminal("s1", "t1", "p2", "s2")]);

        // Busy events never report the caller.
        let mut idle = projection.clone();
        idle.busy.clear();
        assert_eq!(admitted(&filter, &projection.diff(&idle)), ["busy s2"]);
        let caller = Filter::resolve(&FilterSpec::default(), Some("s1"), &projection).unwrap();
        assert_eq!(
            admitted(&caller, &projection.diff(&idle)),
            ["busy s2", "busy s5"]
        );
        assert_replays(&caller, &projection, &idle);

        // EONW's 32-terminal bound does not apply.
        projection.busy = (0..34)
            .map(|index| busy_terminal("s1", "t1", "p1", &format!("s{index}")))
            .collect();
        assert_eq!(filter.snapshot(&projection).busy.len(), 33);
    }

    /// A small deterministic generator (xorshift64).
    struct Random(u64);

    impl Random {
        fn below(&mut self, bound: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % bound as u64) as usize
        }

        fn chance(&mut self, percent: usize) -> bool {
            self.below(100) < percent
        }
    }

    /// Random window changes of every kind a supervisor makes, keeping the
    /// projection's invariants.
    struct Script {
        random: Random,
        next: usize,
    }

    impl Script {
        fn id(&mut self, prefix: &str) -> String {
            self.next += 1;
            format!("{prefix}{}", self.next)
        }

        fn new_pane(&mut self) -> eonw::Pane {
            let id = self.id("p");
            let session = self.id("terminal-");
            pane(&id, &session)
        }

        fn step(&mut self, projection: &mut Projection) {
            let r = &mut self.random;
            let w = r.below(projection.workspaces.len());
            let t = r.below(projection.workspaces[w].tabs.len());
            match r.below(25) {
                0 => {
                    let pane = self.new_pane();
                    let id = self.id("t");
                    projection.workspaces[w]
                        .tabs
                        .push(tab(&id, vec![vec![pane]], Vec::new()));
                }
                1 => {
                    projection.workspaces[w].tabs.remove(t);
                }
                2 => {
                    let tabs = &mut projection.workspaces[w].tabs;
                    let other = self.random.below(tabs.len());
                    tabs.swap(t, other);
                }
                3 if projection.workspaces.len() > 1 => {
                    let moved = projection.workspaces[w].tabs.remove(t);
                    let to = (w + 1) % projection.workspaces.len();
                    let at = self.random.below(projection.workspaces[to].tabs.len() + 1);
                    projection.workspaces[to].tabs.insert(at, moved);
                }
                4 | 5 => {
                    // A new pane in a new column at either edge.
                    let pane = self.new_pane();
                    let left = self.random.chance(50);
                    let tab = &mut projection.workspaces[w].tabs[t];
                    tab.selected_pane = Some(pane.id.clone());
                    let column = Column {
                        expanded: pane.id.clone(),
                        panes: vec![pane],
                    };
                    if left {
                        tab.columns.insert(0, column);
                    } else {
                        tab.columns.push(column);
                    }
                }
                6 | 22..=24 => {
                    let pane = self.new_pane();
                    let tab = &mut projection.workspaces[w].tabs[t];
                    if let Some(column) = tab.columns.first_mut() {
                        column.panes.insert(0, pane);
                    }
                }
                7 => {
                    // The selected pane moves sideways, joining a neighbor
                    // or starting a new edge column.
                    let side = self.random.below(3);
                    let tab = &mut projection.workspaces[w].tabs[t];
                    let Some(selected) = tab.selected_pane.clone() else {
                        return;
                    };
                    let Some((c, p)) = tab.columns.iter().enumerate().find_map(|(c, column)| {
                        column
                            .panes
                            .iter()
                            .position(|pane| pane.id == selected)
                            .map(|p| (c, p))
                    }) else {
                        return;
                    };
                    let pane = tab.columns[c].panes.remove(p);
                    let column = Column {
                        expanded: pane.id.clone(),
                        panes: vec![pane.clone()],
                    };
                    match side {
                        0 => tab.columns.insert(0, column),
                        1 => tab.columns.push(column),
                        _ => {
                            let to = (c + 1) % tab.columns.len();
                            tab.columns[to].panes.push(pane);
                            tab.columns[to].expanded = selected;
                        }
                    }
                }
                8 => {
                    let tab = &mut projection.workspaces[w].tabs[t];
                    if let Some(column) = tab.columns.last_mut() {
                        column.panes.pop();
                    }
                }
                9 => {
                    for pane in projection.workspaces[w].tabs[t]
                        .columns
                        .iter_mut()
                        .flat_map(|c| &mut c.panes)
                        .take(1)
                    {
                        pane.live = false;
                    }
                }
                10 => {
                    let label = self.id("label");
                    for pane in projection.workspaces[w].tabs[t]
                        .columns
                        .iter_mut()
                        .flat_map(|c| &mut c.panes)
                        .take(1)
                    {
                        pane.label = label.clone();
                    }
                }
                11 => {
                    let tab = &mut projection.workspaces[w].tabs[t];
                    let panes: Vec<_> = tab.panes().map(|pane| pane.id.clone()).collect();
                    if !panes.is_empty() {
                        let chosen = panes[self.random.below(panes.len())].clone();
                        tab.selected_pane = Some(chosen);
                    }
                }
                12 => {
                    let id = self.id("u");
                    let session = self.id("terminal-");
                    let show = self.random.chance(50);
                    let tab = &mut projection.workspaces[w].tabs[t];
                    tab.popups.push(popup(&id, "agent", &session));
                    if show {
                        tab.selected_popup = Some(id);
                    }
                }
                13 => {
                    let tab = &mut projection.workspaces[w].tabs[t];
                    tab.selected_popup = match &tab.selected_popup {
                        Some(_) => None,
                        None => tab.popups.first().map(|popup| popup.id.clone()),
                    };
                }
                14 => {
                    projection.workspaces[w].tabs[t].popups.pop();
                }
                15 => {
                    // Park the first popup as a pane in a new column.
                    let id = self.id("p");
                    let tab = &mut projection.workspaces[w].tabs[t];
                    if !tab.popups.is_empty() {
                        let popup = tab.popups.remove(0);
                        let mut parked = pane(&id, &popup.session);
                        parked.attention = popup.attention;
                        tab.columns.push(Column {
                            expanded: id,
                            panes: vec![parked],
                        });
                    }
                }
                16 | 17 => {
                    let states = [
                        None,
                        Some(State::NeedsInput),
                        Some(State::Error),
                        Some(State::Finished),
                        Some(State::Working),
                        Some(State::Idle),
                    ];
                    let state = states[self.random.below(states.len())];
                    let stale = self.random.chance(30);
                    let tab = &mut projection.workspaces[w].tabs[t];
                    let attention = state.map(|state| attention(state, stale));
                    let count = tab.panes().count() + tab.popups.len();
                    let pick = self.random.below(count.max(1));
                    let panes = tab.columns.iter_mut().flat_map(|c| &mut c.panes);
                    let mut holders: Vec<&mut Option<eonw::Attention>> = panes
                        .map(|pane| &mut pane.attention)
                        .chain(tab.popups.iter_mut().map(|popup| &mut popup.attention))
                        .collect();
                    if let Some(slot) = holders.get_mut(pick) {
                        **slot = attention;
                    }
                }
                18 => {
                    // Busy comes and goes on several terminals, past EONW's
                    // 32 bound.
                    let sessions: Vec<String> = projection
                        .terminals()
                        .map(|terminal| terminal.session().to_owned())
                        .collect();
                    for _ in 0..8 {
                        let session = &sessions[self.random.below(sessions.len())];
                        if let Some(index) = projection
                            .busy
                            .iter()
                            .position(|b| b.target.terminal == *session)
                        {
                            if self.random.chance(25) {
                                projection.busy.remove(index);
                            }
                        } else {
                            projection.busy.push(BusyTerminal {
                                target: TerminalTarget {
                                    workspace: String::new(),
                                    tab: String::new(),
                                    id: TargetId::Pane(String::new()),
                                    terminal: session.clone(),
                                },
                                name: String::new(),
                            });
                        }
                    }
                }
                19 => {
                    let pane = self.new_pane();
                    let id = self.id("s");
                    let tab_id = self.id("t");
                    projection.workspaces.push(WorkspaceState {
                        item: WorkspaceItem {
                            id,
                            name: None,
                            shown: false,
                            tabs: 1,
                            attention: None,
                            recency: 0,
                            active_tab: tab_id.clone(),
                        },
                        tabs: vec![tab(&tab_id, vec![vec![pane]], Vec::new())],
                    });
                }
                20 => {
                    let show = self.random.chance(50);
                    let item = &mut projection.workspaces[w].item;
                    if show {
                        let shown = item.id.clone();
                        for workspace in &mut projection.workspaces {
                            workspace.item.shown = workspace.item.id == shown;
                        }
                    } else {
                        item.name = Some(self.id("name"));
                    }
                }
                _ => {
                    let directory = self.random.chance(50);
                    let workspace = &mut projection.workspaces[w];
                    let tab = &mut workspace.tabs[t];
                    if directory {
                        tab.directory = self.id("/d").into_bytes();
                    } else {
                        workspace.item.active_tab = tab.id.clone();
                    }
                }
            }
            settle(projection);
        }
    }

    /// Restores what a supervisor guarantees after a change: no empty
    /// column, tab or workspace, a selected pane that its column expands, a
    /// live chosen popup and active tab, one shown workspace, and the queue
    /// and busy terminals derived from the terminals.
    fn settle(projection: &mut Projection) {
        for workspace in &mut projection.workspaces {
            for tab in &mut workspace.tabs {
                tab.columns.retain(|column| !column.panes.is_empty());
                for column in &mut tab.columns {
                    if !column.panes.iter().any(|pane| pane.id == column.expanded) {
                        column.expanded = column.panes[0].id.clone();
                    }
                }
                let selected = tab
                    .selected_pane
                    .clone()
                    .filter(|id| tab.panes().any(|pane| pane.id == *id))
                    .or_else(|| tab.columns.first().map(|column| column.expanded.clone()));
                if let Some(id) = &selected {
                    for column in &mut tab.columns {
                        if column.panes.iter().any(|pane| pane.id == *id) {
                            column.expanded = id.clone();
                        }
                    }
                }
                tab.selected_pane = selected;
                if !tab
                    .popups
                    .iter()
                    .any(|popup| Some(&popup.id) == tab.selected_popup.as_ref())
                {
                    tab.selected_popup = None;
                }
            }
            workspace
                .tabs
                .retain(|tab| !tab.columns.is_empty() || !tab.popups.is_empty());
            if !workspace
                .tabs
                .iter()
                .any(|tab| tab.id == workspace.item.active_tab)
                && let Some(tab) = workspace.tabs.first()
            {
                workspace.item.active_tab = tab.id.clone();
            }
            workspace.item.tabs = workspace.tabs.len();
        }
        if projection.workspaces.len() > 1 {
            projection
                .workspaces
                .retain(|workspace| !workspace.tabs.is_empty());
        }
        if projection.workspaces[0].tabs.is_empty() {
            projection.workspaces[0]
                .tabs
                .push(tab("t0", vec![vec![pane("p0", "s0")]], Vec::new()));
            projection.workspaces[0].item.active_tab = "t0".into();
            projection.workspaces[0].item.tabs = 1;
        }
        if !projection.workspaces.iter().any(|w| w.item.shown) {
            projection.workspaces[0].item.shown = true;
        }
        let terminals: Vec<(TerminalTarget, String, Option<State>)> = projection
            .terminals()
            .map(|terminal| {
                let name = match terminal.holder {
                    Holder::Pane { pane, .. } => pane.label.clone(),
                    Holder::Popup(popup) => popup.entry.clone(),
                };
                (
                    terminal.target(),
                    name,
                    terminal.attention().map(|attention| attention.state),
                )
            })
            .collect();
        let busy: HashSet<String> = projection
            .busy
            .iter()
            .map(|terminal| terminal.target.terminal.clone())
            .collect();
        projection.busy = terminals
            .iter()
            .filter(|(target, ..)| busy.contains(&target.terminal))
            .map(|(target, name, _)| BusyTerminal {
                target: target.clone(),
                name: name.clone(),
            })
            .collect();
        let mut queue: Vec<_> = terminals
            .iter()
            .filter_map(|(target, _, state)| {
                let state = (*state)?;
                let id = match &target.id {
                    TargetId::Pane(id) | TargetId::Popup(id) => id,
                };
                (state <= State::Finished)
                    .then(|| queue_item(&target.workspace, &target.tab, id, state))
            })
            .collect();
        queue.sort_by_key(|item| item.state);
        projection.queue = queue;
    }

    /// What the v37 codec checks of a pane or popup event: but for a close,
    /// its `column` and `expanded` and its `chosen` agree with the order it
    /// carries, and only a close may carry the empty order of a gone tab.
    fn assert_placed(event: &Event) {
        match event {
            Event::Pane(PaneChange::Closed, _) => {}
            Event::Pane(change, item) => {
                let column = &item.columns[item.column - 1];
                assert!(column.panes.contains(&item.pane.id), "{change:?} {item:?}");
                assert_eq!(column.expanded == item.pane.id, item.expanded, "{item:?}");
            }
            Event::Popup(PopupChange::Shown, item) => assert!(item.chosen, "{item:?}"),
            Event::Popup(PopupChange::Hidden, item) => assert!(!item.chosen, "{item:?}"),
            _ => {}
        }
    }

    /// What a stream rebuilds, per kind it admits: the workspaces, their
    /// active tabs (with both `workspace` and `tab`), each workspace's tabs,
    /// each tab's panes and popups without their attention, each
    /// terminal's attention and place, the queue, and the busy terminals.
    #[derive(Debug, PartialEq)]
    struct Facts {
        workspaces: Option<Vec<(String, Option<String>, bool)>>,
        active_tabs: Option<Vec<(String, String)>>,
        tabs: Option<Vec<(String, Vec<TabFacts>)>>,
        panes: Option<BTreeMap<String, Selected<Column>>>,
        popups: Option<BTreeMap<String, Selected<eonw::Popup>>>,
        attention: Option<Attentions>,
        queue: Option<Vec<QueueItem>>,
        busy: Option<Vec<BusyTerminal>>,
    }

    /// A tab's ID, directory and whether it is pending.
    type TabFacts = (String, Vec<u8>, bool);

    /// A tab's columns or popups and the one it selects.
    type Selected<T> = (Vec<T>, Option<String>);

    /// Each terminal's attention with its tab and pane or popup.
    type Attentions = BTreeMap<String, (String, TargetId, eonw::Attention)>;

    fn attentions(state: &SnapshotState) -> Attentions {
        let mut attentions = Attentions::new();
        for tab in state.tabs.iter().flat_map(|group| &group.tabs) {
            let panes = tab.panes().map(|pane| {
                (
                    &pane.session,
                    TargetId::Pane(pane.id.clone()),
                    &pane.attention,
                )
            });
            let popups = tab.popups.iter().map(|popup| {
                (
                    &popup.session,
                    TargetId::Popup(popup.id.clone()),
                    &popup.attention,
                )
            });
            for (session, id, attention) in panes.chain(popups) {
                if let Some(attention) = attention {
                    attentions.insert(session.clone(), (tab.id.clone(), id, attention.clone()));
                }
            }
        }
        attentions
    }

    /// What a stream with `filter` rebuilds (EON-C39 "Applying events"):
    /// the kinds it admits, where `workspace` and `queue` count under
    /// `--tab` and `--target` only when `--kind` names them, and whether
    /// `--tab` leaves out workspace-wide values.
    struct Rebuilt {
        kinds: Vec<Kind>,
        tabbed: bool,
    }

    fn rebuilt(filter: &Filter) -> Rebuilt {
        let every = [
            Kind::Workspace,
            Kind::Tab,
            Kind::Pane,
            Kind::Popup,
            Kind::Attention,
            Kind::Queue,
            Kind::Busy,
        ];
        let named = filter.tabs.len() + filter.terminals.len() > 0;
        let kinds = every
            .into_iter()
            .filter(|kind| filter.kinds.is_empty() || filter.kinds.contains(kind))
            .filter(|kind| {
                !named
                    || !matches!(kind, Kind::Workspace | Kind::Queue)
                    || filter.kinds.contains(kind)
            })
            .collect();
        Rebuilt {
            kinds,
            tabbed: !filter.tabs.is_empty(),
        }
    }

    fn facts(state: &SnapshotState, attention: &Attentions, rebuilt: &Rebuilt) -> Facts {
        let has = |kind| rebuilt.kinds.contains(&kind);
        // A `--tab` stream keeps neither a session's active tab nor window order;
        // the full queue stays current when `--kind` explicitly names it.
        let tabbed = rebuilt.tabbed;
        let tabs = || state.tabs.iter().flat_map(|group| &group.tabs);
        let unattended = |mut pane: eonw::Pane| {
            pane.attention = None;
            pane
        };
        let mut busy = state.busy.clone();
        // Window order needs the workspaces, tabs, panes and popups; a stream
        // without one keeps the set, and its tab groups by workspace ID.
        let layout = [Kind::Workspace, Kind::Tab, Kind::Pane, Kind::Popup];
        let ordered = !tabbed && layout.iter().all(|kind| has(*kind));
        if !ordered {
            busy.sort_by(|a, b| a.target.terminal.cmp(&b.target.terminal));
        }
        Facts {
            workspaces: has(Kind::Workspace).then(|| {
                state
                    .workspaces
                    .iter()
                    .map(|w| (w.id.clone(), w.name.clone(), w.shown))
                    .collect()
            }),
            active_tabs: (has(Kind::Workspace) && has(Kind::Tab) && !tabbed).then(|| {
                state
                    .workspaces
                    .iter()
                    .map(|w| (w.id.clone(), w.active_tab.clone()))
                    .collect()
            }),
            tabs: has(Kind::Tab).then(|| {
                let mut groups = state
                    .tabs
                    .iter()
                    .map(|group| {
                        let tabs = group.tabs.iter();
                        let tabs = tabs.map(|t| (t.id.clone(), t.directory.clone(), t.pending));
                        (group.workspace.clone(), tabs.collect())
                    })
                    .collect::<Vec<_>>();
                if !ordered {
                    groups.sort();
                }
                groups
            }),
            panes: has(Kind::Pane).then(|| {
                tabs()
                    .filter(|tab| !tab.columns.is_empty())
                    .map(|tab| {
                        let columns = tab.columns.iter().map(|column| Column {
                            panes: column.panes.iter().cloned().map(unattended).collect(),
                            expanded: column.expanded.clone(),
                        });
                        (
                            tab.id.clone(),
                            (columns.collect(), tab.selected_pane.clone()),
                        )
                    })
                    .collect()
            }),
            popups: has(Kind::Popup).then(|| {
                tabs()
                    .filter(|tab| !tab.popups.is_empty())
                    .map(|tab| {
                        let popups = tab.popups.iter().cloned().map(|mut popup| {
                            popup.attention = None;
                            popup
                        });
                        (
                            tab.id.clone(),
                            (popups.collect(), tab.selected_popup.clone()),
                        )
                    })
                    .collect()
            }),
            attention: has(Kind::Attention).then(|| attention.clone()),
            queue: has(Kind::Queue).then(|| state.attention_queue.clone()),
            busy: has(Kind::Busy).then_some(busy),
        }
    }

    /// A `--tab t1 --kind workspace,tab` stream keeps the workspaces and t1
    /// current while another tab is activated, moved or closed, but not a
    /// workspace's `active_tab`: `tab-activated` for another tab does not
    /// pass `--tab`, and the consumer reads `eon session --json` for it.
    #[test]
    fn a_tab_stream_keeps_its_tab_current_but_not_the_active_tab() {
        let before = base();
        let filter = resolve(
            FilterSpec {
                tabs: vec!["t1".into()],
                kinds: vec![Kind::Workspace, Kind::Tab],
                ..FilterSpec::default()
            },
            &before,
        );
        let rebuilt = rebuilt(&filter);
        let mut state = filter.snapshot(&before);
        let mut turn = |from: &Projection, to: &Projection| {
            let events = from.diff(to);
            for event in events.iter().filter(|event| filter.admits(event)) {
                apply(&mut state, event);
            }
            let none = Attentions::new();
            let expected = filter.snapshot(to);
            assert_eq!(
                facts(&state, &none, &rebuilt),
                facts(&expected, &none, &rebuilt),
                "{:?}",
                described(&events)
            );
            let active = |state: &SnapshotState| state.workspaces[0].active_tab.clone();
            (
                admitted(&filter, &events),
                active(&state),
                active(&expected),
            )
        };

        // Switching s1 to t2.
        let mut switched = before.clone();
        workspace_mut(&mut switched, "s1").item.active_tab = "t2".into();
        assert_eq!(
            turn(&before, &switched),
            (Vec::new(), "t1".into(), "t2".into())
        );

        // Moving the active t2 to s2, so s1 activates t4.
        let mut grown = switched.clone();
        workspace_mut(&mut grown, "s1").tabs.push(tab(
            "t4",
            vec![vec![pane("p5", "s6")]],
            Vec::new(),
        ));
        assert_eq!(turn(&switched, &grown).0, Vec::<String>::new());
        let mut moved = grown.clone();
        let t2 = workspace_mut(&mut moved, "s1").tabs.remove(1);
        workspace_mut(&mut moved, "s2").tabs.insert(0, t2);
        workspace_mut(&mut moved, "s1").item.active_tab = "t4".into();
        assert_eq!(turn(&grown, &moved), (Vec::new(), "t1".into(), "t4".into()));

        // Closing the active t4 while t5 opens and is activated.
        let mut closed = moved.clone();
        let s1 = workspace_mut(&mut closed, "s1");
        s1.tabs.retain(|tab| tab.id != "t4");
        s1.tabs
            .push(tab("t5", vec![vec![pane("p6", "s7")]], Vec::new()));
        s1.item.active_tab = "t5".into();
        assert_eq!(
            turn(&moved, &closed),
            (Vec::new(), "t1".into(), "t5".into())
        );

        // Activating t1 itself passes, and the stream knows it again.
        let mut back = closed.clone();
        workspace_mut(&mut back, "s1").item.active_tab = "t1".into();
        assert_eq!(
            turn(&closed, &back),
            (vec!["tab-activated t1".into()], "t1".into(), "t1".into())
        );
    }

    /// EON-C39's "Applying events" over random sequences. An unfiltered
    /// stream's events, applied to its snapshot, give each next snapshot; a
    /// stream filtered only by `--tab` or `--kind` rebuilds the state of the
    /// kinds it admits; under `--workspace`, `--target` or `--state` the
    /// stream is a change feed for the objects it admits.
    #[test]
    fn applying_the_events_to_a_snapshot_gives_the_next_snapshot() {
        let streams: [(FilterSpec, Option<&str>); 11] = [
            (FilterSpec::default(), None),
            (FilterSpec::default(), Some("s4")),
            (
                FilterSpec {
                    tabs: vec!["t1".into(), "t3".into()],
                    ..FilterSpec::default()
                },
                Some("s1"),
            ),
            (
                FilterSpec {
                    tabs: vec!["t1".into()],
                    kinds: vec![Kind::Workspace, Kind::Queue, Kind::Pane],
                    ..FilterSpec::default()
                },
                None,
            ),
            (
                FilterSpec {
                    tabs: vec!["t1".into()],
                    kinds: vec![Kind::Workspace, Kind::Tab],
                    ..FilterSpec::default()
                },
                None,
            ),
            (
                FilterSpec {
                    kinds: vec![Kind::Pane],
                    ..FilterSpec::default()
                },
                None,
            ),
            (
                FilterSpec {
                    kinds: vec![Kind::Attention, Kind::Busy],
                    ..FilterSpec::default()
                },
                None,
            ),
            (
                FilterSpec {
                    kinds: vec![Kind::Workspace, Kind::Tab, Kind::Popup],
                    ..FilterSpec::default()
                },
                None,
            ),
            (
                FilterSpec {
                    kinds: vec![Kind::Queue],
                    ..FilterSpec::default()
                },
                None,
            ),
            (
                FilterSpec {
                    workspaces: vec!["s1".into()],
                    ..FilterSpec::default()
                },
                None,
            ),
            (
                FilterSpec {
                    targets: vec!["p1".into()],
                    states: vec![State::NeedsInput, State::Working],
                    ..FilterSpec::default()
                },
                None,
            ),
        ];
        let mut busiest = 0;
        for seed in 1..=6 {
            for (spec, caller) in &streams {
                let mut script = Script {
                    random: Random(seed * 7919),
                    next: 100,
                };
                let mut projection = base();
                settle(&mut projection);
                let filter = Filter::resolve(spec, *caller, &projection).unwrap();
                let feed = spec.workspaces.len() + spec.targets.len() + spec.states.len() > 0;
                let rebuilt = rebuilt(&filter);
                let mut state = filter.snapshot(&projection);
                let mut attention = attentions(&state);
                for turn in 0..400 {
                    let mut next = projection.clone();
                    // Several changes coalesce into each turn.
                    for _ in 0..1 + script.random.below(3) {
                        script.step(&mut next);
                    }
                    if filter.gone(&next) {
                        break;
                    }
                    let events = projection.diff(&next);
                    events.iter().for_each(assert_placed);
                    let context =
                        || format!("seed {seed} {spec:?} turn {turn}: {:?}", described(&events));
                    if feed {
                        // Each admitted event is about the admitted objects.
                        for event in events.iter().filter(|event| filter.admits(event)) {
                            let values = event.values();
                            assert!(
                                listed(&filter.workspaces, &values.workspaces),
                                "{}",
                                context()
                            );
                            assert!(
                                listed(&filter.terminals, &values.terminals),
                                "{}",
                                context()
                            );
                        }
                        projection = next;
                        continue;
                    }
                    for event in events.iter().filter(|event| filter.admits(event)) {
                        apply(&mut state, event);
                        if let Event::Attention {
                            target,
                            attention: now,
                            ..
                        } = event
                        {
                            match now {
                                Some(now) => attention.insert(
                                    target.terminal.clone(),
                                    (target.tab.clone(), target.id.clone(), now.clone()),
                                ),
                                None => attention.remove(&target.terminal),
                            };
                        }
                    }
                    let expected = filter.snapshot(&next);
                    assert_eq!(
                        facts(&state, &attention, &rebuilt),
                        facts(&expected, &attentions(&expected), &rebuilt),
                        "{}",
                        context()
                    );
                    if spec == &FilterSpec::default() {
                        assert_eq!(
                            underived(state.clone()),
                            underived(expected),
                            "{}",
                            context()
                        );
                    }
                    busiest = busiest.max(next.busy.len());
                    projection = next;
                }
            }
        }
        assert!(busiest > 32, "reached {busiest} busy");
    }
}
