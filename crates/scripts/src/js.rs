use std::ops::Range;

use anyhow::anyhow;
use ariadne::{ColorGenerator, Label, Report, ReportBuilder, ReportKind, Source};
use oxc::{
    allocator::Allocator,
    codegen::{CodeGenerator, CodegenReturn},
    diagnostics::{OxcDiagnostic, Severity},
    minifier::{CompressOptions, Minifier, MinifierOptions},
    parser::Parser,
    semantic::SemanticBuilder,
    span::SourceType,
    transformer::{EnvOptions, Targets, TransformOptions, Transformer},
};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum JsError {
    #[error("File not found")]
    NotFound,
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

pub async fn transform(
    path: &std::path::Path,
    minify: bool,
    source_map: bool,
) -> Result<CodegenReturn, JsError> {
    let filename = path.file_name().ok_or(JsError::NotFound)?.to_string_lossy().into_owned();
    let source_text = tokio::fs::read_to_string(&path).await.map_err(|_| JsError::NotFound)?;
    let source_type = SourceType::from_path(path).map_err(|_| JsError::NotFound)?;
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, &source_text, source_type).parse();
    handle_errors(parsed.errors, &filename, &source_text)?;
    let program = allocator.alloc(parsed.program);

    let builder_return = SemanticBuilder::new(&source_text).build(program);
    handle_errors(builder_return.errors, &filename, &source_text)?;
    let (symbols, scopes) = builder_return.semantic.into_symbol_table_and_scope_tree();

    let transform_options = TransformOptions::from_preset_env(&EnvOptions {
        targets: Targets::from_query("defaults"),
        ..EnvOptions::default()
    })
    .map_err(|v| anyhow!("{}", v.first().unwrap()))?;

    let transform_return =
        Transformer::new(&allocator, path, &source_text, parsed.trivias.clone(), transform_options)
            .build_with_symbols_and_scopes(symbols, scopes, program);
    handle_errors(transform_return.errors, &filename, &source_text)?;

    let mangler = if minify {
        Minifier::new(MinifierOptions {
            mangle: minify,
            compress: CompressOptions { drop_console: false, ..CompressOptions::all_true() },
        })
        .build(&allocator, program)
        .mangler
    } else {
        None
    };

    let mut codegen = CodeGenerator::new()
        .with_options(oxc::codegen::CodegenOptions { minify, ..Default::default() })
        .with_mangler(mangler);
    if source_map {
        let name = path.file_name().unwrap().to_string_lossy();
        codegen = codegen.enable_source_map(&name, &source_text);
    }
    Ok(codegen.build(program))
}

fn handle_errors(
    errors: Vec<OxcDiagnostic>,
    filename: &str,
    source_text: &str,
) -> Result<(), anyhow::Error> {
    let mut has_error = false;
    for diagnostic in errors {
        let mut colors = ColorGenerator::new();
        type ReportSpan<'a> = (&'a str, Range<usize>);
        let spans = diagnostic.labels.as_deref().unwrap_or_default();
        let offset = spans
            .iter()
            .find_map(|label| label.primary().then_some(label.offset()))
            .or_else(|| spans.first().map(|span| span.offset()))
            .unwrap_or_default();
        let mut report: ReportBuilder<ReportSpan> = Report::build(
            match diagnostic.severity {
                Severity::Advice => ReportKind::Advice,
                Severity::Warning => ReportKind::Warning,
                Severity::Error => ReportKind::Error,
            },
            filename,
            offset,
        )
        .with_message(diagnostic.message.clone());
        if let Some(number) = diagnostic.code.number.as_deref() {
            report = report.with_code(number);
        }
        for span in spans {
            let offset = span.offset();
            let mut label = Label::new((filename, offset..offset)).with_color(colors.next());
            if let Some(message) = span.label() {
                label = label.with_message(message);
            }
            report = report.with_label(label);
        }
        if let Some(help) = diagnostic.help.as_deref() {
            report = report.with_help(help);
        }
        if let Some(url) = diagnostic.url.as_deref() {
            report = report.with_note(url);
        }
        report.finish().print((filename, Source::from(source_text)))?;
        if diagnostic.severity == Severity::Error {
            has_error = true;
        }
    }
    if has_error { Err(anyhow!("Failed to transform JS")) } else { Ok(()) }
}
