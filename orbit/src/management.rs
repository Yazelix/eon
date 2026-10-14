use crate::{Result, attachment::is_disconnect, platform};
use orbit_protocol::management::{
    self as protocol, Failure, FailureCode, LiveIdentity, ProcessOutcome, Record, ServerMessage,
    TerminationReason, Tombstone,
};
use std::{
    io::{self, Read, Write},
    os::unix::{
        net::{UnixListener, UnixStream},
        process::ExitStatusExt,
    },
    path::{Path, PathBuf},
    process::ExitStatus,
    time::{Duration, Instant},
};

const ARGUMENT: &str = "--management-v1";
const NEGOTIATION_TIMEOUT: Duration = Duration::from_secs(1);

pub(super) struct Launch {
    session_id: String,
    run_id: String,
    component_generation: String,
}

struct Client {
    stream: UnixStream,
    peer_uid: u32,
    input: Vec<u8>,
    output: Vec<u8>,
    output_offset: usize,
    close_after_flush: bool,
    negotiation_deadline: Option<Instant>,
}

pub(super) struct Prepared {
    launch: Launch,
    management_path: PathBuf,
    listener: UnixListener,
    socket_guard: platform::SocketGuard,
}

pub(super) struct Owner {
    listener: UnixListener,
    _socket_guard: platform::SocketGuard,
    record: platform::RecordGuard,
    identity: LiveIdentity,
    client: Option<Client>,
}

pub(super) fn take_launch(arguments: &mut Vec<String>) -> Result<Option<Launch>> {
    let launch_end = arguments
        .iter()
        .position(|argument| argument == "--")
        .unwrap_or(arguments.len());
    let mut positions = arguments[..launch_end]
        .iter()
        .enumerate()
        .filter_map(|(index, argument)| (argument == ARGUMENT).then_some(index));
    let Some(index) = positions.next() else {
        return Ok(None);
    };
    if positions.next().is_some() {
        return Err("duplicate --management-v1 argument".into());
    }
    let values = arguments
        .get(index + 1..index + 4)
        .filter(|_| index + 4 <= launch_end)
        .ok_or("--management-v1 requires Session, run, and component identities")?;
    for (field, value) in [
        ("Session ID", &values[0]),
        ("run ID", &values[1]),
        ("component generation", &values[2]),
    ] {
        if value.is_empty() || value.len() > protocol::MAX_ID_BYTES {
            return Err(format!(
                "{field} must contain 1 through {} UTF-8 bytes",
                protocol::MAX_ID_BYTES
            )
            .into());
        }
    }
    let launch = Launch {
        session_id: values[0].clone(),
        run_id: values[1].clone(),
        component_generation: values[2].clone(),
    };
    arguments.drain(index..index + 4);
    Ok(Some(launch))
}

fn artifact_path(socket: &Path, suffix: &str) -> PathBuf {
    let mut path = socket.as_os_str().to_os_string();
    path.push(suffix);
    PathBuf::from(path)
}

impl Launch {
    pub(super) fn prepare(self, socket: &Path) -> Result<Prepared> {
        let management_path = artifact_path(socket, ".management");
        let (listener, socket_guard) = platform::create_listener(&management_path)?;
        Ok(Prepared {
            launch: self,
            management_path,
            listener,
            socket_guard,
        })
    }
}

impl Prepared {
    pub(super) fn publish(self, presentation_path: &Path) -> Result<Owner> {
        let identity = LiveIdentity {
            session_id: self.launch.session_id,
            run_id: self.launch.run_id,
            component_generation: self.launch.component_generation,
            record_generation: protocol::RECORD_GENERATION,
            management_generation: protocol::VERSION,
            process_id: std::process::id(),
            process_start: platform::process_start_identity()?,
            uid: platform::current_uid(),
            presentation: platform::endpoint_identity(presentation_path)?,
            management: platform::endpoint_identity(&self.management_path)?,
        };
        let live_record = protocol::encode_record(&Record::Live(identity.clone()))?;
        let record = platform::RecordGuard::publish(
            &artifact_path(presentation_path, ".record"),
            &live_record,
        )?;
        Ok(Owner {
            listener: self.listener,
            _socket_guard: self.socket_guard,
            record,
            identity,
            client: None,
        })
    }
}

impl Owner {
    pub(super) fn listener(&self) -> &UnixListener {
        &self.listener
    }

    pub(super) fn client_readiness(&self) -> Option<(&UnixStream, bool)> {
        self.client
            .as_ref()
            .map(|client| (&client.stream, !client.output.is_empty()))
    }

    pub(super) fn release_expired(&mut self, now: Instant) {
        if self
            .client
            .as_ref()
            .and_then(|client| client.negotiation_deadline)
            .is_some_and(|deadline| now >= deadline)
        {
            self.client = None;
        }
    }

    pub(super) fn accept(&mut self) -> Result {
        loop {
            match self.listener.accept() {
                Ok((mut stream, _)) => {
                    if self.client.is_some() {
                        let busy = protocol::encode_server_message(&ServerMessage::Busy)?;
                        let _ = stream.write_all(&busy);
                        continue;
                    }
                    stream.set_nonblocking(true)?;
                    self.client = Some(Client {
                        peer_uid: platform::peer_uid(&stream)?,
                        stream,
                        input: Vec::new(),
                        output: Vec::new(),
                        output_offset: 0,
                        close_after_flush: false,
                        negotiation_deadline: Some(Instant::now() + NEGOTIATION_TIMEOUT),
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error.into()),
            }
        }
    }

    pub(super) fn read_request(&mut self) -> Result<bool> {
        let request = {
            let Some(client) = self.client.as_mut() else {
                return Ok(false);
            };
            if !client.output.is_empty() {
                return Ok(false);
            }
            let remaining = protocol::MAX_MESSAGE_BYTES.saturating_sub(client.input.len());
            if remaining == 0 {
                queue_failure(client, "management request exceeded its bound")?;
                return Ok(false);
            }
            let mut bytes = [0; protocol::MAX_MESSAGE_BYTES];
            match client.stream.read(&mut bytes[..remaining]) {
                Ok(0) => {
                    self.client = None;
                    return Ok(false);
                }
                Ok(read) => client.input.extend_from_slice(&bytes[..read]),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(false),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => return Ok(false),
                Err(error) if is_disconnect(&error) => {
                    self.client = None;
                    return Ok(false);
                }
                Err(error) => return Err(error.into()),
            }
            let length = match protocol::client_message_len(&client.input) {
                Ok(Some(length)) => length,
                Ok(None) => return Ok(false),
                Err(error) => {
                    queue_failure(client, &error.to_string())?;
                    return Ok(false);
                }
            };
            if client.input.len() != length {
                queue_failure(client, "management requests cannot be pipelined")?;
                return Ok(false);
            }
            let message = match protocol::decode_client_message(&client.input) {
                Ok(message) => message,
                Err(error) => {
                    queue_failure(client, &error.to_string())?;
                    return Ok(false);
                }
            };
            client.input.clear();
            (
                message,
                client.peer_uid,
                client.negotiation_deadline.is_none(),
            )
        };

        match request {
            (protocol::ClientMessage::Acquire { expected, record }, peer_uid, false) => {
                let valid = peer_uid == self.identity.uid
                    && expected == self.identity
                    && self
                        .record
                        .identity()
                        .is_ok_and(|current| current == record)
                    && platform::endpoint_matches(&self.identity.presentation)
                    && platform::endpoint_matches(&self.identity.management);
                let client = self.client.as_mut().expect("request has one client");
                if valid {
                    client.negotiation_deadline = None;
                    queue_response(client, &ServerMessage::Lease(self.identity.clone()))?;
                } else {
                    queue_response(
                        client,
                        &ServerMessage::Failure(Failure {
                            code: FailureCode::InvalidIdentity,
                            detail: "live Orbit identity did not match".into(),
                        }),
                    )?;
                    client.close_after_flush = true;
                }
                Ok(false)
            }
            (protocol::ClientMessage::Status, _, true) => {
                let identity = self.identity.clone();
                queue_response(
                    self.client.as_mut().expect("request has one client"),
                    &ServerMessage::Status(identity),
                )?;
                Ok(false)
            }
            (protocol::ClientMessage::Stop, _, true) => Ok(true),
            _ => {
                let client = self.client.as_mut().expect("request has one client");
                queue_response(
                    client,
                    &ServerMessage::Failure(Failure {
                        code: FailureCode::InvalidRequest,
                        detail: "acquire the management lease before this request".into(),
                    }),
                )?;
                client.close_after_flush = true;
                Ok(false)
            }
        }
    }

    pub(super) fn flush(&mut self) -> Result {
        let mut disconnect = false;
        if let Some(client) = &mut self.client {
            while client.output_offset < client.output.len() {
                match client.stream.write(&client.output[client.output_offset..]) {
                    Ok(0) => {
                        disconnect = true;
                        break;
                    }
                    Ok(written) => client.output_offset += written,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                    Err(error) if is_disconnect(&error) => {
                        disconnect = true;
                        break;
                    }
                    Err(error) => return Err(error.into()),
                }
            }
            if client.output_offset == client.output.len() {
                client.output.clear();
                client.output_offset = 0;
                disconnect |= client.close_after_flush;
            }
        }
        if disconnect {
            self.client = None;
        }
        Ok(())
    }

    pub(super) fn finish_explicit(&mut self, status: ExitStatus) -> Result<Tombstone> {
        self.finish(TerminationReason::ExplicitStop, status)
    }

    pub(super) fn finish_natural(&mut self, status: ExitStatus) -> Result<Tombstone> {
        self.finish(TerminationReason::NaturalExit, status)
    }

    fn finish(&mut self, reason: TerminationReason, status: ExitStatus) -> Result<Tombstone> {
        let outcome = if let Some(code) = status.code() {
            ProcessOutcome::ExitCode(code)
        } else if let Some(signal) = status.signal() {
            ProcessOutcome::Signal(signal)
        } else {
            return Err("PTY child had neither an exit code nor a termination signal".into());
        };
        let tombstone = Tombstone {
            identity: self.identity.clone(),
            reason,
            outcome,
        };
        let bytes = protocol::encode_record(&Record::Tombstone(tombstone.clone()))?;
        self.record.replace_and_preserve(&bytes)?;
        Ok(tombstone)
    }

    pub(super) fn reply_stopped(&mut self, tombstone: &Tombstone) -> Result {
        let Some(client) = self
            .client
            .as_mut()
            .filter(|client| client.negotiation_deadline.is_none())
        else {
            return Ok(());
        };
        let response = protocol::encode_server_message(&ServerMessage::Stopped(tombstone.clone()))?;
        if client.stream.set_nonblocking(false).is_ok()
            && client
                .stream
                .set_write_timeout(Some(NEGOTIATION_TIMEOUT))
                .is_ok()
        {
            let _ = client.stream.write_all(&response);
        }
        Ok(())
    }
}

fn queue_response(client: &mut Client, message: &ServerMessage) -> Result {
    if !client.output.is_empty() {
        return Err("management response already in flight".into());
    }
    client.output = protocol::encode_server_message(message)?;
    client.output_offset = 0;
    Ok(())
}

fn queue_failure(client: &mut Client, detail: &str) -> Result {
    queue_response(
        client,
        &ServerMessage::Failure(Failure {
            code: FailureCode::InvalidRequest,
            detail: detail.into(),
        }),
    )?;
    client.close_after_flush = true;
    Ok(())
}
