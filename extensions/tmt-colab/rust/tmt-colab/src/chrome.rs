//! Compile-time shared presentation and Colab-owned viewport/metric inputs.
//! Guidance stays styled without a Vite app build or a Node serving prerequisite.

pub const STYLESHEET_PATH: &str = "/assets/chrome.css";

pub fn stylesheet() -> &'static str {
    concat!(
        include_str!("../../../../../design/browser-ui/generated/static.css"),
        "\n",
        include_str!("../../../typescript/app/src/colab-header.css"),
        "\n",
        include_str!("../../../typescript/app/src/reader-style.css"),
        "\n",
        include_str!("../../../typescript/app/src/notice-card.css"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guidance_embeds_the_checked_shared_output_without_runtime_generation() {
        let shared = include_str!("../../../../../design/browser-ui/generated/static.css");
        let css = stylesheet();
        assert!(css.starts_with(shared));
        assert_eq!(css.matches(shared).count(), 1);
        assert!(css.contains(".tmt-ui-header"));
        assert!(css.contains(".tmt-ui-notice"));
        assert!(css.contains("--tmt-ui-host-action-padding"));
        assert!(!css.contains("@import"));
        assert!(!css.contains("url("));
        assert!(!css.contains(".colab-header"));
        assert!(std::ptr::eq(css, stylesheet()));
    }
}
