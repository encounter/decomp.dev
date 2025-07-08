use std::convert::Infallible;

use axum::{
    extract::{FromRequestParts, OptionalFromRequestParts, Request},
    http::{Extensions, HeaderValue, StatusCode, header::CONTENT_TYPE, request::Parts},
    middleware::Next,
    response::Response,
};
use decomp_dev_auth::generate_nonce;

#[derive(Debug, Clone)]
pub struct Nonce(pub String);

impl Nonce {
    fn from_extensions(extensions: &Extensions) -> Option<Self> {
        extensions.get::<Nonce>().cloned()
    }
}

impl<S> FromRequestParts<S> for Nonce
where S: Send + Sync
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(req: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Self::from_extensions(&req.extensions)
            .ok_or((StatusCode::INTERNAL_SERVER_ERROR, "Nonce not found"))
    }
}

impl<S> OptionalFromRequestParts<S> for Nonce
where S: Send + Sync
{
    type Rejection = Infallible;

    async fn from_request_parts(
        req: &mut Parts,
        _state: &S,
    ) -> Result<Option<Self>, Self::Rejection> {
        Ok(Self::from_extensions(&req.extensions))
    }
}

#[derive(Debug, Clone, Default)]
pub struct ExtraDomains(pub Vec<String>);

pub async fn csp_middleware(mut req: Request, next: Next) -> Response {
    let nonce = generate_nonce();
    req.extensions_mut().insert(Nonce(nonce.clone()));
    let mut response = next.run(req).await;
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if content_type.starts_with("text/html") {
        let extra_domains =
            response.extensions().get::<ExtraDomains>().map(|e| e.0.clone()).unwrap_or_default();
        let response_headers = response.headers_mut();
        let mut header = "default-src 'none';base-uri 'none';script-src ".to_string();
        let nonce_value = format!("'nonce-{nonce}'");
        header.push_str(&nonce_value);
        #[cfg(debug_assertions)]
        {
            // tower-livereload script
            header.push_str(" 'sha256-L/4du8mXhXqvOm9Re02dTBSI4mWBbsqtG8F+xh3jiJc='");
        }
        header.push_str(";style-src ");
        header.push_str(&nonce_value);
        header.push_str(";img-src 'self' data:");
        for domain in &extra_domains {
            header.push(' ');
            header.push_str(domain);
        }
        header.push_str(";font-src 'self'");
        for domain in &extra_domains {
            header.push(' ');
            header.push_str(domain);
        }
        header.push_str(";connect-src 'self' https://umami.decomp.dev");
        for domain in &extra_domains {
            header.push(' ');
            header.push_str(domain);
            if let Some(domain) = domain.strip_prefix("https://") {
                header.push_str(&format!(" wss://{domain}"));
            } else if let Some(domain) = domain.strip_prefix("http://") {
                header.push_str(&format!(" ws://{domain}"));
            }
        }
        header.push_str(";manifest-src 'self'");
        response_headers.insert("Content-Security-Policy", header.parse().unwrap());
        response_headers
            .insert("Cross-Origin-Embedder-Policy", HeaderValue::from_static("require-corp"));
        response_headers
            .insert("Cross-Origin-Opener-Policy", HeaderValue::from_static("same-origin"));
        response_headers
            .insert("Referrer-Policy", HeaderValue::from_static("strict-origin-when-cross-origin"));
        response_headers.insert("X-Frame-Options", HeaderValue::from_static("DENY"));
    }
    let response_headers = response.headers_mut();
    response_headers.insert("X-Content-Type-Options", HeaderValue::from_static("nosniff"));
    response
}
