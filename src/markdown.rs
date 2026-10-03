//! Turning fetched HTML into something an agent can read cheaply.
//!
//! Pure functions over text and DOM nodes: no network, no I/O. Deciding which
//! part of a page to convert is [`crate::extract`]'s job.

use std::rc::Rc;

use htmd::Node;

use crate::error::Error;

/// Elements that carry no readable content and are dropped from Markdown.
pub const SKIPPED_TAGS: [&str; 8] = [
    "head", "script", "style", "noscript", "template", "iframe", "svg", "canvas",
];

fn converter() -> htmd::HtmlToMarkdown {
    // Compact list markers: fewer characters for the same structure.
    let options = htmd::options::Options {
        bullet_list_marker: htmd::options::BulletListMarker::Dash,
        ul_bullet_spacing: 1,
        ol_number_spacing: 1,
        ..htmd::options::Options::default()
    };
    htmd::HtmlToMarkdown::builder()
        .options(options)
        .skip_tags(SKIPPED_TAGS.to_vec())
        .build()
}

/// Converts an HTML document or fragment to Markdown, whole.
///
/// Scripts, styles and the other [`SKIPPED_TAGS`] are removed; everything else
/// (including navigation and footers) is kept, in document order. To keep only
/// the main content, go through [`crate::extract::Document`] and [`from_node`].
///
/// # Errors
/// [`Error::PageContent`] if the HTML cannot be read.
pub fn from_html(html: &str) -> Result<String, Error> {
    converter()
        .convert(html)
        .map_err(|e| Error::PageContent(format!("cannot convert HTML to Markdown: {e}")))
}

/// Converts an already parsed (and possibly pruned) DOM node to Markdown.
#[must_use]
pub fn from_node(node: &Rc<Node>) -> String {
    converter().tree_to_markdown(node).trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"<!doctype html>
<html><head><title>
  Tom &amp; Jerry&#39;s   page </title>
<style>body { color: red }</style>
<script>var hidden = "secret";</script></head>
<body>
<h1>Hello</h1>
<p>A <strong>bold</strong> claim with a <a href="https://a.example/x">link</a>.</p>
<ul><li>one</li><li>two</li></ul>
<ol><li>first</li></ol>
<pre><code>let x = 1;</code></pre>
<noscript>enable js</noscript>
<script>track()</script>
</body></html>"#;

    #[test]
    fn converts_structure_and_drops_non_content() {
        let markdown = from_html(PAGE).unwrap();
        assert!(markdown.contains("# Hello"), "{markdown}");
        assert!(markdown.contains("**bold**"), "{markdown}");
        assert!(
            markdown.contains("[link](https://a.example/x)"),
            "{markdown}"
        );
        assert!(markdown.contains("- one\n- two"), "{markdown}");
        assert!(markdown.contains("1. first"), "{markdown}");
        assert!(markdown.contains("```\nlet x = 1;\n```"), "{markdown}");
        for dropped in ["secret", "track()", "color: red", "enable js", "Jerry"] {
            assert!(!markdown.contains(dropped), "{dropped} in {markdown}");
        }
    }

    #[test]
    fn fragments_and_empty_input_convert() {
        assert_eq!(
            from_html("<p>just a paragraph</p>").unwrap(),
            "just a paragraph"
        );
        assert_eq!(from_html("").unwrap(), "");
    }
}
