use super::*;
use eon_workspace_protocol::v24::Pane;

mod bar;
mod links;
mod sidebar;
mod terminal;
mod workspace;

fn draw_style(selected: bool) -> DrawStyle {
    DrawStyle {
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
        selected,
        background_is_default: !selected,
        protected: false,
        underline: Underline::None,
    }
}

fn draw_cell(text: &str, width: CellWidth, selected: bool) -> DrawCell {
    DrawCell {
        width,
        text: text.into(),
        hyperlink: String::new(),
        style: draw_style(selected),
    }
}
