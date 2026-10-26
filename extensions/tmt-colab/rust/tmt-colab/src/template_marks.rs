//! The two template marks of a page's content metadata (#5066): `meta.template` makes the page a
//! template and `meta.fromTemplate` records what a page was created from. Pure values: no Yjs,
//! storage, lookup of other pages or authority. A fold of one page cannot know the others, so a
//! name's uniqueness in the space is the owner's check when the mark is set (`templates`), never a
//! rule of the value.
use crate::closed::{Result, keys, plain, require, text};
use serde_json::{Value, json};

/// The compiled templates, in the order `template ls` lists them. Their names are reserved: no
/// page can be marked with one, though a page can be created from one.
pub const BUILT_IN: [&str; 6] = ["starter", "report", "review", "compare", "board", "gallery"];
/// The longest template name.
pub const NAME_CHARS: usize = 40;
/// The longest description, in UTF-8 bytes.
pub const DESCRIPTION_BYTES: usize = 200;
/// A page revision token starts with its version and carries a SHA-256 digest as lowercase hex.
const REVISION_PREFIX: &str = "v1:";
const REVISION_HEX: usize = 64;

/// A template name: 1 to 40 characters of `a-z`, `0-9` and `-`.
pub fn valid_name(name: &str) -> bool {
    (1..=NAME_CHARS).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

pub fn is_built_in(name: &str) -> bool {
    BUILT_IN.contains(&name)
}

/// A description: one trimmed line of at most 200 bytes with no control characters; it may be empty.
/// Trimmed as the app trims (`String.prototype.trim`): white space and the byte order mark.
pub fn valid_description(description: &str) -> bool {
    description.len() <= DESCRIPTION_BYTES
        && description.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}') == description
        && !description.chars().any(char::is_control)
}

/// A template revision (`revision_of`): `v1:` and 64 lowercase hex digits.
pub fn valid_revision(revision: &str) -> bool {
    revision.strip_prefix(REVISION_PREFIX).is_some_and(|hex| {
        hex.len() == REVISION_HEX && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    })
}

/// The revision a page created from a user template records: `v1:` and the lowercase hex SHA-256 of
/// the template's source as copied. A page's own revision token moves with every change anywhere in
/// the space, so it cannot say whether a template changed; the digest of the copied content does,
/// and `template show` prints the template's current one to compare.
pub fn revision_of(source: &str) -> String {
    let digest = tmt_colab_model::crypto::digest(source.as_bytes());
    let mut revision = String::from(REVISION_PREFIX);
    for byte in digest {
        revision.push_str(&format!("{byte:02x}"));
    }
    revision
}

/// `meta.template`: exactly `{name, description}`, the name not a built-in one.
pub fn validate_template(value: &Value) -> Result<()> {
    keys(value, &["name", "description"], &[])?;
    let name = text(&value["name"])?;
    require(valid_name(name) && !is_built_in(name))?;
    require(valid_description(plain(
        &value["description"],
        DESCRIPTION_BYTES,
    )?))
}

/// `meta.fromTemplate`: exactly `{name}` or `{name, revision}`; a built-in name is allowed.
pub fn validate_from(value: &Value) -> Result<()> {
    keys(value, &["name"], &["revision"])?;
    require(valid_name(text(&value["name"])?))?;
    value
        .get("revision")
        .map_or(Ok(()), |revision| require(valid_revision(text(revision)?)))
}

/// The `meta.template` value for a name and a description, or `Invalid`.
pub fn template(name: &str, description: &str) -> Result<Value> {
    let value = json!({"name": name, "description": description});
    validate_template(&value)?;
    Ok(value)
}

/// The `meta.fromTemplate` value for a template name and, for a user template, its revision.
pub fn from_template(name: &str, revision: Option<&str>) -> Result<Value> {
    let value = match revision {
        Some(revision) => json!({"name": name, "revision": revision}),
        None => json!({"name": name}),
    };
    validate_from(&value)?;
    Ok(value)
}

/// The mark of a validated content metadata, if the page is a template.
pub fn mark_of_meta(meta: &Value) -> Option<(&str, &str)> {
    let mark = meta.get("template")?;
    Some((mark["name"].as_str()?, mark["description"].as_str()?))
}

/// What the page was created from, if the metadata records it: the name and the revision.
pub fn source_of_meta(meta: &Value) -> Option<(&str, Option<&str>)> {
    let from = meta.get("fromTemplate")?;
    Some((
        from["name"].as_str()?,
        from.get("revision").and_then(Value::as_str),
    ))
}

/// One structured action on `meta.template`: `template.set` with `{name, description}` marks the
/// page (or changes the description of a marked one) and returns the new value; `template.clear`
/// with `{}` removes the mark and returns `None`. The page stays a page either way.
pub fn apply(action: &str, body: &Value) -> Result<Option<Value>> {
    match action {
        "template.set" => {
            validate_template(body)?;
            Ok(Some(body.clone()))
        }
        "template.clear" => {
            keys(body, &[], &[])?;
            Ok(None)
        }
        _ => Err(crate::closed::Invalid),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_one_to_forty_lowercase_letters_digits_or_hyphens() {
        for good in ["a", "weekly-review", "a1-b2", &"x".repeat(40)] {
            assert!(valid_name(good), "{good}");
        }
        for bad in [
            "",
            "Weekly",
            "two words",
            "has_underscore",
            "é",
            &"x".repeat(41),
        ] {
            assert!(!valid_name(bad), "{bad}");
        }
    }

    #[test]
    fn the_six_built_in_names_are_reserved_for_marks_but_allowed_as_a_source() {
        assert_eq!(BUILT_IN.len(), 6);
        for name in BUILT_IN {
            assert!(valid_name(name) && is_built_in(name));
            assert!(template(name, "").is_err(), "{name}");
            assert!(from_template(name, None).is_ok(), "{name}");
        }
        assert!(!is_built_in("weekly-review"));
    }

    #[test]
    fn a_mark_is_exactly_name_and_description_with_a_bounded_trimmed_line() {
        assert!(validate_template(&json!({"name": "weekly-review", "description": ""})).is_ok());
        assert!(
            validate_template(&json!({"name": "weekly-review", "description": "Shipped, open"}))
                .is_ok()
        );
        for bad in [
            json!({"name": "weekly-review"}),
            json!({"description": "x"}),
            json!({"name": "weekly-review", "description": "x", "extra": 1}),
            json!({"name": "weekly-review", "description": null}),
            json!({"name": "weekly-review", "description": " padded "}),
            json!({"name": "weekly-review", "description": "two\nlines"}),
            json!({"name": "weekly-review", "description": "x".repeat(201)}),
            json!({"name": "Weekly", "description": ""}),
            json!({"name": "report", "description": ""}),
            json!("weekly-review"),
            json!([]),
        ] {
            assert!(validate_template(&bad).is_err(), "{bad}");
        }
        assert!(
            validate_template(&json!({"name": "weekly-review", "description": "x".repeat(200)}))
                .is_ok()
        );
    }

    const REVISION: &str = "v1:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn a_source_is_a_name_and_for_a_user_template_a_page_revision() {
        for good in [
            json!({"name": "report"}),
            json!({"name": "weekly-review", "revision": REVISION}),
        ] {
            assert!(validate_from(&good).is_ok(), "{good}");
        }
        for bad in [
            json!({}),
            json!({"name": "weekly-review", "revision": 130}),
            json!({"name": "weekly-review", "revision": "130"}),
            json!({"name": "weekly-review", "revision": ""}),
            json!({"name": "weekly-review", "revision": &REVISION[3..]}),
            json!({"name": "weekly-review", "revision": REVISION.to_uppercase()}),
            json!({"name": "weekly-review", "revision": format!("{REVISION}0")}),
            json!({"name": "weekly-review", "revision": format!("v2:{}", &REVISION[3..])}),
            json!({"name": "weekly-review", "revision": null}),
            json!({"name": "weekly-review", "other": "x"}),
            json!({"name": "Weekly"}),
        ] {
            assert!(validate_from(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_revision_is_the_digest_of_the_copied_source_and_follows_the_grammar() {
        let revision = revision_of("<p>Weekly</p>");
        assert!(valid_revision(&revision), "{revision}");
        assert_eq!(revision, revision_of("<p>Weekly</p>"));
        assert_ne!(revision, revision_of("<p>Weekly </p>"));
    }

    #[test]
    fn an_action_sets_replaces_or_clears_and_refuses_anything_else() {
        let set = apply(
            "template.set",
            &json!({"name": "weekly-review", "description": "Shipped, open"}),
        )
        .unwrap();
        let meta = json!({"layout": "column", "template": set});
        assert_eq!(
            mark_of_meta(&meta),
            Some(("weekly-review", "Shipped, open"))
        );
        assert_eq!(apply("template.clear", &json!({})), Ok(None));
        for (action, body) in [
            ("template.set", json!({"name": "report", "description": ""})),
            ("template.set", json!({"name": "weekly-review"})),
            ("template.clear", json!({"name": "weekly-review"})),
            ("template.clear", Value::Null),
            ("template.rename", json!({})),
        ] {
            assert!(apply(action, &body).is_err(), "{action} {body}");
        }
    }

    #[test]
    fn the_source_of_a_page_reads_back_with_or_without_a_revision() {
        let meta = json!({"fromTemplate": {"name": "report"}});
        assert_eq!(source_of_meta(&meta), Some(("report", None)));
        let meta = json!({"fromTemplate": {"name": "weekly-review", "revision": REVISION}});
        assert_eq!(
            source_of_meta(&meta),
            Some(("weekly-review", Some(REVISION)))
        );
        assert_eq!(source_of_meta(&json!({})), None);
    }

    #[test]
    fn shared_vectors_match_the_closed_native_codec() {
        let corpus: Value = serde_json::from_str(include_str!(
            "../../../contracts/vectors/template-marks-v1.json"
        ))
        .unwrap();
        for case in corpus["cases"].as_array().unwrap() {
            let result = match case["kind"].as_str().unwrap() {
                "template" => validate_template(&case["value"]),
                "fromTemplate" => validate_from(&case["value"]),
                _ => panic!("unknown vector kind"),
            };
            assert_eq!(
                result.is_ok(),
                case["valid"].as_bool().unwrap(),
                "{}",
                case["name"]
            );
        }
    }
}
