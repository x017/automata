//! Just enough DER (ASN.1) for RSA keys and a self-signed `X.509`
//! certificate: encoding them, and reading the private key files
//! (PKCS #1 and PKCS #8) developers already have.

/// A tag-length-value with `content`.
#[must_use]
pub fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    let len = content.len();
    if len < 0x80 {
        out.push(super::low_byte(len));
    } else {
        let bytes: Vec<u8> = len
            .to_be_bytes()
            .into_iter()
            .skip_while(|b| *b == 0)
            .collect();
        out.push(0x80 | super::low_byte(bytes.len()));
        out.extend(bytes);
    }
    out.extend_from_slice(content);
    out
}

/// A SEQUENCE of already encoded `parts`.
#[must_use]
pub fn sequence(parts: &[&[u8]]) -> Vec<u8> {
    tlv(0x30, &parts.concat())
}

/// A SET of one already encoded value.
#[must_use]
pub fn set(part: &[u8]) -> Vec<u8> {
    tlv(0x31, part)
}

/// A non-negative INTEGER from big-endian bytes.
#[must_use]
pub fn unsigned(be: &[u8]) -> Vec<u8> {
    let trimmed: Vec<u8> = be.iter().copied().skip_while(|b| *b == 0).collect();
    let mut content = Vec::with_capacity(trimmed.len() + 1);
    if trimmed.first().is_none_or(|b| b & 0x80 != 0) {
        content.push(0);
    }
    content.extend(trimmed);
    tlv(0x02, &content)
}

/// A BIT STRING holding whole bytes.
#[must_use]
pub fn bit_string(bytes: &[u8]) -> Vec<u8> {
    let mut content = vec![0];
    content.extend_from_slice(bytes);
    tlv(0x03, &content)
}

/// NULL.
pub const NULL: [u8; 2] = [0x05, 0x00];

/// The OID 1.2.840.113549.1.1.1 (rsaEncryption).
pub const RSA_ENCRYPTION: [u8; 11] = [
    0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01,
];

/// The OID 1.2.840.113549.1.1.11 (sha256WithRSAEncryption).
pub const SHA256_WITH_RSA: [u8; 11] = [
    0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0b,
];

/// The OID 2.5.4.3 (common name).
pub const COMMON_NAME: [u8; 5] = [0x06, 0x03, 0x55, 0x04, 0x03];

/// The OID 2.5.4.10 (organization).
pub const ORGANIZATION: [u8; 5] = [0x06, 0x03, 0x55, 0x04, 0x0a];

/// A `UTF8String`.
#[must_use]
pub fn utf8(s: &str) -> Vec<u8> {
    tlv(0x0c, s.as_bytes())
}

/// An `X.509` name with a common name and an organization (left out
/// when empty).
#[must_use]
pub fn name(common: &str, organization: &str) -> Vec<u8> {
    let cn = set(&sequence(&[&COMMON_NAME, &utf8(common)]));
    if organization.is_empty() {
        return sequence(&[&cn]);
    }
    let o = set(&sequence(&[&ORGANIZATION, &utf8(organization)]));
    sequence(&[&cn, &o])
}

/// An OCTET STRING.
#[must_use]
pub fn octets(bytes: &[u8]) -> Vec<u8> {
    tlv(0x04, bytes)
}

/// Reads DER values one after another.
pub struct Reader<'a> {
    data: &'a [u8],
}

impl<'a> Reader<'a> {
    /// A reader over `data`.
    #[must_use]
    pub fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    /// Whether everything has been read.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// The next value's tag and content.
    ///
    /// # Errors
    ///
    /// When the data ends early or uses a length form DER doesn't allow.
    pub fn read(&mut self) -> Result<(u8, &'a [u8]), String> {
        let bad = || "not a valid DER key file".to_string();
        let (&tag, rest) = self.data.split_first().ok_or_else(bad)?;
        let (&first, mut rest) = rest.split_first().ok_or_else(bad)?;
        let len = if first < 0x80 {
            usize::from(first)
        } else {
            let count = usize::from(first & 0x7f);
            if count == 0 || count > 4 || rest.len() < count {
                return Err(bad());
            }
            let (bytes, after) = rest.split_at(count);
            rest = after;
            bytes.iter().fold(0usize, |n, b| (n << 8) | usize::from(*b))
        };
        if rest.len() < len {
            return Err(bad());
        }
        let (content, after) = rest.split_at(len);
        self.data = after;
        Ok((tag, content))
    }

    /// The next value, which must have `tag`.
    ///
    /// # Errors
    ///
    /// As [`read`](Self::read), or when the tag differs.
    pub fn expect(&mut self, tag: u8) -> Result<&'a [u8], String> {
        match self.read()? {
            (t, content) if t == tag => Ok(content),
            (t, _) => Err(format!("unexpected DER tag {t:#04x} (wanted {tag:#04x})")),
        }
    }

    /// The next INTEGER as big-endian bytes, without a leading zero.
    ///
    /// # Errors
    ///
    /// As [`expect`](Self::expect).
    pub fn integer(&mut self) -> Result<&'a [u8], String> {
        let bytes = self.expect(0x02)?;
        Ok(bytes.strip_prefix(&[0]).unwrap_or(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_and_integers() {
        assert_eq!(tlv(0x04, &[1, 2]), vec![0x04, 2, 1, 2]);
        let long = tlv(0x04, &[0; 300]);
        assert_eq!(&long[..4], &[0x04, 0x82, 0x01, 0x2c]);
        assert_eq!(unsigned(&[0, 0, 0x7f]), vec![0x02, 1, 0x7f]);
        assert_eq!(unsigned(&[0x80]), vec![0x02, 2, 0, 0x80]);
        assert_eq!(unsigned(&[0]), vec![0x02, 1, 0]);
    }

    #[test]
    fn values_read_back() {
        let long = vec![7u8; 300];
        let data = sequence(&[&unsigned(&[0x80, 1]), &octets(&long), &NULL]);
        let mut outer = Reader::new(&data);
        let mut inner = Reader::new(outer.expect(0x30).unwrap());
        assert!(outer.is_empty());
        assert_eq!(inner.integer().unwrap(), &[0x80, 1]);
        assert_eq!(inner.expect(0x04).unwrap(), &long[..]);
        assert!(inner.integer().is_err());
        assert!(Reader::new(&[0x30, 0x05, 1]).read().is_err());
        assert!(Reader::new(&[0x30, 0x80]).read().is_err());
    }
}
