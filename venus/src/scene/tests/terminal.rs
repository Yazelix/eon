use super::*;

#[test]
fn background_opacity_provenance_distinguishes_default_explicit_selection_and_inverse() {
    let mut style = CellStyle {
        foreground: StyleColor::None,
        background: StyleColor::None,
        underline_color: StyleColor::None,
        bold: false,
        italic: false,
        faint: false,
        blink: false,
        inverse: false,
        invisible: false,
        strikethrough: false,
        overline: false,
        selected: false,
        protected: false,
        underline: Underline::None,
    };

    assert!(is_default_background(style));
    style.background = StyleColor::Rgb(Rgb::BLACK);
    assert!(!is_default_background(style));
    style.background = StyleColor::None;
    style.selected = true;
    assert!(!is_default_background(style));
    style.selected = false;
    style.inverse = true;
    assert!(!is_default_background(style));
}

#[test]
fn glyph_runs_preserve_authoritative_cell_starts() {
    let scene = Scene {
        revision: 1,
        columns: 10,
        rows: 1,
        screen: Screen::Primary,
        title: String::new(),
        working_directory: String::new(),
        graphics: SceneGraphics::default(),
        background: Color::default(),
        foreground: Color::default(),
        cursor: None,
        content: vec![DrawRow {
            wrapped: false,
            wrap_continuation: false,
            cells: vec![
                draw_cell("A", CellWidth::Narrow, false),
                draw_cell("e\u{301}", CellWidth::Narrow, false),
                draw_cell("क्ष", CellWidth::Narrow, false),
                draw_cell("界", CellWidth::Wide, false),
                draw_cell("", CellWidth::SpacerTail, false),
                draw_cell("👩‍💻", CellWidth::Wide, false),
                draw_cell("", CellWidth::SpacerTail, false),
                draw_cell("א", CellWidth::Narrow, false),
                draw_cell("𐐀", CellWidth::Narrow, false),
                draw_cell("│", CellWidth::Narrow, false),
            ],
        }],
    };

    let actual = scene
        .glyph_runs()
        .into_iter()
        .map(|run| (run.column, run.columns, run.text))
        .collect::<Vec<_>>();
    assert_eq!(
        actual,
        [
            (0, 1, "A"),
            (1, 1, "e\u{301}"),
            (2, 1, "क्ष"),
            (3, 2, "界"),
            (5, 2, "👩‍💻"),
            (7, 1, "א"),
            (8, 1, "𐐀"),
            (9, 1, "│"),
        ]
        .map(|(column, columns, text)| (column, columns, text.to_owned()))
    );
}

#[test]
fn accessibility_projection_keeps_authoritative_selected_cells() {
    let mut scene = Scene {
        revision: 3,
        columns: 4,
        rows: 2,
        screen: Screen::Primary,
        title: String::new(),
        working_directory: String::new(),
        graphics: SceneGraphics::default(),
        background: Color::default(),
        foreground: Color::default(),
        cursor: Some(DrawCursor {
            visible: true,
            blinking: false,
            password_input: false,
            shape: CursorShape::Block,
            column: 0,
            row: 0,
            at_wide_tail: false,
            color: Color::default(),
            explicit_color: false,
        }),
        content: vec![
            DrawRow {
                wrapped: false,
                wrap_continuation: false,
                cells: vec![
                    draw_cell("a", CellWidth::Narrow, false),
                    draw_cell("e\u{301}", CellWidth::Narrow, true),
                    draw_cell("", CellWidth::Narrow, true),
                    draw_cell("", CellWidth::Narrow, false),
                ],
            },
            DrawRow {
                wrapped: false,
                wrap_continuation: false,
                cells: vec![
                    draw_cell("界", CellWidth::Wide, true),
                    draw_cell("", CellWidth::SpacerTail, true),
                    draw_cell("", CellWidth::Narrow, false),
                    draw_cell("", CellWidth::Narrow, false),
                ],
            },
        ],
    };

    let content = scene.accessible_content();
    assert_eq!(content.plain_text(), "ae\u{301} \n界");
    assert_eq!(
        content.selection,
        Some(AccessibleSelection {
            anchor: AccessiblePosition {
                row: 0,
                character_index: 1,
            },
            focus: AccessiblePosition {
                row: 1,
                character_index: 1,
            },
        })
    );
    assert_eq!(content.rows[0].character_lengths, [1, 3, 1, 1]);
    assert_eq!(content.rows[0].column_spans, [0..1, 1..2, 2..3, 3..3]);
    assert_eq!(content.rows[1].column_spans.len(), 1);
    assert_eq!(content.rows[1].column_spans[0], 0..2);

    for cell in scene.content.iter_mut().flat_map(|row| &mut row.cells) {
        cell.style.selected = false;
    }
    for (row, column, at_wide_tail, character_index) in [
        (0, 0, false, 0),
        (0, 1, false, 1),
        (0, 2, false, 2),
        (0, 3, false, 2),
        (1, 0, false, 0),
        (1, 1, true, 0),
        (1, 2, false, 1),
        (1, 3, false, 1),
    ] {
        let cursor = scene.cursor.as_mut().unwrap();
        cursor.row = row;
        cursor.column = column;
        cursor.at_wide_tail = at_wide_tail;
        let caret = AccessiblePosition {
            row: usize::from(row),
            character_index,
        };
        assert_eq!(
            scene.accessible_content().selection,
            Some(AccessibleSelection {
                anchor: caret,
                focus: caret,
            }),
            "cursor at {row},{column}"
        );
    }
    scene.content[1].cells.clear();
    let caret = AccessiblePosition {
        row: 1,
        character_index: 0,
    };
    assert_eq!(
        scene.accessible_content().selection,
        Some(AccessibleSelection {
            anchor: caret,
            focus: caret,
        })
    );
    scene.cursor = None;
    assert_eq!(scene.accessible_content().selection, None);
}

#[test]
fn accessibility_projection_keeps_cells_atomic_and_marks_oversized_text() {
    let scene = |cells: Vec<DrawCell>| Scene {
        revision: 4,
        columns: u16::try_from(cells.len()).unwrap(),
        rows: 1,
        screen: Screen::Primary,
        title: String::new(),
        working_directory: String::new(),
        graphics: SceneGraphics::default(),
        background: Color::default(),
        foreground: Color::default(),
        cursor: None,
        content: vec![DrawRow {
            wrapped: false,
            wrap_continuation: false,
            cells,
        }],
    };
    let selected_first_cell = Some(AccessibleSelection {
        anchor: AccessiblePosition {
            row: 0,
            character_index: 0,
        },
        focus: AccessiblePosition {
            row: 0,
            character_index: 1,
        },
    });

    let content = scene(vec![
        draw_cell("e\u{301}", CellWidth::Narrow, true),
        draw_cell("👩‍💻", CellWidth::Wide, false),
        draw_cell("", CellWidth::SpacerTail, false),
    ])
    .accessible_content();
    assert_eq!(content.rows[0].character_lengths, [3, 11]);
    assert_eq!(content.selection, selected_first_cell);

    let representable = format!("e{}", "\u{301}".repeat(127));
    let content =
        scene(vec![draw_cell(&representable, CellWidth::Narrow, true)]).accessible_content();
    assert_eq!(content.plain_text(), representable);
    assert_eq!(content.rows[0].character_lengths, [255]);

    let oversized = format!("e{}", "\u{301}".repeat(128));
    let scene = scene(vec![draw_cell(&oversized, CellWidth::Narrow, true)]);
    assert_eq!(scene.glyph_runs()[0].text, oversized);
    let content = scene.accessible_content();
    assert_eq!(content.plain_text(), "\u{fffd}");
    assert_eq!(content.rows[0].character_lengths, [3]);
    assert_eq!(content.selection, selected_first_cell);
}
