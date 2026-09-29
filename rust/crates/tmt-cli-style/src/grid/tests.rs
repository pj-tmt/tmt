use super::*;

fn content(basis: usize) -> Track {
    Track {
        basis,
        min: 4,
        max: None,
        grow: 0,
        shrink: 0,
        priority: None,
    }
}

fn widths(tracks: &[Track], available: usize) -> Vec<Option<usize>> {
    solve(tracks, Some(available), 1)
}

#[test]
fn without_a_known_width_every_column_keeps_its_basis() {
    let tracks = [
        content(10),
        Track::fixed(3),
        Track {
            grow: 1,
            ..content(5)
        },
    ];
    assert_eq!(solve(&tracks, None, 2), [Some(10), Some(3), Some(5)]);
}

#[test]
fn a_minimum_raises_the_basis_and_a_maximum_caps_it() {
    let tracks = [
        Track {
            min: 8,
            ..content(3)
        },
        Track {
            max: Some(6),
            ..content(20)
        },
    ];
    assert_eq!(widths(&tracks, 80), [Some(8), Some(6)]);
}

#[test]
fn lower_tiers_shrink_first_widest_first_down_to_their_minimum() {
    let tracks = [
        Track {
            shrink: 1,
            ..content(12)
        },
        Track::fixed(5),
        content(20),
        content(10),
    ];
    // 12+5+20+10 plus three gaps is 50: the widest shrinks to the next
    // widest, then (as in `tmt ls`) the leftmost of equals gives way first.
    assert_eq!(widths(&tracks, 40), [Some(12), Some(5), Some(10), Some(10)]);
    assert_eq!(widths(&tracks, 34), [Some(12), Some(5), Some(4), Some(10)]);
    // Tier 0 at its minimum: tier 1 gives way next; fixed never does.
    assert_eq!(widths(&tracks, 25), [Some(9), Some(5), Some(4), Some(4)]);
}

#[test]
fn columns_step_aside_by_priority_once_minimums_do_not_fit() {
    let tracks = [
        Track::fixed(10),
        Track {
            priority: Some(1),
            ..Track::fixed(8)
        },
        Track {
            priority: Some(2),
            ..Track::fixed(6)
        },
        Track {
            grow: 1,
            ..content(4)
        },
    ];
    assert_eq!(widths(&tracks, 31), [Some(10), Some(8), Some(6), Some(4)]);
    // The highest priority number goes first; the rest are laid out again
    // and the growing column takes the freed width.
    assert_eq!(widths(&tracks, 30), [Some(10), Some(8), None, Some(10)]);
    assert_eq!(widths(&tracks, 16), [Some(10), None, None, Some(5)]);
    // Columns without a priority never step aside, even when over.
    assert_eq!(widths(&tracks, 5), [Some(10), None, None, Some(4)]);
}

#[test]
fn equal_priorities_step_aside_from_the_right() {
    let tracks = [
        Track {
            priority: Some(1),
            ..Track::fixed(5)
        },
        Track {
            priority: Some(1),
            ..Track::fixed(5)
        },
    ];
    assert_eq!(widths(&tracks, 8), [Some(5), None]);
}

#[test]
fn leftover_width_goes_to_growing_columns_by_share_up_to_their_maximum() {
    let tracks = [
        Track {
            grow: 1,
            ..content(4)
        },
        Track {
            grow: 3,
            ..content(4)
        },
        Track::fixed(2),
    ];
    // 4+4+2+2 gaps = 12; 20 leaves 8: 2 and 6.
    assert_eq!(widths(&tracks, 20), [Some(6), Some(10), Some(2)]);
    let capped = [
        Track {
            grow: 1,
            max: Some(5),
            ..content(4)
        },
        Track {
            grow: 1,
            ..content(4)
        },
    ];
    assert_eq!(widths(&capped, 20), [Some(5), Some(14)]);
    // Rounding remainders go one cell at a time from the left.
    let even = [
        Track {
            grow: 1,
            min: 0,
            ..content(1)
        },
        Track {
            grow: 1,
            min: 0,
            ..content(1)
        },
    ];
    assert_eq!(widths(&even, 6), [Some(3), Some(2)]);
}

#[test]
fn a_span_covers_its_shown_columns_and_the_gaps_between_them() {
    let solved = [Some(10), None, Some(6), Some(4)];
    assert_eq!(span(&solved, 0..4, 2), 10 + 6 + 4 + 2 * 2);
    assert_eq!(span(&solved, 1..2, 2), 0);
    assert_eq!(span(&solved, 2..3, 2), 6);
}

#[test]
fn fit_pads_by_alignment_and_cuts_at_the_end_or_the_middle() {
    assert_eq!(fit("ab", 5, Align::Left, Truncate::End), "ab   ");
    assert_eq!(fit("ab", 5, Align::Right, Truncate::End), "   ab");
    assert_eq!(fit("ab", 5, Align::Center, Truncate::End), " ab  ");
    assert_eq!(fit("abcdefgh", 5, Align::Left, Truncate::End), "abcd…");
    assert_eq!(fit("abcdefgh", 5, Align::Left, Truncate::Middle), "ab…gh");
    assert_eq!(fit("abcdefgh", 6, Align::Left, Truncate::Middle), "abc…gh");
    assert_eq!(fit("abc", 1, Align::Left, Truncate::End), "…");
    assert_eq!(fit("abc", 0, Align::Left, Truncate::End), "");
}

#[test]
fn fit_measures_display_cells_so_wide_characters_line_up() {
    assert_eq!(fit("中文", 4, Align::Left, Truncate::End), "中文");
    assert_eq!(fit("中文", 6, Align::Right, Truncate::End), "  中文");
    // A wide character never straddles the cut: the cell is padded instead.
    assert_eq!(fit("中文字", 4, Align::Left, Truncate::End), "中… ");
    assert_eq!(fit("中文字", 4, Align::Left, Truncate::Middle), "中 …");
    assert_eq!(fit("🚀🚀🚀", 5, Align::Left, Truncate::End), "🚀🚀…");
    for text in ["中文字", "a中b文c", "🚀x🚀y", "plain text"] {
        for width in 0..12 {
            for truncate in [Truncate::End, Truncate::Middle] {
                let fitted = fit(text, width, Align::Left, truncate);
                assert_eq!(fitted.width(), width, "{text:?} at {width} ({truncate:?})");
            }
        }
    }
}

/// A small deterministic generator: the property runs the same cases every time.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, bound: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        ((self.0 >> 33) % bound as u64) as usize
    }
}

#[test]
fn solved_rows_never_exceed_the_width_unless_nothing_more_can_give_way() {
    let mut random = Lcg(517);
    for _ in 0..2_000 {
        let tracks: Vec<Track> = (0..1 + random.next(8))
            .map(|_| {
                let min = random.next(6);
                Track {
                    basis: random.next(40),
                    min,
                    max: (random.next(3) == 0).then(|| min + random.next(30)),
                    grow: random.next(3) as u16,
                    shrink: random.next(3) as u8,
                    priority: (random.next(2) == 0).then(|| random.next(4) as u16),
                }
            })
            .collect();
        let available = random.next(120);
        let gap = random.next(3);
        let solved = solve(&tracks, Some(available), gap);
        assert_eq!(
            solved,
            solve(&tracks, Some(available), gap),
            "deterministic"
        );
        let used = span(&solved, 0..solved.len(), gap);
        if used > available {
            // Over only when every shown column is at its minimum and none
            // may step aside.
            for (track, width) in tracks.iter().zip(&solved) {
                let Some(width) = width else { continue };
                assert!(track.priority.is_none(), "{tracks:?} at {available}");
                assert_eq!(*width, track.min, "{tracks:?} at {available}");
            }
        }
        for (track, width) in tracks.iter().zip(&solved) {
            let Some(width) = width else { continue };
            assert!(*width >= track.min, "never below a minimum");
            if let Some(max) = track.max {
                assert!(*width <= max.max(track.min), "never above a maximum");
            }
        }
    }
}

#[test]
fn fit_escapes_control_characters_before_measuring() {
    for (raw, shown) in [
        ("a\u{1b}[31mb", "a\\u{1b}[31mb"),
        ("a\rb", "a\\rb"),
        ("a\tb", "a\\tb"),
        ("a\u{2028}b", "a\\u{2028}b"),
    ] {
        assert_eq!(fit(raw, 20, Align::Left, Truncate::End).trim_end(), shown);
        let cut = fit(raw, 5, Align::Left, Truncate::End);
        assert_eq!(cut.width(), 5, "{raw:?}");
        assert!(!cut.chars().any(|c| c.is_control()), "{raw:?}: {cut:?}");
        // Idempotent: fitting already escaped text changes nothing more.
        assert_eq!(fit(shown, 20, Align::Left, Truncate::End).trim_end(), shown);
    }
}
