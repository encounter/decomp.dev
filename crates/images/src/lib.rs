pub mod badge;
pub mod svg;
pub mod treemap;

use std::{io::Cursor, str::FromStr};

use axum::{
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use decomp_dev_core::{AppError, util::join_normalized};
use image::{
    DynamicImage, ExtendedColorType, ImageError, ImageFormat, error::UnsupportedErrorKind,
};
use mime::Mime;

pub fn image_mime_from_ext(ext: &str) -> Option<Mime> {
    ImageFormat::from_extension(ext).map(|format| Mime::from_str(format.to_mime_type()).unwrap())
}

pub async fn get_og() -> Result<Response, AppError> {
    let path = join_normalized("assets", "og.svg");
    let svg_src = tokio::fs::read_to_string(&path)
        .await
        .map_err(|_| AppError::Status(StatusCode::NOT_FOUND))?;
    let data = svg::render_image(&svg_src, ImageFormat::Png)?;
    Ok((
        [(header::CONTENT_TYPE, "image/png"), (header::CACHE_CONTROL, "public, max-age=3600")],
        data,
    )
        .into_response())
}

pub fn encode_image(image: &DynamicImage, format: ImageFormat) -> Result<Vec<u8>, anyhow::Error> {
    match format {
        ImageFormat::WebP => {
            let encoder = webp::Encoder::from_image(image)
                .map_err(|s| anyhow::anyhow!("Failed to create WebP encoder: {}", s))?;
            let memory = encoder.encode(75.0);
            Ok(memory.to_vec())
        }
        format => {
            let mut buf = Cursor::new(Vec::new());
            match image.write_to(&mut buf, format) {
                Ok(()) => {}
                Err(ImageError::Unsupported(e))
                    if matches!(
                        e.kind(),
                        UnsupportedErrorKind::Color(ExtendedColorType::Rgba8)
                    ) =>
                {
                    // Convert to RGB and try again
                    let image = image.to_rgb8();
                    image.write_to(&mut buf, format)?;
                }
                Err(e) => return Err(anyhow::Error::from(e).context("Failed to encode image")),
            }
            Ok(buf.into_inner())
        }
    }
}
