//! State vocabulary and bounded glob resolution; projections share this owner.

use super::{Layout, field_name, invalid};
use crate::core::SquadError;
use std::collections::BTreeMap;
use toml_edit::{Item, TableLike};

const MAX_PATTERNS: usize = 64;
const MAX_MATCH_BYTES: usize = 256;
const WORDS: usize = (MAX_MATCH_BYTES + 1).div_ceil(64);
type Bits = [u64; WORDS];
pub type Rank = (u16, bool, usize);
const DEFAULT_RANK: Rank = (1000, true, usize::MAX);

#[derive(Debug, Clone, PartialEq, Eq)]
struct StateSetting {
    color: Option<String>,
    rank: Rank,
}

impl Default for StateSetting {
    fn default() -> Self {
        Self {
            color: None,
            rank: DEFAULT_RANK,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Pattern {
    glob: Glob,
    entry: StateSetting,
}

/// Exact entries (including presets) precede ordered patterns, then the default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct States {
    exact: BTreeMap<String, StateSetting>,
    patterns: Vec<Pattern>,
}

impl States {
    pub fn preset(layout: Layout) -> Self {
        let exact = layout
            .states()
            .iter()
            .enumerate()
            .map(|(index, state)| {
                let color = layout
                    .state_colors()
                    .iter()
                    .find(|(name, _)| name == state)
                    .map(|(_, color)| (*color).to_owned());
                (
                    (*state).to_owned(),
                    StateSetting {
                        color,
                        rank: (index as u16, true, index),
                    },
                )
            })
            .collect();
        Self {
            exact,
            patterns: Vec::new(),
        }
    }

    fn resolve(&self, state: Option<&str>) -> Option<&StateSetting> {
        let state = state?;
        self.exact.get(state).or_else(|| {
            self.patterns
                .iter()
                .find(|pattern| pattern.glob.matches(state))
                .map(|pattern| &pattern.entry)
        })
    }

    pub fn rank(&self, state: Option<&str>) -> Rank {
        self.resolve(state).map_or(DEFAULT_RANK, |entry| entry.rank)
    }

    pub fn color(&self, state: Option<&str>) -> Option<&str> {
        self.resolve(state)?.color.as_deref()
    }

    pub(super) fn read(
        table: Option<&dyn TableLike>,
        squad: &str,
        layout: Layout,
    ) -> Result<Self, SquadError> {
        let mut states = Self::preset(layout);
        let Some(table) = table else {
            return Ok(states);
        };
        let place = format!("squad.{squad}.states");
        if let Some(item) = table.get("states") {
            let table = item
                .as_table_like()
                .ok_or_else(|| invalid(format!("`{place}` must be a table of states.")))?;
            for (state, settings) in table.iter() {
                let settings = settings
                    .as_table_like()
                    .filter(|_| field_name(state))
                    .ok_or_else(|| invalid(format!("`{place}.{state}` must be a table.")))?;
                let entry = states.exact.entry(state.to_owned()).or_default();
                for (key, value) in settings.iter() {
                    let place = format!("{place}.{state}.{key}");
                    match key {
                        "color" => entry.color = Some(color(value, &place)?),
                        "sort" => entry.rank = (sort(value, &place)?, false, entry.rank.2),
                        _ => {
                            return Err(invalid(format!(
                                "`{place}` is not a state setting; use color or sort."
                            )));
                        }
                    }
                }
            }
        }
        let place = format!("squad.{squad}.state_patterns");
        if let Some(item) = table.get("state_patterns") {
            let patterns = item
                .as_array_of_tables()
                .ok_or_else(|| invalid(format!("`{place}` must be an array of tables.")))?;
            if patterns.len() > MAX_PATTERNS {
                return Err(invalid(format!(
                    "`{place}` must contain at most {MAX_PATTERNS} patterns."
                )));
            }
            for (index, pattern) in patterns.iter().enumerate() {
                let place = format!("{place}[{index}]");
                for (key, _) in pattern.iter() {
                    if !matches!(key, "match" | "color" | "sort" | "ignore_case") {
                        return Err(invalid(format!(
                            "`{place}.{key}` is not a pattern setting; use match, color, sort or ignore_case."
                        )));
                    }
                }
                let text = pattern.get("match").and_then(Item::as_str)
                    .filter(|text| !text.is_empty() && text.len() <= MAX_MATCH_BYTES)
                    .ok_or_else(|| invalid(format!("`{place}.match` must be a nonempty string of at most {MAX_MATCH_BYTES} UTF-8 bytes.")))?;
                let color_place = format!("{place}.color");
                let color = color(pattern.get("color").unwrap_or(&Item::None), &color_place)?;
                let rank = pattern
                    .get("sort")
                    .map(|value| sort(value, &format!("{place}.sort")))
                    .transpose()?
                    .map_or(DEFAULT_RANK, |sort| (sort, false, usize::MAX));
                let ignore_case = pattern
                    .get("ignore_case")
                    .map(|value| {
                        value.as_bool().ok_or_else(|| {
                            invalid(format!("`{place}.ignore_case` must be a boolean."))
                        })
                    })
                    .transpose()?
                    .unwrap_or(false);
                states.patterns.push(Pattern {
                    glob: Glob::new(text, ignore_case),
                    entry: StateSetting {
                        color: Some(color),
                        rank,
                    },
                });
            }
        }
        Ok(states)
    }
}

fn color(value: &Item, place: &str) -> Result<String, SquadError> {
    value
        .as_str()
        .filter(|color| crate::look::known(color))
        .map(str::to_owned)
        .ok_or_else(|| invalid(format!("`{place}` must be {}.", crate::look::names())))
}

fn sort(value: &Item, place: &str) -> Result<u16, SquadError> {
    value
        .as_integer()
        .and_then(|sort| u16::try_from(sort).ok())
        .filter(|sort| *sort <= 999)
        .ok_or_else(|| invalid(format!("`{place}` must be 0-999.")))
}

fn bit(bits: &mut Bits, index: usize) {
    bits[index / 64] |= 1 << (index % 64);
}

fn shifted(bits: Bits) -> Bits {
    std::array::from_fn(|index| {
        (bits[index] << 1) | if index == 0 { 0 } else { bits[index - 1] >> 63 }
    })
}

/// A bitset NFA consumes each Unicode scalar once. Adjacent stars collapse,
/// so epsilon closure needs one shift. Fixed pattern/count caps bound the
/// work per input character; no retry, recursion or backtracking is possible.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Glob {
    literals: BTreeMap<String, Bits>,
    stars: Bits,
    any: Bits,
    end: usize,
    ignore_case: bool,
}

impl Glob {
    fn key(character: char, ignore_case: bool) -> String {
        if ignore_case {
            character.to_lowercase().collect()
        } else {
            character.to_string()
        }
    }

    fn new(text: &str, ignore_case: bool) -> Self {
        let mut glob = Self {
            literals: BTreeMap::new(),
            stars: [0; WORDS],
            any: [0; WORDS],
            end: 0,
            ignore_case,
        };
        let mut previous_star = false;
        for character in text.chars() {
            if character == '*' && previous_star {
                continue;
            }
            match character {
                '*' => bit(&mut glob.stars, glob.end),
                '?' => bit(&mut glob.any, glob.end),
                literal => bit(
                    glob.literals
                        .entry(Self::key(literal, ignore_case))
                        .or_insert([0; WORDS]),
                    glob.end,
                ),
            }
            previous_star = character == '*';
            glob.end += 1;
        }
        glob
    }

    fn closure(&self, active: Bits) -> Bits {
        let advance = shifted(std::array::from_fn(|index| {
            active[index] & self.stars[index]
        }));
        std::array::from_fn(|index| active[index] | advance[index])
    }

    fn matches(&self, text: &str) -> bool {
        let mut active = [0; WORDS];
        bit(&mut active, 0);
        active = self.closure(active);
        for character in text.chars() {
            let literal = self
                .literals
                .get(&Self::key(character, self.ignore_case))
                .copied()
                .unwrap_or([0; WORDS]);
            let advance = shifted(std::array::from_fn(|index| {
                active[index] & (literal[index] | self.any[index])
            }));
            active = self.closure(std::array::from_fn(|index| {
                advance[index] | (active[index] & self.stars[index])
            }));
        }
        active[self.end / 64] & (1 << (self.end % 64)) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(text: &str, layout: Layout) -> Result<States, SquadError> {
        let document = text.parse::<toml_edit::DocumentMut>().unwrap();
        States::read(document["squad"]["x"].as_table_like(), "x", layout)
    }

    #[test]
    fn glob_matches_scalars_and_literals_without_backtracking() {
        for (pattern, text, expected) in [
            ("*", "", true),
            ("a**b", "ab", true),
            ("a*b", "axyb", true),
            ("a*b", "axybc", false),
            ("?", "é", true),
            ("?", "", false),
            ("?", "ab", false),
            ("a.b", "a.b", true),
            ("a.b", "axb", false),
            ("[ab]", "a", false),
            ("blocked*", "blocked", true),
            ("blocked*", "Blocked-on-ci", false),
            ("*a*a*b", "aaaaaac", false),
        ] {
            assert_eq!(
                Glob::new(pattern, false).matches(text),
                expected,
                "{pattern} {text}"
            );
        }
        assert!(Glob::new("Ä?*", true).matches("äİrest"));
        assert!(Glob::new("İ", true).matches("İ"));
        assert!(!Glob::new("İ", true).matches("i"));
        let pattern = format!("{}b", "*a".repeat(127));
        assert!(!Glob::new(&pattern, false).matches(&"a".repeat(100_000)));
        assert!(Glob::new(&"?".repeat(256), false).matches(&"x".repeat(256)));
        for prefix in [63, 64, 127, 128, 254] {
            let pattern = format!("{}*?", "a".repeat(prefix));
            assert!(Glob::new(&pattern, false).matches(&format!("{}xyz", "a".repeat(prefix))));
            assert!(!Glob::new(&pattern, false).matches(&"a".repeat(prefix)));
        }
    }

    #[test]
    fn exact_and_preset_win_entirely_then_first_pattern_then_default() {
        let states = read(
            r#"
[squad.x.states]
blocked = { color = "red", sort = 7 }
custom = { color = "dim" }
empty = {}
[[squad.x.state_patterns]]
match = "blocked*"
color = "blocked"
sort = 0
[[squad.x.state_patterns]]
match = "*"
color = "accent"
sort = 1
"#,
            Layout::Crew,
        )
        .unwrap();
        assert_eq!(states.color(Some("blocked")), Some("red"));
        assert!(states.rank(Some("blocked")) > states.rank(Some("idle")));
        assert_eq!(states.color(Some("working")), Some("working"));
        assert_eq!(states.color(Some("blocked-on-ci")), Some("blocked"));
        assert_eq!(states.rank(Some("blocked-on-ci")), (0, false, usize::MAX));
        assert_eq!(states.color(Some("custom")), Some("dim"));
        assert_eq!(states.rank(Some("custom")), DEFAULT_RANK);
        assert_eq!(states.color(Some("empty")), None);
        assert_eq!(states.rank(Some("empty")), DEFAULT_RANK);
        assert_eq!(states.color(Some("other")), Some("accent"));
        assert_eq!(states.color(None), None);
        let insensitive = read(
            "[[squad.x.state_patterns]]\nmatch = 'Blocked*'\ncolor = 'amber'\nignore_case = true\n",
            Layout::Minimal,
        )
        .unwrap();
        assert_eq!(insensitive.color(Some("blocked-on-ci")), Some("amber"));
        assert_eq!(insensitive.rank(Some("blocked-on-ci")), DEFAULT_RANK);
        assert_eq!(insensitive.color(Some("other")), None);
    }

    #[test]
    fn validation_names_each_bad_setting_and_caps() {
        for (setting, key) in [
            ("match = ''\ncolor = 'dim'", "match"),
            ("match = 4\ncolor = 'dim'", "match"),
            ("match = '*'", "color"),
            ("match = '*'\ncolor = 'pink'", "color"),
            ("match = '*'\ncolor = 4", "color"),
            ("match = '*'\ncolor = 'dim'\nsort = 1000", "sort"),
            ("match = '*'\ncolor = 'dim'\nsort = -1", "sort"),
            ("match = '*'\ncolor = 'dim'\nsort = 'first'", "sort"),
            ("match = '*'\ncolor = 'dim'\nignore_case = 1", "ignore_case"),
            ("match = '*'\ncolor = 'dim'\nlabel = 'alias'", "label"),
        ] {
            let text = format!(
                "[[squad.x.state_patterns]]\nmatch = 'valid'\ncolor = 'dim'\n[[squad.x.state_patterns]]\n{setting}\n"
            );
            let error = read(&text, Layout::Crew).unwrap_err();
            assert_eq!(error.code, "SQUAD_CONFIG_INVALID");
            assert!(
                error
                    .message
                    .contains(&format!("squad.x.state_patterns[1].{key}")),
                "{}",
                error.message
            );
        }
        let one = "[[squad.x.state_patterns]]\nmatch = '*'\ncolor = 'dim'\n";
        assert!(read(&one.repeat(64), Layout::Crew).is_ok());
        assert!(
            read(&one.repeat(65), Layout::Crew)
                .unwrap_err()
                .message
                .contains("at most 64")
        );
        assert!(
            read(
                &format!(
                    "[[squad.x.state_patterns]]\nmatch = '{}'\ncolor = 'dim'\n",
                    "é".repeat(129)
                ),
                Layout::Crew
            )
            .unwrap_err()
            .message
            .contains("state_patterns[0].match")
        );
        assert!(
            read("[squad.x]\nstate_patterns = '*'\n", Layout::Crew)
                .unwrap_err()
                .message
                .contains("squad.x.state_patterns")
        );
    }
}
