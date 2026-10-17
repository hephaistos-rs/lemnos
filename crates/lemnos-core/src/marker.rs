//! The marker tag that ties a machine record to the guest Lemnos created.
//!
//! A provider's ID for a guest is not enough to know it is still the same
//! guest: Proxmox hands a freed VM ID to the next guest made, whoever makes
//! it. So every guest Lemnos creates also carries a tag of its own,
//! `lemnos-` and 12 random lower-case hex digits, and the record keeps a
//! copy. A record applies to a guest only while both agree.
//!
//! The digits come from the operating system's random source by way of a
//! cryptographically secure generator, never from the guest's ID, so a
//! marker can't be worked out from anything else about the machine. The
//! format fits Proxmox's rule for tags: a letter or digit first, then
//! letters, digits, `-`, `_`, `+` and `.`.

const PREFIX: &str = "lemnos-";
const DIGITS: usize = 12;

/// A fresh marker, e.g. `lemnos-3f9a0c7e21bd`.
pub fn generate() -> String {
    let bytes: [u8; DIGITS / 2] = rand::random();
    let mut marker = String::with_capacity(PREFIX.len() + DIGITS);
    marker.push_str(PREFIX);
    for byte in bytes {
        marker.push_str(&format!("{byte:02x}"));
    }
    marker
}

/// Whether `tag` has the marker's form. Lemnos never takes such a tag off a
/// guest, and doesn't show one to anyone but admins.
pub fn is_marker(tag: &str) -> bool {
    tag.strip_prefix(PREFIX).is_some_and(|digits| {
        digits.len() == DIGITS
            && digits
                .chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_marker_has_the_form() {
        let marker = generate();
        assert!(is_marker(&marker), "{marker}");
        assert_eq!(marker.len(), "lemnos-".len() + 12);
    }

    #[test]
    fn markers_differ() {
        let markers: std::collections::HashSet<_> = (0..1000).map(|_| generate()).collect();
        assert_eq!(markers.len(), 1000);
    }

    #[test]
    fn other_tags_are_not_markers() {
        for tag in [
            "lemnos",
            "lemnos-",
            "personal-jo",
            "lemnos-3f9a0c7e21b",
            "lemnos-3f9a0c7e21bdd",
            "lemnos-3F9A0C7E21BD",
            "lemnos-3f9a0c7e21bg",
            "xlemnos-3f9a0c7e21bd",
        ] {
            assert!(!is_marker(tag), "{tag}");
        }
        assert!(is_marker("lemnos-3f9a0c7e21bd"));
    }
}
