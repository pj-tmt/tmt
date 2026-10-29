//! The templates of one space (#5066): the compiled ones and the pages marked as templates. A
//! selector (a name, or a page ID or a unique prefix of one) finds one; the space's default is what
//! `page create` starts from when none is named. Marks are read through `template_index`, so a
//! lookup folds only the pages whose progress moved. A refusal carries the fix its CLI prints.
use crate::{
    Result,
    decoder::{Decoder, PageLayout},
    inspection,
    keyring::{Keyring, Layout},
    page, short_links,
    store::Store,
    template_index::{self, Marks},
    template_marks,
    templates::{self, Template},
};
use serde_json::Value;
use std::collections::BTreeMap;

/// A refusal that names its fix: the CLI prints the message as `error:` and the hint as `hint:`.
#[derive(Debug)]
pub struct Refusal {
    code: &'static str,
    message: String,
    hint: Option<String>,
}
/// A refusal for the caller to return.
pub fn refusal(code: &'static str, message: String, hint: Option<String>) -> Box<Refusal> {
    Refusal::new(code, message, hint)
}

/// A create re-read a different template before preparing or committing any new page.
pub fn changed() -> Box<Refusal> {
    refusal(
        "COLAB_TEMPLATE_CHANGED",
        "The template changed while the page was being created; nothing was created.".into(),
        Some("Retry the same page create command to use the current template.".into()),
    )
}

impl Refusal {
    fn new(code: &'static str, message: String, hint: Option<String>) -> Box<Self> {
        Box::new(Self {
            code,
            message,
            hint,
        })
    }
    pub fn code(&self) -> &'static str {
        self.code
    }
    pub fn hint(&self) -> Option<&str> {
        self.hint.as_deref()
    }
}
impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for Refusal {}

/// A page marked as a template.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct User {
    /// The template's ID is its page's ID.
    pub page: String,
    pub name: String,
    pub description: String,
    /// The page's layout kind (`freeform` when it names none).
    pub layout: String,
    pub author: Option<String>,
}

/// One template, compiled or a page of the space.
pub enum Found {
    BuiltIn(&'static Template),
    User(User),
}
impl Found {
    pub fn name(&self) -> &str {
        match self {
            Self::BuiltIn(template) => template.name,
            Self::User(user) => &user.name,
        }
    }
}

/// The user templates of a space and how many pages each name started.
pub struct Catalog {
    pages: Vec<String>,
    /// By name, then page: two pages carrying one name (a sync race) are both kept, so a lookup can
    /// refuse with both IDs instead of choosing.
    user: BTreeMap<String, Vec<User>>,
    used: BTreeMap<String, usize>,
}

/// Every template mark of the space, folding only the pages whose progress moved since the last scan.
pub fn catalog(
    layout: &Layout,
    store: &Store,
    key: &Keyring,
    decoder: &mut Decoder,
) -> Result<Catalog> {
    let pages = inspection::all_page_ids(store, key)?;
    let marks = template_index::scan_with_live(layout, store, key, decoder, &pages, Some(&pages))?;
    Ok(Catalog::from_marks(pages, marks))
}

impl Catalog {
    /// A space without templates of its own: where there is no space yet.
    pub fn empty() -> Self {
        Self::from_marks(Vec::new(), BTreeMap::new())
    }

    fn from_marks(pages: Vec<String>, marks: BTreeMap<String, Marks>) -> Self {
        let mut user: BTreeMap<String, Vec<User>> = BTreeMap::new();
        let mut used = BTreeMap::new();
        for (page, marks) in marks {
            if let Some(from) = marks.from {
                *used.entry(from.name).or_insert(0) += 1;
            }
            if let Some(template) = marks.template {
                user.entry(template.name.clone()).or_default().push(User {
                    page,
                    name: template.name,
                    description: template.description,
                    layout: template.layout,
                    author: template.author,
                });
            }
        }
        Self { pages, user, used }
    }

    /// The user templates by name.
    pub fn user_templates(&self) -> impl Iterator<Item = &User> {
        self.user.values().flatten()
    }

    /// How many pages were created from the template of this name.
    pub fn used_by(&self, name: &str) -> usize {
        self.used.get(name).copied().unwrap_or(0)
    }

    /// The short form of a page ID that no other page of the space shares.
    pub fn short_id<'a>(&self, page: &'a str) -> &'a str {
        short_links::shortest_id(page, &self.pages)
    }

    /// The user template of this name, and none when no page carries it. Two pages carrying one
    /// name refuse with both IDs: no silent choice.
    pub fn user_named(&self, name: &str) -> Result<Option<&User>> {
        match self.user.get(name).map(Vec::as_slice) {
            None | Some([]) => Ok(None),
            Some([only]) => Ok(Some(only)),
            Some(several) => {
                let ids: Vec<&str> = several
                    .iter()
                    .map(|user| self.short_id(&user.page))
                    .collect();
                Err(Refusal::new(
                    "COLAB_TEMPLATE_AMBIGUOUS",
                    format!("{} names {} pages: {}", name, ids.len(), ids.join(", ")),
                    Some(format!("tmt colab template show {}", ids[0])),
                ))
            }
        }
    }

    /// The user template that a page ID, or a unique prefix of one, names.
    fn user_page(&self, selector: &str) -> Result<Option<&User>> {
        let matched = short_links::matches(selector, &self.pages);
        let users: Vec<&User> = matched
            .iter()
            .filter_map(|id| self.user_templates().find(|user| user.page == *id))
            .collect();
        match (matched.as_slice(), users.as_slice()) {
            ([], _) => Ok(None),
            ([only], []) => Err(Refusal::new(
                "COLAB_TEMPLATE_UNKNOWN",
                format!("Page {} is not a template", self.short_id(only)),
                Some(format!(
                    "tmt colab template save {} --name NAME",
                    self.short_id(only)
                )),
            )),
            (_, [only]) => Ok(Some(only)),
            _ => Err(Refusal::new(
                "COLAB_TEMPLATE_AMBIGUOUS",
                format!("{selector} names more than one page"),
                Some("tmt colab template ls".to_owned()),
            )),
        }
    }

    /// The template a selector names: a compiled name, a user template's name, or a template page's
    /// ID or unique prefix of one.
    pub fn find(&self, selector: &str) -> Result<Found> {
        if let Some(template) = templates::find(selector) {
            return Ok(Found::BuiltIn(template));
        }
        if let Some(user) = self.user_named(selector)? {
            return Ok(Found::User(user.clone()));
        }
        if let Some(user) = self.user_page(selector)? {
            return Ok(Found::User(user.clone()));
        }
        Err(unknown(selector))
    }
}

/// `No template named NAME`, with the list that shows the names.
pub fn unknown(selector: &str) -> Box<Refusal> {
    Refusal::new(
        "COLAB_TEMPLATE_UNKNOWN",
        format!("No template named {selector}"),
        Some("tmt colab template ls".to_owned()),
    )
}

/// A name for `template save` or a rename: its grammar, not a compiled name, and not another page's
/// template. `page` is the page being saved, whose own name may be saved again.
pub fn admit_name(catalog: &Catalog, name: &str, page: &str) -> Result<()> {
    if template_marks::is_built_in(name) {
        return Err(Refusal::new(
            "COLAB_TEMPLATE_NAME_RESERVED",
            format!("{name} is a built-in template name"),
            Some(format!(
                "tmt colab template save {} --name {name}-ops",
                catalog.short_id(page)
            )),
        ));
    }
    if !template_marks::valid_name(name) {
        return Err(Refusal::new(
            "COLAB_TEMPLATE_NAME_INVALID",
            format!(
                "{name:?} is not a template name: use 1 to {} lowercase letters, digits and hyphens",
                template_marks::NAME_CHARS
            ),
            Some(format!(
                "tmt colab template save {} --name weekly-review",
                catalog.short_id(page)
            )),
        ));
    }
    if let Some(other) = catalog
        .user
        .get(name)
        .into_iter()
        .flatten()
        .find(|user| user.page != page)
    {
        return Err(Refusal::new(
            "COLAB_TEMPLATE_NAME_TAKEN",
            format!(
                "{name} is already page {}'s template",
                catalog.short_id(&other.page)
            ),
            Some(format!(
                "tmt colab template remove {name}, or choose another --name"
            )),
        ));
    }
    Ok(())
}

/// What `page create` starts from when no template is named, and the default it fell back from
/// when the space's chosen default is gone.
pub struct Default {
    pub found: Found,
    /// The setting named a template that no longer exists: the selector that did not resolve.
    pub gone: Option<String>,
}

/// The space's default template: its setting, else TMT's `starter`. A setting that names a template
/// that is gone (its mark removed, its page deleted) is not a failure: the create starts from
/// `starter` and says so.
pub fn default(catalog: &Catalog, setting: Option<&str>) -> Result<Default> {
    let starter = || Found::BuiltIn(templates::find("starter").expect("starter is compiled"));
    let Some(selector) = setting else {
        return Ok(Default {
            found: starter(),
            gone: None,
        });
    };
    match catalog.find(selector) {
        Ok(found) => Ok(Default { found, gone: None }),
        Err(error)
            if error
                .downcast_ref::<Refusal>()
                .is_some_and(|refusal| refusal.code() == "COLAB_TEMPLATE_UNKNOWN") =>
        {
            Ok(Default {
                found: starter(),
                gone: Some(selector.to_owned()),
            })
        }
        Err(error) => Err(error),
    }
}

/// A page read for a template's use: its source and what a page created from it starts with.
pub struct Content {
    pub source: String,
    /// `None` for a page that names no layout (a freeform page).
    pub layout: Option<PageLayout>,
    /// The page's revision token, for a write that must find the page as it was read.
    pub revision: String,
    /// The page's content metadata, for the registry and the marks.
    pub meta: Value,
    pub title: String,
}

/// The current content of a page. A dashboard has no source of its own to start a page from, so it
/// is refused as not being a template.
pub fn read_page(
    store: &Store,
    key: &Keyring,
    decoder: &mut Decoder,
    page_id: &str,
) -> Result<Content> {
    let snapshot = page::snapshot(store, key, page_id, false)?;
    let view = snapshot.materialize(key, page_id, decoder)?;
    if view.meta.get("pageType").is_some() {
        return Err(Refusal::new(
            "COLAB_TEMPLATE_UNSUPPORTED",
            "A dashboard cannot be a template".to_owned(),
            None,
        ));
    }
    Ok(Content {
        revision: page::token(
            &key.space_id,
            page_id,
            &snapshot.authority.head,
            snapshot.epoch,
            &snapshot.cuts,
        )?,
        layout: crate::decoder::layout_of_meta(&view.meta),
        source: view.source,
        title: view.title,
        meta: view.meta,
    })
}

/// The current content of a user template's page. A page that is not (or no longer) that template,
/// because its mark was removed or changed since the catalog was read, is refused: a stale catalog
/// never starts a page from an ordinary one.
pub fn read_user(
    store: &Store,
    key: &Keyring,
    decoder: &mut Decoder,
    user: &User,
) -> Result<Content> {
    let content = read_page(store, key, decoder, &user.page)?;
    match template_marks::mark_of_meta(&content.meta) {
        Some((name, _)) if name == user.name => Ok(content),
        _ => Err(unknown(&user.name)),
    }
}

/// The rules of a template page: its HTML comments in document order, each on one line. They are
/// the instructions the page's author wrote for the agent that fills a copy of it.
pub fn rules(source: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = source;
    while let Some(start) = rest.find("<!--") {
        let after = &rest[start + 4..];
        let Some(end) = after.find("-->") else { break };
        let text = after[..end]
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if !text.is_empty() {
            found.push(text);
        }
        rest = &after[end + 3..];
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(page: &str, name: &str) -> User {
        User {
            page: page.into(),
            name: name.into(),
            description: "d".into(),
            layout: "column".into(),
            author: None,
        }
    }
    fn catalog_of(users: &[User], uses: &[(&str, usize)]) -> Catalog {
        let mut user = BTreeMap::<String, Vec<User>>::new();
        for one in users {
            user.entry(one.name.clone()).or_default().push(one.clone());
        }
        Catalog {
            pages: users.iter().map(|one| one.page.clone()).collect(),
            user,
            used: uses.iter().map(|(name, n)| ((*name).into(), *n)).collect(),
        }
    }
    const A: &str = "3f9b02c7-0000-4000-8000-000000000001";
    const B: &str = "7a1c40e2-0000-4000-8000-000000000002";
    const C: &str = "9c2d11aa-0000-4000-8000-000000000003";

    fn code(error: Box<dyn std::error::Error + Send + Sync>) -> (&'static str, String) {
        let refusal = error.downcast::<Refusal>().expect("a template refusal");
        (
            refusal.code(),
            refusal.hint().unwrap_or_default().to_owned(),
        )
    }

    #[test]
    fn a_selector_is_a_compiled_name_a_user_name_or_a_template_page() {
        let catalog = catalog_of(&[user(A, "weekly-review"), user(B, "design-proposal")], &[]);
        assert!(matches!(catalog.find("report"), Ok(Found::BuiltIn(t)) if t.name == "report"));
        assert!(matches!(catalog.find("weekly-review"), Ok(Found::User(u)) if u.page == A));
        assert!(
            matches!(catalog.find("3f9b02c7"), Ok(Found::User(u)) if u.name == "weekly-review")
        );
        assert!(matches!(catalog.find(A), Ok(Found::User(u)) if u.page == A));
        assert_eq!(
            code(catalog.find("weekly").err().unwrap()),
            ("COLAB_TEMPLATE_UNKNOWN", "tmt colab template ls".into())
        );
    }

    #[test]
    fn a_page_that_is_not_a_template_is_named_as_such_with_the_command_that_makes_it_one() {
        let mut catalog = catalog_of(&[user(A, "weekly-review")], &[]);
        catalog.pages.push(C.into());
        let (code, hint) = code(catalog.find("9c2d11aa").err().unwrap());
        assert_eq!(code, "COLAB_TEMPLATE_UNKNOWN");
        assert!(hint.starts_with("tmt colab template save 9c2d11aa --name"));
    }

    #[test]
    fn two_pages_with_one_name_are_refused_with_both_ids_and_never_chosen_between() {
        let catalog = catalog_of(&[user(A, "weekly-review"), user(B, "weekly-review")], &[]);
        let error = catalog.find("weekly-review").err().unwrap();
        let message = error.to_string();
        assert!(
            message.contains("3f9b02c7") && message.contains("7a1c40e2"),
            "{message}"
        );
        assert_eq!(code(error).0, "COLAB_TEMPLATE_AMBIGUOUS");
        // Either page is still reachable by its ID.
        assert!(matches!(catalog.find("7a1c40e2"), Ok(Found::User(u)) if u.page == B));
    }

    #[test]
    fn a_name_is_admitted_unless_it_is_reserved_malformed_or_another_pages() {
        let mut catalog = catalog_of(&[user(A, "weekly-review")], &[]);
        catalog.pages.push(C.into());
        assert!(admit_name(&catalog, "notes", C).is_ok());
        // Saving a page again under its own name is allowed.
        assert!(admit_name(&catalog, "weekly-review", A).is_ok());
        let (code_, hint) = code(admit_name(&catalog, "report", C).err().unwrap());
        assert_eq!(code_, "COLAB_TEMPLATE_NAME_RESERVED");
        assert_eq!(hint, "tmt colab template save 9c2d11aa --name report-ops");
        assert_eq!(
            code(admit_name(&catalog, "Weekly", C).err().unwrap()).0,
            "COLAB_TEMPLATE_NAME_INVALID"
        );
        let error = admit_name(&catalog, "weekly-review", C).err().unwrap();
        assert_eq!(
            error.to_string(),
            "weekly-review is already page 3f9b02c7's template"
        );
        assert_eq!(
            code(error).1,
            "tmt colab template remove weekly-review, or choose another --name"
        );
    }

    #[test]
    fn the_default_is_the_setting_and_falls_back_to_starter_when_it_is_gone() {
        let catalog = catalog_of(&[user(A, "weekly-review")], &[]);
        let none = default(&catalog, None).unwrap();
        assert_eq!((none.found.name(), none.gone), ("starter", None));
        let set = default(&catalog, Some(A)).unwrap();
        assert_eq!((set.found.name(), set.gone), ("weekly-review", None));
        let gone = default(&catalog, Some("7a1c40e2-0000-4000-8000-0000000000ff")).unwrap();
        assert_eq!(gone.found.name(), "starter");
        assert_eq!(
            gone.gone.as_deref(),
            Some("7a1c40e2-0000-4000-8000-0000000000ff")
        );
        let builtin = default(&catalog, Some("report")).unwrap();
        assert_eq!(builtin.found.name(), "report");
    }

    #[test]
    fn rules_are_the_html_comments_in_order_on_one_line_each() {
        let source = "<!-- layout: column.\n  Keep it. -->\n<p>x</p><!--  -->\n<!-- Header: week -->tail<!-- open";
        assert_eq!(rules(source), ["layout: column. Keep it.", "Header: week"]);
        assert!(rules("<p>no comments</p>").is_empty());
    }

    #[test]
    fn the_uses_of_a_name_count_the_pages_created_from_it() {
        let catalog = catalog_of(&[], &[("weekly-review", 4)]);
        assert_eq!(catalog.used_by("weekly-review"), 4);
        assert_eq!(catalog.used_by("report"), 0);
    }
}
