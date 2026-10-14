use orbit_protocol::session::{
    ClientMessage, FocusEvent, KeyAction, KeyEvent, Modifiers, MouseAction, MouseButton,
    MouseEvent, PhysicalKey, SelectionAction, SelectionPosition, SurfaceSize,
};
use winit::{
    event::{
        ElementState, Ime, KeyEvent as WinitKeyEvent, MouseButton as WinitMouseButton,
        MouseScrollDelta,
    },
    keyboard::{Key, KeyCode, ModifiersState, PhysicalKey as WinitPhysicalKey},
    platform::modifier_supplement::KeyEventExtModifierSupplement,
};

const IME_REJECTED: &str = "native input method commit is not accepted semantic key text";

/// Stateful translation from native events to Orbit-owned semantic values.
#[derive(Clone, Debug, Default)]
pub struct InputState {
    modifiers: Modifiers,
    focus_event: Option<FocusEvent>,
    pressed_keys: Vec<WinitPhysicalKey>,
    retired_keys: Vec<WinitPhysicalKey>,
    composing: bool,
    preedit: String,
    cursor: (f32, f32),
    pressed_buttons: Vec<WinitMouseButton>,
    scroll: (f64, f64),
    selection_position: Option<SelectionPosition>,
    captured_shortcuts: Vec<WinitPhysicalKey>,
}

impl InputState {
    pub fn set_modifiers(&mut self, modifiers: ModifiersState) {
        let mut result = Modifiers::empty();
        for (active, value) in [
            (modifiers.shift_key(), Modifiers::SHIFT),
            (modifiers.control_key(), Modifiers::CTRL),
            (modifiers.alt_key(), Modifiers::ALT),
            (modifiers.super_key(), Modifiers::SUPER),
        ] {
            if active {
                result = result.union(value);
            }
        }
        self.modifiers = result;
    }

    #[must_use]
    pub fn preedit(&self) -> &str {
        &self.preedit
    }

    #[must_use]
    pub fn modifiers(&self) -> Modifiers {
        self.modifiers
    }

    pub fn native_focus(&mut self, native_focused: bool, terminal_focused: bool) -> ClientMessage {
        let event = if terminal_focused {
            FocusEvent::Gained
        } else {
            FocusEvent::Lost
        };
        if !native_focused {
            self.retire_orbit_generation();
            self.modifiers = Modifiers::empty();
            self.retired_keys.clear();
        }
        self.focus_event = Some(event);
        ClientMessage::Focus(event)
    }

    pub fn terminal_focus(&mut self, focused: bool) -> ClientMessage {
        let event = if focused {
            FocusEvent::Gained
        } else {
            self.retire_orbit_generation();
            FocusEvent::Lost
        };
        self.focus_event = Some(event);
        ClientMessage::Focus(event)
    }

    /// Retire state that can only be paired within one Orbit attachment.
    /// The returned loss retires an attached endpoint without changing the focus replay state.
    pub fn retire_orbit_generation(&mut self) -> Option<ClientMessage> {
        let mut retired = std::mem::take(&mut self.pressed_keys);
        retired.append(&mut self.captured_shortcuts);
        self.retired_keys.extend(retired);
        self.pressed_buttons.clear();
        self.reset_scroll();
        self.cancel_selection();
        self.clear_composition();
        (self.focus_event == Some(FocusEvent::Gained))
            .then_some(ClientMessage::Focus(FocusEvent::Lost))
    }

    /// Consume the remainder of a key sequence retired with an old attachment.
    pub fn suppresses_retired_key(
        &mut self,
        key: WinitPhysicalKey,
        state: ElementState,
        repeat: bool,
    ) -> bool {
        let Some(index) = self
            .retired_keys
            .iter()
            .position(|candidate| *candidate == key)
        else {
            return false;
        };
        match state {
            ElementState::Released => {
                self.retired_keys.swap_remove(index);
                true
            }
            ElementState::Pressed if repeat => true,
            ElementState::Pressed => {
                self.retired_keys.swap_remove(index);
                false
            }
        }
    }

    #[must_use]
    pub fn latest_focus(&self) -> Option<ClientMessage> {
        self.focus_event.map(ClientMessage::Focus)
    }

    #[must_use]
    pub fn key(&self, event: &WinitKeyEvent) -> Option<ClientMessage> {
        if !self.key_is_current(event.physical_key, event.state, event.repeat) {
            return None;
        }
        let code = match event.physical_key {
            WinitPhysicalKey::Code(code) => Some(code),
            WinitPhysicalKey::Unidentified(_) => None,
        };
        let text = event.text.as_deref().and_then(key_text);
        let (unshifted_codepoint, consumed_modifiers) = layout_metadata(
            self.modifiers,
            text.as_deref(),
            event.key_without_modifiers(),
        );
        Some(ClientMessage::Key(KeyEvent {
            action: match (event.state, event.repeat) {
                (ElementState::Released, _) => KeyAction::Release,
                (ElementState::Pressed, true) => KeyAction::Repeat,
                (ElementState::Pressed, false) => KeyAction::Press,
            },
            key: code.map_or(PhysicalKey::UNIDENTIFIED, physical_key),
            modifiers: self.modifiers,
            consumed_modifiers,
            composing: self.composing,
            text,
            unshifted_codepoint,
        }))
    }

    fn key_is_current(&self, key: WinitPhysicalKey, state: ElementState, repeat: bool) -> bool {
        self.pressed_keys.contains(&key) || (state == ElementState::Pressed && !repeat)
    }

    /// Record an admitted pressed state or forget an observed release.
    pub fn commit_key(&mut self, key: WinitPhysicalKey, state: ElementState) {
        self.pressed_keys.retain(|pressed| *pressed != key);
        if state == ElementState::Pressed {
            self.pressed_keys.push(key);
        }
    }

    /// Track native IME state and return a semantic commit or explicit rejection.
    pub fn ime(&mut self, event: Ime) -> Result<Option<ClientMessage>, &'static str> {
        match event {
            Ime::Enabled => Ok(None),
            Ime::Disabled => {
                self.clear_composition();
                Ok(None)
            }
            Ime::Preedit(text, _) => {
                self.composing = !text.is_empty();
                self.preedit = text;
                Ok(None)
            }
            Ime::Commit(text) => {
                self.clear_composition();
                if text.is_empty() {
                    return Ok(None);
                }
                let text = key_text(&text).ok_or(IME_REJECTED)?;
                Ok(Some(ClientMessage::Key(KeyEvent {
                    action: KeyAction::Press,
                    key: PhysicalKey::UNIDENTIFIED,
                    modifiers: self.modifiers,
                    consumed_modifiers: Modifiers::empty(),
                    composing: false,
                    text: Some(text),
                    unshifted_codepoint: None,
                })))
            }
        }
    }

    fn clear_composition(&mut self) {
        self.composing = false;
        self.preedit.clear();
    }

    pub fn move_pointer(&mut self, x: f64, y: f64) -> Option<ClientMessage> {
        let (x, y) = coordinates(x, y)?;
        self.cursor = (x, y);
        Some(ClientMessage::Mouse(MouseEvent {
            action: MouseAction::Motion,
            button: self.pressed_buttons.last().copied().map(mouse_button),
            modifiers: self.modifiers,
            x,
            y,
        }))
    }

    /// Build a button transition without committing state before it is queued.
    pub fn mouse_button(
        &self,
        state: ElementState,
        button: WinitMouseButton,
        accept_press: bool,
    ) -> Option<ClientMessage> {
        if self.is_selecting() {
            return None;
        }
        match state {
            ElementState::Pressed if !accept_press || self.pressed_buttons.contains(&button) => {
                return None;
            }
            ElementState::Released if !self.pressed_buttons.contains(&button) => return None,
            _ => {}
        }
        let action = match state {
            ElementState::Pressed => MouseAction::Press,
            ElementState::Released => MouseAction::Release,
        };
        let button = mouse_button(button);
        Some(ClientMessage::Mouse(MouseEvent {
            action,
            button: Some(button),
            modifiers: self.modifiers,
            x: self.cursor.0,
            y: self.cursor.1,
        }))
    }

    /// Record an admitted pressed state or forget an observed release.
    pub fn commit_mouse_button(&mut self, state: ElementState, button: WinitMouseButton) {
        self.pressed_buttons.retain(|pressed| *pressed != button);
        if state == ElementState::Pressed {
            self.pressed_buttons.push(button);
        }
    }

    /// Normalize one native wheel event into a bounded number of whole Orbit steps.
    pub fn wheel(
        &mut self,
        delta: MouseScrollDelta,
        cell_width: f32,
        cell_height: f32,
    ) -> Vec<ClientMessage> {
        if self.is_selecting() {
            return Vec::new();
        }
        const MAX_STEPS: usize = 32;
        let (horizontal, vertical) = match delta {
            MouseScrollDelta::LineDelta(horizontal, vertical) => {
                (f64::from(horizontal), f64::from(vertical))
            }
            MouseScrollDelta::PixelDelta(position)
                if cell_width.is_finite()
                    && cell_height.is_finite()
                    && cell_width > 0.0
                    && cell_height > 0.0 =>
            {
                (
                    position.x / f64::from(cell_width),
                    position.y / f64::from(cell_height),
                )
            }
            MouseScrollDelta::PixelDelta(_) => return Vec::new(),
        };
        if !horizontal.is_finite() || !vertical.is_finite() {
            return Vec::new();
        }

        self.scroll.0 += horizontal;
        self.scroll.1 += vertical;
        let horizontal = self.scroll.0.trunc();
        let vertical = self.scroll.1.trunc();
        self.scroll.0 %= 1.0;
        self.scroll.1 %= 1.0;

        let mut messages = Vec::with_capacity(MAX_STEPS);
        for (steps, positive, negative) in [
            (vertical, MouseButton::Four, MouseButton::Five),
            (horizontal, MouseButton::Six, MouseButton::Seven),
        ] {
            let button = if steps > 0.0 { positive } else { negative };
            let count = (steps.abs().min(MAX_STEPS as f64) as usize)
                .min(MAX_STEPS.saturating_sub(messages.len()));
            messages.extend((0..count).map(|_| {
                ClientMessage::Mouse(MouseEvent {
                    action: MouseAction::Press,
                    button: Some(button),
                    modifiers: self.modifiers,
                    x: self.cursor.0,
                    y: self.cursor.1,
                })
            }));
        }
        messages
    }

    pub fn reset_scroll(&mut self) {
        self.scroll = (0.0, 0.0);
    }

    #[must_use]
    pub fn selection_button(
        &self,
        state: ElementState,
        button: WinitMouseButton,
        size: SurfaceSize,
        frame_revision: Option<u64>,
        time_ns: u64,
    ) -> Option<ClientMessage> {
        if button != WinitMouseButton::Left {
            return None;
        }
        match state {
            ElementState::Pressed
                if self.selection_position.is_none() && self.pressed_buttons.is_empty() =>
            {
                Some(ClientMessage::Selection(SelectionAction::Begin {
                    frame_revision: frame_revision?,
                    position: selection_position(self.cursor, size, false)?,
                    time_ns,
                    modifiers: self.modifiers,
                }))
            }
            ElementState::Released if self.selection_position.is_some() => {
                Some(ClientMessage::Selection(SelectionAction::Finish {
                    position: selection_position(self.cursor, size, true)?,
                    modifiers: self.modifiers,
                }))
            }
            _ => None,
        }
    }

    #[must_use]
    pub fn selection_motion(&self, size: SurfaceSize) -> Option<ClientMessage> {
        let previous = self.selection_position?;
        let position = selection_position(self.cursor, size, true)?;
        (position != previous).then_some(ClientMessage::Selection(SelectionAction::Update {
            position,
            modifiers: self.modifiers,
        }))
    }

    pub fn commit_selection(&mut self, message: &ClientMessage) {
        self.selection_position = match message {
            ClientMessage::Selection(SelectionAction::Begin { position, .. })
            | ClientMessage::Selection(SelectionAction::Update { position, .. }) => Some(*position),
            ClientMessage::Selection(SelectionAction::Finish { .. } | SelectionAction::Cancel) => {
                None
            }
            _ => self.selection_position,
        };
    }

    pub fn cancel_selection(&mut self) {
        self.selection_position = None;
    }

    #[must_use]
    pub fn is_selecting(&self) -> bool {
        self.selection_position.is_some()
    }

    #[must_use]
    pub fn selection_at_top(&self, size: SurfaceSize) -> Option<SelectionPosition> {
        if !self.is_selecting() {
            return None;
        }
        let position = selection_position(self.cursor, size, false)?;
        (position.y < size.padding_top as f32 + size.cell_height as f32).then_some(position)
    }

    #[must_use]
    pub fn pointer_busy(&self) -> bool {
        self.is_selecting() || !self.pressed_buttons.is_empty()
    }

    #[must_use]
    pub fn consumes_shortcut(
        &mut self,
        key: WinitPhysicalKey,
        state: ElementState,
        repeat: bool,
        recognized: bool,
    ) -> bool {
        if let Some(index) = self
            .captured_shortcuts
            .iter()
            .position(|candidate| *candidate == key)
        {
            if state == ElementState::Released {
                self.captured_shortcuts.swap_remove(index);
            }
            return true;
        }
        if state == ElementState::Pressed && !repeat && recognized {
            self.captured_shortcuts.push(key);
            return true;
        }
        false
    }
}

fn selection_position(
    cursor: (f32, f32),
    size: SurfaceSize,
    clamp: bool,
) -> Option<SelectionPosition> {
    let axis = |coordinate: f32, padding: u32, cell: u32, count: u16| {
        if cell == 0 || count == 0 {
            return None;
        }
        let start = padding as f32;
        let end = start + cell as f32 * f32::from(count);
        if !clamp && !(start..end).contains(&coordinate) {
            return None;
        }
        Some(coordinate.clamp(start, end - 0.5))
    };
    Some(SelectionPosition {
        x: axis(cursor.0, size.padding_left, size.cell_width, size.cols)?,
        y: axis(cursor.1, size.padding_top, size.cell_height, size.rows)?,
    })
}

/// Map winit's physical identity directly into Orbit's accepted semantic type.
#[must_use]
fn physical_key(code: KeyCode) -> PhysicalKey {
    use KeyCode as W;
    match code {
        W::Backquote => PhysicalKey::BACKQUOTE,
        W::Backslash => PhysicalKey::BACKSLASH,
        W::BracketLeft => PhysicalKey::BRACKET_LEFT,
        W::BracketRight => PhysicalKey::BRACKET_RIGHT,
        W::Comma => PhysicalKey::COMMA,
        W::Digit0 => PhysicalKey::DIGIT_0,
        W::Digit1 => PhysicalKey::DIGIT_1,
        W::Digit2 => PhysicalKey::DIGIT_2,
        W::Digit3 => PhysicalKey::DIGIT_3,
        W::Digit4 => PhysicalKey::DIGIT_4,
        W::Digit5 => PhysicalKey::DIGIT_5,
        W::Digit6 => PhysicalKey::DIGIT_6,
        W::Digit7 => PhysicalKey::DIGIT_7,
        W::Digit8 => PhysicalKey::DIGIT_8,
        W::Digit9 => PhysicalKey::DIGIT_9,
        W::Equal => PhysicalKey::EQUAL,
        W::IntlBackslash => PhysicalKey::INTL_BACKSLASH,
        W::IntlRo => PhysicalKey::INTL_RO,
        W::IntlYen => PhysicalKey::INTL_YEN,
        W::KeyA => PhysicalKey::A,
        W::KeyB => PhysicalKey::B,
        W::KeyC => PhysicalKey::C,
        W::KeyD => PhysicalKey::D,
        W::KeyE => PhysicalKey::E,
        W::KeyF => PhysicalKey::F,
        W::KeyG => PhysicalKey::G,
        W::KeyH => PhysicalKey::H,
        W::KeyI => PhysicalKey::I,
        W::KeyJ => PhysicalKey::J,
        W::KeyK => PhysicalKey::K,
        W::KeyL => PhysicalKey::L,
        W::KeyM => PhysicalKey::M,
        W::KeyN => PhysicalKey::N,
        W::KeyO => PhysicalKey::O,
        W::KeyP => PhysicalKey::P,
        W::KeyQ => PhysicalKey::Q,
        W::KeyR => PhysicalKey::R,
        W::KeyS => PhysicalKey::S,
        W::KeyT => PhysicalKey::T,
        W::KeyU => PhysicalKey::U,
        W::KeyV => PhysicalKey::V,
        W::KeyW => PhysicalKey::W,
        W::KeyX => PhysicalKey::X,
        W::KeyY => PhysicalKey::Y,
        W::KeyZ => PhysicalKey::Z,
        W::Minus => PhysicalKey::MINUS,
        W::Period => PhysicalKey::PERIOD,
        W::Quote => PhysicalKey::QUOTE,
        W::Semicolon => PhysicalKey::SEMICOLON,
        W::Slash => PhysicalKey::SLASH,
        W::AltLeft => PhysicalKey::ALT_LEFT,
        W::AltRight => PhysicalKey::ALT_RIGHT,
        W::Backspace => PhysicalKey::BACKSPACE,
        W::CapsLock => PhysicalKey::CAPS_LOCK,
        W::ContextMenu => PhysicalKey::CONTEXT_MENU,
        W::ControlLeft => PhysicalKey::CONTROL_LEFT,
        W::ControlRight => PhysicalKey::CONTROL_RIGHT,
        W::Enter => PhysicalKey::ENTER,
        W::SuperLeft => PhysicalKey::META_LEFT,
        W::SuperRight => PhysicalKey::META_RIGHT,
        W::ShiftLeft => PhysicalKey::SHIFT_LEFT,
        W::ShiftRight => PhysicalKey::SHIFT_RIGHT,
        W::Space => PhysicalKey::SPACE,
        W::Tab => PhysicalKey::TAB,
        W::Convert => PhysicalKey::CONVERT,
        W::KanaMode => PhysicalKey::KANA_MODE,
        W::NonConvert => PhysicalKey::NON_CONVERT,
        W::Delete => PhysicalKey::DELETE,
        W::End => PhysicalKey::END,
        W::Help => PhysicalKey::HELP,
        W::Home => PhysicalKey::HOME,
        W::Insert => PhysicalKey::INSERT,
        W::PageDown => PhysicalKey::PAGE_DOWN,
        W::PageUp => PhysicalKey::PAGE_UP,
        W::ArrowDown => PhysicalKey::ARROW_DOWN,
        W::ArrowLeft => PhysicalKey::ARROW_LEFT,
        W::ArrowRight => PhysicalKey::ARROW_RIGHT,
        W::ArrowUp => PhysicalKey::ARROW_UP,
        W::NumLock => PhysicalKey::NUM_LOCK,
        W::Numpad0 => PhysicalKey::NUMPAD_0,
        W::Numpad1 => PhysicalKey::NUMPAD_1,
        W::Numpad2 => PhysicalKey::NUMPAD_2,
        W::Numpad3 => PhysicalKey::NUMPAD_3,
        W::Numpad4 => PhysicalKey::NUMPAD_4,
        W::Numpad5 => PhysicalKey::NUMPAD_5,
        W::Numpad6 => PhysicalKey::NUMPAD_6,
        W::Numpad7 => PhysicalKey::NUMPAD_7,
        W::Numpad8 => PhysicalKey::NUMPAD_8,
        W::Numpad9 => PhysicalKey::NUMPAD_9,
        W::NumpadAdd => PhysicalKey::NUMPAD_ADD,
        W::NumpadBackspace => PhysicalKey::NUMPAD_BACKSPACE,
        W::NumpadClear => PhysicalKey::NUMPAD_CLEAR,
        W::NumpadClearEntry => PhysicalKey::NUMPAD_CLEAR_ENTRY,
        W::NumpadComma => PhysicalKey::NUMPAD_COMMA,
        W::NumpadDecimal => PhysicalKey::NUMPAD_DECIMAL,
        W::NumpadDivide => PhysicalKey::NUMPAD_DIVIDE,
        W::NumpadEnter => PhysicalKey::NUMPAD_ENTER,
        W::NumpadEqual => PhysicalKey::NUMPAD_EQUAL,
        W::NumpadMemoryAdd => PhysicalKey::NUMPAD_MEMORY_ADD,
        W::NumpadMemoryClear => PhysicalKey::NUMPAD_MEMORY_CLEAR,
        W::NumpadMemoryRecall => PhysicalKey::NUMPAD_MEMORY_RECALL,
        W::NumpadMemoryStore => PhysicalKey::NUMPAD_MEMORY_STORE,
        W::NumpadMemorySubtract => PhysicalKey::NUMPAD_MEMORY_SUBTRACT,
        W::NumpadMultiply | W::NumpadStar => PhysicalKey::NUMPAD_MULTIPLY,
        W::NumpadParenLeft => PhysicalKey::NUMPAD_PAREN_LEFT,
        W::NumpadParenRight => PhysicalKey::NUMPAD_PAREN_RIGHT,
        W::NumpadSubtract => PhysicalKey::NUMPAD_SUBTRACT,
        W::Escape => PhysicalKey::ESCAPE,
        W::Fn => PhysicalKey::FN,
        W::FnLock => PhysicalKey::FN_LOCK,
        W::PrintScreen => PhysicalKey::PRINT_SCREEN,
        W::ScrollLock => PhysicalKey::SCROLL_LOCK,
        W::Pause => PhysicalKey::PAUSE,
        W::BrowserBack => PhysicalKey::BROWSER_BACK,
        W::BrowserFavorites => PhysicalKey::BROWSER_FAVORITES,
        W::BrowserForward => PhysicalKey::BROWSER_FORWARD,
        W::BrowserHome => PhysicalKey::BROWSER_HOME,
        W::BrowserRefresh => PhysicalKey::BROWSER_REFRESH,
        W::BrowserSearch => PhysicalKey::BROWSER_SEARCH,
        W::BrowserStop => PhysicalKey::BROWSER_STOP,
        W::Eject => PhysicalKey::EJECT,
        W::LaunchApp1 => PhysicalKey::LAUNCH_APP_1,
        W::LaunchApp2 => PhysicalKey::LAUNCH_APP_2,
        W::LaunchMail => PhysicalKey::LAUNCH_MAIL,
        W::MediaPlayPause => PhysicalKey::MEDIA_PLAY_PAUSE,
        W::MediaSelect => PhysicalKey::MEDIA_SELECT,
        W::MediaStop => PhysicalKey::MEDIA_STOP,
        W::MediaTrackNext => PhysicalKey::MEDIA_TRACK_NEXT,
        W::MediaTrackPrevious => PhysicalKey::MEDIA_TRACK_PREVIOUS,
        W::Power => PhysicalKey::POWER,
        W::Sleep => PhysicalKey::SLEEP,
        W::AudioVolumeDown => PhysicalKey::AUDIO_VOLUME_DOWN,
        W::AudioVolumeMute => PhysicalKey::AUDIO_VOLUME_MUTE,
        W::AudioVolumeUp => PhysicalKey::AUDIO_VOLUME_UP,
        W::WakeUp => PhysicalKey::WAKE_UP,
        W::Copy => PhysicalKey::COPY,
        W::Cut => PhysicalKey::CUT,
        W::Paste => PhysicalKey::PASTE,
        W::F1 => PhysicalKey::F1,
        W::F2 => PhysicalKey::F2,
        W::F3 => PhysicalKey::F3,
        W::F4 => PhysicalKey::F4,
        W::F5 => PhysicalKey::F5,
        W::F6 => PhysicalKey::F6,
        W::F7 => PhysicalKey::F7,
        W::F8 => PhysicalKey::F8,
        W::F9 => PhysicalKey::F9,
        W::F10 => PhysicalKey::F10,
        W::F11 => PhysicalKey::F11,
        W::F12 => PhysicalKey::F12,
        W::F13 => PhysicalKey::F13,
        W::F14 => PhysicalKey::F14,
        W::F15 => PhysicalKey::F15,
        W::F16 => PhysicalKey::F16,
        W::F17 => PhysicalKey::F17,
        W::F18 => PhysicalKey::F18,
        W::F19 => PhysicalKey::F19,
        W::F20 => PhysicalKey::F20,
        W::F21 => PhysicalKey::F21,
        W::F22 => PhysicalKey::F22,
        W::F23 => PhysicalKey::F23,
        W::F24 => PhysicalKey::F24,
        W::F25 => PhysicalKey::F25,
        _ => PhysicalKey::UNIDENTIFIED,
    }
}

fn layout_metadata(
    modifiers: Modifiers,
    text: Option<&str>,
    key: Key,
) -> (Option<char>, Modifiers) {
    let Key::Character(unshifted_text) = key else {
        return (None, Modifiers::empty());
    };
    // Other modifiers can also produce layout text, so infer only Shift by itself.
    let consumed = if modifiers == Modifiers::SHIFT
        && text.is_some_and(|text| text != unshifted_text.as_str())
    {
        Modifiers::SHIFT
    } else {
        Modifiers::empty()
    };
    let mut characters = unshifted_text.chars();
    (
        characters.next().filter(|_| characters.next().is_none()),
        consumed,
    )
}

fn coordinates(x: f64, y: f64) -> Option<(f32, f32)> {
    let maximum = f64::from(u16::MAX);
    (x.is_finite() && y.is_finite())
        .then(|| (x.clamp(0.0, maximum) as f32, y.clamp(0.0, maximum) as f32))
}

fn key_text(text: &str) -> Option<String> {
    (!text.is_empty()
        && text.len() <= orbit_protocol::session::MAX_KEY_TEXT_BYTES
        && !text.chars().any(|character| {
            matches!(
                character,
                '\0'..='\u{1f}' | '\u{7f}' | '\u{f700}'..='\u{f8ff}'
            )
        }))
    .then(|| text.to_owned())
}

fn mouse_button(button: winit::event::MouseButton) -> MouseButton {
    match button {
        winit::event::MouseButton::Left => MouseButton::Left,
        winit::event::MouseButton::Middle => MouseButton::Middle,
        winit::event::MouseButton::Right => MouseButton::Right,
        winit::event::MouseButton::Back => MouseButton::Eight,
        winit::event::MouseButton::Forward => MouseButton::Nine,
        winit::event::MouseButton::Other(10) => MouseButton::Ten,
        winit::event::MouseButton::Other(11) => MouseButton::Eleven,
        winit::event::MouseButton::Other(_) => MouseButton::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_layout_independent_physical_keys() {
        assert_eq!(physical_key(KeyCode::KeyA), PhysicalKey::A);
        assert_eq!(physical_key(KeyCode::IntlRo), PhysicalKey::INTL_RO);
        assert_eq!(
            physical_key(KeyCode::NumpadEnter),
            PhysicalKey::NUMPAD_ENTER
        );
        assert_eq!(physical_key(KeyCode::F25), PhysicalKey::F25);
        assert_eq!(physical_key(KeyCode::Lang1), PhysicalKey::UNIDENTIFIED);
    }

    #[test]
    fn derives_layout_metadata_without_a_keyboard_map() {
        let mut input = InputState::default();
        input.set_modifiers(
            ModifiersState::SHIFT
                | ModifiersState::CONTROL
                | ModifiersState::ALT
                | ModifiersState::SUPER,
        );
        assert_eq!(
            input.modifiers(),
            Modifiers::SHIFT
                .union(Modifiers::CTRL)
                .union(Modifiers::ALT)
                .union(Modifiers::SUPER)
        );

        let character = |modifiers, text, unshifted: &str| {
            layout_metadata(modifiers, text, Key::Character(unshifted.into()))
        };
        for (text, unshifted, expected) in [
            ("?", "/", (Some('/'), Modifiers::SHIFT)),
            ("Ч", "ч", (Some('ч'), Modifiers::SHIFT)),
            ("SS", "ss", (None, Modifiers::SHIFT)),
            (" ", " ", (Some(' '), Modifiers::empty())),
        ] {
            assert_eq!(character(Modifiers::SHIFT, Some(text), unshifted), expected);
        }
        for modifier in [Modifiers::CTRL, Modifiers::ALT, Modifiers::SUPER] {
            assert_eq!(
                character(Modifiers::SHIFT.union(modifier), Some("?"), "/"),
                (Some('/'), Modifiers::empty())
            );
        }
        assert_eq!(
            layout_metadata(
                Modifiers::SHIFT,
                Some("?"),
                Key::Named(winit::keyboard::NamedKey::Enter),
            ),
            (None, Modifiers::empty())
        );
    }

    #[test]
    fn ime_distinguishes_committed_rejected_and_state_only_text() {
        let mut input = InputState::default();
        assert_eq!(input.ime(Ime::Enabled), Ok(None));
        assert_eq!(input.ime(Ime::Preedit("a".into(), Some((1, 1)))), Ok(None));
        assert_eq!(input.preedit(), "a");
        let Ok(Some(ClientMessage::Key(event))) = input.ime(Ime::Commit("啊".into())) else {
            panic!("expected a semantic key commit");
        };
        assert_eq!(event.key, PhysicalKey::UNIDENTIFIED);
        assert_eq!(event.text.as_deref(), Some("啊"));
        assert!(!event.composing);
        assert!(input.preedit().is_empty());
        assert_eq!(input.ime(Ime::Preedit("stale".into(), None)), Ok(None));
        assert_eq!(input.ime(Ime::Disabled), Ok(None));
        assert!(input.preedit().is_empty());

        assert_eq!(input.ime(Ime::Commit(String::new())), Ok(None));
        let exact_bound = "x".repeat(orbit_protocol::session::MAX_KEY_TEXT_BYTES);
        let Ok(Some(ClientMessage::Key(event))) = input.ime(Ime::Commit(exact_bound.clone()))
        else {
            panic!("expected an exact-bound semantic key commit");
        };
        assert_eq!(event.text, Some(exact_bound));

        for rejected in [
            "x".repeat(orbit_protocol::session::MAX_KEY_TEXT_BYTES + 1),
            "\r".into(),
            "\u{f700}".into(),
        ] {
            assert_eq!(input.ime(Ime::Preedit("discarded".into(), None)), Ok(None));
            assert_eq!(input.ime(Ime::Commit(rejected)), Err(IME_REJECTED));
            assert!(input.preedit().is_empty());
        }
    }

    #[test]
    fn focus_loss_clears_transient_native_input_state() {
        let mut input = InputState::default();
        assert_eq!(input.latest_focus(), None);
        let gained = ClientMessage::Focus(FocusEvent::Gained);
        assert_eq!(input.native_focus(true, true), gained);
        assert_eq!(input.latest_focus(), Some(gained));
        input.set_modifiers(ModifiersState::CONTROL);
        assert_eq!(input.ime(Ime::Preedit("compose".into(), None)), Ok(None));
        let key = WinitPhysicalKey::Code(KeyCode::KeyA);
        input.commit_key(key, ElementState::Pressed);
        input.commit_mouse_button(ElementState::Pressed, WinitMouseButton::Left);

        let lost = ClientMessage::Focus(FocusEvent::Lost);
        assert_eq!(input.native_focus(false, false), lost);
        assert_eq!(input.latest_focus(), Some(lost));
        assert!(input.preedit().is_empty());
        assert!(!input.composing);
        let Some(ClientMessage::Mouse(event)) = input.move_pointer(10.0, 20.0) else {
            panic!("expected semantic pointer motion");
        };
        assert_eq!(event.button, None);
        assert_eq!(event.modifiers, Modifiers::empty());
        assert!(!input.suppresses_retired_key(key, ElementState::Released, false));
    }

    #[test]
    fn terminal_focus_preserves_native_modifiers_but_retires_terminal_state() {
        use ElementState::{Pressed, Released};

        let mut input = InputState::default();
        let control = WinitPhysicalKey::Code(KeyCode::ControlLeft);
        let paste = WinitPhysicalKey::Code(KeyCode::Paste);
        input.native_focus(true, true);
        input.set_modifiers(ModifiersState::CONTROL);
        input.commit_key(control, Pressed);
        input.commit_mouse_button(Pressed, WinitMouseButton::Left);
        assert_eq!(input.ime(Ime::Preedit("compose".into(), None)), Ok(None));
        input.commit_selection(&ClientMessage::Selection(SelectionAction::Begin {
            frame_revision: 1,
            position: SelectionPosition { x: 0.0, y: 0.0 },
            time_ns: 0,
            modifiers: Modifiers::empty(),
        }));
        assert!(input.consumes_shortcut(paste, Pressed, false, true));
        assert!(input.consumes_shortcut(
            WinitPhysicalKey::Code(KeyCode::KeyH),
            Pressed,
            false,
            true
        ));

        assert_eq!(
            input.terminal_focus(false),
            ClientMessage::Focus(FocusEvent::Lost)
        );
        assert_eq!(input.modifiers(), Modifiers::CTRL);
        assert!(input.preedit().is_empty());
        assert!(!input.is_selecting());
        assert!(
            input
                .mouse_button(Released, WinitMouseButton::Left, true)
                .is_none()
        );
        assert!(input.suppresses_retired_key(control, Pressed, true));
        assert!(input.suppresses_retired_key(control, Released, false));
        assert!(input.suppresses_retired_key(paste, Released, false));
        assert!(input.suppresses_retired_key(
            WinitPhysicalKey::Code(KeyCode::KeyH),
            Released,
            false
        ));

        assert_eq!(
            input.terminal_focus(true),
            ClientMessage::Focus(FocusEvent::Gained)
        );
        assert_eq!(input.modifiers(), Modifiers::CTRL);
    }

    #[test]
    fn retiring_an_attachment_keeps_only_current_native_truth() {
        use ElementState::{Pressed, Released};

        let mut input = InputState::default();
        let key = WinitPhysicalKey::Code(KeyCode::KeyA);
        input.native_focus(true, true);
        input.set_modifiers(ModifiersState::ALT);
        assert!(!input.key_is_current(key, Released, false));
        assert!(!input.key_is_current(key, Pressed, true));
        assert!(input.key_is_current(key, Pressed, false));
        input.commit_key(key, Pressed);
        assert!(input.key_is_current(key, Pressed, true));
        assert!(input.key_is_current(key, Released, false));
        input.commit_mouse_button(Pressed, WinitMouseButton::Right);
        assert_eq!(input.ime(Ime::Preedit("stale".into(), None)), Ok(None));

        assert_eq!(
            input.retire_orbit_generation(),
            Some(ClientMessage::Focus(FocusEvent::Lost))
        );
        assert_eq!(
            input.latest_focus(),
            Some(ClientMessage::Focus(FocusEvent::Gained))
        );
        assert_eq!(input.modifiers(), Modifiers::ALT);
        assert!(input.preedit().is_empty());
        assert!(
            input
                .mouse_button(Released, WinitMouseButton::Right, true)
                .is_none()
        );
        assert!(input.suppresses_retired_key(key, Released, false));
        assert!(!input.suppresses_retired_key(key, Released, false));
    }

    #[test]
    fn pointer_motion_keeps_the_remaining_pressed_button() {
        for (released, expected) in [
            (WinitMouseButton::Left, MouseButton::Right),
            (WinitMouseButton::Right, MouseButton::Left),
        ] {
            let mut input = InputState::default();
            input.commit_mouse_button(ElementState::Pressed, WinitMouseButton::Left);
            input.commit_mouse_button(ElementState::Pressed, WinitMouseButton::Right);
            input.commit_mouse_button(ElementState::Released, released);
            let Some(ClientMessage::Mouse(event)) = input.move_pointer(10.0, 20.0) else {
                panic!("expected semantic pointer motion");
            };
            assert_eq!(event.button, Some(expected));
        }
    }

    #[test]
    fn button_pairing_survives_presentation_and_queue_gates() {
        let mut input = InputState::default();
        assert!(
            input
                .mouse_button(ElementState::Pressed, WinitMouseButton::Left, false)
                .is_none()
        );
        assert!(
            input
                .mouse_button(ElementState::Released, WinitMouseButton::Left, true)
                .is_none()
        );
        assert!(
            input
                .mouse_button(ElementState::Pressed, WinitMouseButton::Right, true)
                .is_some()
        );
        let Some(ClientMessage::Mouse(motion)) = input.move_pointer(10.0, 20.0) else {
            panic!("expected semantic pointer motion");
        };
        assert_eq!(motion.button, None);

        input.commit_mouse_button(ElementState::Pressed, WinitMouseButton::Right);
        assert!(
            input
                .mouse_button(ElementState::Pressed, WinitMouseButton::Right, true)
                .is_none()
        );
        assert!(matches!(
            input.mouse_button(ElementState::Released, WinitMouseButton::Right, false),
            Some(ClientMessage::Mouse(MouseEvent {
                action: MouseAction::Release,
                button: Some(MouseButton::Right),
                ..
            }))
        ));
        let Some(ClientMessage::Mouse(motion)) = input.move_pointer(10.0, 20.0) else {
            panic!("expected semantic pointer motion");
        };
        assert_eq!(motion.button, Some(MouseButton::Right));

        input.commit_mouse_button(ElementState::Released, WinitMouseButton::Right);
        let Some(ClientMessage::Mouse(motion)) = input.move_pointer(10.0, 20.0) else {
            panic!("expected semantic pointer motion");
        };
        assert_eq!(motion.button, None);
    }

    #[test]
    fn pointer_coordinates_stay_inside_orbit_domain() {
        assert_eq!(coordinates(-1.0, -1.0), Some((0.0, 0.0)));
        let maximum = f64::from(u16::MAX);
        assert_eq!(
            coordinates(maximum + 1.0, maximum + 1.0),
            Some((f32::from(u16::MAX), f32::from(u16::MAX)))
        );
        assert_eq!(coordinates(f64::INFINITY, 0.0), None);
    }

    #[test]
    fn wheel_accumulates_fractional_deltas_caps_bursts_and_resets() {
        let mut input = InputState::default();
        let lines = |x, y| winit::event::MouseScrollDelta::LineDelta(x, y);
        let pixels = |x, y| {
            winit::event::MouseScrollDelta::PixelDelta(winit::dpi::PhysicalPosition::new(x, y))
        };
        let buttons = |messages: Vec<ClientMessage>| {
            messages
                .into_iter()
                .map(|message| match message {
                    ClientMessage::Mouse(event) => event.button.unwrap(),
                    _ => panic!("expected semantic wheel input"),
                })
                .collect::<Vec<_>>()
        };

        assert!(input.wheel(lines(0.0, 0.4), 10.0, 20.0).is_empty());
        assert_eq!(
            buttons(input.wheel(lines(0.0, 0.6), 10.0, 20.0)),
            [MouseButton::Four]
        );
        assert!(input.wheel(pixels(4.0, 8.0), 10.0, 20.0).is_empty());
        assert_eq!(
            buttons(input.wheel(pixels(6.0, 12.0), 10.0, 20.0)),
            [MouseButton::Four, MouseButton::Six]
        );

        let burst = input.wheel(lines(0.0, -1_000.0), 10.0, 20.0);
        assert_eq!(burst.len(), 32);
        assert!(
            buttons(burst)
                .into_iter()
                .all(|button| button == MouseButton::Five)
        );
        assert!(input.wheel(lines(0.0, f32::NAN), 10.0, 20.0).is_empty());

        assert!(input.wheel(lines(0.0, 0.75), 10.0, 20.0).is_empty());
        input.reset_scroll();
        assert!(input.wheel(lines(0.0, 0.25), 10.0, 20.0).is_empty());
    }

    #[test]
    fn left_drag_and_copy_are_explicit_revision_bound_actions() {
        let mut input = InputState::default();
        let size = SurfaceSize {
            cols: 4,
            rows: 3,
            screen_width: 50,
            screen_height: 70,
            cell_width: 10,
            cell_height: 20,
            padding_top: 5,
            padding_bottom: 5,
            padding_left: 5,
            padding_right: 5,
        };
        let expected_update = |x, y| {
            ClientMessage::Selection(SelectionAction::Update {
                position: SelectionPosition { x, y },
                modifiers: Modifiers::empty(),
            })
        };
        input.move_pointer(16.0, 26.0).unwrap();
        input.commit_mouse_button(ElementState::Pressed, WinitMouseButton::Right);
        assert!(
            input
                .selection_button(
                    ElementState::Pressed,
                    WinitMouseButton::Left,
                    size,
                    Some(9),
                    123,
                )
                .is_none()
        );
        input.commit_mouse_button(ElementState::Released, WinitMouseButton::Right);
        let begin = input
            .selection_button(
                ElementState::Pressed,
                WinitMouseButton::Left,
                size,
                Some(9),
                123,
            )
            .unwrap();
        assert_eq!(
            begin,
            ClientMessage::Selection(SelectionAction::Begin {
                frame_revision: 9,
                position: SelectionPosition { x: 16.0, y: 26.0 },
                time_ns: 123,
                modifiers: Modifiers::empty(),
            })
        );
        input.commit_selection(&begin);
        assert!(input.is_selecting());
        assert!(
            input
                .mouse_button(ElementState::Pressed, WinitMouseButton::Right, true)
                .is_none()
        );
        assert!(
            input
                .wheel(MouseScrollDelta::LineDelta(0.0, 1.0), 10.0, 20.0)
                .is_empty()
        );

        input.move_pointer(-1.0, -1.0).unwrap();
        let update = input.selection_motion(size).unwrap();
        assert_eq!(update, expected_update(5.0, 5.0));
        assert!(input.selection_at_top(size).is_none());

        input.move_pointer(16.0, 6.0).unwrap();
        assert_eq!(
            input.selection_at_top(size),
            Some(SelectionPosition { x: 16.0, y: 6.0 })
        );

        input.move_pointer(36.0, 46.0).unwrap();
        let update = input.selection_motion(size).unwrap();
        assert_eq!(update, expected_update(36.0, 46.0));
        input.commit_selection(&update);
        assert!(input.selection_motion(size).is_none());
        let finish = input
            .selection_button(
                ElementState::Released,
                WinitMouseButton::Left,
                size,
                None,
                999,
            )
            .unwrap();
        assert_eq!(
            finish,
            ClientMessage::Selection(SelectionAction::Finish {
                position: SelectionPosition { x: 36.0, y: 46.0 },
                modifiers: Modifiers::empty(),
            })
        );
        assert!(input.is_selecting());
        input.commit_selection(&finish);
        assert!(!input.is_selecting());

        input.set_modifiers(ModifiersState::SHIFT);
        assert!(matches!(
            input.selection_button(
                ElementState::Pressed,
                WinitMouseButton::Left,
                size,
                Some(10),
                456,
            ),
            Some(ClientMessage::Selection(SelectionAction::Begin {
                modifiers: Modifiers::SHIFT,
                ..
            }))
        ));

        input.set_modifiers(ModifiersState::CONTROL | ModifiersState::SHIFT);
        assert!(input.consumes_shortcut(
            WinitPhysicalKey::Code(KeyCode::KeyC),
            ElementState::Pressed,
            false,
            true,
        ));
        input.set_modifiers(ModifiersState::empty());
        assert!(input.consumes_shortcut(
            WinitPhysicalKey::Code(KeyCode::KeyC),
            ElementState::Released,
            false,
            false,
        ));
        assert!(!input.consumes_shortcut(
            WinitPhysicalKey::Code(KeyCode::KeyX),
            ElementState::Pressed,
            false,
            false,
        ));
    }

    #[test]
    fn native_control_text_does_not_replace_physical_key_meaning() {
        assert_eq!(key_text("a"), Some("a".into()));
        assert_eq!(key_text("界"), Some("界".into()));
        assert_eq!(key_text("\r"), None);
        assert_eq!(key_text("\u{f700}"), None);
    }

    #[test]
    fn recognized_shortcuts_are_one_shot() {
        use winit::event::ElementState::{Pressed, Released};

        let mut input = InputState::default();
        let v_physical = WinitPhysicalKey::Code(KeyCode::KeyV);
        assert!(!input.consumes_shortcut(v_physical, Pressed, false, false));
        assert!(input.consumes_shortcut(v_physical, Pressed, false, true));
        assert!(input.consumes_shortcut(v_physical, Pressed, true, false));
        assert!(input.consumes_shortcut(v_physical, Released, false, false));
        assert!(!input.consumes_shortcut(v_physical, Released, false, false));

        let paste_physical = WinitPhysicalKey::Code(KeyCode::Paste);
        assert!(input.consumes_shortcut(paste_physical, Pressed, false, true));
        input.native_focus(false, false);
        assert!(!input.consumes_shortcut(paste_physical, Released, false, false));

        assert!(input.consumes_shortcut(v_physical, Pressed, false, true));
        assert!(input.consumes_shortcut(paste_physical, Pressed, false, true));
        assert!(input.consumes_shortcut(v_physical, Released, false, false));
        assert!(input.consumes_shortcut(paste_physical, Released, false, false));

        assert!(input.consumes_shortcut(v_physical, Pressed, false, true));
        assert!(input.consumes_shortcut(v_physical, Released, false, false));
    }

    #[test]
    fn workspace_shortcut_capture_starts_once_and_retains_release() {
        use ElementState::{Pressed, Released};
        use KeyCode::{Escape, KeyH, KeyJ};

        let mut input = InputState::default();
        let mut shortcut = |key, state, repeat, recognized| {
            input.consumes_shortcut(WinitPhysicalKey::Code(key), state, repeat, recognized)
        };
        assert!(!shortcut(KeyH, Pressed, true, true));
        assert!(shortcut(KeyH, Pressed, false, true));
        assert!(shortcut(KeyH, Pressed, true, true));
        assert!(shortcut(KeyH, Released, false, false));
        assert!(shortcut(Escape, Pressed, false, true));
        assert!(shortcut(Escape, Released, false, false));
        assert!(!shortcut(KeyJ, Released, false, false));
    }
}
