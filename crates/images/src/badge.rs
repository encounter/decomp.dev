use anyhow::{Result, anyhow};
use decomp_dev_core::util::size;
use image::ImageFormat;
use objdiff_core::bindings::report::Measures;
use serde::{Deserialize, Serialize};

use crate::svg;

#[derive(Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ShieldParams {
    label: Option<String>,
    label_color: Option<String>,
    color: Option<String>,
    style: Option<String>,
    measure: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ShieldResponse {
    schema_version: u32,
    label: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    style: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    label_color: Option<String>,
}

fn format_percent(value: f32) -> String { format!("{:.2}%", value) }

fn format_bytes(a: u64, b: u64) -> String {
    let mut a_buf = num_format::Buffer::default();
    let mut b_buf = num_format::Buffer::default();
    a_buf.write_formatted(&a, &num_format::Locale::en);
    b_buf.write_formatted(&b, &num_format::Locale::en);
    format!("{} B / {} B", a_buf, b_buf)
}

fn format_num(a: u32, b: u32) -> String {
    let mut a_buf = num_format::Buffer::default();
    let mut b_buf = num_format::Buffer::default();
    a_buf.write_formatted(&a, &num_format::Locale::en);
    b_buf.write_formatted(&b, &num_format::Locale::en);
    format!("{} / {}", a_buf, b_buf)
}

fn format_size(a: u64, b: u64) -> String { format!("{} / {}", size(a), size(b)) }

pub fn render(
    measures: &Measures,
    default_label: &str,
    params: &ShieldParams,
) -> Result<ShieldResponse> {
    let label = params.label.clone().unwrap_or_else(|| default_label.to_string());
    let message = if let Some(measure) = &params.measure {
        match measure.as_str() {
            "fuzzy_match_percent" | "fuzzy_match" => format_percent(measures.fuzzy_match_percent),
            "matched_code_percent" | "matched_code" | "code" => {
                format_percent(measures.matched_code_percent)
            }
            "matched_code_bytes" => format_bytes(measures.matched_code, measures.total_code),
            "matched_code_size" => format_size(measures.matched_code, measures.total_code),
            "matched_data_percent" | "matched_data" | "data" => {
                format_percent(measures.matched_data_percent)
            }
            "matched_data_bytes" => format_bytes(measures.matched_data, measures.total_data),
            "matched_data_size" => format_size(measures.matched_data, measures.total_data),
            "matched_functions" | "functions" => {
                format_num(measures.matched_functions, measures.total_functions)
            }
            "matched_functions_percent" => format_percent(measures.matched_functions_percent),
            "complete_code_percent" | "complete_code" => {
                format_percent(measures.complete_code_percent)
            }
            "complete_code_bytes" => format_bytes(measures.complete_code, measures.total_code),
            "complete_code_size" => format_size(measures.complete_code, measures.total_code),
            "complete_data_percent" | "complete_data" => {
                format_percent(measures.complete_data_percent)
            }
            "complete_data_bytes" => format_bytes(measures.complete_data, measures.total_data),
            "complete_data_size" => format_size(measures.complete_data, measures.total_data),
            "complete_units" => format_num(measures.complete_units, measures.total_units),
            "complete_units_percent" => {
                let percent = if measures.total_units == 0 {
                    100.0
                } else {
                    measures.complete_units as f32 / measures.total_units as f32 * 100.0
                };
                format_percent(percent)
            }
            _ => return Err(anyhow!("Unknown measure")),
        }
    } else {
        format_percent(measures.matched_code_percent)
    };
    Ok(ShieldResponse {
        schema_version: 1,
        label,
        message,
        color: Some(params.color.clone().unwrap_or_else(|| "informational".to_string())),
        style: params.style.clone(),
        label_color: params.label_color.clone(),
    })
}

pub fn render_svg(
    measures: &Measures,
    default_label: &str,
    params: &ShieldParams,
) -> Result<String> {
    let response = render(measures, default_label, params)?;
    let mut builder = badge_maker::BadgeBuilder::new();
    builder.label(&response.label).message(&response.message);
    if let Some(color) = &response.color {
        builder.color_parse(color);
    }
    if let Some(style) = &response.style {
        builder.style_parse(style);
    }
    if let Some(label_color) = &response.label_color {
        builder.label_color_parse(label_color);
    }
    let badge = builder.build()?;
    Ok(badge.svg())
}

pub fn render_image(
    measures: &Measures,
    default_label: &str,
    params: &ShieldParams,
    format: ImageFormat,
) -> Result<Vec<u8>> {
    let svg = render_svg(measures, default_label, params)?;
    svg::render_image(&svg, format)
}
