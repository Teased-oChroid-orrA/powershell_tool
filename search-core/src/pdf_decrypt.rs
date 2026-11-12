//! PDFs with an `/Encrypt` dictionary but an empty user password open in any
//! viewer (the restrictions are only "owner" flags: no printing, no copying),
//! yet every stream in them is ciphertext. The extractor scans raw bytes, so
//! without this step such a file yields no text at all. `lopdf` loads the
//! document (it tries the empty password), decrypts each object, and writes it
//! back unencrypted; the extractor then runs on that copy. A file that needs a
//! real password stays as it was and extracts to nothing, as before.

use std::borrow::Cow;

/// Beyond this the load (which holds the whole object table in memory) is not
/// worth it; such a file is left encrypted.
const MAX_DECRYPT_BYTES: usize = 128 * 1024 * 1024;

/// The same bytes when the file is not encrypted (or cannot be opened with an
/// empty password), else an equivalent unencrypted PDF. Idempotent: the output
/// carries no `/Encrypt`, so a second call returns it unchanged.
pub(crate) fn open_with_empty_password(bytes: &[u8]) -> Cow<'_, [u8]> {
    if bytes.len() > MAX_DECRYPT_BYTES || !bytes.windows(8).any(|w| w == b"/Encrypt") {
        return Cow::Borrowed(bytes);
    }
    match decrypt(bytes) {
        Some(plain) => Cow::Owned(plain),
        None => Cow::Borrowed(bytes),
    }
}

fn decrypt(bytes: &[u8]) -> Option<Vec<u8>> {
    // lopdf can panic on malformed input; a bad file must not take the search down.
    std::panic::catch_unwind(|| {
        let mut doc = lopdf::Document::load_mem(bytes).ok()?;
        if doc.is_encrypted() {
            return None; // needs a real password
        }
        let mut out = Vec::with_capacity(bytes.len());
        doc.save_to(&mut out).ok()?;
        Some(out)
    })
    .ok()
    .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/test-data/").to_owned() + name).unwrap()
    }

    #[test]
    fn every_encryption_scheme_with_an_empty_user_password_extracts_its_text() {
        for name in ["encrypted-rc4-40.pdf", "encrypted-rc4-128.pdf", "encrypted-aes-128.pdf", "encrypted-aes-256.pdf"] {
            let bytes = fixture(name);
            assert!(bytes.windows(8).any(|w| w == b"/Encrypt"), "{name} is not encrypted");
            let (lines, _) = crate::extraction::extract_pdf_lines(&bytes, 30, None, false);
            let text = lines.unwrap_or_default().join("\n");
            assert!(text.contains("This page mentions apple."), "{name}: {text:?}");
            assert!(text.contains("This page mentions banana."), "{name}: {text:?}");
        }
    }

    #[test]
    fn plain_and_unopenable_files_are_returned_untouched() {
        let plain = fixture("test.pdf");
        assert!(matches!(open_with_empty_password(&plain), Cow::Borrowed(_)));
        let junk = b"%PDF-1.6\n1 0 obj << /Encrypt 9 0 R >> endobj\ntrailer << /Encrypt 9 0 R >>\n%%EOF";
        assert!(matches!(open_with_empty_password(junk), Cow::Borrowed(_)));
    }

    #[test]
    fn decrypting_twice_is_a_no_op() {
        let once = open_with_empty_password(&fixture("encrypted-aes-128.pdf")).into_owned();
        assert!(matches!(open_with_empty_password(&once), Cow::Borrowed(_)));
    }
}
