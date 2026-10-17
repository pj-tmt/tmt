use super::*;
use crate::{action::Verb, config::OpsKeyScope};
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
    let draft = config.preview_host_action("bind.j", "swap left").unwrap();
    assert_eq!(f.text(), original);
    assert_eq!(
        draft
            .workspace_keys(OpsKeyScope::Member, None, None)
            .unwrap()
            .bindings["j"]
            .verb,
        Verb::Swap
    );
    assert!(config.set_host_action("bind.j", "swap left").unwrap());
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
    let (action, source) = config
        .host_action_binding(OpsKeyScope::Reader, Some(&row), Some(&row), Some("product"))
        .unwrap();
    assert_eq!(action.text, "swap right");
    assert_eq!(source, "squad.product.section[0].bind.j");
    config.set_host_action(&source, "swap down").unwrap();
    assert_eq!(config.configured_bindings().unwrap()["j"].text, "jump");
    assert_eq!(
        config.host_bindings().unwrap()["tmux"]["j"].text,
        "run echo {pane}"
    );
    assert_eq!(
        config.sections("product").unwrap()[0].bind["j"].text,
        "swap down"
    );
    let (_, source) = config
        .host_action_binding(OpsKeyScope::Overview, Some(&row), Some(&row), None)
        .unwrap();
    assert_eq!(source, "bind.tmux.j");
    config.set_host_action(&source, "swap up").unwrap();
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
    for (source, line) in [
        ("bind.j", "swap somewhere"),
        ("bind.j", "jump lead"),
        ("bind.j", "open {link}"),
        ("bind.o", "swap left"),
        ("squad.missing.section[0].bind.j", "swap left"),
    ] {
        assert!(
            config.preview_host_action(source, line).is_err(),
            "{source}: {line}"
        );
        assert_eq!(f.text(), "[bind]\nj='jump'\n");
    }
    fs::write(&f.path, "# external edit\n[bind]\nj='jump'\n").unwrap();
    assert_eq!(
        config
            .set_host_action("bind.j", "swap left")
            .unwrap_err()
            .code,
        "SQUAD_CONFIG_CHANGED"
    );
    assert_eq!(config.configured_bindings().unwrap()["j"].text, "jump");
    assert!(f.text().starts_with("# external edit"));
}
