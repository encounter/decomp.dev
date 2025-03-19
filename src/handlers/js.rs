use std::{ffi::OsStr, ops::Range};

use anyhow::{anyhow, Result};
use ariadne::{ColorGenerator, Label, Report, ReportBuilder, ReportKind, Source};
use axum::{
    extract::Path,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};
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

use crate::{handlers::AppError, util::join_normalized};

pub async fn get_js(Path(filename): Path<String>) -> Result<Response, AppError> {
    let mut path = join_normalized("js", &filename);
    #[derive(Debug, Copy, Clone, Eq, PartialEq)]
    enum ResponseType {
        Js,
        SourceMap,
    }
    let response_type;
    if path.extension() == Some(OsStr::new("js")) {
        response_type = ResponseType::Js;
    } else if path.extension() == Some(OsStr::new("map")) {
        path = path.with_extension("");
        if path.extension() != Some(OsStr::new("js")) {
            return Err(AppError::Status(StatusCode::NOT_FOUND));
        }
        response_type = ResponseType::SourceMap;
    } else {
        return Err(AppError::Status(StatusCode::NOT_FOUND));
    }
    path = path.with_extension("");
    let minify = path.extension() == Some(OsStr::new("min"));
    path = path.with_extension("ts");
    let ret = transform(&path, minify, response_type == ResponseType::SourceMap).await?;
    let (data, content_type) = match response_type {
        ResponseType::Js => (
            format!("{}\n//# sourceMappingURL={}.map", ret.code, filename),
            mime::APPLICATION_JAVASCRIPT_UTF_8.as_ref(),
        ),
        ResponseType::SourceMap => {
            (ret.map.unwrap().to_json_string(), mime::APPLICATION_JSON.as_ref())
        }
    };
    Ok((
        [
            (header::CONTENT_TYPE, content_type),
            #[cfg(not(debug_assertions))]
            (header::CACHE_CONTROL, "public, max-age=3600"),
            #[cfg(debug_assertions)]
            (header::CACHE_CONTROL, "no-cache"),
        ],
        data,
    )
        .into_response())
}

async fn transform(
    path: &std::path::Path,
    minify: bool,
    source_map: bool,
) -> Result<CodegenReturn, AppError> {
    let filename = path
        .file_name()
        .ok_or(AppError::Status(StatusCode::NOT_FOUND))?
        .to_string_lossy()
        .into_owned();
    let source_text = tokio::fs::read_to_string(&path)
        .await
        .map_err(|_| AppError::Status(StatusCode::NOT_FOUND))?;
    let source_type =
        SourceType::from_path(path).map_err(|_| AppError::Status(StatusCode::NOT_FOUND))?;
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
) -> Result<(), AppError> {
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
            if let Some(message) = span.label().as_deref() {
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
    if has_error {
        Err(AppError::Status(StatusCode::INTERNAL_SERVER_ERROR))
    } else {
        Ok(())
    }
}
