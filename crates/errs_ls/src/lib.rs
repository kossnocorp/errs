//! `ErrorScript` hover presentation. Link targets come from native types and symbols, never printed names.

use std::fmt::Write as _;

use errs::{Effect, ErrorSet};
use tsrs_ast::{self as ast, SourceFile};
use tsrs_checker::{Checker, Type, TypeFlags};
use tsrs_core::{P, TextRange};
use tsrs_lsproto::Location;

/// The language service supplies its normal source-map-aware location conversion.
pub fn render_effect(
    checker: &mut Checker,
    effect: &Effect<P<Type>>,
    location: &impl Fn(P<SourceFile>, TextRange) -> Location,
) -> String {
    let mut text = format!("throws: {}", render_set(checker, &effect.throws, location));
    if !effect.rejects.is_empty() {
        write!(text, "  \nrejects: {}", render_set(checker, &effect.rejects, location)).unwrap();
    }
    if !effect.deferred.is_empty() {
        write!(text, "  \non iteration: {}", render_set(checker, &effect.deferred, location))
            .unwrap();
    }
    text
}

fn render_set(
    checker: &mut Checker,
    set: &ErrorSet<P<Type>>,
    location: &impl Fn(P<SourceFile>, TextRange) -> Location,
) -> String {
    let mut names: Vec<_> = set
        .types
        .iter()
        .map(|&ty| (checker.type_to_string_exported(ty), render_type(checker, ty, location)))
        .collect();
    names.sort();
    names.dedup();
    let mut parts: Vec<_> = names.into_iter().map(|(_, markdown)| markdown).collect();
    if set.unknown {
        parts.push("`unknown`".to_string());
    }
    if parts.is_empty() { "`never`".to_string() } else { parts.join(" | ") }
}

fn render_type(
    checker: &mut Checker,
    ty: P<Type>,
    location: &impl Fn(P<SourceFile>, TextRange) -> Location,
) -> String {
    let symbol = ty.alias().and_then(|alias| alias.symbol()).or_else(|| ty.symbol());
    if symbol.is_none() && ty.flags().intersects(TypeFlags::UnionOrIntersection) {
        let separator = if ty.flags().intersects(TypeFlags::Union) { " | " } else { " & " };
        return ty
            .types()
            .iter()
            .map(|&part| {
                let text = render_type(checker, part, location);
                if ty.flags().intersects(TypeFlags::Intersection)
                    && part.flags().intersects(TypeFlags::Union)
                {
                    format!("({text})")
                } else {
                    text
                }
            })
            .collect::<Vec<_>>()
            .join(separator);
    }
    let label = inline_code(&checker.type_to_string_exported(ty));
    let target = symbol.and_then(|symbol| {
        symbol.declarations().iter().find_map(|declaration| {
            let name = declaration.name()?;
            let file = ast::get_source_file_of_node(name)?;
            let start = tsrs_scanner::get_token_pos_of_node(name, file, false);
            Some(location(
                file,
                TextRange::new(start, name.end().expect_source("declaration name")),
            ))
        })
    });
    let Some(target) = target else { return label };
    // Angle-bracket destinations handle parentheses; encode characters that could end the destination.
    let uri = target.uri.replace('<', "%3C").replace('>', "%3E").replace('\\', "%5C");
    format!(
        "[{label}](<{uri}#L{},{}>)",
        target.range.start.line + 1,
        target.range.start.character + 1
    )
}

fn inline_code(text: &str) -> String {
    // Type displays can contain template literals or string literals with Markdown punctuation.
    let ticks = text.split(|c| c != '`').map(str::len).max().unwrap_or(0) + 1;
    let delimiter = "`".repeat(ticks);
    if ticks == 1 { format!("`{text}`") } else { format!("{delimiter} {text} {delimiter}") }
}
