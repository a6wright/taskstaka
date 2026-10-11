//! The three diagrams that explain stacks, inline SVG with CSS animation so
//! they move without JavaScript and sit still under prefers-reduced-motion.
//! Task 5 fills `ALL`; until then `inline` is a no-op pass-through.

pub const ALL: &[(&str, &str)] = &[];

/// Replace `<!-- diagram:NAME -->` with the figure. Unknown names are left in place.
pub fn inline(html: &str) -> String {
    let mut out = html.to_string();
    for (name, svg) in ALL {
        let marker = format!("<!-- diagram:{name} -->");
        let figure = format!("<figure class=\"diagram\" id=\"diagram-{name}\">{svg}</figure>");
        out = out.replace(&marker, &figure);
    }
    out
}
