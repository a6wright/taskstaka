//! The brand mark as raw RGBA for the status item (a template image on
//! macOS, so it follows the menu bar's light/dark rendering) and for the
//! Linux indicator. Decoded once from the PNG the web UI already serves.

/// `crates/taskstaka/assets/mark.png`, the favicon's source.
const MARK_PNG: &[u8] = include_bytes!("../../taskstaka/assets/mark.png");

/// The mark as 8-bit RGBA, with its width and height.
pub fn mark_rgba() -> (Vec<u8>, u32, u32) {
    let decoder = png::Decoder::new(std::io::Cursor::new(MARK_PNG));
    let mut reader = decoder.read_info().expect("mark.png is a PNG");
    let mut buf = vec![0; reader.output_buffer_size().expect("png has a size")];
    let info = reader.next_frame(&mut buf).expect("mark.png decodes");
    buf.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf
            .chunks(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => buf
            .chunks(2)
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => unreachable!("read_info expands palettes"),
    };
    (rgba, info.width, info.height)
}

/// The status item icon.
pub fn tray_icon() -> tray_icon::Icon {
    let (rgba, w, h) = mark_rgba();
    tray_icon::Icon::from_rgba(rgba, w, h).expect("mark as a tray icon")
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mark_decodes_to_square_rgba_with_transparency() {
        let (rgba, w, h) = mark_rgba();
        assert_eq!((w, h), (128, 128));
        assert_eq!(rgba.len(), (w * h * 4) as usize);
        let alphas: Vec<u8> = rgba.iter().skip(3).step_by(4).copied().collect();
        assert!(alphas.contains(&0), "corners are transparent");
        assert!(alphas.contains(&255), "the mark itself is opaque");
    }
}
