//! ORB-C17's history read: the active screen's retained history followed by
//! its live area as lossy plain text, which libghostty's plain-text formatter
//! extracts in one pass over the whole screen. It reads the terminal only: no
//! terminal, viewport, selection or presentation state changes, and nothing
//! reaches the PTY.

use crate::frame::title_and_directory;
use libghostty_vt::{
    Terminal,
    fmt::{Format, Formatter},
    screen::Screen as GhosttyScreen,
};
use orbit_protocol::{
    management::{HistoryText, MAX_HISTORY_TEXT_BYTES, Screen},
    session::MAX_METADATA_BYTES,
};

/// The active screen's retained history and live area at `revision`, oldest
/// first: one line per logical line, so soft wraps are joined, each with its
/// trailing blanks trimmed and wide-cell spacers omitted, and trailing blank
/// lines trimmed. A control character other than the line feeds becomes
/// U+FFFD. The alternate screen retains no history, so it reads its live area
/// alone. When the text passes 8 MiB, leading whole lines are left out and
/// counted, or, when the newest line alone passes it, that line's leading
/// bytes are cut too. None when extraction fails, which the reply refuses,
/// never partial text.
pub(crate) fn read(terminal: &Terminal<'static, '_>, revision: u64) -> Option<HistoryText> {
    let rows = terminal.rows().ok()?;
    let columns = terminal.cols().ok()?;
    let screen = match terminal.active_screen().ok()? {
        GhosttyScreen::Alternate => Screen::Alternate,
        _ => Screen::Primary,
    };
    // The formatter's own trim treats a blank cell that carries a combining
    // mark as blank and drops the mark, so only real spaces are trimmed here.
    let mut formatter = Formatter::new(terminal).ok()?;
    formatter
        .set_format(Format::Plain)
        .ok()?
        .set_unwrap(true)
        .ok()?
        .set_trim(false)
        .ok()?;
    let bytes = formatter.format_alloc(None).ok()?;
    let text = trimmed(std::str::from_utf8(&bytes).ok()?);
    let (omitted_lines, cut_bytes, text) = fit(&text, MAX_HISTORY_TEXT_BYTES);
    let title = title_and_directory(terminal).0.to_owned();
    let directory = terminal.pwd().ok()?.to_vec();
    if title.len() + directory.len() > MAX_METADATA_BYTES {
        return None;
    }
    Some(HistoryText {
        revision,
        screen,
        rows,
        columns,
        title,
        directory,
        omitted_lines,
        cut_bytes,
        lossy: true,
        text: text.to_owned(),
    })
}

/// The text with each line's trailing spaces and the trailing blank lines
/// trimmed, and every control character but line feeds as U+FFFD.
fn trimmed(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.split('\n') {
        out.extend(
            line.trim_end_matches(' ')
                .chars()
                .map(|c| if c.is_control() { '\u{fffd}' } else { c }),
        );
        out.push('\n');
    }
    out.truncate(out.trim_end_matches('\n').len());
    out
}

/// The last whole lines of `text` that fit within `bound` bytes, how many
/// leading lines it left out, and how many leading bytes it cut from the
/// newest line kept (0 unless that line alone exceeds `bound`, when its tail
/// is kept from a character boundary). The text is never empty because of
/// the bound.
fn fit(text: &str, bound: usize) -> (u32, u32, &str) {
    let Some(earliest) = text.len().checked_sub(bound).filter(|&at| at > 0) else {
        return (0, 0, text);
    };
    let lines_before = |at: usize| {
        u32::try_from(
            text.as_bytes()[..at]
                .iter()
                .filter(|&&b| b == b'\n')
                .count(),
        )
        .unwrap_or(u32::MAX)
    };
    // The first line that starts at or after `earliest` begins after the line
    // feed that ends the line before it.
    let start = text.as_bytes()[earliest - 1..]
        .iter()
        .position(|&b| b == b'\n')
        .map(|at| earliest + at);
    if let Some(start) = start.filter(|&start| start < text.len()) {
        return (lines_before(start), 0, &text[start..]);
    }
    let cut = (earliest..text.len())
        .find(|&at| text.is_char_boundary(at))
        .unwrap_or(text.len());
    let line = text[..cut].rfind('\n').map_or(0, |at| at + 1);
    (
        lines_before(line),
        u32::try_from(cut - line).unwrap_or(u32::MAX),
        &text[cut..],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(terminal: &Terminal<'static, 'static>) -> HistoryText {
        read(terminal, 7).unwrap()
    }

    /// History then the live area, oldest first, as one line per logical
    /// line: soft wraps joined, trailing blanks trimmed, wide characters
    /// without spacers and combining marks kept.
    #[test]
    fn history_and_screen_read_as_logical_lines() {
        let mut terminal = Terminal::new(6, 3).unwrap();
        terminal.set_scrollback_max_bytes(Some(1 << 20)).unwrap();
        terminal.vt_write("one  \r\ntwo\r\nabcdefgh\r\n漢字e\u{301}\r\nlast".as_bytes());
        assert!(terminal.scrollback_rows().unwrap() > 0);
        let history = text(&terminal);
        assert_eq!(history.text, "one\ntwo\nabcdefgh\n漢字e\u{301}\nlast");
        assert_eq!((history.rows, history.columns), (3, 6));
        assert_eq!(history.screen, Screen::Primary);
        assert_eq!(history.revision, 7);
        assert_eq!((history.omitted_lines, history.cut_bytes), (0, 0));
        assert!(history.lossy);
    }

    /// A combining mark on a space is grapheme content, kept inside a line
    /// and at its end, while real trailing spaces still go.
    #[test]
    fn space_backed_combining_marks_are_kept() {
        let mut terminal = Terminal::new(10, 3).unwrap();
        terminal.vt_write("A \u{301}B\r\nC \u{301}\r\nD \u{301}  ".as_bytes());
        assert_eq!(text(&terminal).text, "A \u{301}B\nC \u{301}\nD \u{301}");
    }

    /// With no history, the text is the live area; trailing blank lines go.
    #[test]
    fn empty_history_reads_the_screen() {
        let mut terminal = Terminal::new(8, 4).unwrap();
        terminal.vt_write(b"a\r\n\r\nb\r\n");
        assert_eq!(terminal.scrollback_rows().unwrap(), 0);
        assert_eq!(text(&terminal).text, "a\n\nb");
        assert_eq!(text(&Terminal::new(8, 4).unwrap()).text, "");
    }

    /// Reading ignores the viewport and moves nothing: a terminal scrolled to
    /// the top of its history still reads all of it.
    #[test]
    fn reading_ignores_and_keeps_the_viewport() {
        let mut terminal = Terminal::new(8, 2).unwrap();
        terminal.set_scrollback_max_bytes(Some(4096)).unwrap();
        terminal.vt_write(b"one\r\ntwo\r\nthree\r\nfour");
        terminal.scroll_viewport(libghostty_vt::terminal::ScrollViewport::Top);
        let before = terminal.scrollbar().unwrap();
        assert_eq!(text(&terminal).text, "one\ntwo\nthree\nfour");
        let after = terminal.scrollbar().unwrap();
        assert_eq!(before.offset, after.offset);
    }

    /// The alternate screen retains no history: it reads its own live area
    /// alone, and says so.
    #[test]
    fn the_alternate_screen_reads_its_live_area_alone() {
        let mut terminal = Terminal::new(8, 2).unwrap();
        terminal.set_scrollback_max_bytes(Some(4096)).unwrap();
        terminal.vt_write(b"one\r\ntwo\r\nthree\x1b[?1049h\x1b[Hfull");
        let history = text(&terminal);
        assert_eq!(history.screen, Screen::Alternate);
        assert_eq!(history.text, "full");
    }

    /// Output the history budget evicted is not part of the terminal: the
    /// text starts at the earliest retained line.
    #[test]
    fn eviction_starts_the_text_at_the_earliest_retained_line() {
        // A page holds fewer rows the wider the screen (115 at 400 columns),
        // so 1000 lines evict several pages; every line costs a write.
        let mut terminal = Terminal::new(400, 4).unwrap();
        terminal.set_scrollback_max_bytes(Some(1)).unwrap();
        let lines: Vec<String> = (0..1000).map(|n| format!("line {n}")).collect();
        terminal.vt_write(lines.join("\r\n").as_bytes());
        let history = text(&terminal);
        let first = history.text.lines().next().unwrap();
        let first: usize = first.strip_prefix("line ").unwrap().parse().unwrap();
        assert!(first > 0, "the budget evicted nothing");
        assert_eq!(history.text, lines[first..].join("\n"));
        assert_eq!(history.omitted_lines, 0);
    }

    /// Past the bound, leading whole lines go and are counted.
    #[test]
    fn long_text_keeps_the_last_lines_that_fit() {
        let line = "a".repeat(1000);
        let text = vec![line.as_str(); 9000].join("\n");
        let (omitted, cut, kept) = fit(&text, MAX_HISTORY_TEXT_BYTES);
        assert_eq!(cut, 0);
        assert!(kept.len() <= MAX_HISTORY_TEXT_BYTES);
        let lines = 9000 - omitted as usize;
        assert_eq!(kept.len(), lines * 1001 - 1);
        assert!((lines + 1) * 1001 - 1 > MAX_HISTORY_TEXT_BYTES);
        assert_eq!(fit("ab\ncd", 5), (0, 0, "ab\ncd"));
        assert_eq!(fit("ab\ncd", 4), (1, 0, "cd"));
        assert_eq!(fit("ab\ncd", 3), (1, 0, "cd"));
        assert_eq!(fit("ab\n\ncd", 3), (1, 0, "\ncd"));
        assert_eq!(fit("ab\n\ncd", 2), (2, 0, "cd"));
    }

    /// When the newest line alone passes the bound, its tail is kept from a
    /// character boundary, every earlier line is counted and the cut bytes
    /// reported; the text is never empty.
    #[test]
    fn an_over_bound_newest_line_keeps_its_tail() {
        let line = "a".repeat(MAX_HISTORY_TEXT_BYTES + 1000);
        let text = format!("first\nsecond\n{line}");
        let (omitted, cut, kept) = fit(&text, MAX_HISTORY_TEXT_BYTES);
        assert_eq!((omitted, cut), (2, 1000));
        assert_eq!(kept.len(), MAX_HISTORY_TEXT_BYTES);

        // A three-byte character straddles the cut: the tail starts after it.
        let line = format!("€{}", "b".repeat(MAX_HISTORY_TEXT_BYTES - 1));
        let (omitted, cut, kept) = fit(&line, MAX_HISTORY_TEXT_BYTES);
        assert_eq!((omitted, cut), (0, 3));
        assert_eq!(kept, &line[3..]);
        assert!(!kept.is_empty() && kept.len() < MAX_HISTORY_TEXT_BYTES);

        assert_eq!(fit("abcdef", 4), (0, 2, "cdef"));
        assert_eq!(fit("x\nabcdef", 4), (1, 2, "cdef"));
        assert_eq!(fit("x\n€€", 4), (1, 3, "€"));
        // A trailing line feed: the newest line is empty, and the line
        // before it is cut instead, so the text is still not empty.
        assert_eq!(fit("x\nabcdef\n", 4), (1, 3, "def\n"));
    }
}
