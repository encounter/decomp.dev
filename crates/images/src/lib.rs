pub mod badge;
pub mod svg;
pub mod treemap;

use std::str::FromStr;

use axum::{
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use decomp_dev_core::{AppError, util::join_normalized};
use mime::Mime;

pub fn image_mime_from_ext(ext: &str) -> Option<Mime> {
    image::ImageFormat::from_extension(ext)
        .map(|format| Mime::from_str(format.to_mime_type()).unwrap())
}

pub async fn get_og() -> Result<Response, AppError> {
    let path = join_normalized("assets", "og.svg");
    let svg_src = tokio::fs::read_to_string(&path)
        .await
        .map_err(|_| AppError::Status(StatusCode::NOT_FOUND))?;
    let data = svg::render_image(&svg_src, image::ImageFormat::Png)?;
    Ok((
        [(header::CONTENT_TYPE, "image/png"), (header::CACHE_CONTROL, "public, max-age=3600")],
        data,
    )
        .into_response())
}
