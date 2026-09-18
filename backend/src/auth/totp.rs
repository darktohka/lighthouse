//! TOTP (RFC 6238) and HOTP (RFC 4226) primitives, base32 and backup codes.
//!
//! Everything here is built from crates the project already ships — `sha1` for
//! the HMAC, `rand` for secrets — so no dependency is added. The HMAC-SHA1
//! construction mirrors `tokens::hmac_sha256`: block size 64, inner/outer pads,
//! the key hashed first when it exceeds one block.
//!
//! Authenticator apps implement TOTP with HMAC-SHA1, 6 digits and a 30-second
//! period by default; the `otpauth://` URI emitted by [`otpauth_uri`] pins those
//! parameters so the enrolment QR code cannot drift from what [`verify`] checks.

use sha1::{Digest, Sha1};
use subtle::ConstantTimeEq;

/// Alphabet for RFC 4648 base32 (no padding is emitted).
const BASE32_ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// Characters used for backup codes. `0`/`O` and `1`/`I` are deliberately kept
/// so the codes match the requested `A1B0-A2C4` shape; users copy them, they do
/// not retype them from memory often enough to warrant an ambiguity carve-out.
const BACKUP_ALPHABET: &[u8; 36] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

/// Number of bytes in a freshly generated TOTP secret (160 bits, per RFC 4226).
pub const SECRET_BYTES: usize = 20;
/// Number of digits in a generated TOTP code.
pub const DIGITS: u32 = 6;
/// TOTP time step in seconds.
pub const PERIOD: u64 = 30;
/// Number of backup codes minted per enrolment.
pub const BACKUP_CODE_COUNT: usize = 8;
/// Characters in a backup code, excluding the separating dash.
pub const BACKUP_CODE_LEN: usize = 8;

// ---- base32 ----------------------------------------------------------------

/// Encodes bytes as unpadded RFC 4648 base32 (the form authenticator apps want).
pub fn base32_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(5) * 8);
    for chunk in data.chunks(5) {
        let mut buffer = [0u8; 5];
        buffer[..chunk.len()].copy_from_slice(chunk);
        let bits = ((buffer[0] as u64) << 32)
            | ((buffer[1] as u64) << 24)
            | ((buffer[2] as u64) << 16)
            | ((buffer[3] as u64) << 8)
            | (buffer[4] as u64);
        let indices = [
            (bits >> 35) & 0x1f,
            (bits >> 30) & 0x1f,
            (bits >> 25) & 0x1f,
            (bits >> 20) & 0x1f,
            (bits >> 15) & 0x1f,
            (bits >> 10) & 0x1f,
            (bits >> 5) & 0x1f,
            bits & 0x1f,
        ];
        // 5 bytes -> 8 characters, 4 bytes -> 7, … as the trailing bits are
        // only meaningful when the corresponding input byte exists.
        let characters = match chunk.len() {
            1 => 2,
            2 => 4,
            3 => 5,
            4 => 7,
            _ => 8,
        };
        for index in indices.iter().take(characters) {
            out.push(BASE32_ALPHABET[*index as usize] as char);
        }
    }
    out
}

/// Decodes RFC 4648 base32, accepting lower case and ignoring `=` padding.
/// Returns `None` for characters outside the alphabet.
pub fn base32_decode(input: &str) -> Option<Vec<u8>> {
    let cleaned: Vec<u8> = input
        .bytes()
        .filter(|byte| *byte != b'=' && !byte.is_ascii_whitespace())
        .map(|byte| byte.to_ascii_uppercase())
        .collect();

    let mut out = Vec::with_capacity(cleaned.len() * 5 / 8);
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    for byte in cleaned {
        let value = BASE32_ALPHABET.iter().position(|c| *c == byte)? as u32;
        buffer = (buffer << 5) | value;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Some(out)
}

// ---- HMAC-SHA1 / HOTP / TOTP ----------------------------------------------

/// RFC 2104 HMAC-SHA1.
fn hmac_sha1(key: &[u8], message: &[u8]) -> [u8; 20] {
    const BLOCK: usize = 64;
    let mut block = [0u8; BLOCK];
    if key.len() > BLOCK {
        let digest = Sha1::digest(key);
        block[..digest.len()].copy_from_slice(&digest);
    } else {
        block[..key.len()].copy_from_slice(key);
    }

    let mut inner_pad = [0x36u8; BLOCK];
    let mut outer_pad = [0x5cu8; BLOCK];
    for index in 0..BLOCK {
        inner_pad[index] ^= block[index];
        outer_pad[index] ^= block[index];
    }

    let mut inner = Sha1::new();
    inner.update(inner_pad);
    inner.update(message);
    let inner_digest = inner.finalize();

    let mut outer = Sha1::new();
    outer.update(outer_pad);
    outer.update(inner_digest);
    let digest = outer.finalize();

    let mut result = [0u8; 20];
    result.copy_from_slice(&digest);
    result
}

/// RFC 4226 HOTP: HMAC the big-endian counter and truncate to `digits`.
pub fn hotp(secret: &[u8], counter: u64, digits: u32) -> u32 {
    let digest = hmac_sha1(secret, &counter.to_be_bytes());
    let offset = (digest[19] & 0x0f) as usize;
    let truncated = ((u32::from(digest[offset]) & 0x7f) << 24)
        | (u32::from(digest[offset + 1]) << 16)
        | (u32::from(digest[offset + 2]) << 8)
        | u32::from(digest[offset + 3]);
    truncated % 10u32.pow(digits)
}

/// RFC 6238 TOTP for a Unix timestamp.
pub fn totp(secret: &[u8], unix_secs: i64) -> u32 {
    let counter = (unix_secs.max(0) as u64) / PERIOD;
    hotp(secret, counter, DIGITS)
}

/// Formats a TOTP value as a zero-padded `DIGITS`-digit string.
pub fn format_code(value: u32) -> String {
    format!("{value:0width$}", width = DIGITS as usize)
}

/// Verifies a user-supplied TOTP code, tolerating `window` steps of clock skew
/// in either direction. The comparison is constant-time.
pub fn verify(secret: &[u8], code: &str, unix_secs: i64, window: i64) -> bool {
    verify_step(secret, code, unix_secs, window).is_some()
}

/// Like [`verify`] but returns the matched time step, which callers persist to
/// reject replays of the same code inside its window (RFC 6238 §5.2).
pub fn verify_step(secret: &[u8], code: &str, unix_secs: i64, window: i64) -> Option<u64> {
    let provided = code.trim().parse::<u32>().ok()?;
    let step = unix_secs.max(0) / PERIOD as i64;
    (-window..=window).find_map(|offset| {
        let counter = step + offset;
        if counter < 0 {
            return None;
        }
        let counter = counter as u64;
        let expected = hotp(secret, counter, DIGITS);
        bool::from(expected.ct_eq(&provided)).then_some(counter)
    })
}

// ---- secrets and backup codes ---------------------------------------------

/// Generates a base32-encoded 160-bit TOTP secret.
pub fn generate_secret() -> String {
    let mut bytes = [0u8; SECRET_BYTES];
    rand::fill(&mut bytes);
    base32_encode(&bytes)
}

/// Picks random characters from `alphabet` without modulo bias.
fn random_characters<const N: usize>(alphabet: &[u8]) -> [u8; N] {
    let limit = (256 / alphabet.len()) * alphabet.len();
    let mut out = [0u8; N];
    for slot in out.iter_mut() {
        loop {
            let mut byte = [0u8; 1];
            rand::fill(&mut byte);
            if usize::from(byte[0]) < limit {
                *slot = alphabet[usize::from(byte[0]) % alphabet.len()];
                break;
            }
        }
    }
    out
}

/// Generates one backup code in `XXXX-XXXX` form.
pub fn generate_backup_code() -> String {
    let chars = random_characters::<BACKUP_CODE_LEN>(BACKUP_ALPHABET);
    let text = String::from_utf8_lossy(&chars).to_string();
    format!("{}-{}", &text[..4], &text[4..])
}

/// Generates [`BACKUP_CODE_COUNT`] backup codes.
pub fn generate_backup_codes() -> Vec<String> {
    (0..BACKUP_CODE_COUNT)
        .map(|_| generate_backup_code())
        .collect()
}

/// Normalizes a submitted backup code: upper-cases it, drops separators and
/// whitespace, and re-renders it in canonical `XXXX-XXXX` form.
pub fn normalize_backup_code(raw: &str) -> Option<String> {
    let cleaned: String = raw
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if cleaned.len() != BACKUP_CODE_LEN || !cleaned.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return None;
    }
    Some(format!("{}-{}", &cleaned[..4], &cleaned[4..]))
}

/// Builds the `otpauth://` enrolment URI for `account`, labelled with `issuer`.
pub fn otpauth_uri(issuer: &str, account: &str, secret: &str) -> String {
    let issuer_encoded = encode_component(issuer);
    let account_encoded = encode_component(account);
    format!(
        "otpauth://totp/{issuer_encoded}:{account_encoded}?secret={secret}\
         &issuer={issuer_encoded}&algorithm=SHA1&digits={DIGITS}&period={PERIOD}"
    )
}

/// Percent-encodes a URI component, leaving unreserved characters intact.
fn encode_component(value: &str) -> String {
    use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};

    const UNRESERVED: &AsciiSet = &CONTROLS
        .add(b' ')
        .add(b'"')
        .add(b'#')
        .add(b'%')
        .add(b'&')
        .add(b'\'')
        .add(b'(')
        .add(b')')
        .add(b'*')
        .add(b'+')
        .add(b',')
        .add(b'/')
        .add(b':')
        .add(b';')
        .add(b'<')
        .add(b'=')
        .add(b'>')
        .add(b'?')
        .add(b'@')
        .add(b'[')
        .add(b'\\')
        .add(b']')
        .add(b'^')
        .add(b'`')
        .add(b'{')
        .add(b'|')
        .add(b'}');

    utf8_percent_encode(value, UNRESERVED).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 4226 Appendix D / RFC 6238 Appendix B shared secret.
    const RFC_SECRET: &[u8] = b"12345678901234567890";
    const RFC_SECRET_B32: &str = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";

    #[test]
    fn base32_round_trips_rfc4648_vectors() {
        for (raw, encoded) in [
            (b"".as_slice(), ""),
            (b"f".as_slice(), "MY"),
            (b"fo".as_slice(), "MZXQ"),
            (b"foo".as_slice(), "MZXW6"),
            (b"foob".as_slice(), "MZXW6YQ"),
            (b"fooba".as_slice(), "MZXW6YTB"),
            (b"foobar".as_slice(), "MZXW6YTBOI"),
        ] {
            assert_eq!(base32_encode(raw), encoded);
            assert_eq!(base32_decode(encoded).expect("decode"), raw);
        }
        assert_eq!(
            base32_decode(RFC_SECRET_B32).expect("decode"),
            RFC_SECRET.to_vec()
        );
    }

    #[test]
    fn base32_decode_accepts_lowercase_and_padding() {
        assert_eq!(
            base32_decode("mzxw6ytboi======").expect("decode"),
            b"foobar".to_vec()
        );
        assert!(base32_decode("not-base32!").is_none());
    }

    #[test]
    fn hotp_matches_rfc4226_vectors() {
        let expected = [
            "755224", "287082", "359152", "969429", "338314", "254676", "287922", "162583",
            "399871", "520489",
        ];
        for (counter, code) in expected.iter().enumerate() {
            assert_eq!(
                format_code(hotp(RFC_SECRET, counter as u64, DIGITS)),
                *code,
                "counter {counter}"
            );
        }
    }

    #[test]
    fn totp_matches_rfc6238_sha1_vectors() {
        let vectors = [
            (59_i64, "287082"),
            (1_111_111_109, "081804"),
            (1_111_111_111, "050471"),
            (1_234_567_890, "005924"),
            (2_000_000_000, "279037"),
            (20_000_000_000, "353130"),
        ];
        for (timestamp, expected) in vectors {
            assert_eq!(format_code(totp(RFC_SECRET, timestamp)), expected);
        }
    }

    #[test]
    fn verify_tolerates_one_step_of_drift() {
        let timestamp = 1_234_567_890_i64;
        let code = format_code(totp(RFC_SECRET, timestamp));
        assert!(verify(RFC_SECRET, &code, timestamp, 1));
        assert!(verify(RFC_SECRET, &code, timestamp + PERIOD as i64, 1));
        assert!(verify(RFC_SECRET, &code, timestamp - PERIOD as i64, 1));
        assert!(!verify(RFC_SECRET, &code, timestamp + 2 * PERIOD as i64, 1));
        assert!(!verify(RFC_SECRET, "000000", timestamp, 0) || code == "000000");
        assert!(!verify(RFC_SECRET, "not-digits", timestamp, 1));
    }

    #[test]
    fn generated_secrets_are_base32_and_unique() {
        let first = generate_secret();
        let second = generate_secret();
        assert_eq!(first.len(), 32);
        assert_ne!(first, second);
        assert_eq!(base32_decode(&first).expect("decode").len(), SECRET_BYTES);
    }

    #[test]
    fn backup_codes_have_the_requested_shape() {
        let codes = generate_backup_codes();
        assert_eq!(codes.len(), BACKUP_CODE_COUNT);
        for code in &codes {
            assert_eq!(code.len(), 9, "XXXX-XXXX");
            assert_eq!(code.as_bytes()[4], b'-');
            assert!(code.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
            assert_eq!(
                normalize_backup_code(code).expect("normalize"),
                *code,
                "round-trips through normalization"
            );
        }
    }

    #[test]
    fn backup_code_normalization_accepts_sloppy_input() {
        assert_eq!(
            normalize_backup_code(" ab12-cd34 ").as_deref(),
            Some("AB12-CD34")
        );
        assert_eq!(
            normalize_backup_code("ab12cd34").as_deref(),
            Some("AB12-CD34")
        );
        assert!(normalize_backup_code("short").is_none());
        assert!(normalize_backup_code("way-too-long-code").is_none());
        assert!(normalize_backup_code("ab12-cd3!").is_none());
    }

    #[test]
    fn otpauth_uri_carries_expected_parameters() {
        let uri = otpauth_uri("Lighthouse", "alice", "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ");
        assert!(uri.starts_with("otpauth://totp/Lighthouse:alice?"));
        assert!(uri.contains("secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ"));
        assert!(uri.contains("issuer=Lighthouse"));
        assert!(uri.contains("algorithm=SHA1"));
        assert!(uri.contains("digits=6"));
        assert!(uri.contains("period=30"));
    }
}
