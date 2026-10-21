//! A task made into a page of its own, shared by `tmt colab task convert` and the serve's
//! browser `task.convert`: what a task may be converted, the first text of the page, and the page
//! ID a browser's operation derives, so one operation can never make two pages.
use crate::work_admission::no_such_task;
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Why a task cannot be converted; checked before any page is made.
#[derive(Debug, PartialEq, Eq)]
pub enum Blocked {
    /// The page has no such task.
    Missing,
    /// The task already follows a page.
    Follows,
    /// The task is tracked by an external issue, which a followed page excludes.
    External,
}

impl Blocked {
    /// The refusal sentence for `task_id`, the same for the command and the browser.
    pub fn message(&self, task_id: &str) -> String {
        match self {
            Blocked::Missing => no_such_task(task_id),
            Blocked::Follows => format!("{task_id} already follows a page."),
            Blocked::External => format!(
                "{task_id} has an external ID, so it cannot follow a page; clear the ID first (tmt colab task set)."
            ),
        }
    }
}

/// The task `task_id` of a work value, when it may be converted.
pub fn convertible<'a>(work: &'a Value, task_id: &str) -> Result<&'a Value, Blocked> {
    let task = work["tasks"].get(task_id).ok_or(Blocked::Missing)?;
    if task.get("child").is_some() {
        return Err(Blocked::Follows);
    }
    if task.get("externalId").is_some() {
        return Err(Blocked::External);
    }
    Ok(task)
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
/// The first text of a page made from a task: its title, then its detail in paragraphs. Nothing but
/// the task's own title and detail, escaped, ever goes into it.
pub fn child_source(title: &str, detail: &str) -> String {
    let mut html = format!("<h1>{}</h1>\n", escape_html(title));
    for paragraph in detail
        .split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        html.push_str(&format!(
            "<p>{}</p>\n",
            escape_html(paragraph).replace('\n', "<br>\n")
        ));
    }
    html
}

/// The page a browser's convert operation makes: a name-based ID over the space and the operation,
/// in the shape of a generated ID. The same operation always names the same page, so a lost reply
/// is answered by asking whether that page exists, and a replay finds its page instead of making
/// another. A new operation names a new page.
pub fn child_page_id(space: &str, operation_id: &str) -> String {
    named_id(b"tmt-colab/convert/v1\0", space, operation_id)
}
/// The ID the page creation itself is recorded under. It cannot be the browser's operation ID: the
/// link is recorded under that one, and the owner store keeps one outcome per operation ID.
pub fn create_operation_id(space: &str, operation_id: &str) -> String {
    named_id(b"tmt-colab/convert-create/v1\0", space, operation_id)
}
fn named_id(domain: &[u8], space: &str, operation_id: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(domain);
    hash.update(space.as_bytes());
    hash.update([0]);
    hash.update(operation_id.as_bytes());
    let digest = hash.finalize();
    let mut b = [0u8; 16];
    b.copy_from_slice(&digest[..16]);
    b[6] = (b[6] & 15) | 64;
    b[8] = (b[8] & 63) | 128;
    let h: String = b.iter().map(|v| format!("{v:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_task_converts_unless_it_is_missing_follows_a_page_or_has_an_external_id() {
        let work = json!({"tasks":{
            "T-1":{"title":"a","state":"todo"},
            "T-2":{"title":"b","state":"todo","child":"20000000-0000-4000-8000-000000000002"},
            "T-3":{"title":"c","state":"todo","externalId":"#7"}}});
        assert!(convertible(&work, "T-1").is_ok());
        assert_eq!(convertible(&work, "T-9"), Err(Blocked::Missing));
        assert_eq!(convertible(&work, "T-2"), Err(Blocked::Follows));
        assert_eq!(convertible(&work, "T-3"), Err(Blocked::External));
    }

    #[test]
    fn every_refusal_names_its_task_and_the_command_that_fixes_it() {
        assert_eq!(
            Blocked::Missing.message("T-9"),
            "This page has no task T-9; tmt colab task ls shows its tasks."
        );
        assert_eq!(
            Blocked::Follows.message("T-2"),
            "T-2 already follows a page."
        );
        assert_eq!(
            Blocked::External.message("T-3"),
            "T-3 has an external ID, so it cannot follow a page; clear the ID first (tmt colab task set)."
        );
    }

    #[test]
    fn the_first_text_is_the_escaped_title_and_detail_paragraphs() {
        assert_eq!(
            child_source("A <b> & \"c\"", "one\nline\n\n\n  two  "),
            "<h1>A &lt;b&gt; &amp; &quot;c&quot;</h1>\n<p>one<br>\nline</p>\n<p>two</p>\n"
        );
    }

    #[test]
    fn an_operation_names_one_page_in_the_shape_of_a_generated_id() {
        let space = "s".repeat(43);
        let op = "60000000-0000-4000-8000-000000000001";
        let page = child_page_id(&space, op);
        assert_eq!(page, child_page_id(&space, op));
        assert_ne!(
            page,
            child_page_id(&space, "60000000-0000-4000-8000-000000000002")
        );
        assert_ne!(page, child_page_id(&"t".repeat(43), op));
        tmt_colab_model::values::generated_id(&page).unwrap();
        assert_ne!(page, create_operation_id(&space, op));
    }
}
