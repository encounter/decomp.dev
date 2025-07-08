use std::{io::Cursor, str::FromStr};

use anyhow::anyhow;
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use decomp_dev_core::{AppError, models::ImageId};
use decomp_dev_images::encode_image;
use image::{ImageFormat, ImageReader};

use crate::{AppState, handlers::parse_accept};

#[derive(serde::Deserialize)]
pub struct ImageParams {
    pub id: String,
}

#[derive(serde::Deserialize)]
pub struct ImageQuery {
    #[serde(alias = "w")]
    pub width: Option<u32>,
    #[serde(alias = "h")]
    pub height: Option<u32>,
    #[serde(alias = "b")]
    pub blur: Option<f32>,
}

enum Transform {
    Resize(u32, u32),
    Blur(f32),
}

pub async fn get_image(
    Path(params): Path<ImageParams>,
    Query(query): Query<ImageQuery>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let (params, ext) = extract_extension(params);
    let acceptable = parse_accept(&headers, ext.as_deref());
    if acceptable.is_empty() {
        return Err(AppError::Status(StatusCode::NOT_ACCEPTABLE));
    }

    let id = hex::decode(&params.id).map_err(|_| AppError::Status(StatusCode::NOT_FOUND))?;
    let id: ImageId =
        id.as_slice().try_into().map_err(|_| AppError::Status(StatusCode::NOT_FOUND))?;
    let (mime_str, image_width, image_height, data) =
        state.db.get_image(id).await?.ok_or_else(|| AppError::Status(StatusCode::NOT_FOUND))?;
    let mime_type = mime::Mime::from_str(&mime_str)?;

    let mut transforms = Vec::<Transform>::new();
    let mut current_width = image_width;
    let mut current_height = image_height;
    if let Some(width) = query.width
        && current_width > width
    {
        // Maintaining aspect ratio
        let height = (current_height as f32 * (width as f32 / current_width as f32)) as u32;
        current_width = width;
        current_height = height;
    }
    if let Some(height) = query.height
        && current_height > height
    {
        // Maintaining aspect ratio
        let width = (current_width as f32 * (height as f32 / current_height as f32)) as u32;
        current_width = width;
        current_height = height;
    }
    if current_width != image_width || current_height != image_height {
        transforms.push(Transform::Resize(current_width, current_height));
    }
    if let Some(blur) = query.blur {
        transforms.push(Transform::Blur(blur));
    }

    let mut out_headers = HeaderMap::new();
    out_headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000, immutable"),
    );
    if ext.is_none() {
        out_headers.insert(header::VARY, HeaderValue::from_static("accept"));
    }

    // If no transformations are needed, and the mime type is acceptable, return the original image
    let orig_acceptable = acceptable.iter().any(|m| {
        let essence = m.essence_str();
        essence == mime_type || essence == mime::IMAGE_STAR || essence == mime::STAR_STAR
    });
    if transforms.is_empty() && orig_acceptable {
        out_headers.insert(header::CONTENT_TYPE, mime_str.parse()?);
        return Ok((out_headers, data).into_response());
    }

    let format = ImageFormat::from_mime_type(&mime_str)
        .ok_or_else(|| anyhow!("Invalid image mime type: {}", mime_str))?;
    let mut image = ImageReader::with_format(Cursor::new(&data[..]), format).decode()?;
    for transform in transforms {
        match transform {
            Transform::Resize(width, height) => {
                image = image.resize_exact(width, height, image::imageops::FilterType::Lanczos3);
            }
            Transform::Blur(blur) => {
                image = image.blur(blur);
            }
        }
    }

    let mut out_format = None;
    for mime in acceptable {
        if mime.type_() != mime::IMAGE {
            continue;
        }
        // If a specific image format is requested, use that
        if let Some(format) = ImageFormat::from_mime_type(mime.essence_str()) {
            out_format = Some(format);
            break;
        }
    }
    // Otherwise, use WebP as the default
    let out_format = out_format.unwrap_or(ImageFormat::WebP);
    let encoded = encode_image(&image, out_format)?;

    out_headers.insert(header::CONTENT_TYPE, out_format.to_mime_type().parse()?);
    Ok((out_headers, encoded).into_response())
}

fn extract_extension(params: ImageParams) -> (ImageParams, Option<String>) {
    if let Some((id, ext)) = params.id.rsplit_once('.') {
        return (ImageParams { id: id.to_string(), ..params }, Some(ext.to_string()));
    }
    (params, None)
}
