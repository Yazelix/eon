use eon_workspace_protocol::shortcuts::{
    Action, Direction, FIXED_SHORTCUTS, Platform, PopupPolicy, Side, Trigger, shortcut,
};

#[test]
fn documented_fixed_keys_follow_the_shared_defaults() {
    use Action::*;
    let usage = include_str!("../../../../docs/USAGE.md");
    let interaction = include_str!("../../../../venus/docs/INTERACTION.md");
    // Prose identifies the documented action independently of its current key.
    for (action, usage_action, interaction_action) in [
        (
            ToggleViewer,
            "Open or close shortcut help",
            "Open or close Shortcuts",
        ),
        (
            ToggleHints,
            "Show or hide the bottom shortcut hints",
            "Show or hide the bottom shortcut hints",
        ),
        (
            ToggleSidebar,
            "Show or hide the agent sidebar",
            "Show or hide the agent sidebar",
        ),
        (
            NextAttention,
            "Jump to the next or previous agent",
            "Send `NextAttention`",
        ),
        (
            PreviousAttention,
            "Jump to the next or previous agent",
            "Send `NextAttention`",
        ),
        (
            FocusTabPosition,
            "Select tab positions",
            "Select tab positions",
        ),
        (
            NextTab,
            "Select the next or previous tab",
            "Select the next or previous tab",
        ),
        (
            PreviousTab,
            "Select the next or previous tab",
            "Select the next or previous tab",
        ),
        (
            Focus(Direction::Left),
            "Select the column to the left",
            "Walk columns",
        ),
        (
            Focus(Direction::Right),
            "Select the column to the left",
            "Walk columns",
        ),
        (
            Focus(Direction::Up),
            "Select the previous or next pane",
            "Walk the selected column",
        ),
        (
            Focus(Direction::Down),
            "Select the previous or next pane",
            "Walk the selected column",
        ),
        (
            MovePaneSideways(Side::Left),
            "Move the selected pane into the column",
            "Move the selected pane into the neighbouring column",
        ),
        (
            MovePaneSideways(Side::Right),
            "Move the selected pane into the column",
            "Move the selected pane into the neighbouring column",
        ),
        (
            Move(Direction::Up),
            "Move the selected pane up or down",
            "Move the selected pane within its column",
        ),
        (
            Move(Direction::Down),
            "Move the selected pane up or down",
            "Move the selected pane within its column",
        ),
        (
            Move(Direction::Left),
            "Move the active tab",
            "Move the active tab",
        ),
        (
            Move(Direction::Right),
            "Move the active tab",
            "Move the active tab",
        ),
        (
            CreateTab,
            "Create a tab and choose its directory",
            "Request a pending tab",
        ),
        (
            CreatePane,
            "Create a pane in the selected column",
            "Create a pane in the selected column",
        ),
        (
            CreateColumn,
            "Create a column right",
            "Create a column right",
        ),
        (
            ToggleZoom,
            "Show the selected pane alone",
            "Show the selected pane alone",
        ),
        (
            ClosePane,
            "Close the selected pane",
            "Close the selected pane",
        ),
        (CloseTab, "Close the active tab", "Close the active tab"),
        (
            NewWindow,
            "Open an independent Eon window",
            "Open an independent Eon window",
        ),
        (Detach, "Detach: hide the window", "Detach: hide the window"),
        (
            Quit,
            "Quit this window, stopping its terminals",
            "Quit: close the window",
        ),
    ] {
        for (document, description) in [(usage, usage_action), (interaction, interaction_action)] {
            let keys = document
                .lines()
                .find_map(|line| {
                    let mut columns = line.strip_prefix('|')?.split('|');
                    let keys = columns.next()?;
                    columns.next()?.contains(description).then_some(keys)
                })
                .unwrap_or_else(|| panic!("missing documented action: {description}"));
            for chord in shortcut(action).trigger.chords(Platform::Linux) {
                if shortcut(action).trigger == Trigger::AltDigits
                    && !matches!(chord.key, "Digit1" | "Digit9" | "Digit0")
                {
                    continue;
                }
                assert!(
                    keys.contains(&chord.display(Platform::Linux)),
                    "{description}: missing {chord:?}"
                );
            }
        }
    }
}

#[test]
fn configuration_names_both_protected_key_policies() {
    let document = include_str!("../../../../docs/CONFIGURATION.md");
    let protected = document
        .split("Alt+/ and Alt+0 through Alt+9 are reserved")
        .nth(1)
        .unwrap();
    assert!(protected.contains("loads but gets no key"));
    for descriptor in FIXED_SHORTCUTS {
        if descriptor.popup_policy() == PopupPolicy::Overridable {
            continue;
        }
        for chord in descriptor.trigger.chords(Platform::Linux) {
            // The positional family is written as its two range endpoints.
            if descriptor.trigger == Trigger::AltDigits && !matches!(chord.key, "Digit0" | "Digit9")
            {
                continue;
            }
            assert!(
                document.contains(&chord.display(Platform::Linux)),
                "missing protected key {chord:?}"
            );
        }
    }
}
