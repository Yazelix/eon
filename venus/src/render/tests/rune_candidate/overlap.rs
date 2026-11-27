use super::*;

#[test]
fn rune_header_pilot_refuses_later_text_image_and_cursor() {
    let (device, queue) = wgpu::Device::noop(&DeviceDescriptor::default());
    let mut renderer =
        Renderer::hosted(device, queue, OFFSCREEN, (1280, 800), 1.0, packaged_fonts()).unwrap();
    let mut entry = Entry::new().unwrap();
    for _ in 1..STEP_COUNT {
        entry.advance();
    }
    entry.sync_images(&mut renderer);
    let mut fixture = entry.scene().clone();
    fixture.content.clear();
    fixture.graphics.placements[0].target.left = 0.0;
    fixture.graphics.placements[0].target.top = 0.0;
    for kind in ["text", "image", "cursor"] {
        renderer.enable_header_pilot(true);
        renderer.set_hovered_header(None);
        let metrics = renderer.metrics();
        // An enlarged corner lets the moved outline reach the first terminal row.
        renderer.metrics.padding *= 2.0;
        let mut workspace = fixtures::header_workspace(&mut renderer, 0);
        renderer.metrics = metrics;
        let row = workspace.popup_title_row(metrics).unwrap();
        let mut scene = fixture.clone();
        scene.graphics.placements.retain(|_| kind == "image");
        scene.cursor = scene.cursor.filter(|_| kind == "cursor");
        if kind == "text" {
            workspace.header.height = row.top - tokens::tooltip_gap(metrics) - workspace.header.top;
            let tab = &mut workspace.tabs[0];
            tab.rect.left = row.left + metrics.padding;
            renderer.set_hovered_header(Some((WorkspaceFocus::Tabs, tab.id.clone())));
        } else {
            workspace.terminal.left = row.left - metrics.padding;
            workspace.terminal.top = row.bottom() - metrics.padding;
        }
        let error = renderer
            .prepare_hosted_workspace(&scene, &workspace, entry.generation)
            .expect_err(kind);
        assert!(
            error.0.contains("retained Venus paint overlaps"),
            "{kind}: {error}"
        );
        assert_eq!(renderer.header_paint.overlap, kind != "cursor", "{kind}");
    }
}
