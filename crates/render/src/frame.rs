//! CPU frame composition.
//!
//! A banner frame is small (a 3840 × 56 strip is under 1 MB), so copying the
//! pre-drawn segment images into it every frame is cheap on any CPU. This
//! path needs no GPU at all, which makes it the baseline that runs on every
//! machine and in CI. A GPU backend can be added on top later.

use tiny_skia::Pixmap;

use crate::style::Rgba;

/// Fills `frame` with `background` and draws each `(image, x)` at horizontal
/// pixel offset `x` (may be negative or past the right edge; images are
/// clipped). Images are premultiplied RGBA and blended source-over, so
/// translucent skins work.
pub fn compose<'a, I>(frame: &mut Pixmap, background: Rgba, items: I)
where
    I: IntoIterator<Item = (&'a Pixmap, i32)>,
{
    frame.fill(tiny_skia::Color::from_rgba8(
        background.r,
        background.g,
        background.b,
        background.a,
    ));
    let frame_w = i64::from(frame.width());
    let frame_h = frame.height();
    let dst = frame.data_mut();
    for (image, x) in items {
        let x = i64::from(x);
        let img_w = i64::from(image.width());
        let x0 = x.max(0);
        let x1 = (x + img_w).min(frame_w);
        if x0 >= x1 {
            continue;
        }
        let rows = image.height().min(frame_h);
        let src = image.data();
        for row in 0..i64::from(rows) {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let (src_start, dst_start, len) = (
                ((row * img_w + (x0 - x)) * 4) as usize,
                ((row * frame_w + x0) * 4) as usize,
                ((x1 - x0) * 4) as usize,
            );
            blend_row(
                &mut dst[dst_start..dst_start + len],
                &src[src_start..src_start + len],
            );
        }
    }
}

fn blend_row(dst: &mut [u8], src: &[u8]) {
    for (d, s) in dst
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(src.as_chunks::<4>().0)
    {
        match s[3] {
            255 => *d = *s,
            0 => {}
            a => {
                let inv = 255 - u32::from(a);
                for c in 0..4 {
                    #[allow(clippy::cast_possible_truncation)]
                    {
                        d[c] = (u32::from(s[c]) + u32::from(d[c]) * inv / 255) as u8;
                    }
                }
            }
        }
    }
}

/// Converts an opaque premultiplied RGBA frame to `0x00RRGGBB` pixels, the
/// layout window buffers such as `softbuffer` expect.
///
/// # Panics
///
/// Panics if `out` is shorter than the frame.
pub fn to_xrgb(frame: &Pixmap, out: &mut [u32]) {
    let pixels = (frame.width() * frame.height()) as usize;
    assert!(out.len() >= pixels, "output buffer too small");
    for (o, p) in out.iter_mut().zip(frame.data().as_chunks::<4>().0) {
        *o = (u32::from(p[0]) << 16) | (u32::from(p[1]) << 8) | u32::from(p[2]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, rgba: [u8; 4]) -> Pixmap {
        let mut p = Pixmap::new(w, h).unwrap();
        p.fill(tiny_skia::Color::from_rgba8(
            rgba[0], rgba[1], rgba[2], rgba[3],
        ));
        p
    }

    fn pixel(p: &Pixmap, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * p.width() + x) * 4) as usize;
        p.data()[i..i + 4].try_into().unwrap()
    }

    #[test]
    fn places_and_clips_images() {
        let mut frame = Pixmap::new(10, 2).unwrap();
        let red = solid(4, 2, [255, 0, 0, 255]);
        let green = solid(4, 2, [0, 255, 0, 255]);
        compose(&mut frame, Rgba::hex(0x000000), [(&red, -2), (&green, 8)]);
        assert_eq!(pixel(&frame, 0, 0), [255, 0, 0, 255]);
        assert_eq!(pixel(&frame, 1, 1), [255, 0, 0, 255]);
        assert_eq!(pixel(&frame, 2, 0), [0, 0, 0, 255]);
        assert_eq!(pixel(&frame, 7, 0), [0, 0, 0, 255]);
        assert_eq!(pixel(&frame, 8, 1), [0, 255, 0, 255]);
        assert_eq!(pixel(&frame, 9, 0), [0, 255, 0, 255]);
    }

    #[test]
    fn fully_outside_images_are_skipped() {
        let mut frame = Pixmap::new(4, 1).unwrap();
        let red = solid(2, 1, [255, 0, 0, 255]);
        compose(
            &mut frame,
            Rgba::hex(0x0000FF),
            [(&red, -2), (&red, 4), (&red, 100)],
        );
        assert!(frame.data().chunks(4).all(|p| p == [0, 0, 255, 255]));
    }

    #[test]
    fn taller_images_are_cropped() {
        let mut frame = Pixmap::new(2, 1).unwrap();
        let red = solid(2, 3, [255, 0, 0, 255]);
        compose(&mut frame, Rgba::hex(0), [(&red, 0)]);
        assert_eq!(pixel(&frame, 1, 0), [255, 0, 0, 255]);
    }

    #[test]
    fn translucent_images_blend() {
        let mut frame = Pixmap::new(1, 1).unwrap();
        // 50% red, premultiplied.
        let half_red = solid(1, 1, [255, 0, 0, 128]);
        compose(&mut frame, Rgba::hex(0x0000FF), [(&half_red, 0)]);
        let [r, g, b, a] = pixel(&frame, 0, 0);
        assert_eq!((g, a), (0, 255));
        assert!(r.abs_diff(128) <= 1 && b.abs_diff(127) <= 1, "{r} {b}");
    }

    #[test]
    fn xrgb_conversion() {
        let frame = solid(2, 1, [0x12, 0x34, 0x56, 255]);
        let mut out = [0u32; 2];
        to_xrgb(&frame, &mut out);
        assert_eq!(out, [0x0012_3456; 2]);
    }
}
