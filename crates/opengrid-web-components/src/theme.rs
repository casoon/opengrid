//! The built-in looks (issue #32).
//!
//! The design prototype's five presets — Base, Papier, Violett, Orange, Dunkel —
//! are the element's own: `theme="paper"` on `<opengrid-grid>`,
//! `<opengrid-table>` or `<opengrid-pivot>` picks one, and **Base is the
//! default**, so an element without any page CSS already has the prototype's
//! look. The `--og-*` properties a page sets on the element still win: a page
//! rule on the host beats every `:host` rule, so a preset is where a page's own
//! look starts, not a lock.
//!
//! What a preset cannot switch off stays where it was: under `forced-colors`
//! the grid resets every colour token to a system colour (its stylesheet does
//! that after these rules), the focus ring never uses the accent, and a
//! selected row carries a bar as well as a tint.
//!
//! The fonts are named, not loaded: a request to a font service from a
//! component would be a privacy and CSP decision taken for every page. A page
//! that wants Geist loads it; without it the stack falls back to the system UI
//! font.

/// Picks one of [`LOOKS`]; absent or unknown is the first, Base.
pub const THEME_ATTRIBUTE: &str = "theme";

/// One preset: the values of the page-settable colour, font and radius tokens.
pub struct Look {
    /// The attribute value.
    pub name: &'static str,
    /// `--og-font`, a full font stack.
    pub font: &'static str,
    pub surface: &'static str,
    pub surface_2: &'static str,
    pub ink: &'static str,
    pub ink_muted: &'static str,
    pub line: &'static str,
    pub line_strong: &'static str,
    pub accent: &'static str,
    pub on_accent: &'static str,
    /// `--og-radius` in pixels.
    pub radius: u8,
    /// `color-scheme`, so native controls and scrollbars match.
    pub scheme: &'static str,
}

const GEIST: &str = "\"Geist\", system-ui, sans-serif";
const PLEX: &str = "\"IBM Plex Sans\", system-ui, sans-serif";
/// `--og-font-mono`, the same in every preset.
pub const MONO: &str = "\"Geist Mono\", ui-monospace, SFMono-Regular, Menlo, monospace";

/// The five presets, Base first. The values are the prototype's, with one
/// correction the prototype page made for contrast: Orange's accent is
/// `#b44c1c`, not `#cf5a24`, which drawn as text on white was under 4.5:1.
pub const LOOKS: [Look; 5] = [
    Look {
        name: "base",
        font: GEIST,
        surface: "#ffffff",
        surface_2: "#fafbfc",
        ink: "#14161a",
        ink_muted: "#646b78",
        line: "#eceef2",
        line_strong: "#e0e3e9",
        accent: "#3d5fd6",
        on_accent: "#ffffff",
        radius: 12,
        scheme: "light",
    },
    Look {
        name: "paper",
        font: PLEX,
        surface: "#fffdf8",
        surface_2: "#faf7f0",
        ink: "#1f1d19",
        ink_muted: "#6f685a",
        line: "#ebe6da",
        line_strong: "#ddd7c8",
        accent: "#2a7a59",
        on_accent: "#ffffff",
        radius: 8,
        scheme: "light",
    },
    Look {
        name: "violet",
        font: GEIST,
        surface: "#ffffff",
        surface_2: "#fbfbfd",
        ink: "#17161f",
        ink_muted: "#6c6b80",
        line: "#ececf3",
        line_strong: "#dfdfea",
        accent: "#6b4bc8",
        on_accent: "#ffffff",
        radius: 14,
        scheme: "light",
    },
    Look {
        name: "orange",
        font: GEIST,
        surface: "#ffffff",
        surface_2: "#faf9f7",
        ink: "#111111",
        ink_muted: "#6e6c69",
        line: "#ecebe8",
        line_strong: "#e2e0dc",
        accent: "#b44c1c",
        on_accent: "#ffffff",
        radius: 10,
        scheme: "light",
    },
    Look {
        name: "dark",
        font: GEIST,
        surface: "#15181c",
        surface_2: "#1a1e23",
        ink: "#e6e8eb",
        ink_muted: "#9aa1ab",
        line: "#23282e",
        line_strong: "#2e333a",
        accent: "#4fd1d1",
        on_accent: "#0b1414",
        radius: 12,
        scheme: "dark",
    },
];

/// The declarations of one preset, for the inside of a `:host` rule.
fn declarations(look: &Look) -> String {
    format!(
        "--og-font: {}; --og-font-mono: {MONO}; --og-surface: {}; --og-surface-2: {}; \
         --og-ink: {}; --og-ink-muted: {}; --og-line: {}; --og-line-strong: {}; \
         --og-accent: {}; --og-on-accent: {}; --og-radius: {}px; color-scheme: {};",
        look.font,
        look.surface,
        look.surface_2,
        look.ink,
        look.ink_muted,
        look.line,
        look.line_strong,
        look.accent,
        look.on_accent,
        look.radius,
        look.scheme,
    )
}

/// `:host` with Base, then one `:host([theme="…"])` rule per other preset.
///
/// Placed **before** the element's own `:host` rule, which must then not set
/// these tokens again; the attribute rules win over it by specificity either
/// way, and a page's rule on the host wins over all of them.
pub fn host_rules() -> String {
    let mut css = format!(":host {{ {} }}\n", declarations(&LOOKS[0]));
    for look in &LOOKS[1..] {
        css.push_str(&format!(
            ":host([{THEME_ATTRIBUTE}=\"{}\"]) {{ {} }}\n",
            look.name,
            declarations(look)
        ));
    }
    css
}

/// The stylesheet of `<opengrid-table>` and `<opengrid-pivot>`: the looks, the
/// text in the look's ink and font, and what a column's presentation marks on
/// a table cell — the grid's own rules for the same markers. Everything else
/// about their look is the page's, through their parts (issue #29).
pub fn table_css() -> String {
    format!(
        "{}:host {{ color: var(--og-ink); font-family: var(--og-font); }}\n\
         [data-align=\"end\"] {{ text-align: right; }}\n\
         [data-align=\"center\"] {{ text-align: center; }}\n\
         [data-align=\"start\"] {{ text-align: left; }}\n\
         td[data-align=\"end\"] {{ font-variant-numeric: tabular-nums; }}\n\
         td[data-mono] {{ font-family: var(--og-font-mono); }}\n\
         td[data-emphasis] {{ font-weight: 600; }}\n\
         td[data-muted] {{ color: var(--og-ink-muted); }}\n\
         @media (forced-colors: active) {{ :host {{ --og-ink: CanvasText; --og-ink-muted: CanvasText; }} }}\n{}",
        host_rules(),
        PIVOT_TOOLBAR_CSS
    )
}

/// The pivot's field toolbar (issue #112): its parts only, so a table is
/// untouched. A menu is a popover and needs its place; the rest is the least
/// that makes the controls look like controls — the page styles the parts.
const PIVOT_TOOLBAR_CSS: &str = "\
[part~=\"toolbar\"] { display: flex; flex-wrap: wrap; gap: 0.5rem 1.25rem; align-items: center; margin-block-end: 0.5rem; }
[part~=\"field-group\"] { display: flex; flex-wrap: wrap; gap: 0.25rem; align-items: center; }
[part~=\"menu-label\"] { color: var(--og-ink-muted); margin-inline-end: 0.25rem; }
[part~=\"chip\"] { display: inline-flex; align-items: center; gap: 0.125rem; padding-inline: 0.5rem 0.125rem; border: 1px solid var(--og-line-strong); border-radius: 999px; }
[part~=\"chip-move\"], [part~=\"chip-remove\"] { min-inline-size: 24px; min-block-size: 24px; border: 0; border-radius: 999px; background: none; color: inherit; font: inherit; cursor: pointer; }
[part~=\"add-field\"] { min-block-size: 24px; padding-inline: 0.625rem; border: 1px dashed var(--og-line-strong); border-radius: 999px; background: none; color: inherit; font: inherit; cursor: pointer; }
[part~=\"add-field\"][aria-disabled=\"true\"] { color: var(--og-ink-muted); cursor: not-allowed; }
:is([part~=\"chip-move\"], [part~=\"chip-remove\"], [part~=\"add-field\"]):focus-visible { outline: 2px solid var(--og-accent); outline-offset: 2px; }
[part~=\"field-menu\"] { position: fixed; inset: auto; margin: 0; padding: 0.25rem; min-inline-size: 10rem; border: 1px solid var(--og-line-strong); border-radius: var(--og-radius); background: var(--og-surface); color: var(--og-ink); box-shadow: 0 8px 24px rgb(0 0 0 / 0.12); }
[part~=\"field-menu\"] [role=\"menuitem\"] { padding: 0.375rem 0.625rem; border-radius: 4px; cursor: pointer; }
[part~=\"field-menu\"] [role=\"menuitem\"]:is(:hover, :focus) { background: var(--og-surface-2); outline: 2px solid var(--og-accent); outline-offset: -2px; }
";

/// Adopts [`table_css`] into a table's or pivot's shadow root, once per root.
///
/// Adopted rather than a `<style>` child: both elements clear their root and
/// render it anew for every answer, and an adopted sheet is not a child.
#[cfg(target_arch = "wasm32")]
pub fn adopt_table_look(root: &web_sys::ShadowRoot) {
    use wasm_bindgen::JsValue;
    thread_local! {
        static SHEET: Option<web_sys::CssStyleSheet> = web_sys::CssStyleSheet::new()
            .ok()
            .inspect(|sheet| sheet.replace_sync(&table_css()).unwrap_or_default());
    }
    SHEET.with(|sheet| {
        if let Some(sheet) = sheet {
            let sheets = js_sys::Array::of1(sheet);
            root.set_adopted_style_sheets(&JsValue::from(sheets));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_is_the_default_and_every_other_look_has_its_rule() {
        let css = host_rules();
        assert!(css.starts_with(":host { --og-font: \"Geist\""));
        assert!(css.contains("--og-accent: #3d5fd6;"));
        for look in &LOOKS[1..] {
            assert!(css.contains(&format!(":host([theme=\"{}\"])", look.name)));
        }
        assert!(
            !css.contains(":host([theme=\"base\"])"),
            "base is the plain :host"
        );
    }

    /// WCAG relative luminance of `#rrggbb`.
    fn luminance(hex: &str) -> f64 {
        let channel = |i: usize| {
            let value = u8::from_str_radix(&hex[i..i + 2], 16).unwrap() as f64 / 255.0;
            if value <= 0.03928 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(1) + 0.7152 * channel(3) + 0.0722 * channel(5)
    }

    fn contrast(a: &str, b: &str) -> f64 {
        let (x, y) = (luminance(a), luminance(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    /// 1.4.3: text, muted text and the accent drawn as text reach 4.5:1 on
    /// both surfaces; text on the accent does too.
    #[test]
    fn every_look_keeps_text_contrast() {
        for look in &LOOKS {
            for surface in [look.surface, look.surface_2] {
                for (what, ink) in [
                    ("ink", look.ink),
                    ("muted", look.ink_muted),
                    ("accent", look.accent),
                ] {
                    let ratio = contrast(ink, surface);
                    assert!(
                        ratio >= 4.5,
                        "{}: {what} on {surface} is {ratio:.2}:1",
                        look.name
                    );
                }
            }
            let ratio = contrast(look.on_accent, look.accent);
            assert!(ratio >= 4.5, "{}: on-accent is {ratio:.2}:1", look.name);
        }
    }
}
