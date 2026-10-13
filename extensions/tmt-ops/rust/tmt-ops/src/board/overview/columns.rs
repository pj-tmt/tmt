//! Overview column policy. The TUI solver owns cell geometry and truncation.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Column {
    Agent,
    Squad,
    State,
    TokensPerHour,
    Digest,
    Cron,
    Runtime,
    Model,
}

impl Column {
    pub const ALL: [Self; 8] = [
        Self::Agent,
        Self::Squad,
        Self::State,
        Self::TokensPerHour,
        Self::Digest,
        Self::Cron,
        Self::Runtime,
        Self::Model,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Agent => "Agent",
            Self::Squad => "Squad",
            Self::State => "State",
            Self::TokensPerHour => "TOK/h",
            Self::Digest => "Digest",
            Self::Cron => "Cron",
            Self::Runtime => "Runtime",
            Self::Model => "Model",
        }
    }

    /// Minimum useful cell allocation, not a second layout solver. Agent grows
    /// into the space left by these tracks; the template clips every row to one
    /// line and uses the shared grapheme measurer for ellipses.
    pub fn minimum(self) -> u16 {
        match self {
            Self::Agent => 12,
            Self::Squad | Self::State => 10,
            Self::TokensPerHour | Self::Cron => 7,
            Self::Digest => 12,
            Self::Runtime => 8,
            Self::Model => 14,
        }
    }
}

/// Choose whole columns before materialization. Hidden values remain in the
/// acquired row and its detail; no value is shortened or reformatted here.
pub fn visible(width: u16) -> Vec<Column> {
    let mut columns = Column::ALL.to_vec();
    for removable in [
        Column::Model,
        Column::Digest,
        Column::Runtime,
        Column::Squad,
        Column::Cron,
    ] {
        let needed: u16 = columns.iter().map(|column| column.minimum()).sum::<u16>()
            + columns.len().saturating_sub(1) as u16;
        if needed <= width {
            break;
        }
        columns.retain(|column| *column != removable);
    }
    columns
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn narrow_columns_drop_whole_and_keep_the_monitor_identity() {
        for width in [60, 80, 100, 140, 160] {
            let shown = visible(width);
            assert!(shown.contains(&Column::Agent));
            assert!(shown.contains(&Column::State));
            assert!(shown.contains(&Column::TokensPerHour));
            assert!(
                shown.iter().map(|column| column.minimum()).sum::<u16>() + shown.len() as u16 - 1
                    <= width
            );
            assert!(
                Column::ALL
                    .iter()
                    .filter(|column| shown.contains(column))
                    .copied()
                    .eq(shown.iter().copied())
            );
        }
        assert_eq!(visible(160), Column::ALL);
        assert!(!visible(60).contains(&Column::Model));
        assert_eq!(
            visible(0),
            vec![Column::Agent, Column::State, Column::TokensPerHour]
        );
    }
}
