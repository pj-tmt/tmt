//! The editing reference a `page write` or `page patch` receipt names when the edit went
//! off-guidance (#5029). A normal edit stays quiet: only a lint warning, a non-minimal update or a
//! rewrite of most of a large page earns the one command.
use tmt_colab::{edit_scale::EditScale, page::UpdateSize};

/// The one command a receipt names, as the `tmt colab skill` reference topic.
pub const EDITING: &str = "tmt colab skill editing";

/// Why an edit is named off-guidance; the first that applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    Warnings,
    NotMinimal,
    LargeRewrite,
}

impl Reason {
    /// The human sentence: what happened, then the command to read.
    pub fn hint(self) -> String {
        let what = match self {
            Self::Warnings => "The page check found problems in the result.",
            Self::NotMinimal => "The write was not the minimal change.",
            Self::LargeRewrite => "The write replaced most of the page.",
        };
        format!("{what} Read `{EDITING}` for the small-change workflow.")
    }
}

/// Why `update`, its `scale` and the page check's `warnings` call for the reference. An edit that
/// published nothing has nothing to explain.
pub fn reason(warnings: usize, update: &UpdateSize, scale: EditScale) -> Option<Reason> {
    if update.count == 0 {
        return None;
    }
    if warnings > 0 {
        Some(Reason::Warnings)
    } else if update.minimal == Some(false) {
        Some(Reason::NotMinimal)
    } else if scale.is_large_rewrite() {
        Some(Reason::LargeRewrite)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE: UpdateSize = UpdateSize {
        count: 1,
        bytes: 200,
        minimal: Some(true),
    };
    const LARGE: EditScale = EditScale {
        base_bytes: 4096,
        replaced_bytes: 4000,
    };
    const SMALL: EditScale = EditScale {
        base_bytes: 4096,
        replaced_bytes: 10,
    };

    #[test]
    fn a_normal_edit_names_nothing() {
        assert_eq!(reason(0, &ONE, SMALL), None);
        assert_eq!(reason(0, &UpdateSize::NONE, LARGE), None);
        assert_eq!(reason(3, &UpdateSize::NONE, LARGE), None);
    }

    #[test]
    fn each_trigger_names_the_reference_on_its_own() {
        assert_eq!(reason(1, &ONE, SMALL), Some(Reason::Warnings));
        let loose = UpdateSize {
            minimal: Some(false),
            ..ONE
        };
        assert_eq!(reason(0, &loose, SMALL), Some(Reason::NotMinimal));
        assert_eq!(reason(0, &ONE, LARGE), Some(Reason::LargeRewrite));
    }

    #[test]
    fn the_hint_names_exactly_one_command() {
        for reason in [Reason::Warnings, Reason::NotMinimal, Reason::LargeRewrite] {
            assert_eq!(reason.hint().matches("tmt colab skill").count(), 1);
        }
    }
}
