use anyhow::{Result, anyhow, bail};
use graphql_client::{GraphQLQuery, Response};
use octocrab::Octocrab;

#[allow(clippy::upper_case_acronyms)]
type URI = String;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "graphql/schema.graphql",
    query_path = "graphql/queries.graphql",
    response_derives = "Debug, Clone"
)]
pub struct ViewerQuery;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CurrentUserResponse {
    pub login: String,
    pub url: String,
    pub repositories: Vec<CurrentUserRepository>,
}

#[derive(
    Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum RepositoryPermission {
    None,
    Read,
    Triage,
    Write,
    Maintain,
    Admin,
}

impl From<viewer_query::RepositoryPermission> for RepositoryPermission {
    fn from(value: viewer_query::RepositoryPermission) -> Self {
        match value {
            viewer_query::RepositoryPermission::ADMIN => RepositoryPermission::Admin,
            viewer_query::RepositoryPermission::MAINTAIN => RepositoryPermission::Maintain,
            viewer_query::RepositoryPermission::READ => RepositoryPermission::Read,
            viewer_query::RepositoryPermission::TRIAGE => RepositoryPermission::Triage,
            viewer_query::RepositoryPermission::WRITE => RepositoryPermission::Write,
            viewer_query::RepositoryPermission::Other(_) => RepositoryPermission::None,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CurrentUserRepository {
    pub id: u64,
    pub owner: String,
    pub name: String,
    pub permission: RepositoryPermission,
}

async fn run_query<T: GraphQLQuery>(
    client: &Octocrab,
    variables: T::Variables,
) -> Result<T::ResponseData> {
    let query = T::build_query(variables);
    let response: Response<T::ResponseData> = client.graphql(&query).await?;
    if let Some(errors) = response.errors {
        let message = errors.into_iter().map(|error| error.message).collect::<Vec<_>>().join("\n");
        bail!("GraphQL query failed: {message}");
    }
    response.data.ok_or_else(|| anyhow!("No data returned from GraphQL query"))
}

pub async fn fetch_current_user(client: &Octocrab) -> Result<CurrentUserResponse> {
    let mut result =
        CurrentUserResponse { login: String::new(), url: String::new(), repositories: Vec::new() };
    let mut after = None;
    loop {
        let data =
            run_query::<ViewerQuery>(client, viewer_query::Variables { after: after.clone() })
                .await?
                .viewer;
        result.login = data.login;
        result.url = data.url;
        for repo in data.repositories.nodes.unwrap_or_default().into_iter().flatten() {
            result.repositories.push(CurrentUserRepository {
                id: repo.database_id.unwrap_or_default() as u64,
                owner: repo.owner.login,
                name: repo.name,
                permission: repo
                    .viewer_permission
                    .map(RepositoryPermission::from)
                    .unwrap_or_else(|| RepositoryPermission::None),
            });
        }
        if !data.repositories.page_info.has_next_page {
            break;
        }
        let Some(end_cursor) = data.repositories.page_info.end_cursor else {
            bail!("hasNextPage is true but endCursor is null");
        };
        if after.is_some_and(|a| a == end_cursor) {
            bail!("Infinite loop detected: after cursor is the same as before");
        }
        after = Some(end_cursor);
    }
    Ok(result)
}
