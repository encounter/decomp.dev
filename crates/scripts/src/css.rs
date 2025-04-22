use std::{ffi::OsStr, path::Path};

use anyhow::anyhow;

pub fn transform(path: &Path) -> Result<String, anyhow::Error> {
    let mut path = path.with_extension("");
    let printer_options = lightningcss::stylesheet::PrinterOptions {
        minify: path.extension() == Some(OsStr::new("min")),
        ..Default::default()
    };
    path = path.with_extension("scss");
    let options = grass::Options::default().load_path("node_modules");
    let mut output = grass::from_path(&path, &options)?;
    // Skip lightningcss entirely if we're not minifying
    if printer_options.minify {
        let options = lightningcss::stylesheet::ParserOptions::default();
        let stylesheet = lightningcss::stylesheet::StyleSheet::parse(&output, options)
            .map_err(|e| anyhow!(e.to_string()))?;
        let result = stylesheet.to_css(printer_options)?;
        drop(stylesheet);
        output = result.code;
    }
    Ok(output)
}
