//! Markdown in, pages out. Rendered once at startup; the shell is hand-built
//! like the daemon's web.rs so there is no template engine to learn.

use pulldown_cmark::{Options, Parser, html};

pub struct Page {
    pub path: &'static str,
    pub title: &'static str,
    pub html: String,
}

const CONTENT: [(&str, &str, &str); 4] = [
    ("/", "taskstaka", include_str!("../content/index.md")),
    ("/install", "Install", include_str!("../content/install.md")),
    ("/help", "Help", include_str!("../content/help.md")),
    (
        "/changelog",
        "Changelog",
        include_str!("../content/changelog.md"),
    ),
];

pub fn all() -> Vec<Page> {
    CONTENT
        .iter()
        .map(|(path, title, md)| Page {
            path,
            title,
            html: shell(title, path, &crate::diagrams::inline(&render_markdown(md))),
        })
        .collect()
}

/// Append `{#slug}` to every ATX heading outside fenced code, so anchors are
/// stable and the app can deep-link. Repeats get `-2`, `-3`, …
pub fn with_heading_ids(md: &str) -> String {
    let mut out = String::with_capacity(md.len() + 64);
    let mut seen: std::collections::HashMap<String, usize> = Default::default();
    let mut fenced = false;
    for line in md.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        }
        let hashes = line.chars().take_while(|&c| c == '#').count();
        if !fenced
            && (1..=6).contains(&hashes)
            && line[hashes..].starts_with(' ')
            && !line.contains(" {#")
        {
            let text = line[hashes..].trim();
            let base = slug(text);
            let n = seen
                .entry(base.clone())
                .and_modify(|n| *n += 1)
                .or_insert(1);
            let id = if *n == 1 { base } else { format!("{base}-{n}") };
            out.push_str(&format!("{line} {{#{id}}}\n"));
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn slug(text: &str) -> String {
    let mut s = String::new();
    let mut dash = false;
    for c in text.chars() {
        if c.is_alphanumeric() {
            s.extend(c.to_lowercase());
            dash = false;
        } else if !dash && !s.is_empty() {
            s.push('-');
            dash = true;
        }
    }
    s.trim_end_matches('-').to_string()
}

pub fn render_markdown(md: &str) -> String {
    let md = with_heading_ids(md);
    let opts =
        Options::ENABLE_HEADING_ATTRIBUTES | Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH;
    let mut out = String::with_capacity(md.len() * 2);
    html::push_html(&mut out, Parser::new_ext(&md, opts));
    out
}

const NAV: [(&str, &str); 3] = [
    ("/install", "Install"),
    ("/help", "Help"),
    ("/changelog", "Changelog"),
];

pub fn shell(title: &str, path: &str, body: &str) -> String {
    let nav: String = NAV
        .iter()
        .map(|(href, label)| {
            let cur = if *href == path {
                r#" aria-current="page""#
            } else {
                ""
            };
            format!(r#"<a href="{href}"{cur}>{label}</a>"#)
        })
        .collect();
    let full_title = if path == "/" {
        "taskstaka".to_string()
    } else {
        format!("{title} · taskstaka")
    };
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
<meta name="color-scheme" content="dark light">
<title>{full_title}</title>
<meta name="description" content="taskstaka: a 'what needs action next' task server backed by markdown. Install in one line; works from the CLI, the web, the TUI, the menu bar, and with Claude.">
<link rel="icon" href="/favicon.ico" sizes="any">
<link rel="apple-touch-icon" href="/apple-touch-icon.png">
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600&display=swap">
<link rel="stylesheet" href="/site.css">
</head>
<body>
<header class="top"><a class="brand" href="/"><img src="/mark.png" alt="" width="26" height="26">taskstaka</a><nav>{nav}<a href="https://github.com/a6wright/taskstaka">GitHub</a></nav></header>
<main class="page">
{body}
</main>
<footer class="foot"><span>taskstaka · MIT</span><a href="https://github.com/a6wright/taskstaka/releases">releases</a></footer>
</body>
</html>
"#
    )
}

pub fn not_found() -> String {
    shell(
        "Not found",
        "/404",
        "<h1>Not found</h1><p>There is no page here. Try <a href=\"/install\">Install</a> or <a href=\"/help\">Help</a>.</p>",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_get_slug_ids_and_duplicates_are_suffixed() {
        let md = "# Install\n\ntext\n\n## Updating\n\n## Updating\n\n### Claude Code\n";
        let out = with_heading_ids(md);
        assert!(out.contains("# Install {#install}"), "{out}");
        assert!(out.contains("## Updating {#updating}"), "{out}");
        assert!(out.contains("## Updating {#updating-2}"), "{out}");
        assert!(out.contains("### Claude Code {#claude-code}"), "{out}");
        // Fenced code is left alone even when a line starts with #.
        let fenced = "```\n# not a heading\n```\n";
        assert_eq!(with_heading_ids(fenced), fenced);
    }

    #[test]
    fn markdown_renders_headings_with_ids_and_code_blocks() {
        let html =
            render_markdown("## Updating\n\n```\ncargo install --path crates/taskstaka\n```\n");
        assert!(
            html.contains(r#"<h2 id="updating">Updating</h2>"#),
            "{html}"
        );
        assert!(
            html.contains("<pre><code>cargo install --path crates/taskstaka\n</code></pre>"),
            "{html}"
        );
    }

    #[test]
    fn shell_wraps_the_body_and_marks_the_current_nav_item() {
        let html = shell("Install", "/install", "<p>hi</p>");
        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains("<title>Install · taskstaka</title>"));
        assert!(
            html.contains(r#"<a href="/install" aria-current="page">"#),
            "{html}"
        );
        assert!(!html.contains(r#"<a href="/help" aria-current="page">"#));
        assert!(html.contains("<p>hi</p>"));
    }

    #[test]
    fn every_page_renders_from_its_content_file() {
        let pages = all();
        let paths: Vec<&str> = pages.iter().map(|p| p.path).collect();
        assert_eq!(paths, vec!["/", "/install", "/help", "/changelog"]);
        for p in &pages {
            assert!(p.html.contains("<main"), "{}", p.path);
        }
    }
}
