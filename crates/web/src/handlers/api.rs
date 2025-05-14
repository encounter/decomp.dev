use axum::response::{IntoResponse, Response};
use decomp_dev_auth::CurrentUser;
use decomp_dev_core::AppError;
use maud::{DOCTYPE, html};

use crate::handlers::common::{Load, TemplateContext, nav_links};

pub async fn overview(
    mut ctx: TemplateContext,
    current_user: Option<CurrentUser>,
) -> Result<Response, AppError> {
    let rendered = html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                title { "API • decomp.dev" }
                (ctx.header().await)
                (ctx.chunks("main", Load::Deferred).await)
                (ctx.chunks("api", Load::Deferred).await)
            }
            body {
                header {
                    nav {
                        ul {
                            li {
                                a href="/" { strong { "decomp.dev" } }
                            }
                            li {
                                a href="/api" { "API" }
                            }
                        }
                        (nav_links())
                    }
                }
                main {
                    h1 { "API" }
                    #root {}
                }
            }
            (ctx.footer(current_user.as_ref()))
        }
    };
    Ok((ctx, rendered).into_response())
}
