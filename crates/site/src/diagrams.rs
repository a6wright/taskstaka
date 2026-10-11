//! The three diagrams that explain stacks, inline SVG with CSS animation so
//! they move without JavaScript and sit still under prefers-reduced-motion.

pub const ALL: &[(&str, &str)] = &[
    ("tree", include_str!("../assets/diagrams/tree.svg")),
    ("stack", include_str!("../assets/diagrams/stack.svg")),
    (
        "resurfaced",
        include_str!("../assets/diagrams/resurfaced.svg"),
    ),
];

const CAPTIONS: &[(&str, &str)] = &[
    (
        "tree",
        "A story spawns subtasks. While they are open, the story is blocked and out of your way.",
    ),
    (
        "stack",
        "Next shows the story as a stack. Tick the plate in hand and the next prerequisite moves into it.",
    ),
    (
        "resurfaced",
        "When the last subtask closes, the story comes back to the top of Next, flagged ready.",
    ),
];

/// Replace `<!-- diagram:NAME -->` with the figure. Unknown names are left in place.
pub fn inline(html: &str) -> String {
    let mut out = html.to_string();
    for (name, svg) in ALL {
        let marker = format!("<!-- diagram:{name} -->");
        let caption = CAPTIONS
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, c)| *c)
            .unwrap_or("");
        let figure = format!(
            "<figure class=\"diagram\" id=\"diagram-{name}\">{svg}<figcaption>{caption}</figcaption></figure>"
        );
        out = out.replace(&marker, &figure);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_become_inline_svg_figures() {
        let html = inline("<p>a</p>\n<!-- diagram:stack -->\n<p>b</p>");
        assert!(
            html.contains(r#"<figure class="diagram" id="diagram-stack">"#),
            "{html}"
        );
        assert!(html.contains("<svg"), "{html}");
        assert!(html.contains("<figcaption>"), "{html}");
        assert!(!html.contains("<!-- diagram:stack -->"));
        assert_eq!(
            inline("<!-- diagram:nope -->"),
            "<!-- diagram:nope -->",
            "unknown names are left for the author to notice"
        );
    }

    #[test]
    fn every_diagram_animates_without_js_and_respects_reduced_motion() {
        assert_eq!(ALL.len(), 3);
        for (name, svg) in ALL {
            assert!(svg.contains("<style>"), "{name} has its own styles");
            assert!(svg.contains("@keyframes"), "{name} animates");
            assert!(
                svg.contains("prefers-reduced-motion"),
                "{name} has a still frame"
            );
            assert!(
                svg.contains(r#"role="img""#) && svg.contains("<title"),
                "{name} is labelled"
            );
            assert!(!svg.contains("<script"), "{name} has no script");
        }
    }
}
