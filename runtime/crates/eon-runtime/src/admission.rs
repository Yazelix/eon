//! EONW admission (EON-C10 Control authority): every request is admitted by
//! who asks and by every effect it may have, before any effect.
//!
//! The classifier, [`super::workspace::required_effects`], lives beside the
//! workspace state whose effects it predicts.

use super::control::failure;
use super::managed_environment::TerminalAccess;
use eon_workspace_protocol::v47::{Action, Failure, InvokeIntent, Request};

/// Who asks. Every client of the window's workspace socket is the person
/// (`Owner`), with or without a calling terminal; `EON_TERMINAL` only routes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Principal {
    Owner,
    /// The supervised presentation process, identified by its operating-system
    /// peer identity and process start identity: owner authority plus the
    /// native Scrollback key.
    Native,
    /// The owner without one effect: proves admission precedes effects.
    #[cfg(test)]
    Denying(Effect),
}

/// What a request may do. A request requires every effect it may have in the
/// current state, not one class per verb.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Effect {
    /// Workspace and runtime metadata, and the event stream.
    Inspect,
    /// Selection, focus, names, layout and sidebar, including hiding a
    /// keep-alive popup; no process start or stop.
    Arrange,
    /// Terminal text or history.
    Read,
    /// Terminal input.
    Send,
    /// May stop a terminal, including a popup dismissed or displaced as a
    /// side effect.
    Close,
    /// May start a process.
    Run,
    /// Presentation and window lifecycle: never grantable.
    Lifecycle,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Effects(u8);

impl Effects {
    pub(crate) const NONE: Self = Self(0);

    pub(crate) const fn of(effect: Effect) -> Self {
        Self(1 << effect as u8)
    }

    pub(crate) const fn with(self, effect: Effect) -> Self {
        Self(self.0 | Self::of(effect).0)
    }

    pub(crate) fn with_if(self, condition: bool, effect: Effect) -> Self {
        if condition { self.with(effect) } else { self }
    }

    pub(crate) const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub(crate) const fn contains(self, effect: Effect) -> bool {
        self.0 & Self::of(effect).0 != 0
    }
}

/// A request's effects, and whether `[agents] terminal_access` governs it:
/// exactly the terminal-content commands (EON-C10, EON-C35, EON-C36).
/// Effects describe what a request may do; `terminal_access` is an owner
/// preference over content commands only, so a popup key that sends a fixed
/// key to Eon's own helper terminal stays ungoverned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Required {
    pub(crate) effects: Effects,
    pub(crate) governed: bool,
}

impl Required {
    pub(crate) const fn new(effects: Effects) -> Self {
        Self {
            effects,
            governed: false,
        }
    }

    pub(crate) const fn governed(effects: Effects) -> Self {
        Self {
            effects,
            governed: true,
        }
    }
}

/// The Reserved Stop.
pub(crate) const STOP: Required = Required::new(Effects::of(Effect::Lifecycle));

/// Admits `request` (absent for the Reserved Stop) for `principal`. A refusal
/// happens before any effect and changes nothing.
pub(crate) fn admit(
    principal: Principal,
    request: Option<&Request>,
    required: Required,
    access: TerminalAccess,
) -> Result<(), Failure> {
    #[cfg(test)]
    if let Principal::Denying(effect) = principal
        && required.effects.contains(effect)
    {
        return Err(failure(
            "not-granted",
            format!("this principal is denied {effect:?}"),
        ));
    }
    // The window's Scrollback key is the person's own action (EON-C10).
    let native_key = principal == Principal::Native && request.is_some_and(is_native_key);
    if required.governed && access == TerminalAccess::None && !native_key {
        return Err(terminal_access_off());
    }
    Ok(())
}

/// A callerless Toggle or Focus invocation: what the window's popup keys send.
fn is_native_key(request: &Request) -> bool {
    request.caller.is_none()
        && matches!(
            request.action,
            Action::InvokePopup {
                intent: InvokeIntent::Toggle | InvokeIntent::Focus,
                ..
            }
        )
}

fn terminal_access_off() -> Failure {
    failure(
        "terminal-access-off",
        "terminal content access is off; a person can set [agents] terminal_access = \"window\" in Eon's config.toml and open a new window; this lets any same-user client with socket access read and send across all sessions in that window",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use eon_workspace_protocol::v47::{InvokeIntent, SendTarget, TerminalInput};

    fn request(caller: Option<&str>, action: Action) -> Request {
        Request {
            id: "r".into(),
            caller: caller.map(str::to_owned),
            action,
        }
    }

    fn scrollback(intent: InvokeIntent) -> Action {
        Action::InvokePopup {
            tab: "t1".into(),
            entry: "scrollback".into(),
            expected_instance: None,
            intent,
        }
    }

    #[test]
    fn admit_keeps_owner_and_native_terminal_access_as_today() {
        let read = Required::governed(Effects::of(Effect::Read));
        let scrollback_read = Required::governed(
            Effects::of(Effect::Run)
                .with(Effect::Arrange)
                .with(Effect::Read),
        );
        let send = Required::governed(Effects::of(Effect::Send));
        let popup_key = Required::new(Effects::of(Effect::Run).with(Effect::Send));
        let send_request = request(
            None,
            Action::SendTerminal {
                target: SendTarget::Id("p1".into()),
                input: TerminalInput::Enter,
                force: false,
            },
        );
        let off = |result: Result<(), Failure>| {
            result.is_err_and(|error| error.code == "terminal-access-off")
        };
        for principal in [Principal::Owner, Principal::Native] {
            // Under `window` nothing is refused.
            for required in [read, send, scrollback_read, popup_key, STOP] {
                assert_eq!(
                    admit(
                        principal,
                        Some(&send_request),
                        required,
                        TerminalAccess::Window
                    ),
                    Ok(())
                );
            }
            assert!(off(admit(
                principal,
                Some(&send_request),
                send,
                TerminalAccess::None
            )));
            assert!(off(admit(
                principal,
                Some(&request(
                    None,
                    Action::ReadTerminal {
                        target: "p1".into()
                    }
                )),
                read,
                TerminalAccess::None
            )));
            // A fixed key into Eon's own pair or picker is not a content command.
            assert_eq!(
                admit(
                    principal,
                    Some(&request(None, scrollback(InvokeIntent::Toggle))),
                    popup_key,
                    TerminalAccess::None
                ),
                Ok(())
            );
            assert_eq!(admit(principal, None, STOP, TerminalAccess::None), Ok(()));
        }
        // Only a callerless native Toggle or Focus is exempt.
        for intent in [InvokeIntent::Toggle, InvokeIntent::Focus] {
            let key = request(None, scrollback(intent));
            assert_eq!(
                admit(
                    Principal::Native,
                    Some(&key),
                    scrollback_read,
                    TerminalAccess::None
                ),
                Ok(())
            );
            assert!(off(admit(
                Principal::Owner,
                Some(&key),
                scrollback_read,
                TerminalAccess::None
            )));
            assert!(off(admit(
                Principal::Native,
                Some(&request(Some("terminal-1.run-1"), scrollback(intent))),
                scrollback_read,
                TerminalAccess::None
            )));
        }
        for refused in [
            request(None, scrollback(InvokeIntent::Show)),
            request(
                None,
                Action::Workspace(eon_workspace_protocol::v47::WorkspaceAction::FocusId(
                    "u1".into(),
                )),
            ),
        ] {
            assert!(off(admit(
                Principal::Native,
                Some(&refused),
                scrollback_read,
                TerminalAccess::None
            )));
        }
    }

    #[test]
    fn a_denying_principal_refuses_exactly_its_effect() {
        let effects = [
            Effect::Inspect,
            Effect::Arrange,
            Effect::Read,
            Effect::Send,
            Effect::Close,
            Effect::Run,
            Effect::Lifecycle,
        ];
        for denied in effects {
            for effect in effects {
                let result = admit(
                    Principal::Denying(denied),
                    None,
                    Required::new(Effects::of(effect).with(Effect::Inspect)),
                    TerminalAccess::Window,
                );
                let refused = denied == effect || denied == Effect::Inspect;
                assert_eq!(
                    result.is_err_and(|error| error.code == "not-granted"),
                    refused,
                    "{denied:?} {effect:?}"
                );
            }
        }
    }
}
