//! Presentation that runs between deltas: inline-object motion and whether
//! chart or glyph presentation is still live.

use super::*;

impl UiScene {
    /// Move, scale and fade the inline objects of presenting text nodes to
    /// what the CPU evaluator says their grapheme looks like `now`: the same
    /// arithmetic the text shader applies to the glyphs around them.
    pub(super) fn present_inline_objects(&mut self, now: std::time::Duration) {
        if self.glyph_objects.is_empty() {
            return;
        }
        let mut moved = false;
        // An object scales about its centre where it is drawn: on the page.
        let origins: NodeMap<[f32; 2]> = self
            .glyph_objects
            .keys()
            .map(|&node| (node, self.paint_origin_of(node)))
            .collect();
        for (node, (presentation, objects)) in &self.glyph_objects {
            let [ox, oy] = origins[node];
            for object in objects.iter() {
                let Some(held) = self.primitives.get_mut(&PrimitiveId {
                    node: *node,
                    slot: object.slot,
                }) else {
                    continue;
                };
                // Only a quad carries the transition masks, and its mask
                // stands in for the intro. Any other object (a host texture's
                // animated picture) keeps the intro the glyphs around it play
                // and fades in and out over the masks' progress instead.
                let masked = matches!(held.primitive.kind, ScenePrimitiveKind::Quad { .. });
                let mut reveal = presentation.reveal.clone();
                if masked
                    && presentation.transitions.entrance.is_some()
                    && let Some(r) = &mut reveal
                {
                    r.intro.fade = false;
                    r.intro.pop = false;
                    r.intro.rise_px = 0.;
                }
                let reveal_now = presentation.transitions.freeze_reveal.unwrap_or(now);
                let sample = presentation.sample(reveal.as_ref(), object.ordinal, reveal_now);
                let [cx, cy] = [ox + object.center[0], oy + object.center[1]];
                let scale = sample.scale;
                let local = AffineTransform::from_matrix([
                    scale,
                    0.0,
                    0.0,
                    scale,
                    cx + sample.offset[0] - scale * cx,
                    cy + sample.offset[1] - scale * cy,
                ]);
                let mut masks = Vec::new();
                let mut fade = 1.;
                for (enter, mask) in [
                    (true, &presentation.transitions.entrance),
                    (false, &presentation.transitions.exit),
                ] {
                    if let Some(mask) = mask {
                        let progress = if enter {
                            reveal.as_ref().map_or(1., |r| {
                                let ordinal =
                                    if mask.space == nana_ui_core::PresentationMaskSpace::Target {
                                        0
                                    } else {
                                        object.ordinal as usize
                                    };
                                r.at_s.get(ordinal).map_or(0., |at| {
                                    if mask.duration.is_zero() {
                                        return f32::from(
                                            reveal_now.as_secs_f64()
                                                >= r.start.max(mask.start).as_secs_f64()
                                                    + f64::from(*at),
                                        );
                                    }
                                    ((reveal_now.as_secs_f64()
                                        - r.start.max(mask.start).as_secs_f64()
                                        - f64::from(*at))
                                        / mask.duration.as_secs_f64().max(0.000001))
                                    .clamp(0., 1.) as f32
                                })
                            })
                        } else {
                            mask.progress(now)
                        };
                        if masked {
                            masks.push(nana_ui_core::PresentationMaskSample {
                                mask: mask.clone(),
                                progress,
                                target: nana_ui_core::LogicalRect::new(
                                    object.target.x - held.primitive.bounds.x,
                                    object.target.y - held.primitive.bounds.y,
                                    object.target.width,
                                    object.target.height,
                                ),
                            });
                        } else {
                            fade *= if enter { progress } else { 1. - progress };
                        }
                    }
                }
                if let ScenePrimitiveKind::Quad { surface, .. } = &mut held.primitive.kind
                    && surface.presentation_masks != masks
                {
                    surface.presentation_masks = masks;
                    moved = true;
                }
                let opacity = object.opacity * sample.alpha * fade;
                let transform = object.transform.then(local);
                if held.primitive.opacity != opacity || held.primitive.transform != transform {
                    held.primitive.opacity = opacity;
                    held.primitive.transform = transform;
                    moved = true;
                }
            }
        }
        if moved {
            // Painters key what they recorded on the instance.
            self.instance = next_scene_instance();
        }
    }

    /// Whether a text node's per-glyph presentation still changes what is
    /// drawn at the motion clock's time.
    /// Whether an inline GPU payload's marks or emphasis are still moving:
    /// the painter cannot reuse the painted frame, and the host keeps presenting.
    pub fn gpu_payload_live(&self) -> bool {
        let now = self.compositor_now();
        self.gpu_payload_live.values().any(|until| *until > now)
    }

    pub fn glyph_presentation_live(&self) -> bool {
        let now = self.compositor_now();
        self.glyph_live
            .values()
            .any(|until| until.is_none_or(|until| until > now))
    }
}
