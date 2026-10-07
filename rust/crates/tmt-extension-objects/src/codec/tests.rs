//! Admission is exercised on its own: every refusal is paired with a twin that
//! differs by one token and is accepted, and each guard has a control showing a
//! plain `serde_json` parse would have let the refused body through.
use super::*;

/// One refused body, the accepted body it differs from by one token, and the class.
struct Case {
    name: &'static str,
    accepted: &'static str,
    refused: &'static str,
    class: ErrorClass,
}
const fn case(
    name: &'static str,
    accepted: &'static str,
    refused: &'static str,
    class: ErrorClass,
) -> Case {
    Case {
        name,
        accepted,
        refused,
        class,
    }
}

fn check(cases: &[Case]) {
    for case in cases {
        assert!(
            strict_value(case.accepted.as_bytes()).is_ok(),
            "twin of {} must be accepted",
            case.name
        );
        assert_eq!(
            strict_value(case.refused.as_bytes()).map(|_| ()),
            Err(case.class),
            "{}",
            case.name
        );
    }
}
/// The refused body parses as plain JSON, so only the admission guard stops it.
fn plainly_parses(cases: &[Case]) {
    for case in cases {
        assert!(
            serde_json::from_str::<Value>(case.refused).is_ok(),
            "{} is stopped by the guard, not by the parser",
            case.name
        );
    }
}

#[test]
fn length_prefix_is_bounded_before_any_body_is_read() {
    for (prefix, length) in [
        ([0, 0, 0, 2], 2),
        ([0, 0, 0, 0xff], 255),
        ([0, 0, 0xff, 0xff], 65_535),
        ([0, 1, 0, 0], FRAME_BYTES),
    ] {
        assert_eq!(decode_length(prefix), Ok(length));
    }
    for prefix in [
        [0, 0, 0, 0],
        [0, 0, 0, 1],
        [0, 1, 0, 1],
        [0, 2, 0, 0],
        [0x7f, 0xff, 0xff, 0xff],
        [0xff; 4],
    ] {
        assert_eq!(decode_length(prefix), Err(ErrorClass::Length), "{prefix:?}");
    }
}

const DUPLICATES: &[Case] = &[
    case(
        "top level",
        r#"{"kind":"a","generation":"g"}"#,
        r#"{"kind":"a","kind":"b"}"#,
        ErrorClass::Duplicate,
    ),
    case(
        "same value",
        r#"{"a":1,"b":1}"#,
        r#"{"a":1,"a":1}"#,
        ErrorClass::Duplicate,
    ),
    case(
        "escaped equivalent",
        r#"{"kind":"a","type":"b"}"#,
        r#"{"kind":"a","\u006bind":"b"}"#,
        ErrorClass::Duplicate,
    ),
    case(
        "escaped at both ends",
        r#"{"kind":1,"other":2}"#,
        r#"{"\u006bind":1,"k\u0069nd":2}"#,
        ErrorClass::Duplicate,
    ),
    case(
        "nested",
        r#"{"input":{"a":1,"b":2}}"#,
        r#"{"input":{"a":1,"a":2}}"#,
        ErrorClass::Duplicate,
    ),
    case(
        "nested escaped",
        r#"{"input":{"namespace":1,"other":2}}"#,
        r#"{"input":{"namespace":1,"namespac\u0065":2}}"#,
        ErrorClass::Duplicate,
    ),
    case(
        "inside an array element",
        r#"{"x":[{"a":1,"b":2}]}"#,
        r#"{"x":[{"a":1,"a":2}]}"#,
        ErrorClass::Duplicate,
    ),
    case(
        "surrogate pair spelling",
        "{\"\u{1f600}\":1,\"x\":2}",
        "{\"\u{1f600}\":1,\"\\ud83d\\ude00\":2}",
        ErrorClass::Duplicate,
    ),
    case(
        "empty name",
        r#"{"":1,"a":2}"#,
        r#"{"":1,"":2}"#,
        ErrorClass::Duplicate,
    ),
];

#[test]
fn duplicate_members_are_refused_after_decoding_escapes() {
    check(DUPLICATES);
    plainly_parses(DUPLICATES);
}

const NUMBERS: &[Case] = &[
    case(
        "negative zero",
        r#"{"n":0}"#,
        r#"{"n":-0}"#,
        ErrorClass::Shape,
    ),
    case("negative", r#"{"n":1}"#, r#"{"n":-1}"#, ErrorClass::Shape),
    case("fraction", r#"{"n":1}"#, r#"{"n":1.5}"#, ErrorClass::Shape),
    case(
        "integral fraction",
        r#"{"n":1}"#,
        r#"{"n":1.0}"#,
        ErrorClass::Shape,
    ),
    case(
        "exponent",
        r#"{"n":1000}"#,
        r#"{"n":1e3}"#,
        ErrorClass::Shape,
    ),
    case(
        "capital exponent",
        r#"{"n":1000}"#,
        r#"{"n":1E3}"#,
        ErrorClass::Shape,
    ),
    case(
        "signed exponent",
        r#"{"n":0}"#,
        r#"{"n":1e+0}"#,
        ErrorClass::Shape,
    ),
    case(
        "inside an array",
        r#"{"n":[1,0]}"#,
        r#"{"n":[1,-0]}"#,
        ErrorClass::Shape,
    ),
    case("top level", "0", "-0", ErrorClass::Shape),
    case(
        "one past the safe range",
        r#"{"n":9007199254740991}"#,
        r#"{"n":9007199254740992}"#,
        ErrorClass::Value,
    ),
    case(
        "u64 overflow",
        r#"{"n":18446744073709}"#,
        r#"{"n":18446744073709551616}"#,
        ErrorClass::Value,
    ),
    case(
        "beyond 128 bits",
        r#"{"n":0}"#,
        r#"{"n":340282366920938463463374607431768211456}"#,
        ErrorClass::Value,
    ),
];

#[test]
fn numeric_tokens_must_be_canonical_unsigned_integers() {
    check(NUMBERS);
    plainly_parses(NUMBERS);
}

#[test]
fn a_value_based_check_would_not_see_negative_zero() {
    // The parsed number is an ordinary zero, so only the original token tells it apart.
    let parsed = serde_json::from_str::<Value>("-0").unwrap();
    assert_eq!(parsed.as_i64(), Some(0));
    assert_eq!(strict_value(b"-0").map(|_| ()), Err(ErrorClass::Shape));
}

#[test]
fn a_leading_zero_is_a_syntax_error_not_a_value_error() {
    check(&[
        case("member", r#"{"n":1}"#, r#"{"n":01}"#, ErrorClass::Syntax),
        case("zero zero", r#"{"n":0}"#, r#"{"n":00}"#, ErrorClass::Syntax),
        case(
            "array element",
            r#"{"n":[7]}"#,
            r#"{"n":[07]}"#,
            ErrorClass::Syntax,
        ),
    ]);
}

#[test]
fn text_that_looks_like_a_number_inside_a_string_is_untouched() {
    let body =
        r#"{"a":"-0","b":"1e3","c":"18446744073709551616","d":"1.5","\u0065":"9007199254740992"}"#;
    let value = strict_value(body.as_bytes()).unwrap();
    assert_eq!(value["a"], "-0");
    assert_eq!(value["c"], "18446744073709551616");
    assert_eq!(value["e"], "9007199254740992");
}

#[test]
fn serde_private_markers_are_never_member_names() {
    check(&[
        case(
            "number marker",
            r#"{"$serde_json::number":"1"}"#,
            r#"{"$serde_json::private::Number":"1"}"#,
            ErrorClass::Shape,
        ),
        case(
            "raw value marker",
            r#"{"a":{"$serde_json::raw":1}}"#,
            r#"{"a":{"$serde_json::private::RawValue":1}}"#,
            ErrorClass::Shape,
        ),
        case(
            "escaped marker",
            r#"{"a":{"b":1}}"#,
            r#"{"a":{"\u0024serde_json::private::Number":"1"}}"#,
            ErrorClass::Shape,
        ),
    ]);
}

fn nested(levels: usize, open: &str, close: &str, leaf: &str) -> String {
    format!("{}{leaf}{}", open.repeat(levels), close.repeat(levels))
}

#[test]
fn nesting_is_bounded_in_objects_and_arrays() {
    let object = |levels| nested(levels, r#"{"a":"#, "}", "1");
    let array = |levels| nested(levels, "[", "]", "1");
    for levels in [1, JSON_DEPTH] {
        assert!(
            strict_value(object(levels).as_bytes()).is_ok(),
            "{levels} objects"
        );
        assert!(
            strict_value(array(levels).as_bytes()).is_ok(),
            "{levels} arrays"
        );
    }
    assert_eq!(
        strict_value(object(JSON_DEPTH + 1).as_bytes()).map(|_| ()),
        Err(ErrorClass::Shape)
    );
    assert_eq!(
        strict_value(array(JSON_DEPTH + 1).as_bytes()).map(|_| ()),
        Err(ErrorClass::Shape)
    );
    // The bound is checked before any deeper walk, so a body nested as deeply as
    // the frame size allows is refused without recursing into it.
    let deepest = array(FRAME_BYTES / 2 - 1);
    assert_eq!(
        strict_value(deepest.as_bytes()).map(|_| ()),
        Err(ErrorClass::Shape)
    );
    // The guard, not the parser, refuses a body one level past the bound.
    assert!(serde_json::from_str::<Value>(&array(JSON_DEPTH + 1)).is_ok());
    assert!(serde_json::from_str::<Value>(&object(JSON_DEPTH + 1)).is_ok());
}

#[test]
fn bytes_must_be_one_complete_utf8_json_value() {
    check(&[
        case("unterminated", "{}", "{", ErrorClass::Syntax),
        case(
            "missing value",
            r#"{"a":1}"#,
            r#"{"a":}"#,
            ErrorClass::Syntax,
        ),
        case("trailing comma", "[1]", "[1,]", ErrorClass::Syntax),
        case(
            "not a number",
            r#"{"a":1}"#,
            r#"{"a":NaN}"#,
            ErrorClass::Syntax,
        ),
        case("empty", "{}", "", ErrorClass::Syntax),
        case(
            "raw newline in a string",
            r#"{"a":"\n"}"#,
            "{\"a\":\"\n\"}",
            ErrorClass::Syntax,
        ),
        case("bare word", "null", "nul", ErrorClass::Syntax),
        case("second value", "{}", "{}{}", ErrorClass::Trailing),
        case("trailing text", "{}", "{} x", ErrorClass::Trailing),
        case(
            "trailing comma after the value",
            "{}",
            "{},",
            ErrorClass::Trailing,
        ),
    ]);
    // Insignificant whitespace around one value is not trailing data.
    for body in [" {}", "{} ", "\n{ \"a\" : 1 }\n"] {
        assert!(strict_value(body.as_bytes()).is_ok(), "{body:?}");
    }
    for (label, bytes) in [
        ("lone continuation byte", &b"{\"a\":\"\xff\"}"[..]),
        ("overlong slash", &b"\xc0\xaf"[..]),
        ("truncated sequence", &b"{\"a\":\"\xe2\x82\"}"[..]),
    ] {
        assert_eq!(
            strict_value(bytes).map(|_| ()),
            Err(ErrorClass::Utf8),
            "{label}"
        );
    }
    assert!(strict_value("{\"a\":\"\u{ff}\"}".as_bytes()).is_ok());
}

#[test]
fn body_size_is_bounded_and_the_bound_itself_is_accepted() {
    let padded = |total: usize| {
        let filler = total - r#"{"a":""}"#.len();
        format!(r#"{{"a":"{}"}}"#, "x".repeat(filler))
    };
    let exact = padded(FRAME_BYTES);
    assert_eq!(exact.len(), FRAME_BYTES);
    assert!(strict_value(exact.as_bytes()).is_ok());
    let over = padded(FRAME_BYTES + 1);
    assert_eq!(
        strict_value(over.as_bytes()).map(|_| ()),
        Err(ErrorClass::Length)
    );
    assert_eq!(
        strict_value(&vec![b' '; FRAME_BYTES + 1]).map(|_| ()),
        Err(ErrorClass::Length)
    );
}

#[test]
fn an_admitted_body_returns_its_parsed_value() {
    let value = strict_value(br#"{"a":[1,"x",true,null],"b":{"c":"\u00e9"}}"#).unwrap();
    assert_eq!(value["a"][0], 1);
    assert_eq!(value["a"][1], "x");
    assert_eq!(value["a"][2], true);
    assert!(value["a"][3].is_null());
    assert_eq!(value["b"]["c"], "\u{e9}");
}
