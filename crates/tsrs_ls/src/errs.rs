//! `ErrorScript` code-action adapter. Native checker types and the ordinary change tracker provide
//! scope-aware type syntax, formatting, UTF-16 edits, and the standard LSP quick-fix transport.

use crate::codeactions::{CodeAction, CodeFixContext, CodeFixProvider};
use crate::{astnav, change};
use std::sync::Arc;
use tsrs_ast::{self as ast, Kind};
use tsrs_core::context::Context;
use tsrs_lsproto as lsproto;

pub static RETURNS_FIX_PROVIDER: CodeFixProvider = CodeFixProvider {
    error_codes: &[98003],
    get_code_actions: returns_code_actions,
    get_all_code_actions: None,
};

#[expect(
    clippy::unnecessary_wraps,
    reason = "Native code-fix providers share a fallible callback signature."
)]
fn returns_code_actions(
    ctx: &Context,
    fix: &CodeFixContext,
) -> Result<Vec<CodeAction>, lsproto::Error> {
    if !::errs::enabled() {
        return Ok(Vec::new());
    }
    let file = fix.source_file;
    let position = fix.span.pos().expect_source("code action source span");
    let mut checker = fix.program.get_type_checker_for_file(ctx, file);
    // Only act on a still-current missing-contract diagnostic at the requested source position.
    if !checker
        .returns_contract_diagnostics(file)
        .iter()
        .any(|diagnostic| diagnostic.code() == 98003 && diagnostic.pos() == Some(position))
    {
        return Ok(Vec::new());
    }
    let mut node = astnav::get_token_at_position(file, position);
    while !ast::is_function_like(Some(node)) {
        let Some(parent) = node.parent() else {
            return Ok(Vec::new());
        };
        node = parent;
    }
    let Some((text, annotation)) = checker.suggested_returns_annotation(node) else {
        return Ok(Vec::new());
    };
    let mut tracker = change::new_tracker(
        ctx,
        &fix.program.options(),
        fix.ls.format_options(),
        Arc::clone(&fix.ls.converters),
    );
    if ast::is_in_js_file(Some(node)) {
        // JSDoc belongs to the declaration/variable statement, not inside a single-parameter arrow.
        let mut owner = node;
        if ast::is_function_expression_or_arrow_function(node) {
            while let Some(parent) = owner.parent() {
                if !matches!(
                    parent.kind(),
                    Kind::ParenthesizedExpression
                        | Kind::VariableDeclaration
                        | Kind::VariableDeclarationList
                        | Kind::VariableStatement
                        | Kind::PropertyAssignment
                ) {
                    break;
                }
                owner = parent;
            }
        }
        let existing = owner
            .jsdoc(Some(file.get()))
            .iter()
            .filter_map(|doc| doc.as_jsdoc().tags())
            .flat_map(|tags| tags.nodes().iter())
            .find(|tag| tag.kind() == Kind::JSDocReturnTag)
            .and_then(|tag| tag.type_expression())
            .and_then(|expression| expression.type_node());
        if let Some(existing) = existing {
            tracker.replace_node(file, existing, annotation, None);
        } else {
            let pos = tsrs_scanner::get_token_pos_of_node(owner, file, false);
            let prefix = &file.text()[..pos as usize];
            let line = prefix.rsplit('\n').next().unwrap_or("");
            let indent: String = line.chars().take_while(|c| matches!(c, ' ' | '\t')).collect();
            let newline = if file.text().contains("\r\n") { "\r\n" } else { "\n" };
            tracker.replace_text_range_with_text(
                file,
                tsrs_core::TextRange::new(pos, pos),
                &format!("/** @returns {{{text}}} */{newline}{indent}"),
            );
        }
    } else if let Some(existing) = node.type_node() {
        tracker.replace_node(file, existing, annotation, None);
    } else {
        if node.kind() == Kind::ArrowFunction {
            tracker.parenthesize_arrow_parameters(file, node);
        }
        if !tracker.try_insert_type_annotation(file, node, annotation) {
            return Ok(Vec::new());
        }
    }
    let (mut changes, _) = tracker.get_changes();
    let changes =
        changes.shift_remove(crate::lsconv::Script::original_file_name(&file)).unwrap_or_default();
    if changes.is_empty() {
        return Ok(Vec::new());
    }
    Ok(vec![CodeAction {
        description: format!("Add return type {text}"),
        changes,
        fix_id: String::new(),
    }])
}
