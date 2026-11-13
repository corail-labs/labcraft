//! Whether a trace still matches the files it was made from (notation 6.1).

use sha2::{Digest, Sha256};

use crate::read::TraceHeader;

/// The SHA-256 of these bytes, in lowercase hexadecimal, as the first line of a trace gives it.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut text = String::with_capacity(64);
    for byte in digest {
        for nibble in [byte >> 4, byte & 15] {
            text.push(char::from_digit(u32::from(nibble), 16).unwrap_or('0'));
        }
    }
    text
}

/// The files that `diagram` resolved to draw the function that have changed since the run, or are gone (`None`), in
/// the order given. A file that the first line does not list is stale too: the run did not read it, or the line
/// dropped it, and either way the trace cannot be said to match it.
pub fn stale<'a>(header: &TraceHeader, files: &[(&'a str, Option<&[u8]>)]) -> Vec<&'a str> {
    let hashed: Vec<(&'a str, Option<String>)> = files.iter().map(|(name, bytes)| (*name, bytes.map(sha256_hex))).collect();
    let hashed: Vec<(&'a str, Option<&str>)> = hashed.iter().map(|(name, sha)| (*name, sha.as_deref())).collect();
    stale_hashed(header, &hashed)
}

/// [`stale`], from the SHA-256 of each file, in lowercase or uppercase hexadecimal, rather than its bytes.
pub fn stale_hashed<'a>(header: &TraceHeader, files: &[(&'a str, Option<&str>)]) -> Vec<&'a str> {
    files
        .iter()
        .filter(|(name, sha)| {
            let recorded = header.files.iter().find(|(file, _)| file == name).map(|(_, recorded)| recorded.as_str());
            match (recorded, sha) {
                (Some(recorded), Some(sha)) => !recorded.eq_ignore_ascii_case(sha),
                _ => true,
            }
        })
        .map(|(name, _)| *name)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FIPS 180-2, appendix B.1, and the digest of nothing.
    #[test]
    fn sha256_follows_the_standard() {
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    }

    #[test]
    fn a_file_that_changed_or_went_is_stale() {
        let header = TraceHeader {
            files: vec![
                ("a.lcf".into(), sha256_hex(b"one")),
                ("b.lct".into(), sha256_hex(b"two").to_uppercase()),
                ("c.lcf".into(), sha256_hex(b"three")),
            ],
            ..TraceHeader::default()
        };
        let files: [(&str, Option<&[u8]>); 4] = [("a.lcf", Some(b"one")), ("b.lct", Some(b"two")), ("c.lcf", None), ("d.lcf", Some(b"four"))];
        assert_eq!(stale(&header, &files), ["c.lcf", "d.lcf"]);
        let changed: [(&str, Option<&[u8]>); 1] = [("a.lcf", Some(b"one\n"))];
        assert_eq!(stale(&header, &changed), ["a.lcf"]);
    }
}
