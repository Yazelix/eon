use super::*;

#[test]
fn hyperlinks_follow_visible_heads_and_wide_cells() {
    let mut row = DrawRow {
        wrapped: false,
        wrap_continuation: false,
        cells: vec![
            draw_cell("A", CellWidth::Narrow, false),
            draw_cell("界", CellWidth::Wide, false),
            draw_cell("", CellWidth::SpacerTail, false),
            draw_cell("hidden", CellWidth::Narrow, false),
            draw_cell("B", CellWidth::Narrow, false),
            draw_cell("", CellWidth::SpacerHead, false),
        ],
    };
    for cell in &mut row.cells {
        cell.hyperlink = "https://example.com/exact?x=%26&y=2".into();
    }
    row.cells[2].hyperlink = "https://wrong-tail.invalid".into();
    row.cells[3].style.invisible = true;
    let links = row.hyperlinks(2).collect::<Vec<_>>();
    assert_eq!(links.len(), 2);
    assert_eq!((links[0].row, links[0].column, links[0].columns), (2, 0, 3));
    assert_eq!(links[0].uri, row.cells[0].hyperlink);
    assert!(links[0].contains(2, 2));
    assert!(!links[0].contains(2, 3));
    assert_eq!((links[1].column, links[1].columns), (4, 1));
}

/// Rows of narrow cells padded to `columns`; `true` marks a row that soft-wraps.
fn text_scene(columns: usize, rows: &[(&str, bool)]) -> Scene {
    let content = rows
        .iter()
        .enumerate()
        .map(|(index, (text, wrapped))| DrawRow {
            wrapped: *wrapped,
            wrap_continuation: index > 0 && rows[index - 1].1,
            cells: format!("{text:columns$}")
                .chars()
                .map(|c| draw_cell(&c.to_string(), CellWidth::Narrow, false))
                .collect(),
        })
        .collect::<Vec<_>>();
    Scene {
        revision: 1,
        columns: columns as u16,
        rows: content.len() as u16,
        screen: Screen::Primary,
        title: String::new(),
        working_directory: String::new(),
        graphics: SceneGraphics::default(),
        background: Color::default(),
        foreground: Color::default(),
        cursor: None,
        content,
    }
}

fn link_spans(scene: &Scene) -> Vec<(u16, u16, u16, String)> {
    scene
        .hyperlinks()
        .map(|link| (link.row, link.column, link.columns, link.uri.into_owned()))
        .collect()
}

#[test]
fn plain_text_links_trim_punctuation_and_unopened_brackets() {
    for (text, expected) in [
        ("see https://example.com/a.", Some("https://example.com/a")),
        ("(https://example.com/x)", Some("https://example.com/x")),
        (
            "https://e.org/Rust_(game),",
            Some("https://e.org/Rust_(game)"),
        ),
        ("[HTTP://localhost:5173/]", Some("HTTP://localhost:5173/")),
        ("'http://[::1]:3000/'", Some("http://[::1]:3000/")),
        ("xhttps://example.com", None),
        ("ftp://e.org file:///etc localhost:3000", None),
        ("https://user@example.com/", None),
        ("https://", None),
    ] {
        let scene = text_scene(40, &[(text, false)]);
        let links = link_spans(&scene);
        let Some(uri) = expected else {
            assert!(links.is_empty(), "{text:?}: {links:?}");
            continue;
        };
        let column = text.find(uri).unwrap() as u16;
        assert_eq!(
            links,
            [(0, column, uri.len() as u16, uri.into())],
            "{text:?}"
        );
        let link = scene.hyperlink_at(0, column + 1).unwrap();
        assert_eq!(link.uri, uri);
        assert!(scene.hyperlink_at(0, column + uri.len() as u16).is_none());
    }
}

#[test]
fn plain_text_links_join_soft_wrapped_rows_only() {
    let uri = "https://example.com/p";
    let wrapped = text_scene(14, &[("go https://exa", true), ("mple.com/p ok", false)]);
    assert_eq!(
        link_spans(&wrapped),
        [(0, 3, 11, uri.into()), (1, 0, 10, uri.into())]
    );
    assert_eq!(wrapped.hyperlink_at(1, 9).unwrap().uri, uri);
    assert!(wrapped.hyperlink_at(1, 10).is_none());

    let hard = text_scene(14, &[("go https://exa", false), ("mple.com/p ok", false)]);
    assert_eq!(link_spans(&hard), [(0, 3, 11, "https://exa".into())]);
}

#[test]
fn explicit_links_and_non_uri_cells_bound_plain_text_links() {
    let mut scene = text_scene(40, &[("a https://e.org/x b https://f.org/y", false)]);
    scene.content[0].cells[16].hyperlink = "https://explicit.invalid/".into();
    assert_eq!(
        link_spans(&scene),
        [
            (0, 16, 1, "https://explicit.invalid/".into()),
            (0, 20, 15, "https://f.org/y".into()),
        ]
    );
    assert!(scene.hyperlink_at(0, 3).is_none());

    let row = &mut scene.content[0];
    row.cells[15] = draw_cell("界", CellWidth::Wide, false);
    row.cells[16] = draw_cell("", CellWidth::SpacerTail, false);
    row.cells[33].style.invisible = true;
    assert_eq!(
        link_spans(&scene),
        [
            (0, 2, 13, "https://e.org".into()),
            (0, 20, 13, "https://f.org".into()),
        ]
    );
}
