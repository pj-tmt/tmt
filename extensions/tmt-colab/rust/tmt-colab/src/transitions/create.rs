//! Root-local creation: isolated Yjs preparation, then one owner commit.
use super::{Engine, epoch, membership, request::OwnerContext};
use crate::{
    Result,
    decoder::BaselineInput,
    fold,
    keyring::Keyring,
    page,
    store::{
        Envelope, Namespace, Store, StreamScope,
        owner::{Device, Mutation, OwnerFault},
    },
};
use tmt_colab_model::{crypto, object, values, wrap};

pub(super) struct Selection<'a> {
    pub creator: Option<&'a serde_json::Value>,
    pub template: Option<&'a str>,
    /// The page of the user template `template` names; `None` for a compiled one.
    pub template_page: Option<&'a str>,
    pub shell: bool,
    pub page: &'a str,
    pub title: &'a str,
    pub source: &'a str,
    pub publisher_agent: Option<&'a str>,
    pub creation_recipient: Option<&'a crate::decoder::CreationRecipient>,
    /// The page's chosen layout; a dashboard has none.
    pub layout: Option<&'a crate::decoder::PageLayout>,
}
struct Prepared {
    /// The page's first state as ordered updates, each small enough for every reader.
    updates: Vec<Vec<u8>>,
    secret: [u8; 32],
}
impl Drop for Prepared {
    fn drop(&mut self) {
        self.secret.fill(0);
        self.updates.iter_mut().for_each(|u| u.fill(0));
    }
}
impl Engine {
    pub(super) fn create_page(
        &mut self,
        store: &mut Store,
        key: &Keyring,
        context: OwnerContext<'_>,
        selection: Selection<'_>,
        now: u64,
    ) -> Result<(Vec<u8>, bool)> {
        let Selection {
            creator,
            template,
            template_page,
            shell,
            page,
            title,
            source,
            publisher_agent,
            creation_recipient,
            layout,
        } = selection;
        values::generated_id(context.id)?;
        values::generated_id(page)?;
        values::time(now)?;
        if context.scope.is_some()
            || creation_recipient.is_some_and(|v| !v.valid())
            || layout.is_some_and(|v| !v.valid())
            || (shell && layout.is_some())
            || title.is_empty()
            || publisher_agent.is_some_and(|v| !crate::decoder::valid_publisher_agent(v))
        {
            return Err(OwnerFault::Invalid.into());
        }
        if title.len() > crate::decoder::BASELINE_TITLE_BYTES
            || source.len() > crate::decoder::BASELINE_BYTES
        {
            return Err(OwnerFault::Capacity.into());
        }
        let mut attribution = serde_json::json!({"writerId":key.local_writer()?.0,"timeMs":now});
        if let Some(name) = publisher_agent {
            attribution["displayAgent"] = serde_json::json!({"name":name});
        }
        let creator_selection = creator;
        let creator = creator
            .map(|body| {
                crate::question_registry::set_creator(body, &attribution)
                    .map_err(|_| OwnerFault::Invalid)
            })
            .transpose()?;
        // A user template's page is read inside the transition, from its folded state, so what a
        // copy registers is the template's own and never a client's claim.
        if template_page.is_some() && template.is_none() {
            return Err(OwnerFault::Invalid.into());
        }
        let questions = if let Some(template) = template.filter(|_| template_page.is_none()) {
            if !crate::templates::admits(
                template,
                source,
                creator
                    .as_ref()
                    .and_then(|creator| creator["agent"]["name"].as_str()),
            ) {
                return Err(OwnerFault::Invalid.into());
            }
            let report = crate::page_check::check(source).map_err(|_| OwnerFault::Capacity)?;
            if report.question_sources.len() != report.questions.len() {
                return Err(OwnerFault::Invalid.into());
            }
            let mut questions = serde_json::json!([]);
            for question in report.questions {
                let mut body = serde_json::json!({"id":question.id,"title":question.title});
                if let Some(creator) = &creator {
                    body["agent"] = creator["agent"].clone();
                }
                if let Some(group) = question.group {
                    body["group"] = serde_json::json!(group);
                }
                questions = crate::question_registry::apply(
                    &questions,
                    "question.add",
                    &body,
                    &attribution,
                )
                .map_err(|_| OwnerFault::Invalid)?;
            }
            if questions
                .as_array()
                .is_some_and(|questions| questions.is_empty())
            {
                None
            } else {
                Some(questions)
            }
        } else {
            None
        };
        // A page created from a template records which one (the name; a user template adds its
        // revision), and nothing else: it is independent of the template from then on.
        let from_template = template
            .filter(|_| template_page.is_none())
            .map(|name| {
                crate::template_marks::from_template(name, None).map_err(|_| OwnerFault::Invalid)
            })
            .transpose()?;
        if template_page.is_none() {
            let mut meta = serde_json::json!({});
            if let Some(creator) = &creator {
                meta["creator"] = creator.clone();
            }
            if let Some(questions) = &questions {
                meta["questions"] = questions.clone();
            }
            crate::question_registry::admit_source(source, &meta, page, None)?;
        }
        let mut selection = serde_json::json!({"pageId":page,"title":title,"source":source});
        if let Some(creator) = creator_selection {
            selection["creator"] = creator.clone();
        }
        if let Some(template) = template {
            selection["template"] = serde_json::json!(template);
        }
        if let Some(template_page) = template_page {
            values::generated_id(template_page)?;
            selection["templatePage"] = serde_json::json!(template_page);
        }
        if let Some(agent) = publisher_agent {
            selection["publisherAgent"] = serde_json::json!(agent);
        }
        if let Some(recipient) = creation_recipient {
            selection["creationRecipient"] = serde_json::to_value(recipient)?;
        }
        if let Some(layout) = layout {
            selection["layout"] = serde_json::to_value(layout)?;
        }
        if shell {
            if !source.is_empty() {
                return Err(OwnerFault::Invalid.into());
            }
            selection["pageType"] = serde_json::json!("shell");
        }
        let digest = context.digest(key, "page.create", &selection)?;
        self.run_transition(
            store,
            key,
            Mutation {
                operation_id: context.id,
                digest,
                expected_revision: context.expected,
            },
            |engine, store| {
                store.owner_read(&key.space_id, &key.owner_public(), |tx| {
                    if tx.head().map_or(0, |h| h.revision) != context.expected {
                        return Err(OwnerFault::StaleHead.into());
                    }
                    if tx.page_epoch(page)?.is_some() {
                        return Err(OwnerFault::Conflict.into());
                    }
                    Ok(())
                })?;
                // A user template starts the page with a registry copied from the template's own
                // and a mark of the revision it was copied at.
                let (questions, from_template) = match (template, template_page) {
                    (Some(name), Some(template_page)) => {
                        let (questions, from) = engine.copy_user_template(
                            store,
                            key,
                            UserTemplate {
                                page: template_page,
                                name,
                            },
                            source,
                            creator.as_ref(),
                            &attribution,
                        )?;
                        let mut meta = serde_json::json!({});
                        if let Some(creator) = &creator {
                            meta["creator"] = creator.clone();
                        }
                        if let Some(questions) = &questions {
                            meta["questions"] = questions.clone();
                        }
                        crate::question_registry::admit_source(source, &meta, page, None)?;
                        (questions, Some(from))
                    }
                    _ => (questions.clone(), from_template.clone()),
                };
                let input = BaselineInput {
                    page_state: None,
                    creator: creator.as_ref(),
                    questions: questions.as_ref(),
                    template: None,
                    from_template: from_template.as_ref(),
                    work: None,
                    original_author: publisher_agent,
                    layout,
                    attachments: None,
                    source: source.as_bytes(),
                    title,
                    publisher_agent,
                    creation_recipient,
                    source_digest: crypto::digest(source.as_bytes()),
                };
                let baseline = if shell {
                    engine.decoder(page)?.produce_shell(
                        input,
                        &crate::shell::empty(),
                        true,
                        None,
                    )?
                } else {
                    engine.decoder(page)?.produce_page(input, None)?
                };
                let mut secret = [0; 32];
                getrandom::fill(&mut secret)?;
                Ok(Some(Prepared {
                    updates: baseline.chunks,
                    secret,
                }))
            },
            |tx, prepared| {
                let mut statements = Vec::new();
                if let Some(genesis) = super::initialize_owner(tx, key)? {
                    statements.push(genesis);
                }
                tx.create_page(page)?;
                let shared = key.sign_statement(
                    tx.head(),
                    "page.share",
                    &serde_json::to_vec(
                        &serde_json::json!({"pageId":page,"mode":"private","epoch":"1"}),
                    )?,
                )?;
                tx.append_statement(&shared)?;
                statements.push(shared);
                tx.put_epoch_secret(page, 1, &prepared.secret)?;
                let (states, _) = fold::verify_log(&tx.log()?, key, page)?;
                let authority = states.last().ok_or(OwnerFault::Invalid)?;
                let devices = tx.devices()?;
                let targets = epoch::targets(tx, &devices, key, authority, page, now)?;
                if targets.len() > crate::limits::OWNER_WRAPS {
                    return Err(OwnerFault::Capacity.into());
                }
                let mut wraps = Vec::new();
                for ((kind, id), recipient_key) in targets {
                    let wrapped = key.seal_wrap(
                        &wrap::Header {
                            space: key.space_id.clone(),
                            page: page.into(),
                            epoch: "1".into(),
                            recipient_kind: kind,
                            recipient_id: id,
                            recipient_key,
                            signer_key: key.owner_public(),
                            membership_revision: authority.head.revision.to_string(),
                        },
                        &prepared.secret,
                    )?;
                    tx.put_wrap(&wrapped)?;
                    wraps.push(wrapped);
                }
                let writer = key.local_writer()?.0;
                if authority.revoked_devices.contains(&writer)
                    || tx.device(&writer)?.is_some_and(|d| d.revoked)
                {
                    return Err(page::Fault::Denied.into());
                }
                let issuer = tx.statement(1)?.ok_or(OwnerFault::Invalid)?.hash()?;
                let chain = page::writer_chain(key, &authority.head, &issuer, now)?;
                tx.put_device(&Device {
                    chain,
                    revoked: false,
                })?;
                let mut previous = [0; 32];
                for (index, update) in prepared.updates.iter().enumerate() {
                    let seq = index as u64 + 1;
                    let envelope = key.seal_content(
                        &object::Context {
                            space: key.space_id.clone(),
                            page: page.into(),
                            epoch: "1".into(),
                            kind: "update".into(),
                            namespace: "content".into(),
                            author_device: writer.clone(),
                            membership_revision: authority.head.revision.to_string(),
                            stream_seq: seq.to_string(),
                            prev_hash: previous,
                        },
                        &prepared.secret,
                        update,
                    )?;
                    let hash = envelope.hash()?;
                    tx.append_update(&Envelope {
                        scope: StreamScope {
                            page,
                            epoch: 1,
                            stream: &writer,
                        },
                        namespace: Namespace::Content,
                        seq,
                        hash,
                        previous,
                        bytes: &envelope.to_json()?,
                    })?;
                    previous = hash;
                }
                membership::outcome(&statements, wraps)
            },
        )
    }
}

/// The user template a page is being created from.
struct UserTemplate<'a> {
    page: &'a str,
    name: &'a str,
}
impl Engine {
    /// What a page created from a user template starts with, read from the template page's current
    /// folded state: the registry of its questions (each routed to the creator and open, with no
    /// task or history) and the mark of where it came from. The page's source must be exactly the
    /// template's source with the agents of its blocks set to the creator; the template must still
    /// be the page named, and an ordinary page or a dashboard is never one.
    fn copy_user_template(
        &mut self,
        store: &Store,
        key: &Keyring,
        template: UserTemplate<'_>,
        source: &str,
        creator: Option<&serde_json::Value>,
        attribution: &serde_json::Value,
    ) -> Result<(Option<serde_json::Value>, serde_json::Value)> {
        values::generated_id(template.page)?;
        let snapshot = page::snapshot(store, key, template.page, false)?;
        let view = snapshot.materialize(key, template.page, self.decoder(template.page)?)?;
        let marked = crate::template_marks::mark_of_meta(&view.meta);
        if view.meta.get("pageType").is_some()
            || marked.map(|(name, _)| name) != Some(template.name)
        {
            return Err(OwnerFault::Invalid.into());
        }
        let agent = creator.and_then(|creator| creator["agent"]["name"].as_str());
        if !crate::templates::admits_user(&view.source, source, agent) {
            return Err(OwnerFault::Invalid.into());
        }
        let mut questions = serde_json::json!([]);
        for entry in view
            .meta
            .get("questions")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
        {
            // A copy is routed to its creator, or to nobody when it has none (the page's creator
            // answers for it later); nothing of the template's own routing, state or history comes.
            let mut body = serde_json::json!({"id": entry["id"]});
            if let Some(creator) = creator {
                body["agent"] = creator["agent"].clone();
            }
            for field in ["title", "group"] {
                if let Some(value) = entry.get(field) {
                    body[field] = value.clone();
                }
            }
            questions =
                crate::question_registry::apply(&questions, "question.add", &body, attribution)
                    .map_err(|_| OwnerFault::Invalid)?;
        }
        let revision = crate::template_marks::revision_of(&view.source);
        let from = crate::template_marks::from_template(template.name, Some(&revision))
            .map_err(|_| OwnerFault::Invalid)?;
        let questions = questions
            .as_array()
            .is_some_and(|entries| !entries.is_empty())
            .then_some(questions);
        Ok((questions, from))
    }
}
