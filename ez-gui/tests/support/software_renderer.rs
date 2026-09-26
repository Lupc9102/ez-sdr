//! CPU preview renderer for egui's actual tessellated output. Used only for
//! visual QA where no window server is available, never shipped in the app.
use std::collections::HashMap;

#[derive(Default)]
pub struct SoftwareRenderer {
    textures: HashMap<egui::TextureId, egui::ColorImage>,
}

impl SoftwareRenderer {
    pub fn render(
        &mut self,
        ctx: &egui::Context,
        output: egui::FullOutput,
        width: u32,
        height: u32,
    ) -> image::RgbaImage {
        for (id, delta) in &output.textures_delta.set {
            let egui::ImageData::Color(source) = &delta.image;
            if let Some([x, y]) = delta.pos {
                let target = self.textures.get_mut(id).expect("partial texture exists");
                for row in 0..source.size[1] {
                    let start = (y + row) * target.size[0] + x;
                    target.pixels[start..start + source.size[0]].copy_from_slice(
                        &source.pixels[row * source.size[0]..(row + 1) * source.size[0]],
                    );
                }
            } else {
                self.textures.insert(*id, (**source).clone());
            }
        }
        let mut pixels =
            image::RgbaImage::from_pixel(width, height, image::Rgba([20, 22, 28, 255]));
        let scale = output.pixels_per_point;
        for clipped in ctx.tessellate(output.shapes, scale) {
            let egui::epaint::Primitive::Mesh(mesh) = clipped.primitive else {
                panic!("CPU preview does not support GPU callbacks");
            };
            let texture = self
                .textures
                .get(&mesh.texture_id)
                .expect("mesh texture uploaded");
            let clip = clipped.clip_rect * scale;
            for indices in mesh.indices.chunks_exact(3) {
                let vertices = [
                    mesh.vertices[indices[0] as usize],
                    mesh.vertices[indices[1] as usize],
                    mesh.vertices[indices[2] as usize],
                ];
                let points = vertices.map(|v| v.pos * scale);
                let cross = |a: egui::Pos2, b: egui::Pos2, p: egui::Pos2| {
                    (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x)
                };
                let area = cross(points[0], points[1], points[2]);
                if area.abs() < 1e-6 {
                    continue;
                }
                let min_x = points
                    .iter()
                    .map(|p| p.x)
                    .fold(f32::INFINITY, f32::min)
                    .max(clip.min.x)
                    .max(0.0)
                    .floor() as u32;
                let max_x = points
                    .iter()
                    .map(|p| p.x)
                    .fold(f32::NEG_INFINITY, f32::max)
                    .min(clip.max.x)
                    .min(width as f32)
                    .ceil() as u32;
                let min_y = points
                    .iter()
                    .map(|p| p.y)
                    .fold(f32::INFINITY, f32::min)
                    .max(clip.min.y)
                    .max(0.0)
                    .floor() as u32;
                let max_y = points
                    .iter()
                    .map(|p| p.y)
                    .fold(f32::NEG_INFINITY, f32::max)
                    .min(clip.max.y)
                    .min(height as f32)
                    .ceil() as u32;
                for y in min_y..max_y {
                    for x in min_x..max_x {
                        let point = egui::pos2(x as f32 + 0.5, y as f32 + 0.5);
                        if !clip.contains(point) {
                            continue;
                        }
                        let weights = [
                            cross(points[1], points[2], point) / area,
                            cross(points[2], points[0], point) / area,
                            cross(points[0], points[1], point) / area,
                        ];
                        if weights.iter().any(|weight| *weight < -1e-6) {
                            continue;
                        }
                        // Assign shared edges to only one triangle (top-left rule).
                        let mut inside = true;
                        for i in 0..3 {
                            if weights[i].abs() < 1e-6 {
                                let (mut a, mut b) = (points[(i + 1) % 3], points[(i + 2) % 3]);
                                if area < 0.0 {
                                    std::mem::swap(&mut a, &mut b);
                                }
                                if !(b.y < a.y || (b.y == a.y && b.x > a.x)) {
                                    inside = false;
                                }
                            }
                        }
                        if !inside {
                            continue;
                        }
                        let uv = vertices
                            .iter()
                            .zip(weights)
                            .fold(egui::Vec2::ZERO, |sum, (vertex, weight)| {
                                sum + vertex.uv.to_vec2() * weight
                            });
                        let tx = uv.x * texture.size[0] as f32 - 0.5;
                        let ty = uv.y * texture.size[1] as f32 - 0.5;
                        let mut texel = [0.0; 4];
                        for (dx, wx) in [
                            (0, 1.0 - tx.fract().rem_euclid(1.0)),
                            (1, tx.fract().rem_euclid(1.0)),
                        ] {
                            for (dy, wy) in [
                                (0, 1.0 - ty.fract().rem_euclid(1.0)),
                                (1, ty.fract().rem_euclid(1.0)),
                            ] {
                                let sx = (tx.floor() as i32 + dx)
                                    .clamp(0, texture.size[0] as i32 - 1)
                                    as usize;
                                let sy = (ty.floor() as i32 + dy)
                                    .clamp(0, texture.size[1] as i32 - 1)
                                    as usize;
                                let rgba = texture.pixels[sy * texture.size[0] + sx].to_array();
                                for channel in 0..4 {
                                    texel[channel] += rgba[channel] as f32 * wx * wy / 255.0;
                                }
                            }
                        }
                        let mut color = [0.0; 4];
                        for (vertex, weight) in vertices.iter().zip(weights) {
                            for (channel, value) in vertex.color.to_array().iter().enumerate() {
                                color[channel] += *value as f32 * weight;
                            }
                        }
                        for channel in 0..4 {
                            color[channel] *= texel[channel];
                        }
                        let target = pixels.get_pixel_mut(x, y);
                        for channel in 0..3 {
                            target[channel] = (color[channel]
                                + target[channel] as f32 * (1.0 - color[3] / 255.0))
                                .clamp(0.0, 255.0)
                                as u8;
                        }
                    }
                }
            }
        }
        for id in output.textures_delta.free {
            self.textures.remove(&id);
        }
        pixels
    }
}
