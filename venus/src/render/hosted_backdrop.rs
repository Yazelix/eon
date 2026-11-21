//! The Rune candidate's hosted slot background (eon-yp6vg.7.7). A host's render pass has no
//! clear of its own at the slot, and default-background cells draw no rectangle, so the
//! candidate draws one rectangle of the scene background first. The windowed path clears its
//! own pass with that background instead, and without `candidate-rune` none of this exists.

use super::{
    BufferDescriptor, BufferUsages, RectangleBatch, Renderer, Scene, VERTEX_SIZE, vertex_count,
};

impl Renderer {
    /// Writes the backdrop for `scene`. The first call makes its buffer.
    pub(super) fn prepare_hosted_backdrop(&mut self, scene: &Scene) {
        let mut backdrop = RectangleBatch::new(self.config.width, self.config.height);
        backdrop.push(
            0.0,
            0.0,
            self.config.width as f32,
            self.config.height as f32,
            scene.background,
            self.background_opacity,
        );
        let device = &self.device;
        let (buffer, count) = self.hosted_backdrop.get_or_insert_with(|| {
            // One rectangle is two triangles.
            let buffer = device.create_buffer(&BufferDescriptor {
                label: Some("venus hosted backdrop vertices"),
                size: VERTEX_SIZE * 6,
                usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            (buffer, 0)
        });
        *count = vertex_count(&backdrop.bytes);
        self.queue.write_buffer(buffer, 0, &backdrop.bytes);
    }

    /// Draws the prepared backdrop under the terminal.
    pub(super) fn draw_hosted_backdrop(&self, pass: &mut wgpu::RenderPass<'_>) {
        if let Some((backdrop, count)) = &self.hosted_backdrop
            && *count > 0
        {
            pass.set_pipeline(&self.rectangle_pipeline);
            pass.set_vertex_buffer(0, backdrop.slice(..));
            pass.draw(0..*count, 0..1);
        }
    }
}
