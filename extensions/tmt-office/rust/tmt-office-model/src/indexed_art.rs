//! Narrow validation primitives shared by the two reviewed indexed-art contracts.

#[cfg(test)]
pub const KEY_LIMIT: usize = crate::office_art_reference::KEY_LIMIT;
pub const LABEL_LIMIT: usize = 80;
pub const CREDIT_LIMIT: usize = 120;
pub const LICENSE_LIMIT: usize = 64;
pub const PALETTE_LIMIT: usize = 16;

/// Advisory geometry over a nonempty, already-admitted indexed raster.
pub fn has_opaque_edge(pixels: &[String], index_width: usize) -> bool {
    pixels[0]
        .bytes()
        .chain(pixels[pixels.len() - 1].bytes())
        .any(|value| value != b'0')
        || pixels.iter().any(|row| {
            row[..index_width]
                .bytes()
                .chain(row[row.len() - index_width..].bytes())
                .any(|value| value != b'0')
        })
}

pub fn valid_text(value: &str, max_bytes: usize) -> bool {
    !value.is_empty() && value.len() <= max_bytes && !value.chars().any(char::is_control)
}

pub fn valid_key(value: &str) -> bool {
    crate::office_art_reference::valid_office_art_key(value)
}

pub fn valid_license(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= LICENSE_LIMIT
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".+-".contains(&byte))
}

pub fn valid_palette_with_limit(palette: &[String], limit: usize) -> bool {
    (1..=limit).contains(&palette.len())
        && palette.first().map(String::as_str) == Some("#00000000")
        && palette.iter().skip(1).all(|value| {
            value.len() == 9
                && value.starts_with('#')
                && value[1..]
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                && value.ends_with("ff")
        })
}

pub fn validate_encoded_raster(
    pixels: &[String],
    palette_len: usize,
    expected_width: Option<usize>,
    expected_height: Option<usize>,
    max_side: usize,
    max_cells: usize,
    index_width: usize,
) -> Option<usize> {
    if !(1..=2).contains(&index_width) {
        return None;
    }
    if pixels.is_empty()
        || pixels.len() > max_side
        || expected_height.is_some_and(|value| value != pixels.len())
    {
        return None;
    }
    let row_bytes = pixels.first()?.len();
    if row_bytes % index_width != 0 {
        return None;
    }
    let width = row_bytes / index_width;
    if width == 0
        || width > max_side
        || expected_width.is_some_and(|value| value != width)
        || width.checked_mul(pixels.len())? > max_cells
    {
        return None;
    }
    pixels
        .iter()
        .all(|row| {
            row.len() == row_bytes
                && row.as_bytes().chunks_exact(index_width).all(|cell| {
                    cell.iter()
                        .try_fold(0usize, |index, byte| {
                            let digit = match byte {
                                b'0'..=b'9' => byte - b'0',
                                b'a'..=b'f' => byte - b'a' + 10,
                                _ => return None,
                            };
                            Some(index * 16 + usize::from(digit))
                        })
                        .is_some_and(|index| index < palette_len)
                })
        })
        .then_some(width * pixels.len())
}

#[cfg(test)]
mod quality_tests {
    use super::has_opaque_edge;

    #[test]
    fn edge_checks_whole_palette_indices_for_both_admitted_encodings() {
        for (width, blank, center, edge) in
            [(1, "000", "0f0", "00f"), (2, "000000", "001000", "000010")]
        {
            let mut pixels = vec![blank.to_owned(), center.to_owned(), blank.to_owned()];
            assert!(!has_opaque_edge(&pixels, width));
            pixels[1] = edge.into();
            assert!(has_opaque_edge(&pixels, width));
            pixels[1] = blank.into();
            pixels[0] = center.into();
            assert!(has_opaque_edge(&pixels, width));
        }
    }
}
