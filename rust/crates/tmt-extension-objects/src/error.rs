//! The one refusal vocabulary every admission in the leaf reports.
use std::fmt;

/// Why bytes were refused, coarse enough to be a stable vector expectation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorClass {
    /// The announced or actual size is outside the frame bounds.
    Length,
    /// The body is not UTF-8.
    Utf8,
    /// The text is not JSON (including a malformed numeric literal).
    Syntax,
    /// Two object members have the same decoded name.
    Duplicate,
    /// Valid JSON of the wrong structure or numeric type.
    Shape,
    /// A well-formed value outside its grammar or range.
    Value,
    /// Bytes follow the JSON value.
    Trailing,
}
impl fmt::Display for ErrorClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Length => "frame length is out of bounds",
            Self::Utf8 => "frame is not UTF-8",
            Self::Syntax => "frame is not valid JSON",
            Self::Duplicate => "frame repeats an object member",
            Self::Shape => "frame has the wrong structure",
            Self::Value => "frame has a value outside its grammar",
            Self::Trailing => "frame has trailing bytes",
        })
    }
}
impl std::error::Error for ErrorClass {}
