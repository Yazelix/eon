//! ORB-C17's visible-text read: the active screen's live area as lossy plain
//! text, which libghostty's plain-text formatter extracts row by row. It
//! reads the terminal only: no terminal, viewport, selection or presentation
//! state changes, and nothing reaches the PTY.

use crate::frame::title_and_directory;
use libghostty_vt::{
    Terminal,
    fmt::Format,
    screen::Screen as GhosttyScreen,
    selection::{FormatOptions, Selection},
    terminal::{Point, PointCoordinate},
};
use orbit_protocol::{
    management::{MAX_VISIBLE_TEXT_BYTES, Screen, VisibleText},
    session::MAX_METADATA_BYTES,
};

/// The live area of the active screen at `revision`: one line per display
/// row, top to bottom, joined by line feeds, each with its trailing blanks
/// trimmed and wide-cell spacers omitted, so a soft-wrapped row ends like a
/// hard one. A control character other than the line feeds becomes U+FFFD.
/// When the text passes 1 MiB, leading whole rows are left out and counted.
/// None when extraction fails, which the reply refuses, never partial text.
pub(crate) fn read(terminal: &Terminal<'static, '_>, revision: u64) -> Option<VisibleText> {
    let rows = terminal.rows().ok()?;
    let columns = terminal.cols().ok()?;
    let screen = match terminal.active_screen().ok()? {
        GhosttyScreen::Alternate => Screen::Alternate,
        _ => Screen::Primary,
    };
    let lines = (0..rows)
        .map(|y| row_text(terminal, y, columns))
        .collect::<Option<Vec<_>>>()?;
    let (omitted_rows, text) = fit(&lines);
    let title = title_and_directory(terminal).0.to_owned();
    let directory = terminal.pwd().ok()?.to_vec();
    if title.len() + directory.len() > MAX_METADATA_BYTES {
        return None;
    }
    Some(VisibleText {
        revision,
        screen,
        rows,
        columns,
        title,
        directory,
        omitted_rows,
        lossy: true,
        text,
    })
}

/// One row of the active area as plain trimmed text.
fn row_text(terminal: &Terminal<'static, '_>, y: u16, columns: u16) -> Option<String> {
    if columns == 0 {
        return Some(String::new());
    }
    let at = |x: u16| {
        terminal
            .grid_ref(Point::Active(PointCoordinate { x, y: y.into() }))
            .ok()
    };
    let selection = Selection::new(at(0)?, at(columns - 1)?, false);
    let options = FormatOptions::new()
        .with_emit_format(Format::Plain)
        .with_unwrap(false)
        .with_trim(true)
        .with_selection(&selection);
    let bytes = terminal.format_selection_alloc(None, options).ok()?;
    let bytes = bytes.as_deref().unwrap_or_default();
    let text = std::str::from_utf8(bytes).ok()?;
    Some(sanitized(text.trim_end_matches('\n')))
}

/// The row with every control character, line feeds included, as U+FFFD, so
/// the row stays one line.
fn sanitized(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { '\u{fffd}' } else { c })
        .collect()
}

/// The rows joined by line feeds, leaving out as few leading rows as keep the
/// text within 1 MiB, and how many it left out.
fn fit(lines: &[String]) -> (u32, String) {
    let mut size: usize =
        lines.iter().map(String::len).sum::<usize>() + lines.len().saturating_sub(1);
    let mut first = 0;
    while size > MAX_VISIBLE_TEXT_BYTES && first < lines.len() {
        size -= lines[first].len() + usize::from(first + 1 < lines.len());
        first += 1;
    }
    (
        u32::try_from(first).unwrap_or(u32::MAX),
        lines[first..].join("\n"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(terminal: &Terminal<'static, 'static>) -> VisibleText {
        read(terminal, 7).unwrap()
    }

    /// The live area reads as one trimmed line per row, soft wraps as line
    /// breaks, wide characters without spacers and combining marks kept.
    #[test]
    fn the_live_area_reads_as_one_line_per_row() {
        let mut terminal = Terminal::new(6, 4).unwrap();
        terminal.vt_write("ab  \r\nabcdefgh\r\n漢字e\u{301}".as_bytes());
        let visible = text(&terminal);
        assert_eq!(visible.text, "ab\nabcdef\ngh\n漢字e\u{301}");
        assert_eq!((visible.rows, visible.columns), (4, 6));
        assert_eq!(visible.screen, Screen::Primary);
        assert_eq!((visible.revision, visible.omitted_rows), (7, 0));
        assert!(visible.lossy);
    }

    /// The live area ignores the viewport: a terminal scrolled into history
    /// still reads its bottom rows, and reading moves nothing.
    #[test]
    fn reading_ignores_and_keeps_the_viewport() {
        let mut terminal = Terminal::new(8, 2).unwrap();
        terminal.set_scrollback_max_bytes(Some(4096)).unwrap();
        terminal.vt_write(b"one\r\ntwo\r\nthree\r\nfour");
        terminal.scroll_viewport(libghostty_vt::terminal::ScrollViewport::Top);
        let before = terminal.scrollbar().unwrap();
        assert_eq!(text(&terminal).text, "three\nfour");
        let after = terminal.scrollbar().unwrap();
        assert_eq!(before.offset, after.offset);
    }

    /// The alternate screen reads as such.
    #[test]
    fn the_alternate_screen_reads_its_own_rows() {
        let mut terminal = Terminal::new(8, 2).unwrap();
        terminal.vt_write(b"primary\x1b[?1049h\x1b[Hfull");
        let visible = text(&terminal);
        assert_eq!(visible.screen, Screen::Alternate);
        assert_eq!(visible.text, "full\n");
    }

    /// Past 1 MiB, the leading rows go whole and are counted.
    #[test]
    fn long_text_keeps_the_last_rows_that_fit() {
        let row = "a".repeat(1000);
        let lines: Vec<String> = (0..1100).map(|_| row.clone()).collect();
        let (omitted, text) = fit(&lines);
        assert!(text.len() <= MAX_VISIBLE_TEXT_BYTES);
        let kept = 1100 - omitted as usize;
        assert_eq!(text.len(), kept * 1001 - 1);
        assert!((kept + 1) * 1001 - 1 > MAX_VISIBLE_TEXT_BYTES);
        assert_eq!(fit(&["x".into(), String::new()]), (0, "x\n".into()));
    }

    #[test]
    fn control_characters_become_replacements() {
        assert_eq!(
            sanitized("a\u{1b}b\tc\u{85}"),
            "a\u{fffd}b\u{fffd}c\u{fffd}"
        );
    }
}
