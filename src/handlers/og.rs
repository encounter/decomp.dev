use axum::{
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};

use crate::{handlers::AppError, svg, util::join_normalized};

pub async fn get_og() -> Result<Response, AppError> {
    let path = join_normalized("templates", "og.svg");
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
