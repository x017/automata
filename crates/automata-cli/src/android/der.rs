//! Just enough DER (ASN.1) encoding for an RSA public key and a
//! self-signed X.509 certificate.

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

/// An `X.509` name with a common name and an organization.
#[must_use]
pub fn name(common: &str, organization: &str) -> Vec<u8> {
    let cn = set(&sequence(&[&COMMON_NAME, &utf8(common)]));
    let o = set(&sequence(&[&ORGANIZATION, &utf8(organization)]));
    sequence(&[&cn, &o])
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
}
