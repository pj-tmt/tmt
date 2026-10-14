//! Exact source selection and quote provenance. Display escaping never changes
//! stored byte ranges; selecting or attaching a quote acknowledges no request.
use super::home_leads::MessageKey;
use std::ops::Range;
use tmt_tui::components::Editor;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Quote {
    pub key: MessageKey,
    pub range: Range<usize>,
    pub text: String,
}
impl Quote {
    pub fn label(&self) -> String {
        let request: String = self
            .key
            .request
            .trim_start_matches("req_")
            .chars()
            .take(8)
            .collect();
        format!(
            "quote {} · bytes {}..{}",
            request, self.range.start, self.range.end
        )
    }
    pub fn reference(&self) -> String {
        format!(
            "[Response source: {}; identity {}; bytes {}..{}]\n{}\n[End response source]",
            self.key.request, self.key.lead, self.range.start, self.range.end, self.text
        )
    }
}

pub(super) struct Selection {
    pub key: MessageKey,
    pub source: Editor,
}
impl Selection {
    pub fn new(key: MessageKey, source: String) -> Self {
        let mut source = Editor::from(source);
        source.key(
            ratatui::crossterm::event::KeyEvent::new(
                ratatui::crossterm::event::KeyCode::Home,
                ratatui::crossterm::event::KeyModifiers::CONTROL,
            ),
            80,
            usize::MAX,
        );
        Self { key, source }
    }
    pub fn quote(&self) -> Option<Quote> {
        let range = self.source.selection();
        (!range.is_empty()).then(|| Quote {
            key: self.key.clone(),
            text: self.source.as_str()[range.clone()].to_owned(),
            range,
        })
    }
}

impl super::app::App {
    pub(super) fn reader_event(
        &mut self,
        event: &ratatui::crossterm::event::Event,
    ) -> super::app::Effect {
        use super::app::{Effect, Request, RowTarget};
        use ratatui::crossterm::event::{Event, KeyCode, MouseButton, MouseEventKind};
        if matches!(event, Event::Key(key) if matches!(key.code, KeyCode::Esc | KeyCode::Char('q' | 'v')))
        {
            self.row_details.reader = None;
            self.reconcile_row_details();
            return Effect::None;
        }
        let Some(target) = self
            .row_details
            .reader
            .as_ref()
            .map(|reader| reader.target.clone())
        else {
            return Effect::None;
        };
        if matches!(event, Event::Key(key) if key.code == KeyCode::Char('s') && key.modifiers.is_empty())
        {
            let reply = self
                .reader_reply()
                .filter(|reply| reply.acquired && reply.source);
            if let Some(reply) = reply
                && let Some(key) = reply.key.clone()
            {
                let reader = self.row_details.reader.as_mut().unwrap();
                reader.selection = Some(Selection::new(key, reply.body.clone()));
                reader.frozen = Some(reply);
                *reader.scroll.borrow_mut() = Default::default();
            } else {
                self.notice = Some("Wait for the exact response source before selecting.".into());
            }
            return Effect::None;
        }
        let quote = self
            .row_details
            .reader
            .as_ref()
            .and_then(|reader| reader.selection.as_ref())
            .and_then(Selection::quote);
        if matches!(event, Event::Key(key) if key.code == KeyCode::Char('y') && key.modifiers.is_empty())
        {
            return quote.map_or(Effect::None, |quote| {
                Effect::Act(Request::Copy {
                    text: quote.text,
                    program: self.view.as_ref().and_then(|view| view.clipboard.clone()),
                })
            });
        }
        if matches!(event, Event::Key(key) if key.code == KeyCode::Char('a') && key.modifiers.is_empty())
        {
            let Some(quote) = quote else {
                self.notice = Some("Select a response range first.".into());
                return Effect::None;
            };
            let current = self
                .detail(&target)
                .and_then(|detail| detail.reply)
                .is_some_and(|reply| {
                    reply.acquired
                        && reply.source
                        && reply
                            .key
                            .as_ref()
                            .is_some_and(|key| key.same_source(&quote.key))
                        && reply.body.get(quote.range.clone()) == Some(quote.text.as_str())
                });
            if !current {
                self.notice =
                    Some("The response source changed; reopen it before annotating.".into());
                return Effect::None;
            }
            let super::row_detail::Target::Row(target) = target else {
                return Effect::None;
            };
            let squad = match &target {
                RowTarget::Workspace(_) => self.target_squad(&target).unwrap_or_default(),
                RowTarget::Member { squad, .. } | RowTarget::Lead { squad, .. } => squad.clone(),
            };
            if squad.is_empty() {
                self.notice = Some(
                    "Notes require a squad context; use Talk for this Common pool identity.".into(),
                );
                return Effect::None;
            }
            let Some(send) = self.send_target(target, true) else {
                self.notice = Some("The agent is no longer available; quote not attached.".into());
                return Effect::None;
            };
            self.row_details.reader = None;
            let effect = self.compose_row(send, crate::action::Verb::Annotate, squad);
            if let Some(input) = &mut self.input {
                let bytes = input
                    .response_quotes
                    .iter()
                    .map(|quote| quote.text.len())
                    .sum::<usize>();
                if bytes.saturating_add(quote.text.len()) > super::app::MESSAGE_LIMIT {
                    self.notice =
                        Some("Quote exceeds the 1 MiB attachment limit; not attached.".into());
                } else {
                    input.response_quotes.push(quote);
                }
            }
            return effect;
        }
        let reader = self.row_details.reader.as_mut().unwrap();
        if let Some(selection) = &mut reader.selection {
            let area = reader.source_area.get().unwrap_or_default();
            match event {
                Event::Key(key)
                    if matches!(
                        key.code,
                        KeyCode::Left
                            | KeyCode::Right
                            | KeyCode::Up
                            | KeyCode::Down
                            | KeyCode::Home
                            | KeyCode::End
                            | KeyCode::Char('a')
                    ) =>
                {
                    // Plain a is handled above; Ctrl-A selects all. No character
                    // insertion or deletion reaches a read-only response.
                    selection.source.key(*key, area.width, usize::MAX);
                    let row = selection.source.layout(area.width).cursor.0;
                    reader.scroll.borrow_mut().reveal(row..row + 1);
                    return Effect::None;
                }
                Event::Mouse(mouse)
                    if area.contains((mouse.column, mouse.row).into())
                        && matches!(
                            mouse.kind,
                            MouseEventKind::Down(MouseButton::Left)
                                | MouseEventKind::Drag(MouseButton::Left)
                        ) =>
                {
                    selection.source.place(
                        area.width,
                        reader.scroll.borrow().offset() + usize::from(mouse.row - area.y),
                        usize::from(mouse.column - area.x),
                        matches!(mouse.kind, MouseEventKind::Drag(_))
                            || mouse
                                .modifiers
                                .contains(ratatui::crossterm::event::KeyModifiers::SHIFT),
                    );
                    return Effect::None;
                }
                _ => {}
            }
        }
        reader.scroll.borrow_mut().input(event);
        Effect::None
    }
}
