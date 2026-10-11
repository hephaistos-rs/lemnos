// QR codes, drawn on the server as SVG and embedded straight into the page.
// What they encode (a TOTP secret, say) therefore never leaves Lemnos for an
// image service, and no script is needed to show them.

use qrcodegen::{QrCode, QrCodeEcc};

/// Blank modules around the code. Scanners need this "quiet zone"; the QR
/// standard asks for four.
const BORDER: i32 = 4;

/// `text` as a QR code, in the form of a `data:` URL for an `<img src>`.
/// `None` if the text is too long to fit in a QR code.
pub fn data_url(text: &str) -> Option<String> {
    Some(to_data_url(&svg(text)?))
}

fn svg(text: &str) -> Option<String> {
    // Medium error correction: still scans with ~15% of the code obscured,
    // without making it denser than a phone camera likes.
    let code = QrCode::encode_text(text, QrCodeEcc::Medium).ok()?;
    let size = code.size() + 2 * BORDER;

    // One square per dark module, all in a single path.
    let mut path = String::new();
    for y in 0..code.size() {
        for x in 0..code.size() {
            if code.get_module(x, y) {
                path.push_str(&format!("M{},{}h1v1h-1z", x + BORDER, y + BORDER));
            }
        }
    }
    // Always black on white, whatever the page's theme: many scanners can't
    // read light-on-dark codes.
    Some(format!(
        "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 {size} {size}' \
         shape-rendering='crispEdges'>\
         <rect width='100%' height='100%' fill='white'/>\
         <path d='{path}' fill='black'/></svg>"
    ))
}

/// Percent-encodes the characters that would end a URL early or be read as
/// its fragment, so the SVG survives inside an attribute.
fn to_data_url(svg: &str) -> String {
    let mut url = String::from("data:image/svg+xml,");
    for character in svg.chars() {
        match character {
            '%' => url.push_str("%25"),
            '#' => url.push_str("%23"),
            '<' => url.push_str("%3C"),
            '>' => url.push_str("%3E"),
            '"' => url.push_str("%22"),
            ' ' => url.push_str("%20"),
            other => url.push(other),
        }
    }
    url
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "otpauth://totp/Lemnos:alice?secret=JBSWY3DPEHPK3PXP&issuer=Lemnos";

    #[test]
    fn draws_a_square_code_with_a_quiet_zone() {
        let svg = svg(URL).unwrap();
        let code = QrCode::encode_text(URL, QrCodeEcc::Medium).unwrap();
        let size = code.size() + 2 * BORDER;
        assert!(svg.contains(&format!("viewBox='0 0 {size} {size}'")));
        // The top-left corner of every QR code is a dark finder pattern,
        // which sits just inside the border.
        assert!(svg.contains(&format!("M{BORDER},{BORDER}h1v1h-1z")));
        // One square per dark module, none in the border.
        let dark = (0..code.size())
            .flat_map(|y| (0..code.size()).map(move |x| (x, y)))
            .filter(|&(x, y)| code.get_module(x, y))
            .count();
        assert_eq!(svg.matches("h1v1h-1z").count(), dark);
    }

    #[test]
    fn data_url_is_safe_inside_an_attribute() {
        let url = data_url(URL).unwrap();
        assert!(url.starts_with("data:image/svg+xml,%3Csvg%20xmlns="));
        for forbidden in ['<', '>', '"', '#', ' '] {
            assert!(!url.contains(forbidden), "contains {forbidden:?}");
        }
    }

    #[test]
    fn too_much_text_is_refused() {
        assert!(data_url(&"x".repeat(5000)).is_none());
    }
}
