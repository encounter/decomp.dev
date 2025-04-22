use std::str::FromStr;

use axum::{
    Router,
    http::{HeaderMap, header},
    routing::{get, post},
};
use decomp_dev_images::image_mime_from_ext;
use mime::Mime;

use crate::AppState;

mod common;
mod project;
mod report;
mod treemap;

pub fn build_router() -> Router<AppState> {
    Router::new()
        .route("/api/github/webhook", post(decomp_dev_github::webhook::webhook))
        .route("/api/github/oauth", get(decomp_dev_auth::oauth))
        .route("/login", get(decomp_dev_auth::login))
        .route("/logout", post(decomp_dev_auth::logout))
        .route("/css/{*filename}", get(decomp_dev_scripts::get_css))
        .route("/js/{*filename}", get(decomp_dev_scripts::get_js))
        .route("/assets/{*filename}", get(decomp_dev_images::get_asset))
        .route("/og.png", get(decomp_dev_images::get_og))
        .route("/", get(project::get_projects))
        .route("/{owner}/{repo}", get(report::get_report))
        .route("/{owner}/{repo}/{version}", get(report::get_report))
        .route("/{owner}/{repo}/{version}/{commit}", get(report::get_report))
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
