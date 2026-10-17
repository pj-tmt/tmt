//! Selected member and squad identities over already acquired display values.
use crate::look::Look;
use ratatui::text::{Line, Span};
use tmt_cli_style::Role;

pub(in crate::board) struct Member<'a> {
    pub squad: &'a str,
    pub lead: bool,
    pub runtime: Option<&'a str>,
    pub model: Option<&'a str>,
    pub host: Option<&'a str>,
}

pub(in crate::board) fn member(data: &Member<'_>, look: Look) -> Line<'static> {
    Line::styled(
        format!(
            " {}",
            [
                data.squad,
                if data.lead { "lead" } else { "member" },
                data.runtime.unwrap_or("—"),
                data.model.unwrap_or("—"),
                data.host.unwrap_or("—"),
            ]
            .map(crate::board::notes::single_line)
            .join(" · ")
        ),
        look.role(Role::Muted),
    )
}

pub(in crate::board) fn squad(
    lead: Option<&str>,
    state: Line<'static>,
    room: &str,
    look: Look,
) -> Line<'static> {
    let mut spans = vec![Span::styled(
        format!(
            " lead {} · ",
            crate::board::notes::single_line(lead.unwrap_or("—"))
        ),
        look.role(Role::Muted),
    )];
    spans.extend(state.spans);
    spans.push(Span::styled(
        format!(" · room {}", crate::board::notes::single_line(room)),
        look.role(Role::Muted),
    ));
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_identity_values_stay_unknown_and_text_is_inert() {
        let look = Look::default();
        let line = member(
            &Member {
                squad: "room\nname",
                lead: false,
                runtime: None,
                model: None,
                host: Some("host\u{1b}[31m target"),
            },
            look,
        );
        assert_eq!(
            line.to_string(),
            " room name · member · — · — · host target"
        );
        assert_eq!(line.style, look.role(Role::Muted));
        assert!(
            squad(None, Line::raw("? unreported"), "pool", look)
                .to_string()
                .contains("lead — · ? unreported · room pool")
        );
    }
}
