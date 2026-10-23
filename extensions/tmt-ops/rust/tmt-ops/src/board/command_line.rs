//! Board aliases for the shared literal command display and PATH resolver.
pub(super) use crate::launch::{display_executable as executable_word, line as command_line};

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn words_are_literal_with_only_shell_sensitive_values_quoted() {
        assert_eq!(
            command_line(&[
                "tmt",
                "resume",
                "--",
                "22222222-2222-4222-8222-222222222222"
            ]),
            "tmt resume -- 22222222-2222-4222-8222-222222222222"
        );
        assert_eq!(
            command_line(&[
                "/opt/tmt bin/tmt",
                "resume",
                "--",
                "worker's $(value); name",
                ""
            ]),
            "'/opt/tmt bin/tmt' resume -- 'worker'\\''s $(value); name' ''"
        );
    }

    #[test]
    fn executable_label_obeys_path_order_and_symlink_identity() {
        let root =
            std::env::temp_dir().join(format!("ops-command-{}", crate::id::new_v4().unwrap()));
        std::fs::create_dir(&root).unwrap();
        struct Root(std::path::PathBuf);
        impl Drop for Root {
            fn drop(&mut self) {
                std::fs::remove_dir_all(&self.0).unwrap();
            }
        }
        let _cleanup = Root(root.clone());
        let selected = root.join("selected");
        let other = root.join("other");
        let linked = root.join("linked");
        for directory in [&selected, &other, &linked] {
            std::fs::create_dir(directory).unwrap();
        }
        for directory in [&selected, &other] {
            let file = directory.join("tmt");
            std::fs::write(&file, "fixture").unwrap();
            std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let executable = selected.join("tmt");
        std::os::unix::fs::symlink(&executable, linked.join("tmt")).unwrap();
        let path = std::env::join_paths([&linked, &other]).unwrap();
        assert_eq!(executable_word(&executable, Some(&path)), "tmt");
        let path = std::env::join_paths([&other, &linked]).unwrap();
        assert_eq!(
            executable_word(&executable, Some(&path)),
            executable.to_string_lossy()
        );
        std::fs::set_permissions(other.join("tmt"), std::fs::Permissions::from_mode(0o600))
            .unwrap();
        assert_eq!(
            executable_word(&executable, Some(&path)),
            "tmt",
            "non-executable entries are skipped"
        );
        assert_eq!(
            executable_word(&executable, None),
            executable.to_string_lossy()
        );
    }
}
