//! Hashing primitives implemented with the standard library only.
//!
//! Two hashes are used across the engine, and they serve different purposes:
//!
//! * [`content_fingerprint`] produces a short, stable hex string derived from the
//!   normalized *content* of a memory. It powers deterministic dedupe: two
//!   memories with the same normalized content share a fingerprint regardless of
//!   when or how they were recorded.
//!
//! * [`digest_hex`] produces a 256-bit digest over arbitrary bytes. It backs the
//!   integrity chain in the append-only log, where each record commits to the
//!   digest of the record before it (a hash chain / tamper-evident ledger).
//!
//! The 256-bit digest is a custom sponge-style construction built on a 64-bit
//! mixing permutation. It is *not* SHA-256 and makes no cryptographic security
//! claims; it is a strong, well-distributed checksum suitable for detecting
//! accidental corruption and casual tampering in a local-first file. This
//! limitation is documented honestly in `docs/MEMORY.md`.

/// FNV-1a 64-bit hash. Deterministic and fast; used for content fingerprints.
pub fn fnv1a_64(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x00000100000001b3;
    let mut hash = OFFSET;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

/// Produce a 16-character hex fingerprint from normalized content.
///
/// Normalization: trim, collapse internal whitespace runs to a single space,
/// and lowercase. This means "Loves  DARK\tMode" and "loves dark mode" collapse
/// to the same fingerprint, which is exactly the deterministic dedupe behavior
/// we want for near-identical restatements. It is purely lexical: it will *not*
/// detect paraphrases like "prefers dark theme" (see honest limitations).
pub fn content_fingerprint(content: &str) -> String {
    let normalized = normalize_content(content);
    format!("{:016x}", fnv1a_64(normalized.as_bytes()))
}

/// The normalization applied before fingerprinting. Exposed for tests and for
/// the conflict detector, which reasons about normalized content too.
pub fn normalize_content(content: &str) -> String {
    let mut out = String::new();
    let mut prev_space = false;
    for c in content.trim().chars() {
        if c.is_whitespace() {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            for lc in c.to_lowercase() {
                out.push(lc);
            }
            prev_space = false;
        }
    }
    out
}

/// A 256-bit digest represented as four 64-bit lanes.
struct Digest256 {
    state: [u64; 4],
}

impl Digest256 {
    fn new() -> Self {
        // Distinct, high-entropy initial constants for each lane (fractional
        // bits of square roots, a common way to pick "nothing up my sleeve"
        // numbers).
        Digest256 {
            state: [
                0x6a09e667f3bcc908,
                0xbb67ae8584caa73b,
                0x3c6ef372fe94f82b,
                0xa54ff53a5f1d36f1,
            ],
        }
    }

    fn absorb(&mut self, bytes: &[u8]) {
        let mut counter: u64 = 0;
        for &b in bytes {
            let lane = (counter % 4) as usize;
            self.state[lane] ^= (b as u64).wrapping_add(counter);
            self.state[lane] = mix64(self.state[lane]);
            // Cross-diffuse into the next lane so byte position matters.
            let next = ((counter + 1) % 4) as usize;
            self.state[next] = self.state[next].rotate_left(17) ^ self.state[lane];
            counter = counter.wrapping_add(1);
        }
        // Commit the total length into the state to resist length-extension
        // style collisions between inputs that share a prefix.
        self.state[0] ^= mix64(bytes.len() as u64);
    }

    fn finalize(mut self) -> [u64; 4] {
        // A few finalization rounds to diffuse all lanes into each other.
        for _ in 0..6 {
            self.state[0] = mix64(self.state[0] ^ self.state[3].rotate_left(7));
            self.state[1] = mix64(self.state[1] ^ self.state[0].rotate_left(19));
            self.state[2] = mix64(self.state[2] ^ self.state[1].rotate_left(31));
            self.state[3] = mix64(self.state[3] ^ self.state[2].rotate_left(43));
        }
        self.state
    }
}

/// SplitMix64-style mixing permutation. Bijective over u64 for its core, used
/// here as a strong avalanche mixer.
fn mix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e3779b97f4a7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
    x ^ (x >> 31)
}

/// Compute a 64-hex-character (256-bit) digest of the given bytes.
pub fn digest_hex(bytes: &[u8]) -> String {
    let mut d = Digest256::new();
    d.absorb(bytes);
    let lanes = d.finalize();
    let mut out = String::with_capacity(64);
    for lane in lanes {
        out.push_str(&format!("{:016x}", lane));
    }
    out
}

/// Chain a previous digest with new record bytes to produce the next digest.
/// This is the per-record link in the append-only log's hash chain.
pub fn chain_digest(prev_hex: &str, record_bytes: &[u8]) -> String {
    let mut buf = Vec::with_capacity(prev_hex.len() + 1 + record_bytes.len());
    buf.extend_from_slice(prev_hex.as_bytes());
    buf.push(b'\n');
    buf.extend_from_slice(record_bytes);
    digest_hex(&buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_normalizes_whitespace_and_case() {
        assert_eq!(
            content_fingerprint("Loves  DARK\tmode"),
            content_fingerprint("loves dark mode")
        );
    }

    #[test]
    fn fingerprint_differs_for_different_content() {
        assert_ne!(
            content_fingerprint("loves dark mode"),
            content_fingerprint("loves light mode")
        );
    }

    #[test]
    fn digest_is_stable_and_full_width() {
        let a = digest_hex(b"hello world");
        let b = digest_hex(b"hello world");
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn digest_is_sensitive_to_small_changes() {
        assert_ne!(digest_hex(b"hello world"), digest_hex(b"hello worle"));
        assert_ne!(digest_hex(b"ab"), digest_hex(b"ba"));
    }

    #[test]
    fn chain_depends_on_previous() {
        let x = chain_digest("00", b"record");
        let y = chain_digest("01", b"record");
        assert_ne!(x, y);
    }
}
