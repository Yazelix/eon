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
        .hyperlinks(true)
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
        ("ftp://e.org file:///etc", None),
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
        let link = scene.hyperlink_at(0, column + 1, true).unwrap();
        assert_eq!(link.uri, uri);
        assert!(
            scene
                .hyperlink_at(0, column + uri.len() as u16, true)
                .is_none()
        );
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
    assert_eq!(wrapped.hyperlink_at(1, 9, true).unwrap().uri, uri);
    assert!(wrapped.hyperlink_at(1, 10, true).is_none());

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
    assert!(scene.hyperlink_at(0, 3, true).is_none());

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

#[test]
fn bare_loopback_hosts_link_over_http_only_when_enabled() {
    for (text, expected) in [
        ("on localhost:3000 now", Some("localhost:3000")),
        ("Open localhost:3000.", Some("localhost:3000")),
        ("LocalHost:8080/app/x.", Some("LocalHost:8080/app/x")),
        ("(127.0.0.1:65535)", Some("127.0.0.1:65535")),
        ("127.0.0.1:1/", Some("127.0.0.1:1/")),
        ("'[::1]:5173',", Some("[::1]:5173")),
        ("[::1]:443/a/(b)]", Some("[::1]:443/a/(b)")),
        (
            "localhost:0 localhost:65536 localhost:03000 localhost:12345678901",
            None,
        ),
        ("a:3000 1:2 foo.localhost:3000 git@localhost:3000", None),
        ("10.127.0.0.1:80 localhost:3000abc", None),
        ("localhost:3000.5 localhost:3000.example", None),
        (
            "ftp://localhost:21 x-localhost:1 x_localhost:1 ::localhost:1 [fe80::1]:80 ::ffff:127.0.0.1:80",
            None,
        ),
    ] {
        let scene = text_scene(96, &[(text, false)]);
        assert_eq!(scene.hyperlinks(false).count(), 0, "{text:?}");
        let Some(matched) = expected else {
            assert_eq!(link_spans(&scene), [], "{text:?}");
            continue;
        };
        let (column, target) = (
            text.find(matched).unwrap() as u16,
            format!("http://{matched}"),
        );
        assert_eq!(
            link_spans(&scene),
            [(0, column, matched.len() as u16, target.clone())],
            "{text:?}"
        );
        assert_eq!(scene.hyperlink_at(0, column, true).unwrap().uri, target);
        assert!(scene.hyperlink_at(0, column, false).is_none());
    }

    // A loopback host inside a scheme URL is part of that URL, never a second link.
    for text in ["http://localhost:3000", "https://e.org/?to=localhost:3000"] {
        let scene = text_scene(40, &[(text, false)]);
        assert_eq!(link_spans(&scene), [(0, 0, text.len() as u16, text.into())]);
    }

    // Turning loopback detection off leaves scheme URLs and explicit links alone.
    let mut scene = text_scene(40, &[("a https://e.org/x b localhost:3000", false)]);
    scene.content[0].cells[0].hyperlink = "https://explicit.invalid/".into();
    let off: Vec<_> = scene
        .hyperlinks(false)
        .map(|link| (link.column, link.uri.into_owned()))
        .collect();
    assert_eq!(
        off,
        [
            (0, "https://explicit.invalid/".to_string()),
            (2, "https://e.org/x".to_string()),
        ]
    );

    let wrapped = text_scene(15, &[("go localhost:30", true), ("00/x ok", false)]);
    let target = "http://localhost:3000/x";
    assert_eq!(
        link_spans(&wrapped),
        [(0, 3, 12, target.into()), (1, 0, 4, target.into())]
    );
}
