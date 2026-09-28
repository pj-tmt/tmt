//! Section filters: a small boolean language over row fields, e.g.
//! `pending or state = blocked`, `not presence = offline and (role != lead)`.
//! It compares text only; it never evaluates code or reaches outside a row.

const MAX_LENGTH: usize = 1024;
const MAX_DEPTH: usize = 32;

/// A row's field values by name; absent fields are `None`.
pub trait Row {
    fn value(&self, field: &str) -> Option<&str>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Filter {
    /// The field is present and non-empty.
    Has(String),
    Equals(String, String),
    /// True when the field is absent or differs.
    Differs(String, String),
    Not(Box<Filter>),
    And(Box<Filter>, Box<Filter>),
    Or(Box<Filter>, Box<Filter>),
}

impl Filter {
    pub fn matches(&self, row: &impl Row) -> bool {
        match self {
            Self::Has(field) => row.value(field).is_some_and(|value| !value.is_empty()),
            Self::Equals(field, expected) => row.value(field) == Some(expected.as_str()),
            Self::Differs(field, expected) => row.value(field) != Some(expected.as_str()),
            Self::Not(inner) => !inner.matches(row),
            Self::And(left, right) => left.matches(row) && right.matches(row),
            Self::Or(left, right) => left.matches(row) || right.matches(row),
        }
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        if text.len() > MAX_LENGTH {
            return Err(format!("a filter is at most {MAX_LENGTH} bytes"));
        }
        let mut parser = Parser {
            tokens: tokenize(text)?,
            next: 0,
        };
        let filter = parser.or(0)?;
        match parser.tokens.get(parser.next) {
            None => Ok(filter),
            Some(token) => Err(format!("unexpected {}", token.describe())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Word(String),
    Quoted(String),
    Open,
    Close,
    Equals,
    Differs,
}

impl Token {
    fn describe(&self) -> String {
        match self {
            Self::Word(word) => format!("'{word}'"),
            Self::Quoted(text) => format!("\"{text}\""),
            Self::Open => "'('".into(),
            Self::Close => "')'".into(),
            Self::Equals => "'='".into(),
            Self::Differs => "'!='".into(),
        }
    }
}

fn word_byte(byte: char) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, '_' | '-' | '.' | ':' | '/' | '#')
}

fn tokenize(text: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(&next) = chars.peek() {
        match next {
            ' ' | '\t' => {
                chars.next();
            }
            '(' | ')' => {
                chars.next();
                tokens.push(if next == '(' {
                    Token::Open
                } else {
                    Token::Close
                });
            }
            '=' => {
                chars.next();
                tokens.push(Token::Equals);
            }
            '!' => {
                chars.next();
                if chars.next() != Some('=') {
                    return Err("'!' must be followed by '='".into());
                }
                tokens.push(Token::Differs);
            }
            '"' => {
                chars.next();
                let mut quoted = String::new();
                loop {
                    match chars.next() {
                        None => return Err("unterminated quoted value".into()),
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(escaped @ ('"' | '\\')) => quoted.push(escaped),
                            _ => return Err("only \\\" and \\\\ escapes are allowed".into()),
                        },
                        Some(character) if character.is_control() => {
                            return Err("control characters are not allowed".into());
                        }
                        Some(character) => quoted.push(character),
                    }
                }
                tokens.push(Token::Quoted(quoted));
            }
            character if word_byte(character) => {
                let mut word = String::new();
                while let Some(&character) = chars.peek().filter(|c| word_byte(**c)) {
                    word.push(character);
                    chars.next();
                }
                tokens.push(Token::Word(word));
            }
            other => return Err(format!("unexpected character '{other}'")),
        }
    }
    Ok(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    next: usize,
}

impl Parser {
    fn keyword(&mut self, keyword: &str) -> bool {
        let found =
            matches!(self.tokens.get(self.next), Some(Token::Word(word)) if word == keyword);
        self.next += usize::from(found);
        found
    }

    fn or(&mut self, depth: usize) -> Result<Filter, String> {
        let mut left = self.and(depth)?;
        while self.keyword("or") {
            left = Filter::Or(Box::new(left), Box::new(self.and(depth)?));
        }
        Ok(left)
    }

    fn and(&mut self, depth: usize) -> Result<Filter, String> {
        let mut left = self.not(depth)?;
        while self.keyword("and") {
            left = Filter::And(Box::new(left), Box::new(self.not(depth)?));
        }
        Ok(left)
    }

    fn not(&mut self, depth: usize) -> Result<Filter, String> {
        if depth > MAX_DEPTH {
            return Err(format!("filters nest at most {MAX_DEPTH} levels"));
        }
        if self.keyword("not") {
            return Ok(Filter::Not(Box::new(self.not(depth + 1)?)));
        }
        self.atom(depth)
    }

    fn atom(&mut self, depth: usize) -> Result<Filter, String> {
        let token = self
            .tokens
            .get(self.next)
            .cloned()
            .ok_or("the filter ends early")?;
        self.next += 1;
        match token {
            Token::Open => {
                let inner = self.or(depth + 1)?;
                if self.tokens.get(self.next) != Some(&Token::Close) {
                    return Err("missing ')'".into());
                }
                self.next += 1;
                Ok(inner)
            }
            Token::Word(field) if !["and", "or", "not"].contains(&field.as_str()) => {
                let comparison = match self.tokens.get(self.next) {
                    Some(Token::Equals) => Some(true),
                    Some(Token::Differs) => Some(false),
                    _ => None,
                };
                let Some(equals) = comparison else {
                    return Ok(Filter::Has(field));
                };
                self.next += 1;
                let value = match self.tokens.get(self.next).cloned() {
                    Some(Token::Word(value) | Token::Quoted(value)) => value,
                    _ => return Err(format!("'{field}' needs a value to compare")),
                };
                self.next += 1;
                Ok(if equals {
                    Filter::Equals(field, value)
                } else {
                    Filter::Differs(field, value)
                })
            }
            other => Err(format!("expected a field, found {}", other.describe())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct Fields(HashMap<&'static str, &'static str>);
    impl Row for Fields {
        fn value(&self, field: &str) -> Option<&str> {
            self.0.get(field).copied()
        }
    }

    fn row(pairs: &[(&'static str, &'static str)]) -> Fields {
        Fields(pairs.iter().copied().collect())
    }

    #[test]
    fn precedence_is_not_then_and_then_or() {
        let filter =
            Filter::parse("pending or state = blocked and not presence = offline").unwrap();
        assert!(filter.matches(&row(&[("pending", "approve")])));
        assert!(filter.matches(&row(&[("state", "blocked"), ("presence", "active")])));
        assert!(!filter.matches(&row(&[("state", "blocked"), ("presence", "offline")])));
        assert!(!filter.matches(&row(&[("state", "working")])));
        let grouped =
            Filter::parse("(pending or state = blocked) and presence != offline").unwrap();
        assert!(!grouped.matches(&row(&[("pending", "x"), ("presence", "offline")])));
        assert!(grouped.matches(&row(&[("pending", "x")])), "absent differs");
    }

    #[test]
    fn values_compare_exactly_and_empty_fields_do_not_count_as_present() {
        let quoted = Filter::parse(r#"task = "rotate \"session\" tokens""#).unwrap();
        assert!(quoted.matches(&row(&[("task", "rotate \"session\" tokens")])));
        assert!(!quoted.matches(&row(&[("task", "Rotate \"session\" tokens")])));
        assert!(
            !Filter::parse("pending")
                .unwrap()
                .matches(&row(&[("pending", "")]))
        );
        let link = Filter::parse("pr_link = https://example.com/pull/4").unwrap();
        assert!(link.matches(&row(&[("pr_link", "https://example.com/pull/4")])));
    }

    #[test]
    fn malformed_and_oversized_filters_are_rejected() {
        for text in [
            "",
            "state =",
            "= x",
            "(state",
            "state)",
            "and",
            "a or",
            "not",
            "state ! x",
            "\"unterminated",
            "a = \"\\n\"",
            "a $ b",
            "a b",
        ] {
            assert!(Filter::parse(text).is_err(), "{text:?}");
        }
        assert!(Filter::parse(&format!("{}a{}", "(".repeat(40), ")".repeat(40))).is_err());
        assert!(Filter::parse(&format!("{}a", "not ".repeat(40))).is_err());
        assert!(Filter::parse(&"a or ".repeat(300)).is_err(), "length bound");
    }
}
