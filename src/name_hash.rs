//! The normative Candid name hash.
//!
//! Candid derives the 32-bit ID of a record/variant field label and of a
//! service method from the name's UTF-8 bytes. The function is fixed by the
//! Candid specification — it is the polymorphic-variant hash of Garrigue's
//! ML'98 paper, `h(name) = fold(\s b -> s * 223 + b) 0 name`, evaluated modulo
//! 2^32 — so it is part of this crate's canonical byte layer, not an
//! implementation detail borrowed from a particular parser.
//!
//! It lives here, in the base feature set, because Contract validation must be
//! able to check every `ServiceMethod.id` without linking a Candid *source*
//! engine. The implementation is eight lines and cannot drift silently: unit
//! tests below and `tests/candid_name_hash.rs` pin it against
//! `candid_parser::candid::idl_hash`, which is a dev-dependency and therefore
//! available for comparison in every feature configuration, including
//! `--no-default-features`.

/// Hash a Candid field label or service method name into its Candid ID.
///
/// Multiplication and addition wrap, which is the specified arithmetic: the
/// hash is defined modulo 2^32, so wrapping is the correct answer rather than
/// an overflow to be avoided.
pub(crate) fn candid_name_hash(name: &str) -> u32 {
    let mut hash: u32 = 0;
    for byte in name.as_bytes() {
        hash = hash.wrapping_mul(223).wrapping_add(u32::from(*byte));
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every assertion here also holds for `candid_parser::candid::idl_hash`;
    /// `parity_with_the_reference_implementation` proves it for the same
    /// inputs. The literals are pinned separately so a silent change to *both*
    /// implementations would still fail.
    #[test]
    fn known_hashes_are_stable() {
        assert_eq!(candid_name_hash(""), 0);
        assert_eq!(candid_name_hash("a"), 97);
        assert_eq!(candid_name_hash("ok"), 24_860);
        // 111 * 223 + 107 = 24_860; a two-byte name is the smallest case where
        // the fold's ordering is observable.
        assert_ne!(candid_name_hash("ok"), candid_name_hash("ko"));
    }

    #[test]
    fn documented_collision_pairs_agree() {
        // The pair `tests/canonical_properties.rs` uses to prove that field IDs,
        // not spellings, are the semantic key.
        assert_eq!(candid_name_hash("jhwlzguu"), candid_name_hash("jsyrjsvk"));
    }

    #[test]
    fn hashing_is_over_utf8_bytes_not_chars() {
        // A non-ASCII name must fold its UTF-8 encoding byte by byte.
        let expected = "é"
            .as_bytes()
            .iter()
            .fold(0u32, |hash, byte| hash.wrapping_mul(223) + u32::from(*byte));
        assert_eq!(candid_name_hash("é"), expected);
    }

    #[test]
    fn long_names_wrap_instead_of_overflowing() {
        // 4096 bytes is far past the point where the accumulator wraps; the
        // call must simply return, not panic in a debug build.
        let long = "z".repeat(4096);
        let _ = candid_name_hash(&long);
    }

    #[test]
    fn parity_with_the_reference_implementation() {
        for name in [
            "",
            "a",
            "ok",
            "err",
            "jhwlzguu",
            "jsyrjsvk",
            "hyphen-name",
            "_1_",
            "0",
            "0.did",
            "méthode",
            "日本語",
            "\u{1f600}",
            "\u{0}",
            "\u{7f}\u{80}\u{81}",
            "a\"b",
            "A_very_long_method_name_that_keeps_going_and_going_and_going_0123456789",
        ] {
            assert_eq!(
                candid_name_hash(name),
                candid_parser::candid::idl_hash(name),
                "hash parity for {name:?}"
            );
        }

        // Every single byte value, as a one-byte and as a two-byte name, plus a
        // length sweep that crosses the point where the accumulator wraps.
        for byte in 0u8..=0xff {
            let name = String::from_utf8_lossy(&[byte]).into_owned();
            assert_eq!(
                candid_name_hash(&name),
                candid_parser::candid::idl_hash(&name)
            );
            let pair = format!("{name}{name}");
            assert_eq!(
                candid_name_hash(&pair),
                candid_parser::candid::idl_hash(&pair)
            );
        }
        for length in [1usize, 2, 3, 4, 5, 6, 7, 8, 9, 64, 255, 256, 1024, 4096] {
            let name = "\u{e9}z".repeat(length);
            assert_eq!(
                candid_name_hash(&name),
                candid_parser::candid::idl_hash(&name),
                "hash parity at length {length}"
            );
        }
    }
}
