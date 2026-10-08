//! Each identifier accepts one spelling; every refusal below is one change away
//! from a value the same test shows is accepted.
use super::*;

const UUID: &str = "7f3c1a52-9d4e-4b86-8a21-5c0e9b7d3f14";
/// 32 bytes of 0x01 and 0x02, spelled by hand from the base64url alphabet.
const ONES: &str = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE";
const TWOS: &str = "AgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgI";
/// SHA-256 of the three bytes `abc`.
const DIGEST: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

/// `base` with the character at `index` replaced by `with`.
fn changed(base: &str, index: usize, with: &str) -> String {
    let mut text = base.to_owned();
    text.replace_range(index..=index, with);
    text
}

#[test]
fn uuid_accepts_one_lowercase_v4_spelling() {
    for variant in ['8', '9', 'a', 'b'] {
        let text = changed(UUID, 19, &variant.to_string());
        let id = Uuid4::parse(&text).unwrap();
        assert_eq!(id.to_string(), text);
    }
    let id = Uuid4::parse(UUID).unwrap();
    assert_eq!(id.as_bytes()[0], 0x7f);
    assert_eq!(id.as_bytes()[15], 0x14);
}

#[test]
fn uuid_refuses_every_other_spelling() {
    let refused = [
        ("uppercase hex", changed(UUID, 0, "F")),
        ("version 1", changed(UUID, 14, "1")),
        ("version 5", changed(UUID, 14, "5")),
        ("variant c", changed(UUID, 19, "c")),
        ("variant 7", changed(UUID, 19, "7")),
        ("not hex", changed(UUID, 35, "g")),
        ("hyphen in a digit place", changed(UUID, 5, "-")),
        ("digit in a hyphen place", changed(UUID, 8, "0")),
        ("no hyphens", UUID.replace('-', "")),
        ("braced", format!("{{{UUID}}}")),
        ("trailing space", format!("{UUID} ")),
        ("short", UUID[..35].to_owned()),
        ("long", format!("{UUID}0")),
        ("two-byte character", changed(UUID, 1, "é")),
        ("empty", String::new()),
    ];
    assert!(Uuid4::parse(UUID).is_ok());
    for (label, text) in refused {
        assert_eq!(Uuid4::parse(&text), Err(ErrorClass::Value), "{label}");
    }
}

#[test]
fn counter_spans_the_u64_range_as_a_canonical_string() {
    for text in [
        "1",
        "9",
        "10",
        "9007199254740991",
        "9007199254740992",
        "9007199254740993",
        "18446744073709551615",
    ] {
        let counter = Counter::parse(text).unwrap();
        assert_eq!(counter.to_string(), text);
        assert_eq!(counter.get().to_string(), text);
    }
    assert_eq!(
        Counter::parse("18446744073709551615").unwrap().get(),
        u64::MAX
    );
    assert_eq!(Counter::new(1).unwrap().to_string(), "1");
    assert!(Counter::parse("2").unwrap() > Counter::parse("1").unwrap());
}

#[test]
fn counter_refuses_zero_other_spellings_and_overflow() {
    for (label, text) in [
        ("empty", ""),
        ("zero", "0"),
        ("leading zero", "01"),
        ("plus sign", "+1"),
        ("minus sign", "-1"),
        ("leading space", " 1"),
        ("trailing space", "1 "),
        ("fraction", "1.0"),
        ("exponent", "1e3"),
        ("hex", "0x1"),
        ("full-width digit", "１"),
        ("u64 max plus one", "18446744073709551616"),
        ("twenty nines", "99999999999999999999"),
        ("twenty-one digits", "123456789012345678901"),
    ] {
        assert_eq!(Counter::parse(text), Err(ErrorClass::Value), "{label}");
    }
    assert_eq!(Counter::new(0), Err(ErrorClass::Value));
}

#[test]
fn bytes32_roundtrips_fixed_width_base64url() {
    assert_eq!(Bytes32::from_bytes([1; 32]).to_string(), ONES);
    assert_eq!(Bytes32::from_bytes([2; 32]).to_string(), TWOS);
    assert_eq!(Bytes32::parse(ONES).unwrap().as_bytes(), &[1; 32]);
    assert_eq!(Bytes32::parse(TWOS).unwrap().as_bytes(), &[2; 32]);
    // 0xfb and 0xff exercise the two non-alphanumeric URL-safe characters.
    let edge = Bytes32::from_bytes([0xfb; 32]).to_string();
    assert!(edge.contains('-') && Bytes32::parse(&edge).is_ok());
    let edge = Bytes32::from_bytes([0xff; 32]).to_string();
    assert!(edge.contains('_') && Bytes32::parse(&edge).is_ok());
    let mut counted = [0u8; 32];
    for (index, byte) in counted.iter_mut().enumerate() {
        *byte = index as u8;
    }
    let text = Bytes32::from_bytes(counted).to_string();
    assert_eq!(Bytes32::parse(&text).unwrap().as_bytes(), &counted);
}

#[test]
fn bytes32_refuses_every_other_spelling() {
    let standard = Bytes32::from_bytes([0xfb; 32])
        .to_string()
        .replace('-', "+");
    let refused = [
        ("empty", String::new()),
        ("one character short", ONES[..42].to_owned()),
        ("one character long", format!("{ONES}A")),
        ("padded", format!("{}=", &ONES[..43])),
        ("standard alphabet", standard),
        ("whitespace", changed(ONES, 3, " ")),
        // The last character carries two unused bits; `F` sets one of them.
        ("nonzero trailing bits", changed(ONES, 42, "F")),
        ("two-byte character", changed(ONES, 0, "é")),
        (
            "thirty-one bytes",
            Bytes32::from_bytes([1; 32]).to_string()[..41].to_owned(),
        ),
    ];
    assert!(Bytes32::parse(ONES).is_ok());
    for (label, text) in refused {
        assert_eq!(Bytes32::parse(&text), Err(ErrorClass::Value), "{label}");
    }
}

#[test]
fn digest_is_sixty_four_lowercase_hex_digits() {
    let digest = Sha256Hex::parse(DIGEST).unwrap();
    assert_eq!(digest.to_string(), DIGEST);
    assert_eq!(digest.as_bytes()[0], 0xba);
    assert_eq!(digest.as_bytes()[31], 0xad);
    assert_eq!(Sha256Hex::from_bytes([0; 32]).to_string(), "0".repeat(64));
    let refused = [
        ("uppercase", DIGEST.to_uppercase()),
        ("mixed case", changed(DIGEST, 0, "B")),
        ("not hex", changed(DIGEST, 10, "g")),
        ("whitespace", changed(DIGEST, 10, " ")),
        ("short", DIGEST[..63].to_owned()),
        ("long", format!("{DIGEST}0")),
        ("prefixed", format!("0x{}", &DIGEST[2..])),
        ("two-byte character", changed(DIGEST, 0, "é")),
        ("empty", String::new()),
    ];
    for (label, text) in refused {
        assert_eq!(Sha256Hex::parse(&text), Err(ErrorClass::Value), "{label}");
    }
}
