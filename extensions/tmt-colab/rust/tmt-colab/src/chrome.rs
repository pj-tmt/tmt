//! Static guidance uses the same token values and stylesheet as React chrome,
//! including when no app build is available. No inline style exemption is needed.
use serde_json::Value;
use std::sync::OnceLock;

pub const STYLESHEET_PATH: &str = "/assets/chrome.css";

pub fn stylesheet() -> &'static str {
    static CSS: OnceLock<String> = OnceLock::new();
    CSS.get_or_init(|| {
        let tokens: Value = serde_json::from_str(include_str!(
            "../../../../../design/tokens/tokens.json"
        ))
        .expect("checked-in design tokens");
        let colors = |mode: &str| {
            ["color", "surface"]
                .into_iter()
                .flat_map(|group| tokens[group].as_object().expect("token group"))
                .map(|(name, value)| {
                    format!("--c-{name}:{};", value[mode].as_str().expect("color"))
                })
                .collect::<String>()
        };
        let fonts = tokens["font"]
            .as_object()
            .expect("font tokens")
            .iter()
            .map(|(name, value)| {
                format!("--f-{name}:{};", value["stack"].as_str().expect("font"))
            })
            .collect::<String>();
        let header = &tokens["colab"]["header"];
        let metrics = header
            .as_object()
            .expect("header tokens")
            .iter()
            .map(|(name, value)| {
                format!(
                    "--colab-header-{name}:{};",
                    value.as_str().expect("header metric")
                )
            })
            .collect::<String>();
        format!(
            ":root{{color-scheme:light;{}{fonts}{metrics}}}\
             @media(prefers-color-scheme:dark){{:root:not([data-theme=\"light\"]){{color-scheme:dark;{}}}}}\
             :root[data-theme=\"dark\"]{{color-scheme:dark;{}}}\
             @media(max-width:{}){{:root{{--colab-header-height:{};}}}}\n{}\n{}\n{}",
            colors("light"),
            colors("dark"),
            colors("dark"),
            header["compact-max-width"].as_str().expect("breakpoint"),
            header["compact-height"].as_str().expect("compact height"),
            include_str!("../../../typescript/app/src/colab-header.css"),
            include_str!("../../../typescript/app/src/reader-style.css"),
            include_str!("../../../typescript/app/src/notice-card.css"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guidance_stylesheet_contains_the_real_token_contract() {
        let tokens: Value =
            serde_json::from_str(include_str!("../../../../../design/tokens/tokens.json")).unwrap();
        let css = stylesheet();
        for group in ["color", "surface"] {
            for (name, value) in tokens[group].as_object().unwrap() {
                for mode in ["light", "dark"] {
                    assert!(
                        css.contains(&format!("--c-{name}:{};", value[mode].as_str().unwrap()))
                    );
                }
            }
        }
        for (name, value) in tokens["font"].as_object().unwrap() {
            assert!(css.contains(&format!("--f-{name}:{};", value["stack"].as_str().unwrap())));
        }
        let header = &tokens["colab"]["header"];
        for (name, value) in header.as_object().unwrap() {
            assert!(css.contains(&format!(
                "--colab-header-{name}:{};",
                value.as_str().unwrap()
            )));
        }
        assert!(css.contains(&format!(
            "@media(max-width:{}){{:root{{--colab-header-height:{};}}}}",
            header["compact-max-width"].as_str().unwrap(),
            header["compact-height"].as_str().unwrap()
        )));
        assert!(css.contains(":root[data-theme=\"dark\"]{color-scheme:dark;"));
        assert!(css.contains(".colab-header"));
        assert!(css.contains(".notice"));
        assert!(std::ptr::eq(css, stylesheet()));
    }
}
