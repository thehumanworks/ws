//! Pure extraction of organic results from Brave's rendered search page.

use std::collections::HashSet;
use std::rc::Rc;

use htmd::Node;
use markup5ever_rcdom::NodeData;

use crate::client::Item;
use crate::error::Error;
use crate::extract::Document;

/// Builds one ordinary Brave query URL, percent-encoding UTF-8 query bytes.
#[must_use]
pub fn search_url(query: &str) -> String {
    let mut url = "https://search.brave.com/search?q=".to_owned();
    for byte in query.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            url.push(char::from(byte));
        } else {
            use std::fmt::Write;
            let _ = write!(url, "%{byte:02X}");
        }
    }
    url
}

fn attr(node: &Node, key: &str) -> Option<String> {
    if let NodeData::Element { attrs, .. } = &node.data {
        attrs
            .borrow()
            .iter()
            .find(|attribute| &*attribute.name.local == key)
            .map(|attribute| attribute.value.to_string())
    } else {
        None
    }
}

fn class(node: &Node, name: &str) -> bool {
    attr(node, "class").is_some_and(|value| value.split_ascii_whitespace().any(|word| word == name))
}

fn descendants(node: &Rc<Node>) -> Vec<Rc<Node>> {
    let mut pending = vec![Rc::clone(node)];
    let mut all = Vec::new();
    while let Some(node) = pending.pop() {
        pending.extend(node.children.borrow().iter().rev().map(Rc::clone));
        all.push(node);
    }
    all
}

fn text(node: &Rc<Node>) -> String {
    let mut value = String::new();
    for descendant in descendants(node) {
        if let NodeData::Text { contents } = &descendant.data {
            value.push_str(&contents.borrow());
        }
    }
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn title_link(node: &Rc<Node>, card: &Rc<Node>) -> Option<String> {
    let mut current = Some(Rc::clone(node));
    while let Some(node) = current {
        if let NodeData::Element { name, .. } = &node.data
            && &*name.local == "a"
        {
            return attr(&node, "href");
        }
        if Rc::ptr_eq(&node, card) {
            break;
        }
        current = node.parent.take().and_then(|parent| {
            let upgraded = parent.upgrade();
            node.parent.set(Some(parent));
            upgraded
        });
    }
    None
}

fn sponsored(node: &Rc<Node>) -> bool {
    ["ad", "ads", "advert", "advertisement", "sponsored"]
        .iter()
        .any(|name| class(node, name))
        || attr(node, "data-type")
            .is_some_and(|value| matches!(value.as_str(), "ad" | "ads" | "sponsored"))
        || attr(node, "data-sponsored").is_some_and(|value| value != "false")
}

/// Extracts ranked, deduplicated organic cards; unknown markup is an error,
/// so challenges/layout drift can never masquerade as a zero-result search.
///
/// # Errors
/// Malformed HTML, access-denial markup, missing results, or malformed cards.
pub fn extract(html: &str, limit: u8) -> Result<Vec<Item>, Error> {
    let document = Document::parse(html)?;
    let nodes = descendants(document.node());
    if nodes.iter().any(|node| {
        attr(node, "id").is_some_and(|id| matches!(id.as_str(), "challenge-form" | "captcha"))
            || class(node, "captcha")
    }) {
        return Err(Error::Transport(
            "Brave denied access with a challenge; ws will not retry or bypass it".to_owned(),
        ));
    }
    let main = nodes
        .into_iter()
        .find(|node| attr(node, "id").as_deref() == Some("mixed-main"));
    let Some(main) = main else {
        return Err(Error::UnexpectedResponse(
            "Brave organic result container is missing; access may be denied or markup changed"
                .to_owned(),
        ));
    };
    let mut ranked = Vec::new();
    for (order, card) in descendants(&main)
        .into_iter()
        .filter(|node| {
            class(node, "snippet")
                && attr(node, "data-type").as_deref() == Some("web")
                && !sponsored(node)
        })
        .enumerate()
    {
        let nodes = descendants(&card);
        if nodes
            .iter()
            .any(|node| sponsored(node) || class(node, "ad-label"))
        {
            continue;
        }
        let title_node = nodes
            .iter()
            .find(|node| class(node, "search-snippet-title"))
            .ok_or_else(|| {
                Error::UnexpectedResponse("Brave organic card is missing its title".to_owned())
            })?;
        let title = text(title_node);
        let url = title_link(title_node, &card)
            .filter(|url| crate::validate::url(url).is_ok())
            .ok_or_else(|| {
                Error::UnexpectedResponse(
                    "Brave organic card is missing a valid target URL".to_owned(),
                )
            })?;
        if title.is_empty() {
            return Err(Error::UnexpectedResponse(
                "Brave organic card has an empty title".to_owned(),
            ));
        }
        let description = nodes
            .iter()
            .find(|node| class(node, "generic-snippet"))
            .and_then(|snippet| {
                descendants(snippet)
                    .into_iter()
                    .find(|node| class(node, "content"))
            })
            .map(|node| text(&node))
            .filter(|value| !value.is_empty());
        let rank = attr(&card, "data-pos")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(order);
        ranked.push((
            rank,
            order,
            Item {
                url,
                title,
                description,
                image_url: None,
                favicon_url: None,
                last_modified_date: None,
                extra: serde_json::Map::new(),
            },
        ));
    }
    recognize_empty(&main, ranked.is_empty())?;
    ranked.sort_by_key(|(rank, order, _)| (*rank, *order));
    let mut seen = HashSet::new();
    Ok(ranked
        .into_iter()
        .map(|(_, _, item)| item)
        .filter(|item| seen.insert(item.url.clone()))
        .take(usize::from(limit))
        .collect())
}

fn recognize_empty(main: &Rc<Node>, empty: bool) -> Result<(), Error> {
    if empty {
        let no_results = descendants(main).into_iter().any(|node| {
            attr(&node, "id").as_deref() == Some("no-results") || class(&node, "no-results")
        });
        if no_results {
            return Ok(());
        }
        return Err(Error::UnexpectedResponse(
            "Brave returned no recognizable organic cards; access may be denied or markup changed"
                .to_owned(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_encodes_query_without_changing_unicode_limits() {
        assert_eq!(
            search_url("a &é"),
            "https://search.brave.com/search?q=a%20%26%C3%A9"
        );
        let query = "🦀".repeat(crate::validate::QUERY_MAX_CHARS);
        assert!(crate::validate::query(&query).is_ok());
        assert!(search_url(&query).len() > crate::validate::URL_MAX_CHARS);
    }

    #[test]
    fn snapshot_matches_observed_organic_results() {
        let observed: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/brave-2026-10-03.json")).unwrap();
        let items = extract(include_str!("../tests/fixtures/brave-2026-10-03.html"), 20).unwrap();
        let expected = observed.as_array().unwrap();
        assert_eq!(items.len(), expected.len());
        let compact = |value: &str| {
            value
                .chars()
                .filter(|character| !character.is_whitespace())
                .collect::<String>()
        };
        for (item, expected) in items.iter().zip(expected) {
            assert_eq!(item.title, expected["title"].as_str().unwrap());
            assert_eq!(item.url, expected["url"].as_str().unwrap());
            assert_eq!(
                item.description.as_deref().map(compact),
                expected["description"].as_str().map(compact)
            );
        }
    }

    fn card(position: usize, url: &str, title: &str, extra: &str) -> String {
        format!(
            "<div class='snippet' data-type='web' data-pos='{position}' {extra}><a href='{url}'><span class='search-snippet-title'>{title}</span></a><div class='generic-snippet'><div class='content'>Description &amp; more</div></div><a href='https://sitelink.example'>Sitelink</a></div>"
        )
    }

    #[test]
    fn only_organic_ranked_unique_title_links_are_results() {
        let html = format!(
            "<nav>{}</nav><main id='mixed-main'><div class='snippet' data-type='ad'>Ad</div>{}{}{}{}<div class='snippet' data-type='news'>News</div></main>",
            card(0, "https://nav.example", "Nav", ""),
            card(2, "https://b.example", "B", ""),
            card(1, "https://a.example", "A &amp; decoded", ""),
            card(3, "https://a.example", "Duplicate", ""),
            card(0, "https://ad.example", "Paid", "data-sponsored='true'")
        );
        let items = extract(&html, 10).unwrap();
        assert_eq!(
            items
                .iter()
                .map(|item| (&*item.url, &*item.title))
                .collect::<Vec<_>>(),
            [
                ("https://a.example", "A & decoded"),
                ("https://b.example", "B")
            ]
        );
        assert_eq!(items[0].description.as_deref(), Some("Description & more"));
        assert_eq!(extract(&html, 1).unwrap().len(), 1);
    }

    #[test]
    fn failures_never_look_like_empty_searches() {
        for html in [
            "<html>Access denied</html>",
            "<form id='challenge-form'>Challenge</form>",
            "<main id='mixed-main'></main>",
            "<main id='mixed-main'><div class='snippet' data-type='web'>Missing title</div></main>",
        ] {
            assert!(extract(html, 10).is_err());
        }
        assert!(
            extract(
                "<main id='mixed-main'><div class='no-results'>No results found</div></main>",
                10
            )
            .unwrap()
            .is_empty()
        );
    }
}
