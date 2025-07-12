use anyhow::Result;
use decomp_dev_images::{
    svg,
    treemap::{color_mix, hsl, html_color},
};
use image::ImageFormat;
use maud::{PreEscaped, html};

use crate::handlers::report::ReportTemplateUnit;

pub fn render_svg(units: &[ReportTemplateUnit], w: u32, h: u32) -> String {
    let complete_c0 = html_color(hsl(120, 100, 39));
    let complete_c1 = html_color(hsl(120, 100, 17));
    html! {
        (PreEscaped("<?xml version=\"1.0\" encoding=\"utf-8\"?>"))
        svg xmlns="http://www.w3.org/2000/svg" version="1.1" viewBox=(format!("0 0 {w} {h}")) width=(w) height=(h) {
            style { ".unit { stroke: #000; stroke-width: 0.5; }" }
            @for (i, unit) in units.iter().enumerate() {
                radialGradient id=(format!("unit-{i}")) 
                    gradientUnits="userSpaceOnUse"
                    cx=(format!("{}%", (unit.x + (unit.w * 0.4)) * 100.0))
                    cy=(format!("{}%", (unit.y + (unit.h * 0.4)) * 100.0)) 
                    fr=(format!("{}%", (unit.w + unit.h) * 10.0)) 
                    r=(format!("{}%", (unit.w + unit.h) * 50.0)) {
                    @let pct = unit.fuzzy_match_percent;
                    @if pct == 100.0 {
                        stop offset="0%" stop-color=(complete_c0) {}
                        stop offset="100%" stop-color=(complete_c1) {}
                    } @else {
                        stop offset="0%" stop-color=(html_color(color_mix(hsl(221, 0, 21), hsl(221, 100, 35), pct / 100.0))) {}
                        stop offset="100%" stop-color=(html_color(color_mix(hsl(221, 0, 5), hsl(221, 100, 15), pct / 100.0))) {}
                    }
                }
            }
            @for (i, unit) in units.iter().enumerate() {
                @let pct = unit.fuzzy_match_percent.floor();
                rect.unit
                    width=(format!("{}%", unit.w * 100.0))
                    height=(format!("{}%", unit.h * 100.0))
                    x=(format!("{}%", unit.x * 100.0))
                    y=(format!("{}%", unit.y * 100.0))
                    fill=(format!("url(#unit-{i})")) {}
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
