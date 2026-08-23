//! The application's own icon, drawn from the same vectors as its interface.
//!
//! No image is shipped beside the binary and none is downloaded: the mark is
//! the [`crate::icons`] module's own processor glyph on a rounded badge,
//! tessellated by egui and rasterised here into the pixels a window manager
//! asks for. The window, the taskbar and the desktop entry therefore cannot
//! come to show three different marks, and a change to the glyph reaches all
//! three at once.
//!
//! The desktop entry needs a file rather than pixels in memory, which is what
//! [`png`] is for and why `desdec --write-icon` exists: the installer asks the
//! program it just installed for its own icon instead of carrying a copy that
//! could fall out of step with it.

use eframe::egui;

use crate::icons::{self, Icon};

/// The glyph the badge carries.
///
/// The processor, which is what this program is about. The set is drawn to be
/// read at sixteen pixels, and of all of them it is the one that says
/// `machine code` at that size without a word.
const GLYPH: Icon = Icon::Machine;

/// The badge, and the mark on it.
///
/// A filled blue tile with a white mark rather than the interface's own dark
/// panel: an icon is seen at sixteen pixels against a dock, a menu and a task
/// switcher, none of which the application chooses the colour of, and a dark
/// mark on a dark background disappears into two of the three.
const BADGE: egui::Color32 = egui::Color32::from_rgb(62, 110, 208);
const MARK: egui::Color32 = egui::Color32::WHITE;

/// Room left around the badge, as a fraction of the tile.
///
/// An icon flush with its own edge stands taller than every other icon beside
/// it, which is the one thing a dock notices.
const TILE_MARGIN: f32 = 0.05;

/// How much of the badge the mark takes.
///
/// Larger than the fraction a button gives a glyph. A button's icon sits in a
/// row of them and needs air between it and its neighbours; an icon is alone
/// on its tile, and what it needs is to survive being scaled to sixteen
/// pixels — where the mark drawn at a button's proportions closes up into a
/// blob and the tile is all a reader sees.
const GLYPH_ROOM: f32 = 0.68;

/// And drawn a little heavier than the same glyph in a button.
///
/// The chip's pins are the first thing a downscale loses, and losing them
/// turns a processor into a ring.
const STROKE_WEIGHT: f32 = 1.0;

/// Corner radius of the badge, as a fraction of its side.
const CORNER: f32 = 0.22;

/// Drawn this many times over on a side, and averaged down.
///
/// egui feathers its edges, which is enough antialiasing for a widget on the
/// screen it was laid out for and not enough for a mark someone else will
/// scale to sixteen pixels. Four samples a side is sixteen to a pixel.
const SUPERSAMPLE: u32 = 4;

/// The side the icon is rendered at.
///
/// One of the sizes `hicolor` names, and the largest a menu or a dock asks
/// for in practice. Everything below it is a scale down, which is the
/// direction that survives.
pub const SIDE: u32 = 128;

/// A square image, row by row, four straight — not premultiplied — bytes to a
/// pixel, which is what both `eframe` and PNG are handed.
pub struct Image {
    pub side: u32,
    pub rgba: Vec<u8>,
}

/// The icon `eframe` puts on the window and hands to the taskbar.
#[must_use]
pub fn window_icon() -> egui::IconData {
    let image = render(SIDE);
    egui::IconData {
        rgba: image.rgba,
        width: image.side,
        height: image.side,
    }
}

/// The mark, as pixels.
#[must_use]
pub fn render(side: u32) -> Image {
    let canvas = side * SUPERSAMPLE;
    let extent = as_scalar(canvas);
    let ctx = egui::Context::default();
    // The samples are the antialiasing here, and a feathered edge on top of
    // them is a second one: it lays a half-transparent skirt around every
    // shape, which a scaled-down icon shows as a haze.
    ctx.options_mut(|options| options.tessellation_options.feathering = false);
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::Vec2::splat(extent),
        )),
        ..egui::RawInput::default()
    };
    let output = ctx.run(input, |ctx| {
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                let tile = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::splat(extent));
                let badge = tile.shrink(extent * TILE_MARGIN);
                let painter = ui.painter();
                painter.rect_filled(badge, corner_radius(badge.width()), BADGE);
                // The rectangle that makes the glyph `GLYPH_ROOM` of the
                // badge, since what `icons` scales is the rectangle it is
                // handed and not the glyph inside it.
                let room = egui::Rect::from_center_size(
                    badge.center(),
                    egui::Vec2::splat(badge.width() * GLYPH_ROOM / icons::GLYPH_SCALE),
                );
                icons::draw_with_stroke(
                    painter,
                    room,
                    GLYPH,
                    MARK,
                    icons::stroke_for(room.width()) * STROKE_WEIGHT,
                );
            });
    });
    let primitives = ctx.tessellate(output.shapes, 1.0);
    Image {
        side,
        rgba: reduce(&rasterise(&primitives, canvas), canvas, side),
    }
}

/// The badge's corner, as egui counts one: whole points, and never more than
/// a `u8` holds.
fn corner_radius(side: f32) -> egui::CornerRadius {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to the range a corner radius is written in"
    )]
    let radius = (side * CORNER).clamp(0.0, f32::from(u8::MAX)) as u8;
    egui::CornerRadius::same(radius)
}

/// Every triangle egui produced, painted into a square of premultiplied
/// floating-point pixels.
fn rasterise(primitives: &[egui::ClippedPrimitive], canvas: u32) -> Vec<[f32; 4]> {
    let side = canvas as usize;
    let mut pixels = vec![[0.0_f32; 4]; side * side];
    for primitive in primitives {
        let egui::epaint::Primitive::Mesh(mesh) = &primitive.primitive else {
            continue;
        };
        for triangle in mesh.indices.chunks_exact(3) {
            let corners = [
                mesh.vertices[triangle[0] as usize],
                mesh.vertices[triangle[1] as usize],
                mesh.vertices[triangle[2] as usize],
            ];
            fill(&corners, primitive.clip_rect, side, &mut pixels);
        }
    }
    pixels
}

/// One triangle, with its colour carried across it from its corners.
fn fill(
    corners: &[egui::epaint::Vertex; 3],
    clip: egui::Rect,
    side: usize,
    pixels: &mut [[f32; 4]],
) {
    let area = edge(corners[0].pos, corners[1].pos, corners[2].pos);
    if area.abs() < f32::EPSILON {
        return;
    }
    let bounds =
        egui::Rect::from_points(&[corners[0].pos, corners[1].pos, corners[2].pos]).intersect(clip);
    let limit = as_scalar_usize(side);
    let (low_x, high_x) = span(bounds.left(), bounds.right(), limit);
    let (low_y, high_y) = span(bounds.top(), bounds.bottom(), limit);
    for y in low_y..high_y {
        for x in low_x..high_x {
            let point = egui::pos2(as_scalar_usize(x) + 0.5, as_scalar_usize(y) + 0.5);
            let first = edge(corners[1].pos, corners[2].pos, point) / area;
            let second = edge(corners[2].pos, corners[0].pos, point) / area;
            let third = 1.0 - first - second;
            if first < 0.0 || second < 0.0 || third < 0.0 {
                continue;
            }
            let mut source = [0.0_f32; 4];
            for (weight, corner) in [
                (first, corners[0]),
                (second, corners[1]),
                (third, corners[2]),
            ] {
                for (into, value) in source.iter_mut().zip(channels(corner.color)) {
                    *into += weight * value;
                }
            }
            // Premultiplied, which is how egui keeps a colour, so one over the
            // other is an addition and a fraction of what was already there.
            let behind = 1.0 - source[3];
            let destination = &mut pixels[y * side + x];
            for (into, value) in destination.iter_mut().zip(source) {
                *into = value + *into * behind;
            }
        }
    }
}

/// The rows or columns a shape can touch, clamped to the canvas.
fn span(from: f32, to: f32, limit: f32) -> (usize, usize) {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to the canvas, which is a small positive number"
    )]
    let index = |value: f32| value.clamp(0.0, limit) as usize;
    (index(from.floor()), index(to.ceil()))
}

/// Twice the signed area of the triangle the three points make, which is
/// positive on one side of `a`–`b` and negative on the other.
fn edge(a: egui::Pos2, b: egui::Pos2, c: egui::Pos2) -> f32 {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
}

fn channels(colour: egui::Color32) -> [f32; 4] {
    [
        f32::from(colour.r()) / 255.0,
        f32::from(colour.g()) / 255.0,
        f32::from(colour.b()) / 255.0,
        f32::from(colour.a()) / 255.0,
    ]
}

/// The oversized canvas averaged down to the icon, and its colours put back
/// the way an image file carries them.
fn reduce(pixels: &[[f32; 4]], canvas: u32, side: u32) -> Vec<u8> {
    let step = (canvas / side) as usize;
    let samples = as_scalar_usize(step * step);
    let canvas = canvas as usize;
    let side = side as usize;
    let mut out = Vec::with_capacity(side * side * 4);
    for y in 0..side {
        for x in 0..side {
            let mut total = [0.0_f32; 4];
            for down in 0..step {
                for across in 0..step {
                    let pixel = pixels[(y * step + down) * canvas + x * step + across];
                    for (into, value) in total.iter_mut().zip(pixel) {
                        *into += value;
                    }
                }
            }
            for channel in &mut total {
                *channel /= samples;
            }
            out.extend_from_slice(&straight(total));
        }
    }
    out
}

/// A premultiplied pixel as the four bytes an image file holds.
fn straight(pixel: [f32; 4]) -> [u8; 4] {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to zero..one and multiplied by the range of a byte"
    )]
    let byte = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    let alpha = pixel[3];
    if alpha <= f32::EPSILON {
        return [0, 0, 0, 0];
    }
    [
        byte(pixel[0] / alpha),
        byte(pixel[1] / alpha),
        byte(pixel[2] / alpha),
        byte(alpha),
    ]
}

#[expect(
    clippy::cast_precision_loss,
    reason = "icon sides are three-digit numbers"
)]
fn as_scalar(value: u32) -> f32 {
    value as f32
}

#[expect(
    clippy::cast_precision_loss,
    reason = "canvas coordinates are three- or four-digit numbers"
)]
fn as_scalar_usize(value: usize) -> f32 {
    value as f32
}

/// The image as a PNG.
///
/// Written here rather than with a library. All this needs of an encoder is a
/// file a desktop will read, and PNG's own *stored* deflate block — the one
/// that compresses nothing — is a length, its complement and the bytes. Sixty
/// kilobytes for an icon is no burden; a dependency for thirty lines would be.
#[must_use]
pub fn png(image: &Image) -> Vec<u8> {
    let width = image.side as usize * 4;
    let mut raw = Vec::with_capacity(image.rgba.len() + image.side as usize);
    for row in image.rgba.chunks_exact(width) {
        // Every row unfiltered: filtering is what makes a PNG small, and this
        // one is not trying to be.
        raw.push(0);
        raw.extend_from_slice(row);
    }

    let mut out = Vec::new();
    out.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
    let mut header = Vec::with_capacity(13);
    header.extend_from_slice(&image.side.to_be_bytes());
    header.extend_from_slice(&image.side.to_be_bytes());
    // Eight bits a channel, truecolour with alpha, deflate, no filter, no
    // interlacing.
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, *b"IHDR", &header);
    chunk(&mut out, *b"IDAT", &deflate_stored(&raw));
    chunk(&mut out, *b"IEND", &[]);
    out
}

/// One PNG chunk: its length, its name, its body, and the check over the two
/// of them that a reader compares.
fn chunk(out: &mut Vec<u8>, name: [u8; 4], body: &[u8]) {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "an icon's every chunk is far inside four gigabytes"
    )]
    let length = body.len() as u32;
    out.extend_from_slice(&length.to_be_bytes());
    out.extend_from_slice(&name);
    out.extend_from_slice(body);
    out.extend_from_slice(&crc32(name, body).to_be_bytes());
}

/// A zlib stream carrying the bytes as they are.
fn deflate_stored(raw: &[u8]) -> Vec<u8> {
    // Deflate, a thirty-two kilobyte window, no preset dictionary.
    let mut out = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = if raw.is_empty() {
        vec![&[]]
    } else {
        raw.chunks(0xFFFF).collect()
    };
    let last = blocks.len() - 1;
    for (index, block) in blocks.iter().enumerate() {
        out.push(u8::from(index == last));
        #[expect(
            clippy::cast_possible_truncation,
            reason = "the blocks are cut to fit in the length that follows"
        )]
        let length = block.len() as u16;
        out.extend_from_slice(&length.to_le_bytes());
        out.extend_from_slice(&(!length).to_le_bytes());
        out.extend_from_slice(block);
    }
    out.extend_from_slice(&adler32(raw).to_be_bytes());
    out
}

fn crc32(name: [u8; 4], body: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for byte in name.iter().chain(body) {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut low, mut high) = (1_u32, 0_u32);
    for byte in data {
        low = (low + u32::from(*byte)) % 65521;
        high = (high + low) % 65521;
    }
    (high << 16) | low
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The icon is a square of the size asked for, and every pixel of it is
    /// accounted for.
    #[test]
    fn the_icon_is_a_square_of_straight_pixels() {
        let image = render(32);
        assert_eq!(image.side, 32);
        assert_eq!(image.rgba.len(), 32 * 32 * 4);
    }

    /// A mark, on a badge, with room around it.
    ///
    /// Three assertions about pixels rather than about shapes, because the
    /// shapes were always right: what this guards is the rasteriser between
    /// them and the file — a triangle wound the other way, a weight that went
    /// negative, an alpha divided out twice, and the icon comes out empty
    /// while every drawing test still passes.
    #[test]
    fn the_badge_is_painted_and_the_mark_is_on_it() {
        let image = render(64);
        let at = |x: usize, y: usize| {
            let start = (y * 64 + x) * 4;
            let pixel = &image.rgba[start..start + 4];
            [pixel[0], pixel[1], pixel[2], pixel[3]]
        };

        // The corner of the tile is outside the badge and outside its rounded
        // corner: nothing is drawn there at all.
        assert_eq!(at(0, 0)[3], 0, "the tile's corner must stay clear");
        // The middle of an edge is badge, and the badge is the badge colour.
        let edge = at(32, 6);
        assert!(edge[3] > 200, "the badge must be painted: {edge:?}");
        assert_eq!(
            [edge[0], edge[1], edge[2]],
            [BADGE.r(), BADGE.g(), BADGE.b()],
            "and painted in its own colour"
        );
        // And somewhere in the middle there is something lighter than the
        // badge, which is the mark.
        let lightest = (16..48)
            .flat_map(|y| (16..48).map(move |x| (x, y)))
            .map(|(x, y)| u32::from(at(x, y)[0]))
            .max()
            .unwrap_or_default();
        assert!(
            lightest > u32::from(BADGE.r()) + 40,
            "the mark must be drawn on the badge, lightest was {lightest}"
        );
    }

    /// A PNG a reader will open: the signature, the header it declares, and
    /// the end marker.
    #[test]
    fn the_png_is_one_a_reader_would_accept() {
        let image = render(16);
        let file = png(&image);
        assert_eq!(
            &file[..8],
            &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a],
            "the signature"
        );
        assert_eq!(&file[12..16], b"IHDR");
        assert_eq!(&file[16..20], &16_u32.to_be_bytes(), "the width");
        assert_eq!(&file[20..24], &16_u32.to_be_bytes(), "the height");
        assert_eq!(file[24], 8, "eight bits a channel");
        assert_eq!(file[25], 6, "truecolour with alpha");
        assert_eq!(&file[file.len() - 8..file.len() - 4], b"IEND");
    }

    /// The two checks a PNG carries, against values computed by hand.
    ///
    /// Both are one line of arithmetic that is wrong in a way nothing else
    /// notices: a file with a bad `CRC` still has the right bytes in it, and
    /// only the reader that checks says so.
    #[test]
    fn the_checks_are_the_ones_the_format_defines() {
        // `IEND` carries no body, and its check is the one constant every PNG
        // in the world ends with.
        assert_eq!(crc32(*b"IEND", &[]), 0xAE42_6082);
        // Adler-32 of "abc": the low half is one plus the three bytes, the
        // high half the running total of the low half after each of them.
        assert_eq!(adler32(b"abc"), 0x024D_0127);
        assert_eq!(adler32(&[]), 1);
    }

    /// The stored stream says how long each block is and repeats it inverted,
    /// which is what a decoder checks before reading one.
    #[test]
    fn a_stored_block_declares_its_own_length() {
        let stream = deflate_stored(b"four");
        assert_eq!(&stream[..2], &[0x78, 0x01], "the zlib header");
        assert_eq!(stream[2], 1, "the only block is the last one");
        let length = u16::from_le_bytes([stream[3], stream[4]]);
        let inverted = u16::from_le_bytes([stream[5], stream[6]]);
        assert_eq!(length, 4);
        assert_eq!(inverted, !length);
        assert_eq!(&stream[7..11], b"four");
    }
}
