// Decode only managed fixture launch options; every remaining argument is literal.
// Scripted peers do not execute the consumed hook configuration. The separate
// model-free channel peers execute their generated hooks instead.
pub(crate) fn scripted_launch_options(args: &mut Vec<std::ffi::OsString>) {
    if args.first().is_some_and(|arg| arg == "--no-daemon") {
        assert!(
            args.get(1).is_some_and(|arg| arg == "--add-dir"),
            "managed Codex must name its state root"
        );
        assert!(
            args.get(2)
                .is_some_and(|root| std::path::Path::new(root).is_absolute()),
            "managed Codex state root must be absolute"
        );
        args.drain(..3);
    }
    if args.len() < 2 {
        return;
    }
    let option = &args[args.len() - 2];
    if option == "--settings" {
        let value = args.last().unwrap().to_str().unwrap();
        let settings: serde_json::Value = serde_json::from_str(value).unwrap();
        assert!(settings.is_object(), "session settings must be an object");
    } else if option == "-c" {
        let value = args.last().unwrap().to_str().unwrap();
        let settings: toml_edit::DocumentMut = value.parse().unwrap();
        assert!(
            settings.get("hooks").is_some_and(|v| v.is_table_like()),
            "session override must contain hooks"
        );
    } else {
        return;
    }
    args.truncate(args.len() - 2);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_literal_arguments_include_non_utf8_bytes() {
        use std::os::unix::ffi::OsStringExt;
        let original = vec!["/script".into(), std::ffi::OsString::from_vec(vec![0xff])];
        let mut args = original.clone();
        scripted_launch_options(&mut args);
        assert_eq!(args, original);
    }

    #[test]
    fn malformed_managed_prefixes_and_hook_overrides_are_refused() {
        for raw in [
            vec!["--no-daemon"],
            vec!["--no-daemon", "--other", "/state"],
            vec!["--no-daemon", "--add-dir"],
            vec!["/script", "--settings", "{"],
            vec!["/script", "--settings", "[]"],
            vec!["/script", "-c", "hooks=["],
            vec!["/script", "-c", "model='wrong'"],
        ] {
            let mut args = raw.iter().map(std::ffi::OsString::from).collect();
            assert!(
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| scripted_launch_options(
                    &mut args
                )))
                .is_err(),
                "{raw:?}"
            );
        }
    }

    #[test]
    fn scripted_paths_survive_managed_codex_options_with_or_without_hooks_and_listen() {
        for listen in [false, true] {
            for hooks in [false, true] {
                let mut original: Vec<std::ffi::OsString> =
                    ["/cli", "/scenario", "/report"].map(Into::into).to_vec();
                if listen {
                    original.push("--listen".into());
                }
                let mut args: Vec<std::ffi::OsString> =
                    ["--no-daemon", "--add-dir", "/private-state"]
                        .map(Into::into)
                        .to_vec();
                args.extend(original.clone());
                if hooks {
                    args.extend(["-c".into(), "hooks={Stop=[]}".into()]);
                }
                scripted_launch_options(&mut args);
                assert_eq!(args, original, "listen={listen}, hooks={hooks}");
            }
        }
    }

    #[test]
    #[should_panic(expected = "managed Codex state root must be absolute")]
    fn scripted_managed_codex_refuses_an_invalid_state_root() {
        let mut args = [
            "--no-daemon",
            "--add-dir",
            "relative",
            "/cli",
            "/scenario",
            "/report",
        ]
        .map(Into::into)
        .to_vec();
        scripted_launch_options(&mut args);
    }

    #[test]
    fn scripted_unknown_arguments_are_not_silently_discarded() {
        let original = ["--unknown", "/cli", "/scenario", "/report"]
            .map(Into::into)
            .to_vec();
        let mut args = original.clone();
        scripted_launch_options(&mut args);
        assert_eq!(args, original);
    }

    #[test]
    fn scripted_paths_and_listen_survive_both_provider_launch_hook_options() {
        let original: Vec<std::ffi::OsString> = ["/cli", "/scenario", "/report", "--listen"]
            .map(Into::into)
            .to_vec();
        for (option, value) in [
            ("--settings", r#"{"hooks":{"Stop":[]}}"#),
            ("-c", "hooks={Stop=[]}"),
        ] {
            let mut args = original.clone();
            args.extend([option.into(), value.into()]);
            scripted_launch_options(&mut args);
            assert_eq!(args, original);
        }
    }

    #[test]
    fn scripted_paths_and_listen_survive_the_claude_settings() {
        let original: Vec<std::ffi::OsString> = ["/cli", "/scenario", "/report", "--listen"]
            .map(Into::into)
            .to_vec();
        let mut args = original.clone();
        args.extend(["--settings".into(), r#"{"hooks":{"Stop":[]}}"#.into()]);
        scripted_launch_options(&mut args);
        assert_eq!(args, original);
    }
}
