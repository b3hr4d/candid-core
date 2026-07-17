use std::io::{self, Read};

#[derive(Debug)]
pub(crate) enum BoundedUtf8Error {
    Io(io::Error),
    LimitExceeded { observed: usize },
    InvalidUtf8(std::string::FromUtf8Error),
}

pub(crate) fn read_bounded_utf8(
    reader: impl Read,
    limit: usize,
) -> Result<String, BoundedUtf8Error> {
    let read_limit = u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1);
    let mut bytes = Vec::new();
    reader
        .take(read_limit)
        .read_to_end(&mut bytes)
        .map_err(BoundedUtf8Error::Io)?;

    if bytes.len() > limit {
        return Err(BoundedUtf8Error::LimitExceeded {
            observed: bytes.len(),
        });
    }

    String::from_utf8(bytes).map_err(BoundedUtf8Error::InvalidUtf8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn accepts_exact_limit_and_rejects_one_byte_over() {
        assert_eq!(read_bounded_utf8(Cursor::new(b"abcd"), 4).unwrap(), "abcd");
        assert!(matches!(
            read_bounded_utf8(Cursor::new(b"abcde"), 4),
            Err(BoundedUtf8Error::LimitExceeded { observed: 5 })
        ));
    }

    #[test]
    fn distinguishes_invalid_utf8_within_the_limit() {
        assert!(matches!(
            read_bounded_utf8(Cursor::new([0xff]), 1),
            Err(BoundedUtf8Error::InvalidUtf8(_))
        ));
    }
}
