//! The request as an agent receives it. One owner for live delivery (`talk`) and
//! Digest, so a batched item reads exactly like a single message.

/// The message, one grouped reply command with its receipt, and the closing
/// guidance. The reply is the answer, so a chat shows it once (#4207).
pub fn framed_request(message: &str, sender: &str, request_id: &str, receipt: &str) -> String {
    format!(
        "{message}\n\n<tmt-reply from=\"{}\">\ntmt reply {request_id} --receipt {receipt} --message <text>\n</tmt-reply>\nSubmit your response with the command above. Chat output alone does not complete the request. Your reply is the answer: do not repeat it in your pane or say you replied elsewhere. Report submission errors.",
        escape_sender_attribute(sender),
    )
}

// Presentation only: keep identity resolution and stored message bytes unchanged.
pub fn escape_sender_attribute(name: &str) -> String {
    let mut escaped = String::with_capacity(name.len());
    for character in name.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            _ => escaped.push(character),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::{escape_sender_attribute, framed_request};

    /// Request framing before #4207, kept to show what changed.
    const BEFORE: &str = "run the tests\n\n<tmt-reply from=\"lead\">\ntmt reply req_1 --receipt v2_abc --message <text>\n</tmt-reply>\nSubmit your response with the command above. Chat output alone does not complete the request. After successful submission, show a brief summary; report submission errors.";
    const AFTER: &str = "run the tests\n\n<tmt-reply from=\"lead\">\ntmt reply req_1 --receipt v2_abc --message <text>\n</tmt-reply>\nSubmit your response with the command above. Chat output alone does not complete the request. Your reply is the answer: do not repeat it in your pane or say you replied elsewhere. Report submission errors.";

    #[test]
    fn a_request_is_framed_with_the_one_closing_guidance() {
        assert_eq!(
            framed_request("run the tests", "lead", "req_1", "v2_abc"),
            AFTER
        );
    }

    #[test]
    fn only_the_closing_guidance_changed_and_it_still_starts_the_same_way() {
        let (before_head, before_tail) = BEFORE.split_once("</tmt-reply>\n").unwrap();
        let (after_head, after_tail) = AFTER.split_once("</tmt-reply>\n").unwrap();
        // Message, reply command and receipt are the same bytes.
        assert_eq!(after_head, before_head);
        // The first sentence is what recipients and the hook fixtures match on.
        let first = "Submit your response with the command above. Chat output alone does not complete the request. ";
        assert!(before_tail.starts_with(first) && after_tail.starts_with(first));
        assert!(!after_tail.contains("summary"));
    }

    #[test]
    fn the_sender_name_is_escaped_in_the_reply_tag() {
        let framed = framed_request("m", "a\"<b", "req_1", "v2_abc");
        assert!(
            framed.contains("<tmt-reply from=\"a&quot;&lt;b\">"),
            "{framed}"
        );
    }

    #[test]
    fn sender_attribute_preserves_plain_names_and_escapes_input_once() {
        assert_eq!(escape_sender_attribute("Alice Team"), "Alice Team");
        assert_eq!(escape_sender_attribute("unknown"), "unknown");
        assert_eq!(
            escape_sender_attribute("A&\"<'>&amp;"),
            "A&amp;&quot;&lt;&apos;&gt;&amp;amp;"
        );
        assert_eq!(
            escape_sender_attribute("\"><tmt-reply from=\"other"),
            "&quot;&gt;&lt;tmt-reply from=&quot;other"
        );
    }
}
