//! Minimal PNG encoder (8-bit RGBA, no interlace) used to store generated thumbnails.
//!
//! Thumbnails come back from the Windows shell as raw pixels; writing them straight to
//! disk as PNG keeps the cache small without pulling in a full image crate.

use flate2::write::ZlibEncoder;
use flate2::Compression;
use std::io::Write;

const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
const COLOR_TYPE_RGBA: u8 = 6;

const CRC_TABLE: [u32; 256] = {
    let mut table = [0_u32; 256];
    let mut index = 0_usize;
    while index < 256 {
        let mut value = index as u32;
        let mut bit = 0;
        while bit < 8 {
            value = if value & 1 != 0 { 0xEDB8_8320 ^ (value >> 1) } else { value >> 1 };
            bit += 1;
        }
        table[index] = value;
        index += 1;
    }
    table
};

pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for byte in bytes {
        let slot = ((crc ^ *byte as u32) & 0xFF) as usize;
        crc = CRC_TABLE[slot] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

#[derive(Debug, PartialEq, Eq)]
pub enum PngError {
    Empty,
    PixelCountMismatch { expected: usize, actual: usize },
    EncodeFailed,
}

/// Encode RGBA pixels as a PNG file body.
pub fn encode_rgba(width: u32, height: u32, pixels: &[u8]) -> Result<Vec<u8>, PngError> {
    if width == 0 || height == 0 {
        return Err(PngError::Empty);
    }
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|value| value.checked_mul(4))
        .ok_or(PngError::EncodeFailed)?;
    if pixels.len() != expected {
        return Err(PngError::PixelCountMismatch { expected, actual: pixels.len() });
    }

    let stride = (width as usize) * 4;
    let mut raw = Vec::with_capacity((stride + 1) * height as usize);
    for row in 0..height as usize {
        raw.push(0_u8); // filter type 0: no filtering
        raw.extend_from_slice(&pixels[row * stride..(row + 1) * stride]);
    }

    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(&raw).map_err(|_| PngError::EncodeFailed)?;
    let compressed = encoder.finish().map_err(|_| PngError::EncodeFailed)?;

    let mut output = Vec::with_capacity(compressed.len() + 128);
    output.extend_from_slice(&SIGNATURE);
    let mut header = Vec::with_capacity(13);
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.push(8); // bit depth
    header.push(COLOR_TYPE_RGBA);
    header.push(0); // deflate compression
    header.push(0); // adaptive filtering
    header.push(0); // no interlace
    write_chunk(&mut output, b"IHDR", &header);
    write_chunk(&mut output, b"IDAT", &compressed);
    write_chunk(&mut output, b"IEND", &[]);
    Ok(output)
}

fn write_chunk(output: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    output.extend_from_slice(&(data.len() as u32).to_be_bytes());
    output.extend_from_slice(kind);
    output.extend_from_slice(data);
    let mut crc_input = Vec::with_capacity(4 + data.len());
    crc_input.extend_from_slice(kind);
    crc_input.extend_from_slice(data);
    output.extend_from_slice(&crc32(&crc_input).to_be_bytes());
}

/// Convert the 32-bit BGRA rows Windows hands back into RGBA. Shell bitmaps do not
/// carry meaningful alpha, so it is forced opaque.
pub fn bgra_to_rgba(pixels: &mut [u8]) {
    for chunk in pixels.chunks_exact_mut(4) {
        let blue = chunk[0];
        chunk[0] = chunk[2];
        chunk[2] = blue;
        chunk[3] = 255;
    }
}

/// Flip bottom-up rows (the DIB layout Windows returns) into top-down order.
pub fn flip_vertical(pixels: &mut [u8], width: u32, height: u32) {
    let stride = (width as usize) * 4;
    if stride == 0 || height < 2 {
        return;
    }
    for row in 0..(height as usize) / 2 {
        let top = row * stride;
        let bottom = (height as usize - 1 - row) * stride;
        for column in 0..stride {
            pixels.swap(top + column, bottom + column);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::read::ZlibDecoder;
    use std::io::Read;

    fn sample(width: u32, height: u32) -> Vec<u8> {
        (0..(width * height * 4))
            .map(|index| (index % 251) as u8)
            .collect()
    }

    #[test]
    fn encoded_png_has_a_valid_signature_and_header() {
        let png = encode_rgba(3, 2, &sample(3, 2)).expect("encode");
        assert_eq!(&png[..8], &SIGNATURE);
        assert_eq!(&png[12..16], b"IHDR");
        let length = u32::from_be_bytes([png[8], png[9], png[10], png[11]]);
        assert_eq!(length, 13);
        let width = u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
        let height = u32::from_be_bytes([png[20], png[21], png[22], png[23]]);
        assert_eq!((width, height), (3, 2));
        assert_eq!(png[24], 8, "bit depth");
        assert_eq!(png[25], COLOR_TYPE_RGBA, "colour type");
        assert_eq!(png[png.len() - 8..png.len() - 4], *b"IEND");
    }

    #[test]
    fn every_chunk_carries_a_correct_crc() {
        let png = encode_rgba(4, 4, &sample(4, 4)).expect("encode");
        let mut cursor = 8_usize;
        let mut chunks = 0_usize;
        while cursor < png.len() {
            let length = u32::from_be_bytes([png[cursor], png[cursor + 1], png[cursor + 2], png[cursor + 3]]) as usize;
            let kind = &png[cursor + 4..cursor + 8];
            let data = &png[cursor + 8..cursor + 8 + length];
            let stored = u32::from_be_bytes([
                png[cursor + 8 + length],
                png[cursor + 9 + length],
                png[cursor + 10 + length],
                png[cursor + 11 + length],
            ]);
            let mut input = Vec::with_capacity(4 + length);
            input.extend_from_slice(kind);
            input.extend_from_slice(data);
            assert_eq!(crc32(&input), stored, "CRC mismatch in chunk {:?}", String::from_utf8_lossy(kind));
            cursor += 12 + length;
            chunks += 1;
        }
        assert_eq!(chunks, 3, "IHDR, IDAT, IEND");
    }

    #[test]
    fn pixel_data_round_trips_through_the_deflate_stream() {
        let pixels = sample(5, 3);
        let png = encode_rgba(5, 3, &pixels).expect("encode");
        // Locate IDAT and inflate it.
        let mut cursor = 8_usize;
        let mut idat = Vec::new();
        while cursor < png.len() {
            let length = u32::from_be_bytes([png[cursor], png[cursor + 1], png[cursor + 2], png[cursor + 3]]) as usize;
            if &png[cursor + 4..cursor + 8] == b"IDAT" {
                idat.extend_from_slice(&png[cursor + 8..cursor + 8 + length]);
            }
            cursor += 12 + length;
        }
        let mut inflated = Vec::new();
        ZlibDecoder::new(&idat[..]).read_to_end(&mut inflated).expect("inflate");
        assert_eq!(inflated.len(), (5 * 4 + 1) * 3, "filter byte per scanline");
        for row in 0..3_usize {
            assert_eq!(inflated[row * 21], 0, "scanline {} uses filter 0", row);
            assert_eq!(&inflated[row * 21 + 1..row * 21 + 21], &pixels[row * 20..row * 20 + 20]);
        }
    }

    #[test]
    fn mismatched_pixel_buffers_are_rejected() {
        assert_eq!(encode_rgba(2, 2, &[0_u8; 15]), Err(PngError::PixelCountMismatch { expected: 16, actual: 15 }));
        assert_eq!(encode_rgba(0, 4, &[]), Err(PngError::Empty));
    }

    #[test]
    fn crc32_matches_the_reference_vector() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn bgra_rows_become_opaque_rgba() {
        let mut pixels = vec![0x10, 0x20, 0x30, 0x00, 0x40, 0x50, 0x60, 0xFF];
        bgra_to_rgba(&mut pixels);
        assert_eq!(pixels, vec![0x30, 0x20, 0x10, 0xFF, 0x60, 0x50, 0x40, 0xFF]);
    }

    #[test]
    fn flipping_restores_top_down_row_order() {
        let mut pixels = vec![
            0, 0, 0, 255, // bottom row (first in a DIB)
            1, 1, 1, 255, // top row
        ];
        flip_vertical(&mut pixels, 1, 2);
        assert_eq!(pixels, vec![1, 1, 1, 255, 0, 0, 0, 255]);
    }
}
