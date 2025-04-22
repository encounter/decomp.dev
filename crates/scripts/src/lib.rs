pub mod css;
pub mod js;

use std::ffi::OsStr;

use anyhow::anyhow;
use axum::{
    extract::Path,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use decomp_dev_core::{AppError, util::join_normalized};

pub async fn get_css(Path(filename): Path<String>) -> Result<Response, AppError> {
    let path = join_normalized("css", &filename);
    if path.extension() != Some(OsStr::new("css")) {
        return Err(AppError::Status(StatusCode::NOT_FOUND));
    }
    let output = css::transform(&path).map_err(|e| AppError::Internal(anyhow!(e.to_string())))?;
    Ok((
        [
            (header::CONTENT_TYPE, mime::TEXT_CSS_UTF_8.as_ref()),
            #[cfg(not(debug_assertions))]
            (header::CACHE_CONTROL, "public, max-age=3600"),
            #[cfg(debug_assertions)]
            (header::CACHE_CONTROL, "no-cache"),
        ],
        output,
    )
        .into_response())
}

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
    let ret = match js::transform(&path, minify, response_type == ResponseType::SourceMap).await {
        Ok(ret) => ret,
        Err(js::JsError::NotFound) => return Err(AppError::Status(StatusCode::NOT_FOUND)),
        Err(js::JsError::Internal(e)) => return Err(AppError::Internal(e)),
    };
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
