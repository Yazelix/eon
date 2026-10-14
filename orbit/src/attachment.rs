use crate::Result;
use orbit_protocol::{
    Frame, MAX_FRAME_BYTES,
    session::{self, ClientMessage, Failure, FailureCode, Metadata, ServerMessage, WheelOutcome},
};
use std::{
    collections::VecDeque,
    io::{self, Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    thread,
    time::{Duration, Instant},
};

const NEGOTIATION_TIMEOUT: Duration = Duration::from_secs(1);
const FINISH_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_OUTPUT_BYTES: usize =
    MAX_FRAME_BYTES + session::MAX_PAYLOAD_BYTES + 2 * session::HEADER_BYTES + 4096;

pub(crate) enum Incoming {
    Negotiate(Negotiation),
    Message(ClientMessage),
    Pending,
    Disconnect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Negotiation {
    Interactive,
    Metadata,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    Pending(Instant),
    Interactive,
    Metadata,
}

pub(crate) struct Client {
    stream: UnixStream,
    input: Vec<u8>,
    output: OutputQueue,
    close_after_flush: bool,
    role: Role,
    initial_frame_pending: bool,
    initial_metadata_pending: bool,
    last_metadata: Option<Metadata>,
}

impl Client {
    fn new(stream: UnixStream, negotiation_deadline: Option<Instant>) -> Self {
        Self {
            stream,
            input: Vec::new(),
            output: OutputQueue::default(),
            close_after_flush: false,
            role: negotiation_deadline.map_or(Role::Interactive, Role::Pending),
            initial_frame_pending: false,
            initial_metadata_pending: false,
            last_metadata: None,
        }
    }

    pub(crate) fn accepts_output(&self) -> bool {
        self.role == Role::Interactive && !self.close_after_flush
    }

    pub(crate) fn accepts_metadata(&self) -> bool {
        self.role == Role::Metadata && !self.close_after_flush
    }

    pub(crate) fn poll_stream(&self) -> Option<&UnixStream> {
        (!self.close_after_flush).then_some(&self.stream)
    }

    pub(crate) fn has_input(&self) -> bool {
        !self.input.is_empty()
    }

    pub(crate) fn is_expired(&self, now: Instant) -> bool {
        matches!(self.role, Role::Pending(deadline) if now >= deadline)
    }

    pub(crate) fn read_ready(&mut self) -> Result<bool> {
        if self.close_after_flush {
            return Ok(true);
        }
        let mut bytes = [0; 1024];
        let read = match self.stream.read(&mut bytes) {
            Ok(0) => return Ok(false),
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => return Ok(true),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(true),
            Err(error) if is_disconnect(&error) => return Ok(false),
            Err(error) => return Err(error.into()),
        };
        self.input.extend_from_slice(&bytes[..read]);
        Ok(true)
    }

    pub(crate) fn next_incoming(&mut self) -> Result<Incoming> {
        let length = match session::client_message_len(&self.input) {
            Ok(Some(_)) if self.role == Role::Metadata => {
                return self.reject_protocol("metadata observers cannot send messages".into());
            }
            Ok(Some(length)) if self.input.len() >= length => length,
            Ok(_) => return Ok(Incoming::Pending),
            Err(session::Error::UnsupportedVersion { .. }) => return Ok(Incoming::Disconnect),
            Err(error) => return self.reject_protocol(error.to_string()),
        };
        let message = match session::decode_client_message(&self.input[..length]) {
            Ok(message) => message,
            Err(error) => return self.reject_protocol(error.to_string()),
        };
        drop(self.input.drain(..length));

        if matches!(self.role, Role::Pending(_)) {
            return match message {
                ClientMessage::Hello => Ok(Incoming::Negotiate(Negotiation::Interactive)),
                ClientMessage::ObserveMetadata => Ok(Incoming::Negotiate(Negotiation::Metadata)),
                _ => self.reject_protocol(
                    "first client message must be Hello or ObserveMetadata".into(),
                ),
            };
        }
        if matches!(
            message,
            ClientMessage::Hello | ClientMessage::ObserveMetadata
        ) {
            return self.reject_protocol("attachment role may only be sent once".into());
        }
        Ok(Incoming::Message(message))
    }

    pub(crate) fn begin_interactive(&mut self) -> Result<bool> {
        debug_assert!(matches!(self.role, Role::Pending(_)));
        if !self.push_message(&ServerMessage::Attached)? {
            return Ok(false);
        }
        self.role = Role::Interactive;
        self.initial_frame_pending = true;
        Ok(true)
    }

    pub(crate) fn begin_metadata(&mut self) -> Result<bool> {
        debug_assert!(matches!(self.role, Role::Pending(_)));
        if !self.push_message(&ServerMessage::ObservingMetadata)? {
            return Ok(false);
        }
        self.role = Role::Metadata;
        self.initial_metadata_pending = true;
        Ok(true)
    }

    fn reject_protocol(&mut self, detail: String) -> Result<Incoming> {
        let queued = self.fail(FailureCode::Protocol, detail)?;
        self.close_when_flushed();
        Ok(if queued {
            Incoming::Pending
        } else {
            Incoming::Disconnect
        })
    }

    pub(crate) fn can_push_result_frame(&self) -> bool {
        self.output.can_push_result_frame()
    }

    #[cfg(test)]
    pub(crate) fn can_push_selection_result(&self) -> bool {
        self.output.can_push_selection_result()
    }

    pub(crate) fn can_push_host_selection_result(&self, finishing: bool) -> bool {
        self.output.can_push_host_selection_result(finishing)
    }

    pub(crate) fn supersede_pending_frame(&mut self) {
        self.output.supersede_pending_frame();
    }

    pub(crate) fn can_push_frame_message(&self) -> bool {
        self.output.can_push_frame_message()
    }

    pub(crate) fn can_push_scroll_outcome(&self) -> bool {
        self.output.can_push_scroll_outcome()
    }

    pub(crate) fn can_push_message(&self, message: &ServerMessage) -> Result<bool> {
        self.output.can_push_message(message)
    }

    pub(crate) fn push_message(&mut self, message: &ServerMessage) -> Result<bool> {
        self.output.push_message(message)
    }

    pub(crate) fn push_frame(&mut self, frame: Frame) -> Result<bool> {
        let queued = if self.initial_frame_pending {
            self.output.push_initial_frame(frame)
        } else {
            self.output.push_frame(frame)
        }?;
        if queued {
            self.initial_frame_pending = false;
        }
        Ok(queued)
    }

    pub(crate) fn push_metadata(&mut self, metadata: Metadata) -> Result<bool> {
        if self.last_metadata.as_ref().is_some_and(|previous| {
            previous.title == metadata.title
                && previous.working_directory == metadata.working_directory
        }) {
            return Ok(true);
        }
        let queued = if self.initial_metadata_pending {
            self.output.push_initial_metadata(&metadata)
        } else {
            self.output
                .push_message(&ServerMessage::Metadata(metadata.clone()))
        }?;
        if queued {
            self.initial_metadata_pending = false;
            self.last_metadata = Some(metadata);
        }
        Ok(queued)
    }

    pub(crate) fn fail(&mut self, code: FailureCode, mut detail: String) -> Result<bool> {
        while detail.len() > session::MAX_FAILURE_BYTES {
            detail.pop();
        }
        self.push_message(&ServerMessage::Failure(Failure { code, detail }))
    }

    pub(crate) fn close_when_flushed(&mut self) {
        self.input = Vec::new();
        self.close_after_flush = true;
    }

    pub(crate) fn is_closing(&self) -> bool {
        self.close_after_flush
    }

    pub(crate) fn finish_session(&mut self, code: i32) -> Result {
        if !matches!(self.role, Role::Pending(_)) {
            if !self.close_after_flush {
                let _ = self.push_message(&ServerMessage::Exited { code })?;
            }
            let deadline = Instant::now() + FINISH_TIMEOUT;
            while !self.output.is_empty() && Instant::now() < deadline {
                if !self.output.flush(&mut self.stream)? {
                    break;
                }
                thread::sleep(Duration::from_millis(1));
            }
        }
        Ok(())
    }

    pub(crate) fn flush(&mut self) -> io::Result<bool> {
        Ok(self.output.flush(&mut self.stream)?
            && !(self.close_after_flush && self.output.is_empty()))
    }

    #[cfg(test)]
    pub(crate) fn test_pair(negotiation_deadline: Option<Instant>) -> Result<(Self, UnixStream)> {
        let (stream, peer) = UnixStream::pair()?;
        stream.set_nonblocking(true)?;
        peer.set_read_timeout(Some(Duration::from_secs(2)))?;
        Ok((Self::new(stream, negotiation_deadline), peer))
    }

    #[cfg(test)]
    pub(crate) fn initial_frame_pending(&self) -> bool {
        self.initial_frame_pending
    }

    #[cfg(test)]
    pub(crate) fn output_is_empty(&self) -> bool {
        self.output.is_empty()
    }

    #[cfg(test)]
    pub(crate) fn fill_output_for_test(&mut self) {
        assert!(
            self.output
                .push(vec![0; MAX_OUTPUT_BYTES], MessageClass::Ordered)
        );
    }
}

pub(crate) fn accept(
    listener: &UnixListener,
    active: &Option<Client>,
    observer: &Option<Client>,
    pending: &mut Option<Client>,
) -> Result {
    loop {
        match listener.accept() {
            Ok((mut stream, _)) if pending.is_some() || active.is_some() && observer.is_some() => {
                let busy = session::encode_server_message(&ServerMessage::Busy)?;
                let _ = stream.write_all(&busy);
            }
            Ok((stream, _)) => {
                stream.set_nonblocking(true)?;
                *pending = Some(Client::new(
                    stream,
                    Some(Instant::now() + NEGOTIATION_TIMEOUT),
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        }
    }
}

pub(crate) fn is_disconnect(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::BrokenPipe
    )
}

struct Message {
    bytes: Vec<u8>,
    class: MessageClass,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MessageClass {
    Ordered,
    Preview,
    Frame,
    ScrollOutcome,
    Metadata,
}

#[derive(Default)]
struct OutputQueue {
    messages: VecDeque<Message>,
    offset: usize,
    bytes: usize,
}

impl OutputQueue {
    fn can_push_result_frame(&self) -> bool {
        self.bytes
            .saturating_add(MAX_FRAME_BYTES + 2 * session::HEADER_BYTES)
            <= MAX_OUTPUT_BYTES
    }

    #[cfg(test)]
    fn can_push_selection_result(&self) -> bool {
        self.bytes.saturating_add(
            MAX_FRAME_BYTES + session::MAX_COPY_BYTES + 9 + 3 * session::HEADER_BYTES,
        ) <= MAX_OUTPUT_BYTES
    }

    fn can_push_host_selection_result(&self, finishing: bool) -> bool {
        let pending_frame = self.replaceable_suffix(MessageClass::Frame).1;
        let required = if finishing {
            MAX_FRAME_BYTES + session::MAX_COPY_BYTES + 9 + 3 * session::HEADER_BYTES
        } else {
            MAX_FRAME_BYTES + 2 * session::HEADER_BYTES
        };
        self.bytes
            .saturating_sub(pending_frame)
            .saturating_add(required)
            <= MAX_OUTPUT_BYTES
    }

    fn can_push_frame_message(&self) -> bool {
        self.bytes
            .saturating_sub(self.replaceable_suffix(MessageClass::Frame).1)
            .saturating_add(MAX_FRAME_BYTES + session::HEADER_BYTES)
            <= MAX_OUTPUT_BYTES
    }

    fn can_push_scroll_outcome(&self) -> bool {
        self.bytes
            .saturating_sub(self.replaceable_suffix(MessageClass::ScrollOutcome).1)
            .saturating_add(session::MAX_PAYLOAD_BYTES + session::HEADER_BYTES)
            <= MAX_OUTPUT_BYTES
    }

    fn can_push_message(&self, message: &ServerMessage) -> Result<bool> {
        Ok(self
            .bytes
            .saturating_add(session::encode_server_message(message)?.len())
            <= MAX_OUTPUT_BYTES)
    }

    fn push_message(&mut self, message: &ServerMessage) -> Result<bool> {
        let class = match message {
            ServerMessage::VerticalPreview(_) => MessageClass::Preview,
            ServerMessage::WheelOutcome(WheelOutcome::Viewport { .. }) => MessageClass::Frame,
            ServerMessage::ScrollOutcome(session::ScrollOutcome::Viewport { .. }) => {
                MessageClass::ScrollOutcome
            }
            ServerMessage::Metadata(_) => MessageClass::Metadata,
            _ => MessageClass::Ordered,
        };
        Ok(self.push_replaceable(session::encode_server_message(message)?, class))
    }

    fn push_initial_frame(&mut self, frame: Frame) -> Result<bool> {
        Ok(self.push(
            session::encode_server_message(&ServerMessage::Frame(Box::new(frame)))?,
            MessageClass::Ordered,
        ))
    }

    fn push_frame(&mut self, frame: Frame) -> Result<bool> {
        let message = session::encode_server_message(&ServerMessage::Frame(Box::new(frame)))?;
        Ok(self.push_replaceable(message, MessageClass::Frame))
    }

    fn push_initial_metadata(&mut self, metadata: &Metadata) -> Result<bool> {
        Ok(self.push(
            session::encode_server_message(&ServerMessage::Metadata(metadata.clone()))?,
            MessageClass::Ordered,
        ))
    }

    fn push_replaceable(&mut self, message: Vec<u8>, class: MessageClass) -> bool {
        let (remove, removed_bytes) = self.replaceable_suffix(class);
        if self
            .bytes
            .saturating_sub(removed_bytes)
            .saturating_add(message.len())
            > MAX_OUTPUT_BYTES
        {
            return false;
        }
        for _ in 0..remove {
            let previous = self.messages.pop_back().expect("back existed");
            self.bytes -= previous.bytes.len();
        }
        self.push(message, class)
    }

    fn supersede_pending_frame(&mut self) {
        let (remove, removed_bytes) = self.replaceable_suffix(MessageClass::Frame);
        self.messages.truncate(self.messages.len() - remove);
        self.bytes -= removed_bytes;
    }

    fn replaceable_suffix(&self, class: MessageClass) -> (usize, usize) {
        let replaceable = |back: &Message| match class {
            MessageClass::Ordered => false,
            MessageClass::Preview => back.class == MessageClass::Preview,
            MessageClass::Frame | MessageClass::ScrollOutcome => {
                matches!(back.class, MessageClass::Preview | MessageClass::Frame)
            }
            MessageClass::Metadata => back.class == MessageClass::Metadata,
        };
        let mut remove = 0;
        let mut removed_bytes = 0usize;
        for (index, back) in self.messages.iter().enumerate().rev() {
            if !replaceable(back) || index == 0 && self.offset != 0 {
                break;
            }
            remove += 1;
            removed_bytes += back.bytes.len();
            if class == MessageClass::Preview {
                break;
            }
        }
        (remove, removed_bytes)
    }

    fn push(&mut self, bytes: Vec<u8>, class: MessageClass) -> bool {
        if self.bytes.saturating_add(bytes.len()) > MAX_OUTPUT_BYTES {
            return false;
        }
        self.bytes += bytes.len();
        self.messages.push_back(Message { bytes, class });
        true
    }

    fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    fn flush(&mut self, stream: &mut UnixStream) -> io::Result<bool> {
        while let Some(message) = self.messages.front() {
            match stream.write(&message.bytes[self.offset..]) {
                Ok(0) => return Ok(false),
                Ok(written) => {
                    self.offset += written;
                    self.bytes -= written;
                    if self.offset == message.bytes.len() {
                        self.messages.pop_front();
                        self.offset = 0;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(true),
                Err(error) if is_disconnect(&error) => return Ok(false),
                Err(error) => return Err(error),
            }
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_observer_is_read_only_and_keeps_only_the_latest_change() -> Result {
        let (mut observer, mut peer) =
            Client::test_pair(Some(Instant::now() + NEGOTIATION_TIMEOUT))?;
        peer.write_all(&session::encode_client_message(
            &ClientMessage::ObserveMetadata,
        )?)?;
        assert!(observer.read_ready()?);
        assert!(matches!(
            observer.next_incoming()?,
            Incoming::Negotiate(Negotiation::Metadata)
        ));
        assert!(observer.begin_metadata()?);

        for (revision, title) in [(1, "initial"), (2, "middle"), (3, "latest")] {
            assert!(observer.push_metadata(session::Metadata {
                revision,
                title: title.into(),
                working_directory: "file:///tmp/eon".into(),
            })?);
        }
        let _ = observer.flush()?;
        assert_eq!(
            crate::diagnostic::read_message(&mut peer)?,
            Some(ServerMessage::ObservingMetadata)
        );
        assert!(matches!(
            crate::diagnostic::read_message(&mut peer)?,
            Some(ServerMessage::Metadata(session::Metadata {
                revision: 1,
                ..
            }))
        ));
        assert!(matches!(
            crate::diagnostic::read_message(&mut peer)?,
            Some(ServerMessage::Metadata(session::Metadata {
                revision: 3,
                ..
            }))
        ));

        peer.write_all(&session::encode_client_message(&ClientMessage::Focus(
            session::FocusEvent::Gained,
        ))?)?;
        assert!(observer.read_ready()?);
        assert!(matches!(observer.next_incoming()?, Incoming::Pending));
        assert!(observer.is_closing());
        Ok(())
    }

    #[test]
    fn pending_presentations_keep_scroll_outcomes_and_latest_replaceable_revision() {
        let mut output = OutputQueue::default();
        assert!(output.can_push_scroll_outcome());
        assert!(output.can_push_selection_result());
        assert!(output.push(vec![0], MessageClass::Ordered));
        assert!(output.push_replaceable(vec![1], MessageClass::Frame));
        assert!(output.push_replaceable(vec![2], MessageClass::Preview));
        assert!(output.push_replaceable(vec![3], MessageClass::Preview));

        assert_eq!(output.messages.len(), 3);
        assert!(output.messages.front().unwrap().bytes.ends_with(&[0]));
        assert!(output.messages.back().unwrap().bytes.ends_with(&[3]));

        assert!(output.push_replaceable(vec![4], MessageClass::Frame));
        assert_eq!(output.messages.len(), 2);
        assert!(output.messages.back().unwrap().bytes.ends_with(&[4]));

        assert!(output.push_replaceable(vec![5], MessageClass::Preview));
        assert_eq!(output.messages.len(), 3);
        assert!(output.messages.back().unwrap().bytes.ends_with(&[5]));

        assert!(output.push_replaceable(vec![6], MessageClass::ScrollOutcome));
        assert_eq!(output.messages.len(), 2);
        assert!(output.push_replaceable(vec![7], MessageClass::Frame));
        assert!(output.push_replaceable(vec![8], MessageClass::ScrollOutcome));
        assert_eq!(output.messages.len(), 3);
        assert!(output.messages[1].bytes.ends_with(&[6]));
        assert!(output.messages[2].bytes.ends_with(&[8]));

        let mut full = OutputQueue::default();
        assert!(full.push(
            vec![0; MAX_FRAME_BYTES + session::HEADER_BYTES],
            MessageClass::Ordered
        ));
        assert!(full.push_replaceable(
            vec![1; MAX_FRAME_BYTES + session::HEADER_BYTES],
            MessageClass::Frame
        ));
        assert!(full.can_push_frame_message());
        assert!(!full.can_push_selection_result());

        let frame_bytes = MAX_FRAME_BYTES + session::HEADER_BYTES;
        let outcome_bytes = session::MAX_PAYLOAD_BYTES + session::HEADER_BYTES;
        let mut partial = OutputQueue::default();
        assert!(partial.push(vec![0; frame_bytes], MessageClass::Frame));
        partial.offset = 1;
        partial.bytes -= 1;
        assert!(partial.push_replaceable(vec![1; frame_bytes], MessageClass::Frame));

        assert!(partial.can_push_scroll_outcome());
        assert!(partial.push_replaceable(vec![2; outcome_bytes], MessageClass::ScrollOutcome));
        assert_eq!(partial.messages.len(), 2);
        assert!(partial.messages[0].class == MessageClass::Frame);
        assert!(partial.messages[1].class == MessageClass::ScrollOutcome);
        assert_eq!(partial.offset, 1);
        assert_eq!(partial.bytes, frame_bytes - 1 + outcome_bytes);
        assert!(partial.bytes <= MAX_OUTPUT_BYTES);
    }

    #[test]
    fn fatal_decode_releases_client_input_storage() -> Result {
        let (mut client, mut peer) = Client::test_pair(None)?;
        let mut message =
            session::encode_client_message(&ClientMessage::Mouse(session::MouseEvent {
                action: session::MouseAction::Press,
                button: Some(session::MouseButton::Left),
                modifiers: session::Modifiers::empty(),
                x: 1.0,
                y: 1.0,
            }))?;
        message[session::HEADER_BYTES + 4..session::HEADER_BYTES + 8]
            .copy_from_slice(&f32::MAX.to_bits().to_le_bytes());
        peer.write_all(&message)?;

        assert!(client.read_ready()?);
        assert!(matches!(client.next_incoming()?, Incoming::Pending));
        assert!(client.close_after_flush);
        assert!(client.input.is_empty());
        assert_eq!(client.input.capacity(), 0);
        let _ = client.flush()?;
        assert_eq!(
            crate::diagnostic::read_message(&mut peer)?,
            Some(ServerMessage::Failure(Failure {
                code: FailureCode::Protocol,
                detail: "invalid mouse coordinates".into(),
            }))
        );
        Ok(())
    }

    #[test]
    fn terminal_closing_client_finishes_with_queued_failure_only() -> Result {
        let (mut client, mut peer) = Client::test_pair(None)?;
        let failure = ServerMessage::Failure(Failure {
            code: FailureCode::Protocol,
            detail: "x".repeat(session::MAX_FAILURE_BYTES),
        });
        let message = session::encode_server_message(&failure)?;
        for _ in 0..1024 {
            assert!(client.push_message(&failure)?);
        }
        client.close_when_flushed();

        let actual = std::thread::scope(|scope| -> Result<Vec<u8>> {
            let reader = scope.spawn(move || -> io::Result<Vec<u8>> {
                std::thread::sleep(Duration::from_millis(10));
                let mut actual = Vec::new();
                peer.read_to_end(&mut actual)?;
                Ok(actual)
            });
            client.finish_session(17)?;
            assert!(client.output.is_empty());
            drop(client);
            Ok(reader.join().map_err(|_| "finish reader panicked")??)
        })?;

        assert_eq!(actual, message.repeat(1024));
        Ok(())
    }
}
