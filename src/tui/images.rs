//! Optional kitty graphics protocol support for inline image comparisons.
//!
//! Detection is conservative: images are only emitted when the terminal
//! advertises kitty support. Every failure path falls back to the text view.

use std::{env, io::Cursor, path::Path};

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub(super) fn kitty_supported() -> bool {
    env::var_os("KITTY_WINDOW_ID").is_some()
        || env::var("TERM")
            .map(|term| term.contains("kitty"))
            .unwrap_or(false)
}

pub(super) fn clear_escape() -> String {
    let mut out = String::new();
    for id in 1..=2 {
        out.push_str(&format!("\x1b_Ga=d,d=I,i={id}\x1b\\"));
    }
    out
}

/// Build kitty APC escapes for two images placed side by side at the cursor.
pub(super) fn pair_escape(left: &Path, right: &Path, columns: u16, rows: u16) -> Option<String> {
    let per_image = columns.saturating_sub(2) / 2;
    if per_image < 8 || rows < 4 {
        return None;
    }
    let left = payload(left, per_image, rows)?;
    let right = payload(right, per_image, rows)?;

    let mut out = String::new();
    for (id, data) in [(1, &left), (2, &right)] {
        out.push_str(&format!(
            "\x1b_Gf=100,a=T,t=d,p=1,i={id},c={per_image},r={rows},C=1,z=0;"
        ));
        out.push_str(data);
        out.push_str("\x1b\\");
        if id == 1 {
            out.push_str(&format!("\x1b[{}C", per_image.saturating_add(1)));
        }
    }
    Some(out)
}

fn payload(path: &Path, columns: u16, rows: u16) -> Option<String> {
    let image = image::open(path).ok()?;
    let width = u32::from(columns.saturating_mul(2));
    let height = u32::from(rows.saturating_mul(2));
    let scaled = image.resize(
        width.max(1),
        height.max(1),
        image::imageops::FilterType::Lanczos3,
    );
    let mut png = Cursor::new(Vec::new());
    scaled.write_to(&mut png, image::ImageFormat::Png).ok()?;
    Some(base64(&png.into_inner()))
}

fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        out.push(BASE64[(first >> 2) as usize] as char);
        out.push(BASE64[(((first & 0b11) << 4) | (second >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(BASE64[(((second & 0b1111) << 2) | (third >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(BASE64[(third & 0b11_1111) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn image_pair_is_skipped_when_there_is_no_room() {
        assert!(pair_escape(Path::new("a.png"), Path::new("b.png"), 12, 4).is_none());
    }
}
