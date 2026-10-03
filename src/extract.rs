//! Choosing the part of an HTML page that is worth an agent's attention.
//!
//! The rules are deliberately fixed and explainable, not scored: remove
//! elements that are boilerplate by their own declaration (their tag, `role`
//! or hidden state), then keep the page's declared main content if it has one.
//! Pure functions over a parsed document; no network, no I/O.

use std::rc::Rc;

use htmd::Node;
use html5ever::serialize::{SerializeOpts, serialize};
use markup5ever_rcdom::{NodeData, SerializableHandle};

use crate::error::Error;
use crate::markdown::SKIPPED_TAGS;

/// Elements removed as boilerplate wherever they appear.
///
/// Navigation, page furniture and interactive controls. (`<header>` is removed
/// only outside `<main>`/`<article>`, where it is the site banner rather than
/// a title block.)
pub const BOILERPLATE_TAGS: [&str; 10] = [
    "nav", "footer", "aside", "dialog", "menu", "button", "input", "select", "textarea", "datalist",
];

/// ARIA roles removed as boilerplate.
pub const BOILERPLATE_ROLES: [&str; 11] = [
    "navigation",
    "banner",
    "contentinfo",
    "complementary",
    "search",
    "dialog",
    "alertdialog",
    "menu",
    "menubar",
    "toolbar",
    "tablist",
];

/// Words that mark an element as boilerplate when its `class` or `id` uses them.
///
/// Names are split on anything that is not a letter or digit, so
/// `site-sidebar` and `Sidebar_x1` both contain the word `sidebar`. See
/// [`Document::main_content`] for the guard that keeps layout wrappers such
/// as `<div class="has-sidebar">`.
pub const BOILERPLATE_WORDS: [&str; 24] = [
    "nav",
    "navbar",
    "navigation",
    "menu",
    "dropdown",
    "sidebar",
    "breadcrumb",
    "breadcrumbs",
    "footer",
    "cookie",
    "cookies",
    "consent",
    "advert",
    "advertisement",
    "ads",
    "promo",
    "newsletter",
    "subscribe",
    "share",
    "sharing",
    "social",
    "popup",
    "modal",
    "toolbar",
];

/// A parsed HTML document, or the part of one selected by [`Document::main_content`].
#[derive(Debug, Clone)]
pub struct Document {
    /// The whole parsed tree. It must outlive `root`: dropping a node empties
    /// every descendant, even ones that are still referenced.
    tree: Rc<Node>,
    root: Rc<Node>,
}

fn tag(node: &Node) -> Option<&str> {
    if let NodeData::Element { name, .. } = &node.data {
        Some(&name.local)
    } else {
        None
    }
}

fn attribute(node: &Node, key: &str) -> Option<String> {
    if let NodeData::Element { attrs, .. } = &node.data {
        attrs
            .borrow()
            .iter()
            .find(|a| &*a.name.local == key)
            .map(|a| a.value.to_string())
    } else {
        None
    }
}

fn has_role(node: &Node, roles: &[&str]) -> bool {
    attribute(node, "role").is_some_and(|role| {
        role.split_ascii_whitespace()
            .any(|r| roles.contains(&r.to_ascii_lowercase().as_str()))
    })
}

fn is_hidden(node: &Node) -> bool {
    if attribute(node, "hidden").is_some()
        || attribute(node, "aria-hidden").is_some_and(|v| v.eq_ignore_ascii_case("true"))
    {
        return true;
    }
    attribute(node, "style").is_some_and(|style| {
        let compact: String = style
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            .to_ascii_lowercase();
        compact.contains("display:none") || compact.contains("visibility:hidden")
    })
}

/// Whether headers inside this element belong to the content, not the site.
fn is_content_container(node: &Node) -> bool {
    matches!(tag(node), Some("main" | "article")) || has_role(node, &["main"])
}

/// Whether the element's `class` or `id` contains one of [`BOILERPLATE_WORDS`].
fn is_named_boilerplate(node: &Node) -> bool {
    if matches!(tag(node), Some("html" | "body" | "main" | "article")) {
        return false;
    }
    ["class", "id"]
        .iter()
        .filter_map(|key| attribute(node, key))
        .any(|names| {
            names
                .split(|c: char| !c.is_ascii_alphanumeric())
                .any(|word| BOILERPLATE_WORDS.contains(&word.to_ascii_lowercase().as_str()))
        })
}

fn text_chars(node: &Node) -> usize {
    let mut text = String::new();
    text_of(node, &mut text);
    text.chars().filter(|c| !c.is_whitespace()).count()
}

fn is_boilerplate(node: &Node, in_content: bool, page_chars: usize) -> bool {
    let Some(name) = tag(node) else {
        // Comments and processing instructions carry nothing readable.
        return matches!(
            node.data,
            NodeData::Comment { .. } | NodeData::ProcessingInstruction { .. }
        );
    };
    SKIPPED_TAGS.contains(&name)
        || BOILERPLATE_TAGS.contains(&name)
        || (name == "header" && !in_content)
        || has_role(node, &BOILERPLATE_ROLES)
        || is_hidden(node)
        || (name == "img" && attribute(node, "src").is_some_and(|src| src.starts_with("data:")))
        // A boilerplate name on something holding most of the page's text is a
        // layout wrapper ("has-sidebar"), not the boilerplate itself.
        || (is_named_boilerplate(node) && text_chars(node).saturating_mul(2) < page_chars)
}

fn children(node: &Node) -> Vec<Rc<Node>> {
    node.children.borrow().iter().map(Rc::clone).collect()
}

fn prune(node: &Node, in_content: bool, page_chars: usize) {
    let kept: Vec<Rc<Node>> = children(node)
        .into_iter()
        .filter(|child| !is_boilerplate(child, in_content, page_chars))
        .collect();
    for child in &kept {
        prune(child, in_content || is_content_container(child), page_chars);
    }
    *node.children.borrow_mut() = kept;
}

fn find(node: &Rc<Node>, matches: &dyn Fn(&Node) -> bool, found: &mut Vec<Rc<Node>>) {
    if matches(node) {
        found.push(Rc::clone(node));
    }
    for child in children(node) {
        find(&child, matches, found);
    }
}

fn find_all(node: &Rc<Node>, matches: &dyn Fn(&Node) -> bool) -> Vec<Rc<Node>> {
    let mut found = Vec::new();
    find(node, matches, &mut found);
    found
}

/// Collects readable text: scripts, styles and the other [`SKIPPED_TAGS`]
/// below `node` do not count.
fn text_of(node: &Node, out: &mut String) {
    if let NodeData::Text { contents } = &node.data {
        out.push_str(&contents.borrow());
    }
    for child in children(node) {
        if !tag(&child).is_some_and(|name| SKIPPED_TAGS.contains(&name)) {
            text_of(&child, out);
        }
    }
}

fn has_text(node: &Node) -> bool {
    let mut text = String::new();
    text_of(node, &mut text);
    !text.trim().is_empty()
}

impl Document {
    /// Parses an HTML document or fragment. HTML parsing never fails: broken
    /// markup is repaired the way a browser would.
    ///
    /// # Errors
    /// [`Error::PageContent`] if the parser cannot read the input.
    pub fn parse(html: &str) -> Result<Self, Error> {
        htmd::HtmlToMarkdown::new()
            .html_to_tree(html)
            .map(|tree| Self {
                root: Rc::clone(&tree),
                tree,
            })
            .map_err(|e| Error::PageContent(format!("cannot parse HTML: {e}")))
    }

    /// The root node: the whole document, or the selected content.
    #[must_use]
    pub const fn node(&self) -> &Rc<Node> {
        &self.root
    }

    /// The text of the first `<title>` element, with whitespace collapsed.
    #[must_use]
    pub fn title(&self) -> Option<String> {
        let title = find_all(&self.root, &|n| tag(n) == Some("title"))
            .into_iter()
            .next()?;
        let mut text = String::new();
        text_of(&title, &mut text);
        let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
        (!collapsed.is_empty()).then_some(collapsed)
    }

    /// Selects the content worth reading and discards the rest.
    ///
    /// 1. Non-content and boilerplate elements are removed everywhere: the
    ///    tags in [`SKIPPED_TAGS`] and [`BOILERPLATE_TAGS`], a `<header>`
    ///    outside `<main>`/`<article>`, the roles in [`BOILERPLATE_ROLES`],
    ///    hidden elements, comments and inline `data:` images. So are elements
    ///    whose `class` or `id` uses one of the [`BOILERPLATE_WORDS`], unless
    ///    they hold half or more of the page's text.
    /// 2. Of what is left, the first `<main>` (or `role="main"`) with text is
    ///    kept; failing that the only `<article>`; failing that the `<body>`.
    ///
    /// Removed elements are never brought back, so the result may be empty.
    #[must_use]
    pub fn main_content(self) -> Self {
        prune(&self.tree, false, text_chars(&self.tree));
        let with_text = |nodes: Vec<Rc<Node>>| nodes.into_iter().find(|n| has_text(n));
        let main = with_text(find_all(&self.tree, &|n| {
            tag(n) == Some("main") || has_role(n, &["main"])
        }));
        let root = main
            .or_else(|| {
                let articles = find_all(&self.tree, &|n| tag(n) == Some("article"));
                if articles.len() == 1 {
                    with_text(articles)
                } else {
                    None
                }
            })
            .or_else(|| {
                find_all(&self.tree, &|n| tag(n) == Some("body"))
                    .into_iter()
                    .next()
            })
            .unwrap_or_else(|| Rc::clone(&self.tree));
        Self {
            tree: self.tree,
            root,
        }
    }

    /// Serialises what is inside the root node back to HTML.
    ///
    /// # Errors
    /// [`Error::PageContent`] if serialisation fails.
    pub fn to_html(&self) -> Result<String, Error> {
        let mut bytes = Vec::new();
        let handle = SerializableHandle::from(Rc::clone(&self.root));
        serialize(&mut bytes, &handle, SerializeOpts::default())
            .map_err(|e| Error::PageContent(format!("cannot serialise HTML: {e}")))?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown;

    const PAGE: &str = r#"<!doctype html>
<html><head><title>
  Tom &amp; Jerry&#39;s   page </title>
<style>body { color: red }</style></head>
<body>
<!-- a comment -->
<header><a href="/">SiteLogo</a></header>
<nav><a href="/a">NavLink</a></nav>
<div role="navigation">RoleNav</div>
<div role="search"><input value="SearchBox"></div>
<main>
  <header><h1>Real Title</h1></header>
  <p>Body text with a <a href="https://a.example/x">link</a>.</p>
  <aside>RelatedStuff</aside>
  <div hidden>HiddenAttr</div>
  <div aria-hidden="true">AriaHidden</div>
  <div style="color: red; display: none">DisplayNone</div>
  <button>ClickMe</button>
  <img src="data:image/png;base64,AAAA" alt="InlineImage">
  <img src="/photo.png" alt="Photo">
  <script>track()</script>
</main>
<aside>Sidebar</aside>
<footer>FooterText</footer>
<div role="contentinfo">RoleFooter</div>
</body></html>"#;

    const BOILERPLATE: [&str; 15] = [
        "SiteLogo",
        "NavLink",
        "RoleNav",
        "SearchBox",
        "RelatedStuff",
        "HiddenAttr",
        "AriaHidden",
        "DisplayNone",
        "ClickMe",
        "InlineImage",
        "track()",
        "Sidebar",
        "FooterText",
        "RoleFooter",
        "a comment",
    ];

    fn extracted(html: &str) -> String {
        markdown::from_node(Document::parse(html).unwrap().main_content().node())
    }

    #[test]
    fn main_content_keeps_main_and_drops_boilerplate() {
        let markdown = extracted(PAGE);
        assert_eq!(
            markdown,
            "# Real Title\n\nBody text with a [link](https://a.example/x).\n\n![Photo](/photo.png)"
        );
        for dropped in BOILERPLATE {
            assert!(!markdown.contains(dropped), "{dropped}");
        }
    }

    #[test]
    fn raw_conversion_keeps_everything_but_scripts_and_styles() {
        let markdown = markdown::from_node(Document::parse(PAGE).unwrap().node());
        for kept in [
            "SiteLogo",
            "NavLink",
            "RoleNav",
            "RelatedStuff",
            "HiddenAttr",
            "ClickMe",
            "Sidebar",
            "FooterText",
            "Real Title",
        ] {
            assert!(markdown.contains(kept), "{kept} missing from {markdown}");
        }
        for dropped in ["track()", "color: red", "Jerry"] {
            assert!(!markdown.contains(dropped), "{dropped}");
        }
    }

    #[test]
    fn extracted_html_is_the_inside_of_the_main_element() {
        let html = Document::parse(PAGE)
            .unwrap()
            .main_content()
            .to_html()
            .unwrap();
        assert!(
            html.contains("<header><h1>Real Title</h1></header>"),
            "{html}"
        );
        assert!(
            html.contains(r#"<a href="https://a.example/x">link</a>"#),
            "{html}"
        );
        assert!(!html.contains("<main"), "{html}");
        for dropped in BOILERPLATE {
            assert!(!html.contains(dropped), "{dropped}");
        }
    }

    #[test]
    fn a_single_article_is_selected_when_there_is_no_main() {
        let html = "<body><div>Chrome</div><article><header><h2>Post</h2></header>\
                    <p>Words</p><footer>Tags</footer></article><div>More chrome</div></body>";
        assert_eq!(extracted(html), "## Post\n\nWords");
    }

    #[test]
    fn several_articles_mean_a_listing_so_the_body_is_kept() {
        let html = "<body><header>Banner</header><h1>Blog</h1><article><p>One</p></article>\
                    <article><p>Two</p></article><footer>Foot</footer></body>";
        assert_eq!(extracted(html), "# Blog\n\nOne\n\nTwo");
    }

    #[test]
    fn pages_without_landmarks_keep_the_pruned_body() {
        let html = "<html><body><table><tr><td>Cell</td></tr></table><nav>Menu</nav>\
                    <form><p>Inside a form</p><input value=x></form></body></html>";
        let markdown = extracted(html);
        assert!(markdown.contains("Cell"), "{markdown}");
        assert!(markdown.contains("Inside a form"), "{markdown}");
        assert!(!markdown.contains("Menu"), "{markdown}");
    }

    #[test]
    fn an_empty_main_falls_back_to_the_body_but_never_restores_boilerplate() {
        let html = "<body><main id=app></main><p>Fallback text</p><nav>Menu</nav></body>";
        assert_eq!(extracted(html), "Fallback text");
        assert_eq!(extracted("<body><nav>Only menu</nav></body>"), "");
    }

    #[test]
    fn boilerplate_class_and_id_names_are_dropped_but_layout_wrappers_kept() {
        let html = "<body class=\"has-sidebar\"><div class=\"layout with-sidebar\">\
                    <div class=\"site-dropdown\">Languages</div><div id=\"cookie_notice\">Cookies</div>\
                    <div class=\"Sidebar_x1\">Side</div><div class=\"navigator\">Kept word</div>\
                    <p>The article text is by far the longest thing on this page.</p></div></body>";
        assert_eq!(
            extracted(html),
            "Kept word\n\nThe article text is by far the longest thing on this page."
        );
    }

    #[test]
    fn scripts_do_not_count_as_page_text_when_judging_wrappers() {
        let script = "let bundle = 1;".repeat(200);
        let html = format!(
            "<head><style>{script}</style></head><body class=\"has-sidebar\">\
             <script>{script}</script><div class=\"with-nav\"><p>short article</p></div></body>"
        );
        assert_eq!(extracted(&html), "short article");
    }

    #[test]
    fn role_main_counts_as_main() {
        let html = "<body><div>Chrome</div><div role=\"main\"><p>Core</p></div></body>";
        assert_eq!(extracted(html), "Core");
    }

    #[test]
    fn title_is_decoded_and_collapsed() {
        let title = |html: &str| Document::parse(html).unwrap().title();
        assert_eq!(title(PAGE).as_deref(), Some("Tom & Jerry's page"));
        assert_eq!(
            title("<TITLE lang='en'>Ünïcode &lt;b&gt; &mdash; x</TITLE>").as_deref(),
            Some("Ünïcode <b> — x")
        );
        for none in [
            "",
            "<p>no title</p>",
            "<title></title>",
            "<title>  \n </title>",
        ] {
            assert_eq!(title(none), None, "{none}");
        }
    }
}
