use super::*;
use crate::{
    action::Verb,
    config::{BindingLocation, OpsKeyScope},
};
use serde_json::json;

struct Fixture {
    root: PathBuf,
    path: PathBuf,
}
impl Fixture {
    fn new(text: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("ops-host-binding-{}", crate::id::new_v4().unwrap()));
        fs::create_dir(&root).unwrap();
        let path = root.join("ops.toml");
        fs::write(&path, text).unwrap();
        Self { root, path }
    }
    fn config(&self) -> Config {
        Config::read(self.path.clone()).unwrap()
    }
    fn text(&self) -> String {
        fs::read_to_string(&self.path).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn preview_is_disposable_and_save_preserves_comments_other_keys_and_reload() {
    let original =
        "# retained\n[bind]\nj = 'jump' # host\no = 'open {pr_link}'\nunknown = 'opaque'\n";
    // Unknown events would be rejected by Config, so opaque data belongs at the root.
    let original = original.replace("unknown = 'opaque'\n", "").replacen(
        "# retained\n",
        "# retained\nunknown = 'opaque'\n",
        1,
    );
    let f = Fixture::new(&original);
    let mut config = f.config();
    let draft = config
        .preview_binding(&BindingLocation::Global, "j", Some("swap left"))
        .unwrap();
    assert_eq!(f.text(), original);
    assert_eq!(
        draft
            .workspace_keys(OpsKeyScope::Member, None, None)
            .unwrap()
            .bindings["j"]
            .verb,
        Verb::Swap
    );
    assert!(
        config
            .set_binding(&BindingLocation::Global, "j", Some("swap left"))
            .unwrap()
    );
    assert_eq!(
        f.text(),
        original.replace("j = 'jump'", r#"j = "swap left""#)
    );
    let saved = f.config();
    assert_eq!(
        saved
            .workspace_keys(OpsKeyScope::Reader, None, None)
            .unwrap()
            .bindings["j"]
            .text,
        "swap left"
    );
    assert_eq!(
        saved.configured_bindings().unwrap()["o"].text,
        "open {pr_link}"
    );
}

#[test]
fn editor_changes_only_the_winning_host_or_exact_section() {
    let text = "[bind]\nj='jump'\no='open'\n[bind.tmux]\nj='run echo {pane}'\n[[squad.product.section]]\ntitle='All'\n[squad.product.section.bind]\nj='swap right'\n";
    let f = Fixture::new(text);
    let row = json!({"id":"uuid", "name":"alpha", "fields":{}, "pane":{"id":"%1","location":{"host":"tmux"}}});
    let mut config = f.config();
    let locations = config
        .binding_locations(Some(&row), Some(&row), Some("product"))
        .unwrap();
    let crate::config::ResolvedBinding { action, owner, .. } = config
        .binding(
            OpsKeyScope::Reader,
            &locations,
            "j",
            &crate::config::workspace_defaults(OpsKeyScope::Reader),
        )
        .unwrap();
    assert_eq!(action.unwrap().text, "swap right");
    let owner = owner.unwrap();
    assert_eq!(
        owner,
        BindingLocation::Section {
            squad: "product".into(),
            index: 0
        }
    );
    config.set_binding(&owner, "j", Some("swap down")).unwrap();
    assert_eq!(config.configured_bindings().unwrap()["j"].text, "jump");
    assert_eq!(
        config.host_bindings().unwrap()["tmux"]["j"].text,
        "run echo {pane}"
    );
    assert_eq!(
        config.sections("product").unwrap()[0].bind["j"].text,
        "swap down"
    );
    let locations = config
        .binding_locations(Some(&row), Some(&row), None)
        .unwrap();
    let crate::config::ResolvedBinding { owner, .. } = config
        .binding(
            OpsKeyScope::Member,
            &locations,
            "j",
            &crate::config::workspace_defaults(OpsKeyScope::Reader),
        )
        .unwrap();
    let owner = owner.unwrap();
    assert_eq!(owner, BindingLocation::Host("tmux".into()));
    config.set_binding(&owner, "j", Some("swap up")).unwrap();
    assert_eq!(
        config.sections("product").unwrap()[0].bind["j"].text,
        "swap down"
    );
    assert_eq!(config.configured_bindings().unwrap()["o"].text, "open");
}

#[test]
fn invalid_or_concurrent_edits_keep_the_file_and_in_memory_binding() {
    let f = Fixture::new("[bind]\nj='jump'\n");
    let mut config = f.config();
    for (location, event, line) in [
        (BindingLocation::Global, "j", "swap somewhere"),
        (BindingLocation::Global, "j", "jump lead"),
        (BindingLocation::Global, "esc", "open {link}"),
        (BindingLocation::Global, "ctrl-c", "swap left"),
        (
            BindingLocation::Section {
                squad: "missing".into(),
                index: 0,
            },
            "j",
            "swap left",
        ),
    ] {
        assert!(
            config
                .preview_binding(&location, event, Some(line))
                .is_err(),
            "{event}: {line}"
        );
        assert_eq!(f.text(), "[bind]\nj='jump'\n");
    }
    fs::write(&f.path, "# external edit\n[bind]\nj='jump'\n").unwrap();
    assert_eq!(
        config
            .set_binding(&BindingLocation::Global, "j", Some("swap left"))
            .unwrap_err()
            .code,
        "SQUAD_CONFIG_CHANGED"
    );
    assert_eq!(config.configured_bindings().unwrap()["j"].text, "jump");
    assert!(f.text().starts_with("# external edit"));
}

#[test]
fn the_opener_is_a_global_argv_with_url_as_one_word() {
    let f = Fixture::new("# kept\n");
    let mut config = f.config();
    assert!(config.can_edit_setting("opener", None));
    assert!(!config.can_edit_setting("opener", Some("product")));
    config
        .set_setting(
            None,
            "opener",
            r#"["firefox","--new-tab={url}","my profile"]"#,
        )
        .unwrap();
    assert_eq!(
        f.config().program("opener").unwrap().unwrap(),
        ["firefox", "--new-tab={url}", "my profile"]
    );
    assert!(f.text().contains("# kept"), "{}", f.text());
    let shown = f.config().settings(None, false, None).unwrap();
    let entry = shown.entries.iter().find(|e| e.key == "opener").unwrap();
    assert_eq!(
        entry.value,
        json!(["firefox", "--new-tab={url}", "my profile"])
    );
    assert!(entry.editable);
}

#[test]
fn bad_openers_are_refused_before_anything_is_written() {
    let f = Fixture::new("");
    let mut config = f.config();
    for text in [
        "",
        "[]",
        r#"["bin/open","{url}"]"#,
        r#"["open", 1]"#,
        r#"[""]"#,
        "open {url}",
    ] {
        assert!(
            config.set_setting(None, "opener", text).is_err(),
            "{text:?}"
        );
    }
    assert_eq!(f.text(), "");
}

#[test]
fn punctuation_section_reset_reveals_host_without_touching_global_or_other_sections() {
    assert_eq!(BindingLocation::Global.label("."), "bind.\".\"");
    assert_eq!(
        BindingLocation::Host("tmux".into()).label("/"),
        "bind.tmux.\"/\""
    );
    let f = Fixture::new(
        "[bind]\n\".\"='copy {name}'\n[bind.tmux]\n\".\"='copy {id}'\n[[squad.product.section]]\ntitle='All'\n[squad.product.section.bind]\n\".\"='copy {pane}' # kept\n\"/\"='search'\n",
    );
    let mut config = f.config();
    let section = BindingLocation::Section {
        squad: "product".into(),
        index: 0,
    };
    config
        .set_binding(&section, ".", Some("copy {row:/fields/task}"))
        .unwrap();
    assert!(f.text().contains("# kept"));
    assert_eq!(
        config.sections("product").unwrap()[0].bind["."].text,
        "copy {row:/fields/task}"
    );
    config.set_binding(&section, ".", None).unwrap();
    let row = json!({"name":"worker", "pane":{"location":{"host":"tmux"}}});
    let locations = config
        .binding_locations(Some(&row), Some(&row), Some("product"))
        .unwrap();
    assert_eq!(
        config
            .binding(
                OpsKeyScope::Member,
                &locations,
                ".",
                &crate::config::workspace_defaults(OpsKeyScope::Member)
            )
            .unwrap()
            .action
            .unwrap()
            .text,
        "copy {id}"
    );
    assert_eq!(
        config.sections("product").unwrap()[0].bind["/"].text,
        "search"
    );
    assert_eq!(
        config.configured_bindings().unwrap()["."].text,
        "copy {name}"
    );
}
