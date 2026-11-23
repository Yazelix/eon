use super::*;

fn differing_cells(a: &Scene, b: &Scene) -> usize {
    a.content
        .iter()
        .zip(&b.content)
        .map(|(a, b)| a.cells.iter().zip(&b.cells).filter(|(a, b)| a != b).count())
        .sum()
}

#[test]
fn fixtures_reach_the_scene_through_the_canonical_codec() {
    let mut entry = Entry::new().unwrap();
    let mut attachments = Vec::new();
    for cycle in 0..2_u64 {
        if cycle > 0 {
            entry.advance();
        }
        assert_eq!(entry.position(), (cycle, 0, "text"));
        let text = entry.scene().clone();
        assert_eq!((text.content.len(), text.content[0].cells.len()), (24, 80));
        assert!(entry.pending.is_empty());
        attachments.push(entry.model.image_attachment().unwrap());

        entry.advance();
        assert_eq!(entry.position(), (cycle, 1, "cell"));
        assert_eq!(differing_cells(&text, entry.scene()), 1, "one cell changes");

        entry.advance();
        assert_eq!(entry.position(), (cycle, 2, "full"));
        assert_eq!(
            differing_cells(&text, entry.scene()),
            80 * 24,
            "every cell changes"
        );

        entry.advance();
        assert_eq!(entry.position(), (cycle, 3, "resize"));
        let resized = entry.scene();
        assert_eq!(
            (resized.content.len(), resized.content[0].cells.len()),
            (30, 100)
        );
        assert_eq!(
            resized.content[29].cells[99], text.content[23].cells[79],
            "the corner marker moves to the new corner, so the resize is drawn"
        );
        assert_ne!(resized.content[23].cells[79], text.content[23].cells[79]);
        assert!(resized.graphics.placements.is_empty());

        entry.advance();
        assert_eq!(entry.position(), (cycle, 4, "image"));
        assert!(
            matches!(
                entry.pending.as_slice(),
                [ModelEffect::Resource(resource)] if resource.resource == 1
                    && resource.attachment == attachments[cycle as usize]
            ),
            "exactly one resource arrives before its frame",
        );
        let graphics = &entry.scene().graphics;
        assert_eq!(graphics.attachment, attachments[cycle as usize]);
        assert_eq!(graphics.placements.len(), 1);
        entry.pending.clear();
    }
    assert_ne!(
        attachments[0], attachments[1],
        "a new cycle is a fresh attachment, so revision 1 is accepted again",
    );
    assert_eq!(entry.generation, 10);
}

#[test]
fn admission_creates_once_draws_on_match_and_refuses_a_new_device_or_format() {
    use wgpu::TextureFormat::{Bgra8UnormSrgb, Rgba16Float};
    let first = (1, Bgra8UnormSrgb);
    assert_eq!(admit(None, &first), Admission::Create);
    assert_eq!(admit(Some(&first), &first), Admission::Draw);
    assert_eq!(admit(Some(&first), &(2, Bgra8UnormSrgb)), Admission::Refuse);
    assert_eq!(admit(Some(&first), &(1, Rgba16Float)), Admission::Refuse);

    // Geometry compares on the pixel grid Rune snaps slots to, and on the exact scale.
    let prepared = SlotGeometry::new([37.0, 23.0, 400.0, 260.0], 1.0);
    assert_eq!(SlotGeometry::new([37.2, 22.8, 400.4, 259.6], 1.0), prepared);
    assert_ne!(SlotGeometry::new([37.0, 23.0, 401.0, 260.0], 1.0), prepared);
    assert_ne!(SlotGeometry::new([37.0, 23.0, 400.0, 260.0], 2.0), prepared);
    assert_eq!(prepared.size(), (400, 260));
}

#[test]
fn full_window_grid_lands_with_tiled_resize() {
    use rune::{Env, Runtime, render::KIND_EFFECT};

    let points =
        |([width, height], scale): ([u32; 2], f32)| (width as f32 / scale, height as f32 / scale);
    let sample = |rt: &Runtime<()>, target: [u32; 2]| {
        let (rect, _) = rt.presented(GRID_KEY).unwrap();
        let scale = rt.env.scale;
        let prepared = SlotGeometry::new(rect.map(|value| value * scale), scale);
        let mut slots = rt.instances.iter().filter(|i| i.params[1] == KIND_EFFECT);
        let slot = slots.next().unwrap();
        assert!(slots.next().is_none());
        assert_eq!(
            slot.border[..2],
            [(GRID_MARK & 0xffff) as f32, (GRID_MARK >> 16) as f32]
        );
        let [left, top, right, bottom] = slot.clip;
        assert!(
            left <= 0.0 && top <= 0.0 && right >= target[0] as f32 && bottom >= target[1] as f32
        );
        let drawn = SlotGeometry::new(slot.rect, slot.params[3]);
        (
            [prepared, drawn],
            [Fit::Exact, Fit::Near].map(|fit| prepared.viewport(drawn, fit, [0.0; 2], target)),
        )
    };
    let expected = |([width, height], scale): ([u32; 2], f32)| {
        let rect = [0.0, 0.0, width as f32, height as f32];
        ([SlotGeometry::new(rect, scale); 2], [Some(rect); 2])
    };
    for (initial, resized, dirty_first) in [
        (([1280, 800], 1.0), ([640, 800], 1.0), false),
        (([640, 800], 1.0), ([1280, 800], 1.0), true),
        (([1281, 801], 1.25), ([641, 801], 1.25), false),
        (([1281, 801], 2.0), ([641, 801], 2.0), false),
        (([641, 801], 1.0), ([641, 801], 1.25), true),
    ] {
        let mut rt = Runtime::new(
            (),
            |_, _| rune_host::grid_view(),
            points(initial),
            Env::new(true, initial.1),
        );
        rt.window_focus(true);
        rt.frame(0.0);
        assert_eq!(sample(&rt, initial.0), expected(initial));

        if dirty_first {
            rt.update(|_, tx| tx.animated());
        }
        rt.set_viewport(points(resized), resized.1);
        if !dirty_first {
            rt.invalidate();
        }
        // The focus-loss action commits the dirty view before dispatch.
        rt.window_focus(false);
        rt.frame(0.0);
        let first = sample(&rt, resized.0);
        // This spring-tail sample reproduced the observed 641 pixels on the 640-pixel target.
        rt.frame(0.42);
        assert_eq!(
            [first, sample(&rt, resized.0)],
            [expected(resized); 2],
            "dirty resize from {initial:?} to {resized:?}",
        );
    }
}
