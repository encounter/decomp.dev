use anyhow::Result;
use decomp_dev_images::svg;
use image::ImageFormat;
use maud::{PreEscaped, html};

use crate::handlers::report::ReportTemplateUnit;

pub fn render_svg(units: &[ReportTemplateUnit], w: u32, h: u32) -> String {
    html! {
        (PreEscaped("<?xml version=\"1.0\" encoding=\"utf-8\"?>"))
        svg xmlns="http://www.w3.org/2000/svg" version="1.1" viewBox=(format!("0 0 {w} {h}")) {
            style { ".unit { stroke: #000; stroke-width: 1; }" }
            @for unit in units {
                rect class="unit"
                    width=(format!("{}%", unit.w * 100.0))
                    height=(format!("{}%", unit.h * 100.0))
                    x=(format!("{}%", unit.x * 100.0))
                    y=(format!("{}%", unit.y * 100.0))
                    fill=(unit.color) {}
            }
        }
    }
    .into_string()
}

pub fn render_image(
    units: &[ReportTemplateUnit],
    w: u32,
    h: u32,
    format: ImageFormat,
) -> Result<Vec<u8>> {
    let svg = render_svg(units, w, h);
    svg::render_image(&svg, format)
}
