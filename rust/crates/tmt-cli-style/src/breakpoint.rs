//! Named terminal widths, in cells. The design tokens (`design/tokens/tokens.json`)
//! own the names and values and a test checks this table against that file, as
//! `theme` does for colors. Layouts name a step and never inline a number, so
//! every surface shares the same few steps.

/// A named width step. A width at or above `cells` takes the step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Breakpoint {
    pub name: &'static str,
    pub cells: u16,
}

pub const SM: Breakpoint = Breakpoint {
    name: "sm",
    cells: 80,
};
pub const MD: Breakpoint = Breakpoint {
    name: "md",
    cells: 100,
};
pub const LG: Breakpoint = Breakpoint {
    name: "lg",
    cells: 140,
};

/// Every step, narrowest first.
pub const ALL: [Breakpoint; 3] = [SM, MD, LG];

/// The step with this exact token name, for markup that names a step.
pub fn by_name(name: &str) -> Option<Breakpoint> {
    ALL.into_iter().find(|step| step.name == name)
}

#[cfg(test)]
mod tests;
