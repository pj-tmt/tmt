//! Which binding operations select the caller's host (#4449). A
//! caller-scoped operation stops with `CLEANUP_ERROR` before it opens a
//! store when that selection fails to clean up; an operation on a name never
//! selects it. The selection is injected, and the store is private.

use super::{Report, run};
use crate::{
    caller_context::testing::{Private, Selections, composed},
    invocation::{Invocation, ListScope},
};

fn caller_scoped() -> Vec<Invocation> {
    vec![
        Invocation::Whoami,
        Invocation::Unbind,
        Invocation::Bind {
            name: "alice".into(),
            save: false,
            pane: None,
        },
        Invocation::BindMarked {
            name: "alice".into(),
            save: false,
        },
        Invocation::List {
            target: None,
            room: None,
            scope: ListScope {
                here: true,
                ..ListScope::default()
            },
        },
    ]
}

#[test]
fn a_caller_scoped_operation_whose_selection_fails_cleanup_stops_before_any_store() {
    for request in caller_scoped() {
        let label = format!("{request:?}");
        let private = Private::new("binding-caller-scoped");
        let selections = Selections::default();
        let failure = composed(&selections, &private, |composition| {
            run(request, composition).err()
        })
        .unwrap_or_else(|| panic!("{label}: a cleanup failure"));
        assert_eq!(failure.status, 1, "{label}");
        assert_eq!(
            failure.document()["error"]["code"],
            "CLEANUP_ERROR",
            "{label}"
        );
        assert_eq!(selections.calls(), 1, "{label}");
        assert!(!private.paths().database.exists(), "{label}: no store");
    }
}

#[test]
fn an_operation_on_a_name_never_selects_the_caller_host() {
    let named = [
        Invocation::Remove {
            name: "ghost".into(),
            force: false,
        },
        Invocation::Rename {
            old: "ghost".into(),
            new: "phantom".into(),
        },
        Invocation::List {
            target: Some("ghost".into()),
            room: None,
            scope: ListScope::default(),
        },
    ];
    for request in named {
        let label = format!("{request:?}");
        let private = Private::new("binding-named");
        let selections = Selections::default();
        let failure = composed(&selections, &private, |composition| {
            run(request, composition).err()
        })
        .unwrap_or_else(|| panic!("{label}: the name is missing"));
        assert_eq!(failure.code, "NAME_NOT_FOUND", "{label}");
        assert_eq!(selections.calls(), 0, "{label}");
    }
}

/// The built default, an open question for tmt-lead: a global listing
/// selects the caller's host only to list the caller's server first, so a
/// failed selection lists without that order.
#[test]
fn a_global_listing_whose_selection_fails_cleanup_still_lists() {
    let private = Private::new("binding-global-list");
    let selections = Selections::default();
    let listed = composed(&selections, &private, |composition| {
        run(
            Invocation::List {
                target: None,
                room: None,
                scope: ListScope::default(),
            },
            composition,
        )
    });
    assert!(
        matches!(&listed, Ok((Report::Listed { rows, .. }, None)) if rows.is_empty()),
        "an empty listing"
    );
    assert_eq!(selections.calls(), 1);
}
