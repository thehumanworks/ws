//! Rendering results for people (`text`) and programs (`--json`).

use std::io::{self, Write};

use serde::Serialize;

use crate::client::{Item, Metadata, SearchOutcome};
use crate::config::Preferences;
use crate::provider::Provider;

/// Snippet length shown in text output unless `--full` is given.
pub const SNIPPET_CHARS: usize = 300;

/// Neutralises terminal control sequences in untrusted text: every control
/// character (including ESC, CR and LF) becomes a space.
#[must_use]
pub fn sanitize(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// Shortens `text` to at most `max` characters, marking the cut with `…`.
#[must_use]
pub fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push('…');
    out
}

/// The `--json` contract: one object with the request context, every item and
/// the metadata exactly as Cloudflare returned them (nothing truncated).
#[derive(Debug, Serialize)]
struct JsonOutput<'a> {
    provider: Provider,
    gateway: &'a str,
    #[serde(rename = "requestId", skip_serializing_if = "Option::is_none")]
    request_id: Option<&'a str>,
    items: &'a [Item],
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<&'a Metadata>,
}

/// Writes a search outcome as a single JSON object followed by a newline.
///
/// # Errors
/// Any error from the writer.
pub fn write_json(
    out: &mut dyn Write,
    preferences: &Preferences,
    outcome: &SearchOutcome,
) -> io::Result<()> {
    let document = JsonOutput {
        provider: preferences.provider,
        gateway: &preferences.gateway_id,
        request_id: outcome.request_id.as_deref(),
        items: &outcome.response.items,
        metadata: outcome.response.metadata.as_ref(),
    };
    serde_json::to_writer_pretty(&mut *out, &document)?;
    writeln!(out)
}

/// Writes a search outcome as numbered, terminal-safe text.
///
/// # Errors
/// Any error from the writer.
pub fn write_text(
    out: &mut dyn Write,
    preferences: &Preferences,
    outcome: &SearchOutcome,
    full: bool,
) -> io::Result<()> {
    let items = &outcome.response.items;
    if items.is_empty() {
        writeln!(out, "No results.")?;
    }
    for (index, item) in items.iter().enumerate() {
        writeln!(
            out,
            "{}. {}",
            index.saturating_add(1),
            sanitize(&item.title)
        )?;
        writeln!(out, "   {}", sanitize(&item.url))?;
        if let Some(description) = item.description.as_deref().filter(|d| !d.trim().is_empty()) {
            let clean = sanitize(description);
            let shown = if full {
                clean
            } else {
                truncate(clean.trim(), SNIPPET_CHARS)
            };
            writeln!(out, "   {shown}")?;
        }
        if let Some(date) = &item.last_modified_date {
            writeln!(out, "   last modified: {}", sanitize(date))?;
        }
        writeln!(out)?;
    }
    write!(
        out,
        "{} result{} · provider {} · gateway {} · {} ms",
        items.len(),
        if items.len() == 1 { "" } else { "s" },
        preferences.provider,
        sanitize(&preferences.gateway_id),
        outcome.elapsed.as_millis()
    )?;
    if let Some(id) = &outcome.request_id {
        write!(out, " · request {}", sanitize(id))?;
    }
    writeln!(out)
}

/// Writes the provider table for `ws providers`.
///
/// # Errors
/// Any error from the writer.
pub fn write_providers(out: &mut dyn Write, selected: Provider, json: bool) -> io::Result<()> {
    if json {
        let rows: Vec<serde_json::Value> = Provider::ALL
            .into_iter()
            .map(|p| {
                serde_json::json!({
                    "name": p.as_str(),
                    "usdPerThousandRequests": p.price_per_thousand_usd(),
                    "summary": p.summary(),
                    "cloudflareDefault": p == Provider::default(),
                    "selected": p == selected,
                })
            })
            .collect();
        serde_json::to_writer_pretty(&mut *out, &rows)?;
        return writeln!(out);
    }
    writeln!(out, "  {:<8} {:>12}  NOTES", "PROVIDER", "USD/1K REQS")?;
    for p in Provider::ALL {
        let marker = if p == selected { '*' } else { ' ' };
        let default = if p == Provider::default() {
            " (Cloudflare default)"
        } else {
            ""
        };
        writeln!(
            out,
            "{marker} {:<8} {:>12}  {}{default}",
            p.as_str(),
            p.price_per_thousand_usd(),
            p.summary()
        )?;
    }
    writeln!(
        out,
        "\n* = provider ws will use now. Prices are a 2026-10-02 snapshot of Cloudflare's docs."
    )
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::{Map, Value};

    use super::*;
    use crate::client::SearchResponse;
    use crate::config::Source;

    fn prefs() -> Preferences {
        Preferences {
            provider: Provider::Exa,
            provider_source: Source::Flag,
            gateway_id: "gw".into(),
            gateway_source: Source::Default,
        }
    }

    fn item(title: &str, url: &str, description: Option<&str>) -> Item {
        Item {
            url: url.into(),
            title: title.into(),
            description: description.map(str::to_owned),
            image_url: None,
            favicon_url: None,
            last_modified_date: None,
            extra: Map::new(),
        }
    }

    fn outcome(items: Vec<Item>) -> SearchOutcome {
        SearchOutcome {
            response: SearchResponse {
                items,
                metadata: None,
            },
            request_id: Some("req-9".into()),
            elapsed: Duration::from_millis(42),
        }
    }

    fn text(outcome: &SearchOutcome, full: bool) -> String {
        let mut buffer = Vec::new();
        write_text(&mut buffer, &prefs(), outcome, full).unwrap();
        String::from_utf8(buffer).unwrap()
    }

    #[test]
    fn sanitize_replaces_every_control_character() {
        assert_eq!(sanitize("a\u{1b}[31mb\r\nc\u{7}\u{9b}"), "a [31mb  c  ");
        assert_eq!(sanitize("plain ünïcode 🦀"), "plain ünïcode 🦀");
    }

    #[test]
    fn truncate_counts_characters() {
        assert_eq!(truncate("héllo", 5), "héllo");
        assert_eq!(truncate("héllo!", 5), "héllo…");
        assert_eq!(truncate("", 0), "");
    }

    #[test]
    fn text_output_lists_results_and_footer() {
        let rendered = text(
            &outcome(vec![
                item("First", "https://a.example", Some("about a")),
                item("Second", "https://b.example", None),
            ]),
            false,
        );
        assert_eq!(
            rendered,
            "1. First\n   https://a.example\n   about a\n\n2. Second\n   https://b.example\n\n\
             2 results · provider exa · gateway gw · 42 ms · request req-9\n"
        );
    }

    #[test]
    fn text_output_for_no_results() {
        let rendered = text(&outcome(vec![]), false);
        assert!(rendered.starts_with("No results.\n0 results"));
    }

    #[test]
    fn text_output_truncates_unless_full_and_discloses_it() {
        let long = "x".repeat(SNIPPET_CHARS + 50);
        let short = text(&outcome(vec![item("T", "u", Some(&long))]), false);
        assert!(short.contains(&format!("{}…", "x".repeat(SNIPPET_CHARS))));
        assert!(short.contains("1 result ·"));
        let full = text(&outcome(vec![item("T", "u", Some(&long))]), true);
        assert!(full.contains(&long));
        assert!(!full.contains('…'));
    }

    #[test]
    fn text_output_neutralises_escape_sequences() {
        let evil = item(
            "\u{1b}]0;pwned\u{7}Title",
            "https://x.example/\u{1b}[2J",
            Some("a\nb"),
        );
        let rendered = text(&outcome(vec![evil]), false);
        assert!(!rendered.contains('\u{1b}'));
        assert!(!rendered.contains('\u{7}'));
        assert!(rendered.contains("   a b\n"));
    }

    #[test]
    fn json_output_is_one_complete_object() {
        let long = "x".repeat(SNIPPET_CHARS + 50);
        let mut buffer = Vec::new();
        write_json(
            &mut buffer,
            &prefs(),
            &outcome(vec![item("T", "u", Some(&long))]),
        )
        .unwrap();
        let value: Value = serde_json::from_slice(&buffer).unwrap();
        assert_eq!(value["provider"], "exa");
        assert_eq!(value["gateway"], "gw");
        assert_eq!(value["requestId"], "req-9");
        assert_eq!(value["items"][0]["description"], long);
        assert!(value.get("metadata").is_none());
        assert!(buffer.ends_with(b"\n"));
    }

    #[test]
    fn providers_table_marks_selection_and_default() {
        let mut buffer = Vec::new();
        write_providers(&mut buffer, Provider::Linkup, false).unwrap();
        let rendered = String::from_utf8(buffer).unwrap();
        assert!(rendered.contains("* linkup"));
        assert!(rendered.contains("  ceramic"));
        assert!(rendered.contains("(Cloudflare default)"));
        assert!(rendered.contains("7.00"));
    }

    #[test]
    fn providers_json_lists_all_three() {
        let mut buffer = Vec::new();
        write_providers(&mut buffer, Provider::Ceramic, true).unwrap();
        let value: Value = serde_json::from_slice(&buffer).unwrap();
        let rows = value.as_array().unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0]["name"], "ceramic");
        assert_eq!(rows[0]["selected"], true);
        assert_eq!(rows[0]["cloudflareDefault"], true);
        assert_eq!(rows[1]["selected"], false);
    }
}
