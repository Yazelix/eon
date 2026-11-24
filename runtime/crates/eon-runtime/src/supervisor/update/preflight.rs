use super::{Target, handoff, validate_target};
use crate::{
    checkpoint::{self, Header, update::Scope},
    control::Error,
    diagnostics::DiagnosticLog,
    generation::Identity,
    managed_environment::nonempty_environment_path,
    platform,
    session_registry::Registry,
    supervisor::{
        Inputs, LaunchMode, LaunchOptions, SupervisorState, WindowSettings,
        configuration_directory, current_generation, generation_directory,
        immutable_store_executable, live_caller_row,
    },
    update::{self, Reply, UpdateListener, refusal},
    windows::WINDOW_RUNTIME,
};
use eon_workspace_protocol::update::{
    self as eonu, Code, Direction, HEADER_BYTES, Kind, Operation, Outcome, PREFIX_BYTES, Preflight,
    Preflighted, Refused, RequestId, TargetCheck, TargetVerdict,
};
use rustix::{
    event::{PollFd, PollFlags, Timespec, poll},
    fs::{OFlags, fcntl_getfl, fcntl_setfl},
};
use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io::{self, PipeReader, PipeWriter, Read, Write},
    os::{fd::AsFd, unix::ffi::OsStrExt},
    path::Path,
    process::{Child, Command, Stdio},
    sync::Arc,
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub(crate) const VALIDATE_UPDATE_TARGET: &str = "--update-validate-target";
const CHILD_DEADLINE: Duration = Duration::from_secs(10);

type Verdict = Result<Preflighted, Refused>;

#[derive(Default)]
pub(in crate::supervisor) struct PreflightState {
    pending: Option<Pending>,
    last: Option<Outcome>,
    #[cfg(test)]
    child_timeout: Option<Duration>,
}

impl PreflightState {
    pub(in crate::supervisor) fn with_last(last: Option<Outcome>) -> Self {
        Self {
            last,
            ..Self::default()
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::supervisor) fn serve(
        &mut self,
        listener: &UpdateListener,
        state: &mut SupervisorState,
        inputs: &Inputs,
        root: &Path,
        runtime: &Path,
        log: &Arc<DiagnosticLog>,
        owner: Option<&handoff::Owner<'_>>,
    ) {
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.worker.as_ref().unwrap().is_finished())
        {
            let mut pending = self.pending.take().unwrap();
            let result = pending.join();
            self.record(pending.request, &result);
            pending.reply.take().unwrap().preflight(result, true);
        }
        listener.serve(log, |kind, bytes, reply| {
            let commit = (kind == Kind::Commit
                && cfg!(test)
                && std::env::var_os("EON_UPDATE_TEST_COMMIT").is_some())
            .then_some(owner)
            .flatten();
            if kind != Kind::Preflight && commit.is_none() {
                reply.inspect(kind, bytes, |request| {
                    let mut inspected = state.inspected(request, runtime)?;
                    if owner.is_some_and(|owner| owner.inherited_attached) {
                        inspected.presentation = eonu::Presentation::Attached;
                    }
                    inspected.last = self.last.clone();
                    Ok(inspected)
                });
                return;
            }
            let request = match if commit.is_some() {
                Preflight::decode_commit(bytes).map_err(|error| {
                    refusal(
                        update::frame_request(bytes),
                        Code::Malformed,
                        error.to_string(),
                    )
                })
            } else {
                update::decode_preflight(bytes)
            } {
                Ok(request) => request,
                Err(error) => {
                    reply.preflight(Err(error), false);
                    return;
                }
            };
            let id = request.request;
            let (facts, caller_origin) = match self.admit(state, inputs, &request) {
                Ok(admitted) => admitted,
                Err(error) => {
                    reply.preflight(Err(error), false);
                    return;
                }
            };
            let internal = |detail| refusal(id, Code::Internal, detail);
            if commit.is_some() && request.target_build == request.expected_active_build {
                let result = Err(refusal(
                    id,
                    Code::TargetRefused,
                    "the target build is already active".into(),
                ));
                self.last = Some(handoff::commit_outcome(id, &result));
                reply.commit(result);
                return;
            }
            let prepare = |state: &mut SupervisorState, request: &Preflight| {
                let launch = inputs
                    .launch
                    .as_ref()
                    .map_err(|error| internal(error.clone()))?;
                let boot = platform::boot_time().map_err(|error| internal(error.to_string()))?;
                let exported = state.export(root, Instant::now(), boot).map_err(|error| {
                    let code = if matches!(error, checkpoint::Error::Unsettled(_)) {
                        Code::Unsettled
                    } else {
                        Code::Internal
                    };
                    refusal(id, code, error.to_string())
                })?;
                let mut command = launch.command(&facts);
                command
                    .args([VALIDATE_UPDATE_TARGET, &request.lifetime])
                    .env(WINDOW_RUNTIME, root);
                let scope = exported.scope;
                let check = TargetCheck {
                    lifetime: scope.header.generation,
                    namespace: scope.header.namespace.into_bytes(),
                    created_build: scope.header.created_build,
                    active_build: scope.header.active_build,
                    identity_range: scope.range,
                    target_build: request.target_build.clone(),
                    hosts: state.update_hosts(),
                    checkpoint: exported.bytes,
                };
                Ok((command, check))
            };
            let timeout = CHILD_DEADLINE;
            #[cfg(test)]
            let timeout = self.child_timeout.unwrap_or(timeout);
            if let Some(owner) = commit {
                let result = (|| {
                    let unsettled = |what: &'static str| refusal(id, Code::Unsettled, what.into());
                    let parked = super::quiesce::Quiesce::start(state).map_err(unsettled)?;
                    parked.within_deadline().map_err(unsettled)?;
                    let (command, check) = prepare(parked.state, &request)?;
                    parked.within_deadline().map_err(unsettled)?;
                    {
                        let (cancelled, _cancel) =
                            io::pipe().map_err(|error| internal(error.to_string()))?;
                        validate_exchange(
                            id,
                            command,
                            &check,
                            &cancelled,
                            caller_origin.clone(),
                            log,
                            timeout,
                        )?;
                    }
                    Err(handoff::begin(
                        owner,
                        listener,
                        parked.state,
                        inputs,
                        root,
                        &facts,
                        request,
                        caller_origin,
                        check,
                        &reply,
                    ))
                })();
                self.last = Some(handoff::commit_outcome(id, &result));
                reply.commit(result);
                return;
            }
            let result = prepare(state, &request).and_then(|(command, check)| {
                Pending::start(id, command, check, caller_origin, Arc::clone(log), timeout)
                    .map_err(|error| internal(format!("cannot start target validation: {error}")))
            });
            match result {
                Ok(mut pending) => {
                    pending.reply = Some(reply);
                    self.pending = Some(pending);
                }
                Err(error) => {
                    let result = Err(error);
                    self.record(id, &result);
                    reply.preflight(result, false);
                }
            }
        });
    }

    fn admit(
        &self,
        state: &SupervisorState,
        inputs: &Inputs,
        request: &Preflight,
    ) -> Result<(crate::LaunchFacts, Option<String>), Refused> {
        let fail = |code, detail: &str| refusal(request.request, code, detail.into());
        if request.lifetime != state.component_generation {
            return Err(fail(Code::Lifetime, "the request names another window"));
        }
        if request.expected_active_build != state.active_build {
            return Err(fail(Code::StaleBuild, "the window's active build changed"));
        }
        if self.pending.is_some() {
            return Err(fail(Code::Busy, "another Update exchange is running"));
        }
        let wrapper = Path::new(OsStr::from_bytes(&request.target_wrapper));
        #[cfg(test)]
        let store = inputs.update_store.as_path();
        #[cfg(not(test))]
        let store = Path::new("/nix/store");
        if !immutable_store_executable(wrapper, store).unwrap_or(false) {
            return Err(fail(
                Code::UntrustedTarget,
                "the target must be a canonical immutable executable in /nix/store",
            ));
        }
        let facts = (inputs.target_launch_facts)(wrapper)
            .map_err(|detail| refusal(request.request, Code::UntrustedTarget, detail))?;
        let origin = if let Some(caller) = &request.caller {
            let session = live_caller_row(state, caller).ok_or_else(|| {
                fail(
                    Code::UnknownCaller,
                    "the calling terminal is not live in this window",
                )
            })?;
            let origin = session.inventory().origin_build.ok_or_else(|| {
                fail(
                    Code::UnknownOrigin,
                    "the calling terminal's build is unknown",
                )
            })?;
            if origin == request.target_build && origin != state.active_build {
                return Err(fail(
                    Code::StaleCommand,
                    "the command came from a build this window has already left",
                ));
            }
            Some(origin)
        } else {
            None
        };
        Ok((facts, origin))
    }

    fn record(&mut self, request: RequestId, result: &Verdict) {
        let (code, detail) = match result {
            Ok(reply) => (None, format!("target EONW {}", reply.eonw)),
            Err(refused) => (Some(refused.code), refused.detail.clone()),
        };
        self.last = Some(Outcome {
            request,
            operation: Operation::Preflight,
            refusal: code,
            detail,
        });
    }
}

struct Pending {
    request: RequestId,
    reply: Option<Reply>,
    cancel: Option<PipeWriter>,
    worker: Option<JoinHandle<Verdict>>,
}

impl Pending {
    fn start(
        request: RequestId,
        command: Command,
        check: TargetCheck,
        caller_origin: Option<String>,
        log: Arc<DiagnosticLog>,
        timeout: Duration,
    ) -> io::Result<Self> {
        let (cancelled, cancel) = io::pipe()?;
        let worker = thread::Builder::new()
            .name("eon-update-preflight".into())
            .spawn(move || {
                validate_exchange(
                    request,
                    command,
                    &check,
                    &cancelled,
                    caller_origin,
                    &log,
                    timeout,
                )
            })?;
        Ok(Self {
            request,
            reply: None,
            cancel: Some(cancel),
            worker: Some(worker),
        })
    }

    fn join(&mut self) -> Verdict {
        drop(self.cancel.take());
        self.worker.take().unwrap().join().unwrap_or_else(|_| {
            Err(refusal(
                self.request,
                Code::Internal,
                "target validation worker panicked".into(),
            ))
        })
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        if self.worker.is_some() {
            let _ = self.join();
        }
    }
}

struct ValidationChild(Child);

impl Drop for ValidationChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn validate_exchange(
    request: RequestId,
    command: Command,
    check: &TargetCheck,
    cancelled: &PipeReader,
    caller_origin: Option<String>,
    log: &DiagnosticLog,
    timeout: Duration,
) -> Verdict {
    let result = exchange(command, check, cancelled, log, timeout).map_err(|error| {
        match wire_version(&error) {
            Some(found) => refusal(
                request,
                Code::Version,
                update::version_refusal(eonu::VERSION, found),
            ),
            None => refusal(
                request,
                Code::Internal,
                format!("target validation failed: {error}"),
            ),
        }
    })?;
    match result {
        Ok(verdict) if verdict.target_build == check.target_build => Ok(Preflighted {
            request,
            caller_origin,
            target_build: verdict.target_build,
            eonw: verdict.eonw,
        }),
        Ok(_) => Err(refusal(
            request,
            Code::TargetRefused,
            "validation answered for a different target build".into(),
        )),
        Err(refused) => {
            let code = match refused.code {
                Code::HostProtocol => Code::HostProtocol,
                _ => Code::TargetRefused,
            };
            Err(refusal(request, code, refused.detail))
        }
    }
}

fn exchange(
    mut command: Command,
    check: &TargetCheck,
    cancelled: &PipeReader,
    log: &DiagnosticLog,
    timeout: Duration,
) -> io::Result<Result<TargetVerdict, Refused>> {
    let bytes = check.encode().map_err(io::Error::other)?;
    platform::require_waitable_children()?;
    let deadline = Instant::now() + timeout;
    let mut child = ValidationChild(
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?,
    );
    let mut stdin = child.0.stdin.take();
    let mut stdout = child.0.stdout.take().expect("piped child stdout");
    let mut stderr = child.0.stderr.take().expect("piped child stderr");
    for fd in [
        stdin.as_ref().expect("piped child stdin").as_fd(),
        stdout.as_fd(),
        stderr.as_fd(),
    ] {
        fcntl_setfl(fd, fcntl_getfl(fd)? | OFlags::NONBLOCK)?;
    }

    // Resolve the pipe exchange while the guard owns the child, then reap it
    // before draining any remaining diagnostics. Inherited stderr cannot wait.
    let received = (|| {
        let mut remaining = bytes.as_slice();
        let max = HEADER_BYTES + Kind::Refused.bound().max(Kind::TargetVerdict.bound());
        let mut response = Vec::with_capacity(max);
        let mut buffer = [0; 4096];
        let mut stderr_open = true;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(io::ErrorKind::TimedOut.into());
            }
            let readable = |fd| PollFd::from_borrowed_fd(fd, PollFlags::IN);
            let mut fds = vec![readable(cancelled.as_fd()), readable(stdout.as_fd())];
            let stderr_index = stderr_open.then(|| {
                fds.push(readable(stderr.as_fd()));
                fds.len() - 1
            });
            let stdin_index = stdin.as_ref().map(|pipe| {
                fds.push(PollFd::new(pipe, PollFlags::OUT));
                fds.len() - 1
            });
            let timeout = Timespec {
                tv_sec: left.as_secs() as _,
                tv_nsec: left.subsec_nanos() as _,
            };
            match poll(&mut fds, Some(&timeout)) {
                Ok(_) => {}
                Err(rustix::io::Errno::INTR) => continue,
                Err(error) => return Err(error.into()),
            }
            if !fds[0].revents().is_empty() {
                return Err(io::Error::other(
                    "the window closed during target validation",
                ));
            }
            let stdout_ready = !fds[1].revents().is_empty();
            let stderr_ready = stderr_index.is_some_and(|index| !fds[index].revents().is_empty());
            let stdin_ready = stdin_index.is_some_and(|index| !fds[index].revents().is_empty());
            if stdout_ready {
                let count = next_reply_read(&response)?.min(buffer.len());
                match stdout.read(&mut buffer[..count]) {
                    Ok(0) => {
                        let verdict = decode_verdict(&response)?;
                        if verdict.is_ok() && !remaining.is_empty() {
                            return Err(io::Error::other(
                                "target answered before receiving its checkpoint",
                            ));
                        }
                        return Ok(verdict);
                    }
                    Ok(size) => {
                        if response.len() + size > max {
                            return Err(io::Error::other(
                                "target validation reply exceeds its bound",
                            ));
                        }
                        response.extend_from_slice(&buffer[..size]);
                        next_reply_read(&response)?;
                    }
                    Err(error) if interrupted(&error) => {}
                    Err(error) => return Err(error),
                }
            }
            if stderr_ready {
                match stderr.read(&mut buffer) {
                    Ok(0) => stderr_open = false,
                    Ok(size) => log.append_child_output(&buffer[..size]),
                    Err(error) if interrupted(&error) => {}
                    Err(_) => stderr_open = false,
                }
            }
            if stdin_ready && let Some(pipe) = &mut stdin {
                match pipe.write(remaining) {
                    Ok(size) => {
                        remaining = &remaining[size..];
                        if remaining.is_empty() {
                            stdin = None;
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::BrokenPipe => stdin = None,
                    Err(error) if interrupted(&error) => {}
                    Err(error) => return Err(error),
                }
            }
        }
    })();
    drop(child);
    let mut buffer = [0; 4096];
    for _ in 0..16 {
        match stderr.read(&mut buffer) {
            Ok(size @ 1..) => log.append_child_output(&buffer[..size]),
            _ => break,
        }
    }
    received
}

fn interrupted(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
    )
}

fn require_version(prefix: &[u8]) -> io::Result<()> {
    let version =
        eonu::version(prefix.try_into().expect("EONU prefix")).map_err(io::Error::other)?;
    if version != eonu::VERSION {
        return Err(io::Error::other(eonu::Error::Version(version)));
    }
    Ok(())
}

fn wire_version(error: &io::Error) -> Option<u16> {
    match error.get_ref()?.downcast_ref::<eonu::Error>()? {
        eonu::Error::Version(found) => Some(*found),
        _ => None,
    }
}

fn next_reply_read(bytes: &[u8]) -> io::Result<usize> {
    let Some(prefix) = bytes.get(..PREFIX_BYTES) else {
        return Ok(PREFIX_BYTES - bytes.len());
    };
    require_version(prefix)?;
    let Some(head) = bytes.get(..HEADER_BYTES) else {
        return Ok(HEADER_BYTES - bytes.len());
    };
    let head = head.try_into().expect("EONU header");
    let header = eonu::header(head, Direction::TargetReply).map_err(io::Error::other)?;
    let end = HEADER_BYTES + header.length;
    if bytes.len() > end {
        return Err(io::Error::other(
            "trailing bytes in target validation reply",
        ));
    }
    Ok(end + 1 - bytes.len())
}

fn decode_verdict(bytes: &[u8]) -> io::Result<Result<TargetVerdict, Refused>> {
    let kind = eonu::kind_of(bytes, Direction::TargetReply).map_err(io::Error::other)?;
    if kind == Kind::TargetVerdict {
        return TargetVerdict::decode(bytes)
            .map(Ok)
            .map_err(io::Error::other);
    }
    let refused = Refused::decode(bytes, Direction::TargetReply).map_err(io::Error::other)?;
    if refused.request != [0; 16] {
        return Err(io::Error::other(
            "target validation refusal has a request ID",
        ));
    }
    Ok(Err(refused))
}

pub(crate) fn validate_update_target(
    inputs: &Inputs,
    arguments: &[OsString],
) -> Result<i32, Error> {
    let [lifetime] = arguments else {
        return Err(Error::usage(format!(
            "usage: {VALIDATE_UPDATE_TARGET} LIFETIME"
        )));
    };
    let root = nonempty_environment_path(WINDOW_RUNTIME);
    let mut source = target_stdin().map_err(|error| Error::from(error.to_string()))?;
    let bytes = target_response(inputs, lifetime, root.as_deref(), &mut source)?;
    let mut stdout = io::stdout().lock();
    stdout
        .write_all(&bytes)
        .and_then(|()| stdout.flush())
        .map_err(|error| Error::from(error.to_string()))?;
    Ok(0)
}

#[cfg(test)]
pub(super) fn reply_for_test(inputs: &Inputs, lifetime: &OsStr, out: &mut impl Write) {
    let root = nonempty_environment_path(WINDOW_RUNTIME).unwrap();
    let frame =
        target_response(inputs, lifetime, Some(&root), &mut target_stdin().unwrap()).unwrap();
    out.write_all(&frame).unwrap();
}

fn target_stdin() -> io::Result<File> {
    // Stdin's buffered reader can read beyond the frozen prefix.
    io::stdin().as_fd().try_clone_to_owned().map(File::from)
}

fn target_response(
    inputs: &Inputs,
    lifetime: &OsStr,
    root: Option<&Path>,
    source: &mut impl Read,
) -> Result<Vec<u8>, Error> {
    let result = (|| {
        let failed = |detail| refusal([0; 16], Code::TargetRefused, detail);
        let lifetime = lifetime
            .to_str()
            .filter(|value| eonu::valid_id35(value))
            .ok_or_else(|| failed("invalid validation lifetime".into()))?;
        let root = root
            .filter(|path| path.is_absolute())
            .ok_or_else(|| failed("the validation child has no window runtime directory".into()))?;
        let check = read_check(source).map_err(|error| match wire_version(&error) {
            Some(found) => refusal(
                [0; 16],
                Code::Version,
                update::version_refusal(found, eonu::VERSION),
            ),
            None => refusal([0; 16], Code::Malformed, error.to_string()),
        })?;
        validate_check(inputs, lifetime, root, &check)
    })();
    match result {
        Ok(verdict) => verdict.encode(),
        Err(refused) => refused.encode(),
    }
    .map_err(|error| Error::from(error.to_string()))
}

fn read_check(source: &mut impl Read) -> io::Result<TargetCheck> {
    let mut head = [0; HEADER_BYTES];
    source.read_exact(&mut head[..PREFIX_BYTES])?;
    require_version(&head[..PREFIX_BYTES])?;
    source.read_exact(&mut head[PREFIX_BYTES..])?;
    let header = eonu::header(&head, Direction::TargetCheck).map_err(io::Error::other)?;
    let mut frame = vec![0; HEADER_BYTES + header.length];
    frame[..HEADER_BYTES].copy_from_slice(&head);
    source.read_exact(&mut frame[HEADER_BYTES..])?;
    match source.read_exact(&mut [0]) {
        Ok(()) => Err(io::Error::other("trailing bytes after target check")),
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => {
            TargetCheck::decode(&frame).map_err(io::Error::other)
        }
        Err(error) => Err(error),
    }
}

fn validate_check(
    inputs: &Inputs,
    lifetime: &str,
    root: &Path,
    check: &TargetCheck,
) -> Result<TargetVerdict, Refused> {
    let failed = |detail| refusal([0; 16], Code::TargetRefused, detail);
    if check.lifetime != lifetime || check.namespace != root.as_os_str().as_bytes() {
        return Err(failed("the target check is for another window".into()));
    }
    let identity = Identity::read(&generation_directory(root, lifetime))
        .map_err(failed)?
        .ok_or_else(|| failed("the window has no identity record".into()))?;
    if !identity.matches(lifetime, &check.created_build, &check.active_build) {
        return Err(failed(
            "the window's identity disagrees with the target check".into(),
        ));
    }
    let target_build = current_generation(inputs).map_err(failed)?;
    if check.target_build != target_build {
        return Err(failed("the target check expected another build".into()));
    }
    let config = configuration_directory().map_err(failed)?;
    let settings = WindowSettings::read(
        config,
        LaunchMode::Workspace,
        inputs,
        &LaunchOptions::default(),
        None,
    )
    .map_err(failed)?;
    let target = Target {
        inputs,
        settings: &settings,
        root,
        generation: lifetime,
        base: root,
        registry: Registry::new().ok(),
        log: None,
    };
    check_host_versions(&check.hosts)?;
    let scope = Scope {
        header: Header {
            namespace: root
                .to_str()
                .ok_or_else(|| failed("invalid checkpoint namespace".into()))?
                .into(),
            generation: identity.generation,
            created_build: identity.created_build,
            active_build: identity.active_build,
        },
        range: check.identity_range,
    };
    let boot = platform::boot_time().map_err(|error| failed(error.to_string()))?;
    validate_target(&target, &check.checkpoint, &scope, Instant::now(), boot)
        .map_err(|error| failed(error.to_string()))?;
    Ok(TargetVerdict {
        target_build,
        eonw: eon_workspace_protocol::v50::VERSION,
        update_schema: checkpoint::update::SCHEMA,
    })
}

pub(super) fn check_host_versions(hosts: &[eonu::Host]) -> Result<(), Refused> {
    let management = orbit_protocol::management::VERSION;
    #[cfg(test)]
    let management = management + u16::from(update::fault("host-version"));
    for host in hosts {
        let versions = [
            ("management", Some(host.management), management),
            (
                "record generation",
                Some(host.record_generation),
                orbit_protocol::management::RECORD_GENERATION,
            ),
            ("ORBS", host.orbs, orbit_protocol::session::VERSION),
            ("ORBF", host.orbf, orbit_protocol::VERSION),
        ];
        for (name, found, wanted) in versions {
            if found != Some(wanted) {
                return Err(refusal(
                    [0; 16],
                    Code::HostProtocol,
                    format!(
                        "terminal {} has Orbit {name} {}, the target requires {wanted}; {}",
                        host.terminal,
                        found.map_or_else(|| "unknown".into(), |version| version.to_string()),
                        update::MANUAL_MOVE,
                    ),
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
