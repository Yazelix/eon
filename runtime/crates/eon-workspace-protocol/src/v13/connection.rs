//! Bounded projections of Runtime's connection ownership; no terminal state owner.
use super::*;

pub const MAX_HOSTS: usize = 32;
pub const MAX_WORKSPACES: usize = 64;
pub const MAX_HANDSHAKE_BYTES: usize = 1024;
pub const FULL_SNAPSHOTS: u16 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Compatibility {
    pub session: u16,
    pub frame: u16,
    pub management: u16,
    pub capabilities: u16,
}
impl Default for Compatibility {
    fn default() -> Self {
        Self {
            session: 14,
            frame: 2,
            management: 2,
            capabilities: FULL_SNAPSHOTS,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceInfo {
    pub id: String,
    pub name: String,
    pub build: String,
    pub control_endpoint: Vec<u8>,
    pub compatibility: Compatibility,
    /// A successful Stop receipt, cleared before the supervisor accepts new work.
    pub stopped: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionRun {
    pub session: String,
    pub run: String,
    pub live: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct WorkspaceSnapshot {
    pub workspace: String,
    pub snapshot: Snapshot,
    pub runs: Vec<SessionRun>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Offline,
    Stopped,
    Busy,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceGroup {
    /// Empty means Local; otherwise an explicitly configured OpenSSH host.
    pub host: String,
    pub offline: bool,
    pub workspaces: Vec<WorkspaceInfo>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct WindowState {
    pub generation: u64,
    pub connection: ConnectionState,
    /// The accepted target, including when its last consistent picture is offline.
    pub host: String,
    pub target: Option<WorkspaceInfo>,
    pub groups: Vec<WorkspaceGroup>,
    pub workspace: Option<WorkspaceSnapshot>,
    pub detail: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegistryAction {
    List,
    Create { name: String },
    Ensure { workspace: String },
    Stop { workspace: String },
    Forget { workspace: String },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WindowAction {
    Select {
        host: String,
        workspace: String,
    },
    Disconnect,
    Reconnect,
    Refresh,
    Busy,
    Offline {
        detail: String,
    },
    Present,
    /// One workspace action against the selected generation. No nested routing.
    Apply(Box<Action>),
    /// The existing window CLI stops only the currently selected workspace.
    Stop,
}
fn host(value: &str) -> Result<()> {
    if value.len() > 255
        || value.starts_with('-')
        || value.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(Error::InvalidSnapshot { field: "SSH host" });
    }
    Ok(())
}
fn encode_info(out: &mut Encoder, value: &WorkspaceInfo) -> Result<()> {
    identity("workspace id", &value.id)?;
    nonempty("workspace name", &value.name, MAX_LABEL_BYTES)?;
    nonempty("build", &value.build, MAX_VERSION_BYTES)?;
    if value.control_endpoint.is_empty()
        || value.control_endpoint.len() > MAX_ENDPOINT_BYTES
        || value.control_endpoint.contains(&0)
    {
        return Err(Error::InvalidSnapshot {
            field: "control endpoint",
        });
    }
    if value.compatibility != Compatibility::default() {
        return Err(Error::InvalidSnapshot {
            field: "protocol compatibility",
        });
    }
    out.string(&value.id);
    out.string(&value.name);
    out.string(&value.build);
    out.raw(&value.control_endpoint);
    for version in [
        value.compatibility.session,
        value.compatibility.frame,
        value.compatibility.management,
        value.compatibility.capabilities,
    ] {
        out.number(version);
    }
    out.byte(u8::from(value.stopped));
    Ok(())
}
fn decode_info(input: &mut Decoder<'_>) -> Result<WorkspaceInfo> {
    let value = WorkspaceInfo {
        id: input.string("workspace id", MAX_ID_BYTES)?,
        name: input.string("workspace name", MAX_LABEL_BYTES)?,
        build: input.string("build", MAX_VERSION_BYTES)?,
        control_endpoint: input.raw("control endpoint", MAX_ENDPOINT_BYTES)?.to_vec(),
        compatibility: Compatibility {
            session: input.number()?,
            frame: input.number()?,
            management: input.number()?,
            capabilities: input.number()?,
        },
        stopped: decode_bool(input, "workspace stopped")?,
    };
    encode_info(&mut Encoder::default(), &value)?;
    Ok(value)
}
pub(super) fn encode_registry(out: &mut Encoder, values: &[WorkspaceInfo]) -> Result<()> {
    if values.len() > MAX_WORKSPACES {
        return Err(Error::InvalidSnapshot {
            field: "workspaces",
        });
    }
    let mut ids = HashSet::new();
    out.count(values.len());
    for value in values {
        if !ids.insert(&value.id) {
            return Err(Error::InvalidSnapshot {
                field: "duplicate workspace",
            });
        }
        encode_info(out, value)?;
    }
    Ok(())
}
pub(super) fn decode_registry(input: &mut Decoder<'_>) -> Result<Vec<WorkspaceInfo>> {
    let count = input.count("workspaces", MAX_WORKSPACES)?;
    let values = (0..count)
        .map(|_| decode_info(input))
        .collect::<Result<Vec<_>>>()?;
    encode_registry(&mut Encoder::default(), &values)?;
    Ok(values)
}
pub(super) fn encode_hello(out: &mut Encoder, value: &WorkspaceInfo) -> Result<()> {
    encode_info(out, value)?;
    if out.bytes.len() > MAX_HANDSHAKE_BYTES {
        return Err(Error::InvalidSnapshot {
            field: "handshake size",
        });
    }
    Ok(())
}
pub(super) fn decode_hello(input: &mut Decoder<'_>) -> Result<WorkspaceInfo> {
    let value = decode_info(input)?;
    encode_hello(&mut Encoder::default(), &value)?;
    Ok(value)
}

pub(super) fn encode_state(out: &mut Encoder, value: &WorkspaceSnapshot) -> Result<()> {
    identity("workspace id", &value.workspace)?;
    if value.runs.len() > MAX_SESSIONS {
        return Err(Error::InvalidSnapshot { field: "runs" });
    }
    let sessions: HashSet<_> = value
        .snapshot
        .tabs
        .iter()
        .flat_map(|tab| {
            tab.panes()
                .map(|pane| &pane.session)
                .chain(tab.popups.iter().map(|popup| &popup.session))
        })
        .collect();
    let mut runs = HashSet::new();
    for binding in &value.runs {
        identity("session id", &binding.session)?;
        identity("run id", &binding.run)?;
        if !sessions.contains(&binding.session) || !runs.insert(&binding.session) {
            return Err(Error::InvalidSnapshot {
                field: "session binding",
            });
        }
        if value
            .snapshot
            .tabs
            .iter()
            .flat_map(|tab| tab.panes())
            .any(|pane| pane.session == binding.session && pane.live != binding.live)
        {
            return Err(Error::InvalidSnapshot {
                field: "session liveness",
            });
        }
    }
    if runs.len() != sessions.len() {
        return Err(Error::InvalidSnapshot {
            field: "missing session binding",
        });
    }
    let snapshot = encode_response(&Response::Snapshot(value.snapshot.clone()))?;
    out.string(&value.workspace);
    out.bytes
        .extend_from_slice(&(snapshot.len() as u32).to_le_bytes());
    out.bytes.extend_from_slice(&snapshot);
    out.count(value.runs.len());
    for binding in &value.runs {
        out.string(&binding.session);
        out.string(&binding.run);
        out.bytes.push(u8::from(binding.live));
    }
    Ok(())
}

pub(super) fn decode_state(input: &mut Decoder<'_>) -> Result<WorkspaceSnapshot> {
    let workspace = input.string("workspace id", MAX_ID_BYTES)?;
    let length = u32::from_le_bytes(input.take(4)?.try_into().expect("four bytes")) as usize;
    if length > MAX_MESSAGE_BYTES {
        return Err(Error::InvalidSnapshot {
            field: "snapshot size",
        });
    }
    let bytes = input.take(length)?;
    let (kind, _) = unframe(bytes)?;
    // Check before descending: a nested Window response must never recurse.
    if kind != SNAPSHOT {
        return Err(Error::InvalidKind { value: kind });
    }
    let Response::Snapshot(snapshot) = decode_response(bytes)? else {
        unreachable!("snapshot discriminator")
    };
    let count = input.count("runs", MAX_SESSIONS)?;
    let runs = (0..count)
        .map(|_| {
            Ok(SessionRun {
                session: input.string("session id", MAX_ID_BYTES)?,
                run: input.string("run id", MAX_ID_BYTES)?,
                live: match input.byte()? {
                    0 => false,
                    1 => true,
                    _ => {
                        return Err(Error::InvalidSnapshot {
                            field: "session liveness",
                        });
                    }
                },
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let value = WorkspaceSnapshot {
        workspace,
        snapshot,
        runs,
    };
    encode_state(&mut Encoder::default(), &value)?;
    Ok(value)
}

pub(super) fn encode_window(out: &mut Encoder, value: &WindowState) -> Result<()> {
    host(&value.host)?;
    if value.groups.len() > MAX_HOSTS || value.detail.len() > MAX_DETAIL_BYTES {
        return Err(Error::InvalidSnapshot {
            field: "window state",
        });
    }
    if value.connection == ConnectionState::Connected
        && (value.target.is_none() || value.workspace.is_none())
    {
        return Err(Error::InvalidSnapshot {
            field: "connected target",
        });
    }
    if let Some(workspace) = &value.workspace
        && value
            .target
            .as_ref()
            .is_none_or(|target| target.id != workspace.workspace)
    {
        return Err(Error::InvalidSnapshot {
            field: "displayed workspace identity",
        });
    }
    out.bytes.extend_from_slice(&value.generation.to_le_bytes());
    out.byte(match value.connection {
        ConnectionState::Disconnected => 0,
        ConnectionState::Connecting => 1,
        ConnectionState::Connected => 2,
        ConnectionState::Offline => 3,
        ConnectionState::Stopped => 4,
        ConnectionState::Busy => 5,
    });
    out.string(&value.host);
    out.byte(u8::from(value.target.is_some()));
    if let Some(target) = &value.target {
        encode_info(out, target)?;
    }
    out.count(value.groups.len());
    let mut hosts = HashSet::new();
    for group in &value.groups {
        host(&group.host)?;
        if !hosts.insert(&group.host) {
            return Err(Error::InvalidSnapshot {
                field: "duplicate host",
            });
        }
        out.string(&group.host);
        out.byte(u8::from(group.offline));
        encode_registry(out, &group.workspaces)?;
    }
    out.byte(u8::from(value.workspace.is_some()));
    if let Some(workspace) = &value.workspace {
        encode_state(out, workspace)?;
    }
    out.string(&value.detail);
    Ok(())
}

pub(super) fn decode_window(input: &mut Decoder<'_>) -> Result<WindowState> {
    let generation = u64::from_le_bytes(input.take(8)?.try_into().expect("eight bytes"));
    let connection = match input.byte()? {
        0 => ConnectionState::Disconnected,
        1 => ConnectionState::Connecting,
        2 => ConnectionState::Connected,
        3 => ConnectionState::Offline,
        4 => ConnectionState::Stopped,
        5 => ConnectionState::Busy,
        value => {
            return Err(Error::InvalidTag {
                field: "connection state",
                value,
            });
        }
    };
    let host = input.string("SSH host", 255)?;
    let target = if decode_bool(input, "target")? {
        Some(decode_info(input)?)
    } else {
        None
    };
    let count = input.count("hosts", MAX_HOSTS)?;
    let groups = (0..count)
        .map(|_| {
            Ok(WorkspaceGroup {
                host: input.string("SSH host", 255)?,
                offline: decode_bool(input, "offline")?,
                workspaces: decode_registry(input)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let workspace = if decode_bool(input, "workspace")? {
        Some(decode_state(input)?)
    } else {
        None
    };
    let detail = input.string("connection detail", MAX_DETAIL_BYTES)?;
    let value = WindowState {
        generation,
        connection,
        host,
        target,
        groups,
        workspace,
        detail,
    };
    encode_window(&mut Encoder::default(), &value)?;
    Ok(value)
}

pub(super) fn encode_registry_action(out: &mut Encoder, action: &RegistryAction) -> Result<()> {
    match action {
        RegistryAction::List => out.byte(0),
        RegistryAction::Create { name } => {
            nonempty("workspace name", name, MAX_LABEL_BYTES)?;
            out.byte(1);
            out.string(name);
        }
        RegistryAction::Ensure { workspace }
        | RegistryAction::Stop { workspace }
        | RegistryAction::Forget { workspace } => {
            identity("workspace id", workspace)?;
            out.byte(match action {
                RegistryAction::Ensure { .. } => 2,
                RegistryAction::Stop { .. } => 3,
                RegistryAction::Forget { .. } => 4,
                _ => unreachable!(),
            });
            out.string(workspace);
        }
    }
    Ok(())
}

pub(super) fn decode_registry_action(input: &mut Decoder<'_>) -> Result<RegistryAction> {
    let action = match input.byte()? {
        0 => RegistryAction::List,
        1 => RegistryAction::Create {
            name: input.string("workspace name", MAX_LABEL_BYTES)?,
        },
        2 => RegistryAction::Ensure {
            workspace: input.string("workspace id", MAX_ID_BYTES)?,
        },
        3 => RegistryAction::Stop {
            workspace: input.string("workspace id", MAX_ID_BYTES)?,
        },
        4 => RegistryAction::Forget {
            workspace: input.string("workspace id", MAX_ID_BYTES)?,
        },
        value => {
            return Err(Error::InvalidTag {
                field: "registry action",
                value,
            });
        }
    };
    encode_registry_action(&mut Encoder::default(), &action)?;
    Ok(action)
}

pub(super) fn encode_window_action(
    out: &mut Encoder,
    generation: u64,
    action: &WindowAction,
) -> Result<()> {
    out.bytes.extend_from_slice(&generation.to_le_bytes());
    match action {
        WindowAction::Select {
            host: target,
            workspace,
        } => {
            host(target)?;
            identity("workspace id", workspace)?;
            out.byte(0);
            out.string(target);
            out.string(workspace);
        }
        WindowAction::Disconnect => out.byte(1),
        WindowAction::Reconnect => out.byte(2),
        WindowAction::Refresh => out.byte(5),
        WindowAction::Busy => out.byte(6),
        WindowAction::Present => out.byte(7),
        WindowAction::Offline { detail } => {
            nonempty("connection detail", detail, MAX_DETAIL_BYTES)?;
            out.byte(8);
            out.string(detail);
        }
        WindowAction::Apply(action) => {
            if matches!(
                action.as_ref(),
                Action::Window { .. }
                    | Action::Registry(_)
                    | Action::Hello { .. }
                    | Action::Presentation { .. }
            ) {
                return Err(Error::InvalidSnapshot {
                    field: "nested routing",
                });
            }
            let bytes = encode_request(&Request {
                id: "routed".into(),
                action: *action.clone(),
            })?;
            if bytes.len() > u16::MAX as usize {
                return Err(Error::InvalidSnapshot {
                    field: "routed action size",
                });
            }
            out.byte(3);
            out.raw(&bytes);
        }
        WindowAction::Stop => out.byte(4),
    }
    Ok(())
}

pub(super) fn decode_window_action(input: &mut Decoder<'_>) -> Result<Action> {
    let generation = u64::from_le_bytes(input.take(8)?.try_into().expect("eight bytes"));
    let action = match input.byte()? {
        0 => WindowAction::Select {
            host: input.string("SSH host", 255)?,
            workspace: input.string("workspace id", MAX_ID_BYTES)?,
        },
        1 => WindowAction::Disconnect,
        2 => WindowAction::Reconnect,
        5 => WindowAction::Refresh,
        6 => WindowAction::Busy,
        7 => WindowAction::Present,
        8 => WindowAction::Offline {
            detail: input.string("connection detail", MAX_DETAIL_BYTES)?,
        },
        3 => {
            let bytes = input.raw("routed action", u16::MAX as usize)?;
            let (_, payload) = unframe(bytes)?;
            let mut nested = Decoder::new(payload);
            nested.string("request id", MAX_ID_BYTES)?;
            // Reject another router before entering its decoder.
            if matches!(nested.byte()?, HELLO | REGISTRY | WINDOW | PRESENTATION) {
                return Err(Error::InvalidSnapshot {
                    field: "nested routing",
                });
            }
            WindowAction::Apply(Box::new(decode_request(bytes)?.action))
        }
        4 => WindowAction::Stop,
        value => {
            return Err(Error::InvalidTag {
                field: "window action",
                value,
            });
        }
    };
    encode_window_action(&mut Encoder::default(), generation, &action)?;
    Ok(Action::Window { generation, action })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> WorkspaceInfo {
        WorkspaceInfo {
            id: "w1".into(),
            name: "Remote project".into(),
            build: "build-one".into(),
            control_endpoint: b"/run/user/1000/eon/w1/eon.sock".to_vec(),
            compatibility: Compatibility::default(),
            stopped: false,
        }
    }

    #[test]
    fn routed_actions_round_trip_and_cannot_nest() {
        for action in [
            Action::Hello {
                workspace: "w1".into(),
            },
            Action::Presentation {
                workspace: "w1".into(),
            },
            Action::Registry(RegistryAction::List),
            Action::Registry(RegistryAction::Create {
                name: "project with spaces".into(),
            }),
            Action::Registry(RegistryAction::Ensure {
                workspace: "w1".into(),
            }),
            Action::Registry(RegistryAction::Stop {
                workspace: "w1".into(),
            }),
            Action::Registry(RegistryAction::Forget {
                workspace: "w1".into(),
            }),
            Action::Window {
                generation: 42,
                action: WindowAction::Select {
                    host: "homepc".into(),
                    workspace: "w1".into(),
                },
            },
            Action::Window {
                generation: 42,
                action: WindowAction::Apply(Box::new(Action::CreateColumn)),
            },
            Action::Window {
                generation: 42,
                action: WindowAction::Disconnect,
            },
            Action::Window {
                generation: 42,
                action: WindowAction::Stop,
            },
        ] {
            let request = Request {
                id: "request".into(),
                action,
            };
            assert_eq!(
                decode_request(&encode_request(&request).unwrap()).unwrap(),
                request
            );
        }
        for action in [
            Action::Window {
                generation: 1,
                action: WindowAction::Reconnect,
            },
            Action::Presentation {
                workspace: "w1".into(),
            },
        ] {
            assert!(
                encode_request(&Request {
                    id: "nested".into(),
                    action: Action::Window {
                        generation: 2,
                        action: WindowAction::Apply(Box::new(action.clone()))
                    }
                })
                .is_err()
            );
            let inner = Request {
                id: "inner".into(),
                action,
            };
            let mut payload = Encoder::default();
            payload.string("outer");
            payload.byte(WINDOW);
            payload.bytes.extend_from_slice(&2_u64.to_le_bytes());
            payload.byte(3);
            payload.raw(&encode_request(&inner).unwrap());
            assert!(matches!(
                decode_request(&frame(REQUEST, payload.bytes).unwrap()),
                Err(Error::InvalidSnapshot {
                    field: "nested routing"
                })
            ));
        }
    }

    #[test]
    fn window_projection_keeps_generation_target_and_offline_state() {
        let window = WindowState {
            generation: 99,
            connection: ConnectionState::Offline,
            host: "homepc".into(),
            target: Some(info()),
            groups: vec![WorkspaceGroup {
                host: "homepc".into(),
                offline: true,
                workspaces: vec![info()],
            }],
            workspace: None,
            detail: "SSH disconnected".into(),
        };
        for response in [
            Response::Window(Box::new(window)),
            Response::Hello(info()),
            Response::Registry(vec![info()]),
        ] {
            assert_eq!(
                decode_response(&encode_response(&response).unwrap()).unwrap(),
                response
            );
        }
        let mut incompatible = info();
        incompatible.compatibility.session += 1;
        assert!(encode_response(&Response::Hello(incompatible)).is_err());
        let mut bytes = encode_response(&Response::Hello(info())).unwrap();
        let offset = bytes.len() - 8;
        bytes[offset..offset + 2].copy_from_slice(&13_u16.to_le_bytes());
        assert!(decode_response(&bytes).is_err());
        let mut previous = encode_request(&Request {
            id: "r".into(),
            action: Action::Hello {
                workspace: "w1".into(),
            },
        })
        .unwrap();
        previous[4..6].copy_from_slice(&12_u16.to_le_bytes());
        assert!(matches!(
            decode_request(&previous),
            Err(Error::UnsupportedVersion { version: 12 })
        ));
    }
}
