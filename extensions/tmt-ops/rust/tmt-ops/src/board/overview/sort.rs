//! Pure ordering of acquired rows. Selection remains an identity, never an index.
use super::columns::Column;
use std::cmp::Ordering;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Ascending,
    Descending,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sort {
    pub column: Column,
    pub direction: Direction,
}

impl Sort {
    pub fn initial(column: Column) -> Self {
        Self {
            column,
            direction: if matches!(column, Column::TokensPerHour | Column::Cron) {
                Direction::Descending
            } else {
                Direction::Ascending
            },
        }
    }

    pub fn reverse(self) -> Self {
        Self {
            direction: match self.direction {
                Direction::Ascending => Direction::Descending,
                Direction::Descending => Direction::Ascending,
            },
            ..self
        }
    }

    pub fn next(self) -> Self {
        let position = Column::ALL
            .iter()
            .position(|column| *column == self.column)
            .unwrap();
        Self::initial(Column::ALL[(position + 1) % Column::ALL.len()])
    }
}

/// Numeric values stay numeric. Text is supplied by the existing projection;
/// runtime is the observed driver, not a provider inferred from the model/name.
pub struct Row<'a> {
    pub id: &'a str,
    pub agent: &'a str,
    pub squad: Option<&'a str>,
    pub state: Option<&'a str>,
    pub tokens_per_hour: Option<f64>,
    pub digest: Option<&'a str>,
    pub cron: Option<(u32, u32)>,
    pub runtime: Option<&'a str>,
    pub model: Option<&'a str>,
}

fn known<T>(
    left: Option<T>,
    right: Option<T>,
    direction: Direction,
    compare: impl FnOnce(T, T) -> Ordering,
) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) => {
            let order = compare(left, right);
            if direction == Direction::Descending {
                order.reverse()
            } else {
                order
            }
        }
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

pub fn compare(left: &Row<'_>, right: &Row<'_>, sort: Sort) -> Ordering {
    let text = |left, right| known(left, right, sort.direction, str::cmp);
    let order = match sort.column {
        Column::Agent => text(Some(left.agent), Some(right.agent)),
        Column::Squad => text(left.squad, right.squad),
        Column::State => text(left.state, right.state),
        Column::Digest => text(left.digest, right.digest),
        Column::Runtime => text(left.runtime, right.runtime),
        Column::Model => text(left.model, right.model),
        Column::Cron => known(left.cron, right.cron, sort.direction, |a, b| a.cmp(&b)),
        Column::TokensPerHour => known(
            left.tokens_per_hour
                .filter(|value| value.is_finite() && *value >= 0.0),
            right
                .tokens_per_hour
                .filter(|value| value.is_finite() && *value >= 0.0),
            sort.direction,
            |a, b| a.total_cmp(&b),
        ),
    };
    order.then_with(|| left.id.cmp(right.id))
}

pub fn needs_you(left: &Row<'_>, right: &Row<'_>) -> Ordering {
    let priority = |state| match state {
        Some("blocked") => 0,
        Some("waiting") => 1,
        _ => 2,
    };
    priority(left.state)
        .cmp(&priority(right.state))
        .then_with(|| left.id.cmp(right.id))
}

/// A disappeared target is not silently replaced by a neighbor after sorting.
pub fn selected<'a>(id: &'a str, rows: &[Row<'_>]) -> Option<&'a str> {
    rows.iter().any(|row| row.id == id).then_some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str) -> Row<'_> {
        Row {
            id,
            agent: "same",
            squad: None,
            state: None,
            tokens_per_hour: None,
            digest: None,
            cron: None,
            runtime: None,
            model: None,
        }
    }

    #[test]
    fn missing_is_last_in_both_directions_and_ties_use_identity() {
        for column in Column::ALL {
            let mut a = row("a");
            let mut b = row("b");
            assert_eq!(compare(&a, &b, Sort::initial(column)), Ordering::Less);
            assert_eq!(
                compare(&a, &b, Sort::initial(column).reverse()),
                Ordering::Less
            );
            a.squad = Some("s");
            a.state = Some("idle");
            a.tokens_per_hour = Some(0.0);
            a.digest = Some("off");
            a.cron = Some((0, 0));
            a.runtime = Some("codex");
            a.model = Some("m");
            b.agent = "z";
            if column != Column::Agent {
                for sort in [Sort::initial(column), Sort::initial(column).reverse()] {
                    assert_eq!(compare(&a, &b, sort), Ordering::Less, "{column:?}");
                    assert_eq!(compare(&b, &a, sort), Ordering::Greater, "{column:?}");
                }
            }
        }
    }

    #[test]
    fn numeric_order_is_not_text_and_nonfinite_is_missing() {
        let mut a = row("a");
        let mut b = row("b");
        a.tokens_per_hour = Some(9.0);
        b.tokens_per_hour = Some(100.0);
        let sort = Sort::initial(Column::TokensPerHour);
        assert_eq!(compare(&a, &b, sort), Ordering::Greater);
        assert_eq!(compare(&a, &b, sort.reverse()), Ordering::Less);
        a.tokens_per_hour = Some(f64::NAN);
        assert_eq!(compare(&a, &b, sort.reverse()), Ordering::Greater);
        a.cron = Some((2, 10));
        b.cron = Some((3, 3));
        assert_eq!(
            compare(&a, &b, Sort::initial(Column::Cron)),
            Ordering::Greater
        );
    }

    #[test]
    fn every_known_text_column_reverses_by_value() {
        let mut first = row("z");
        let mut second = row("a");
        first.agent = "Atlas";
        second.agent = "Pat";
        first.squad = Some("alpha");
        second.squad = Some("omega");
        first.state = Some("blocked");
        second.state = Some("working");
        first.digest = Some("off");
        second.digest = Some("on");
        first.runtime = Some("claude");
        second.runtime = Some("codex");
        first.model = Some("model-a");
        second.model = Some("model-z");
        for column in [
            Column::Agent,
            Column::Squad,
            Column::State,
            Column::Digest,
            Column::Runtime,
            Column::Model,
        ] {
            assert_eq!(
                compare(&first, &second, Sort::initial(column)),
                Ordering::Less
            );
            assert_eq!(
                compare(&first, &second, Sort::initial(column).reverse()),
                Ordering::Greater
            );
        }
    }

    #[test]
    fn cycle_default_priority_and_identity_selection() {
        let mut a = row("a");
        let mut b = row("b");
        a.state = Some("waiting");
        b.state = Some("blocked");
        assert_eq!(needs_you(&a, &b), Ordering::Greater);
        assert_eq!(
            Sort::initial(Column::Model).next(),
            Sort::initial(Column::Agent)
        );
        assert_eq!(selected("a", &[b, a]), Some("a"));
        assert_eq!(selected("gone", &[]), None);
    }
}
