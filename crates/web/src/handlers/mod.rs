use std::str::FromStr;

use axum::{
    Router,
    extract::Request,
    http::{HeaderMap, HeaderValue, header, header::Entry},
    response::Response,
    routing::{get, post},
};
use decomp_dev_images::image_mime_from_ext;
use mime::Mime;
use tower::ServiceExt;
use tower_http::services::ServeDir;

use crate::AppState;

mod api;
mod auth;
mod common;
pub mod csp;
mod manage;
mod project;
mod report;
mod treemap;

pub fn build_router() -> Router<AppState> {
    Router::new()
        .nest_service(
            "/static",
            <ServeDir as ServiceExt<Request>>::map_response(
                ServeDir::new("dist/static"),
                |mut response| {
                    add_charset(&mut response);
                    // Cache static (hashed) files for a year, mark immutable
                    add_cache_control(
                        &mut response,
                        HeaderValue::from_static("public, max-age=31536000, immutable"),
                    );
                    response
                },
            ),
        )
        .fallback_service(<ServeDir as ServiceExt<Request>>::map_response(
            ServeDir::new("dist"),
            |mut response| {
                add_charset(&mut response);
                // Cache non-hashed public files for a day, mark must-revalidate
                add_cache_control(
                    &mut response,
                    HeaderValue::from_static("public, max-age=86400, must-revalidate"),
                );
                response
            },
        ))
        .route("/robots.txt", get(common::get_robots))
        .route("/api", get(api::overview))
        .route("/api/github/webhook", post(decomp_dev_github::webhook::webhook))
        .route("/api/github/oauth", get(decomp_dev_auth::oauth))
        .route("/login", get(auth::login))
        .route("/logout", post(auth::logout))
        .route("/manage", get(manage::manage))
        .route("/manage/new", get(manage::new))
        .route("/manage/new", post(manage::new_save))
        .route("/manage/{owner}/{repo}", get(manage::manage_project))
        .route("/manage/{owner}/{repo}", post(manage::manage_project_save))
        .route("/manage/{owner}/{repo}/refresh", post(manage::manage_project_refresh))
        .route("/og.png", get(decomp_dev_images::get_og))
        .route("/", get(project::get_projects))
        .route("/projects", get(project::get_projects))
        .route("/projects.json", get(project::get_projects))
        .route("/projects/{id}", get(report::get_report))
        .route("/{owner}/{repo}", get(report::get_report))
        .route("/{owner}/{repo}/{version}", get(report::get_report))
        .route("/{owner}/{repo}/{version}/{commit}", get(report::get_report))
}

/// Adds a charset to the Content-Type header if it is missing and the type is text/*.
fn add_charset<ResBody>(response: &mut Response<ResBody>) {
    let Entry::Occupied(mut existing) = response.headers_mut().entry(header::CONTENT_TYPE) else {
        return;
    };
    let Some(mime) = existing.get().to_str().ok().and_then(|v| Mime::from_str(v).ok()) else {
        return;
    };
    if mime.type_() == mime::TEXT && mime.get_param("charset").is_none() {
        existing.insert(format!("{};charset=utf-8", mime).parse().unwrap());
    }
}

/// Adds a Cache-Control header if it is missing.
fn add_cache_control<ResBody>(response: &mut Response<ResBody>, value: HeaderValue) {
    response.headers_mut().entry(header::CACHE_CONTROL).or_insert(value);
}

pub fn parse_accept(headers: &HeaderMap, ext: Option<&str>) -> Vec<Mime> {
    // Explicit extension takes precedence
    if let Some(ext) = ext {
        return match ext.to_ascii_lowercase().as_str() {
            "json" => vec![mime::APPLICATION_JSON],
            "binpb" | "proto" => vec![Mime::from_str("application/x-protobuf").unwrap()],
            "svg" => vec![mime::IMAGE_SVG],
            _ => {
                if let Some(mime) = image_mime_from_ext(ext) {
                    vec![mime]
                } else {
                    // An unknown extension should be NOT_ACCEPTABLE, not */*.
                    vec![]
                }
            }
        };
    }
    // Otherwise, parse the Accept header
    let result = headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .iter()
        .flat_map(|s| s.split(','))
        .map(|s| s.trim())
        .filter_map(|s| Mime::from_str(s).ok())
        .collect::<Vec<_>>();
    if result.is_empty() {
        // If no Accept header is present, use */*
        vec![mime::STAR_STAR]
    } else {
        result
    }
}
