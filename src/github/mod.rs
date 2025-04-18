use std::{
    collections::{hash_map::Entry, HashMap},
    ffi::OsStr,
    io::{Cursor, Read},
    pin::pin,
    sync::{Arc, OnceLock},
};

use anyhow::{Context, Result};
use axum::http::StatusCode;
use futures_util::TryStreamExt;
use objdiff_core::bindings::report::Report;
use octocrab::{
    models::{repos::RepoCommitPage, ArtifactId, InstallationId, RunId},
    params::actions::ArchiveFormat,
    GitHubError, Octocrab,
};
use regex::Regex;
use tokio::{
    sync::{Mutex, Semaphore},
    task::JoinSet,
};

use crate::{
    config::GitHubConfig,
    models::{Commit, Project},
    AppState,
};

#[derive(Clone)]
pub struct GitHub {
    pub client: Octocrab,
    pub installations: Option<Arc<Mutex<Installations>>>,
}

pub struct Installations {
    pub app_client: Octocrab,
    pub owner_to_installation: HashMap<String, InstallationId>,
    pub clients: HashMap<InstallationId, Octocrab>,
}

impl Installations {
    pub fn client_for_installation(
        &mut self,
        installation_id: InstallationId,
        owner: Option<&str>,
    ) -> Result<Octocrab> {
        match self.clients.entry(installation_id) {
            Entry::Occupied(entry) => Ok(entry.get().clone()),
            Entry::Vacant(entry) => {
                // Create a new client for the installation
                let client = self.app_client.installation(installation_id)?;
                entry.insert(client.clone());
                if let Some(owner) = owner {
                    self.owner_to_installation.insert(owner.to_string(), installation_id);
                }
                Ok(client)
            }
        }
    }

    pub fn client_for_owner(&mut self, owner: &str) -> Result<Option<Octocrab>> {
        if let Some(installation_id) = self.owner_to_installation.get(owner) {
            return self.client_for_installation(*installation_id, None).map(Some);
        }
        Ok(None)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GetCommit {
    owner: String,
    repo: String,
    sha: String,
}

async fn list_installations(app_client: Octocrab) -> Result<Installations> {
    let mut owner_to_installation = HashMap::new();
    let mut clients = HashMap::new();
    {
        let mut stream =
            pin!(app_client.apps().installations().send().await?.into_stream(&app_client));
        while let Some(installation) = stream.try_next().await? {
            let owner = installation.account.login;
            if owner_to_installation.contains_key(&owner) {
                tracing::warn!("Duplicate installation for {}", owner);
                continue;
            }
            let client = app_client.installation(installation.id)?;
            owner_to_installation.insert(owner.clone(), installation.id);
            clients.insert(installation.id, client);
        }
    }
    Ok(Installations { app_client, owner_to_installation, clients })
}

impl GitHub {
    pub async fn new(config: &GitHubConfig) -> Result<Self> {
        let client = Octocrab::builder()
            .personal_token(config.token.clone())
            .build()
            .context("Failed to create GitHub client")?;
        octocrab::initialise(client.clone());
        let profile = client.current().user().await.context("Failed to fetch current user")?;
        tracing::info!("Logged in as {}", profile.login);

        let installations = if let Some(app_config) = &config.app {
            let app_client = Octocrab::builder()
                .app(
                    app_config.id.into(),
                    jsonwebtoken::EncodingKey::from_rsa_pem(app_config.private_key.as_bytes())?,
                )
                .build()
                .context("Failed to create GitHub client")?;
            let result =
                list_installations(app_client).await.context("Failed to fetch installations")?;
            tracing::info!("Found {} installations", result.clients.len());
            for (owner, installation_id) in &result.owner_to_installation {
                tracing::info!("  - {}: {}", owner, installation_id);
            }
            Some(Arc::new(Mutex::new(result)))
        } else {
            None
        };
        Ok(Self { client, installations })
    }

    pub async fn get_commit(
        &self,
        owner: &str,
        repo: &str,
        sha: &str,
    ) -> Result<Option<RepoCommitPage>> {
        match self.client.repos(owner, repo).list_commits().sha(sha).per_page(1).send().await {
            Ok(page) => Ok(page.items.into_iter().next().map(|c| c.commit)),
            Err(octocrab::Error::GitHub { source, .. })
                if matches!(*source, GitHubError { status_code: StatusCode::NOT_FOUND, .. }) =>
            {
                Ok(None)
            }
            Err(e) => Err(e.into()),
        }
    }

    pub async fn client_for(&self, owner: &str) -> Result<Octocrab> {
        if let Some(installations) = &self.installations {
            let mut installations = installations.lock().await;
            if let Some(client) = installations.client_for_owner(owner)? {
                return Ok(client);
            }
        }
        Ok(self.client.clone())
    }
}

pub async fn run(state: &AppState, repo_id: u64, stop_run_id: u64) -> Result<()> {
    let mut project_info = state
        .db
        .get_project_info_by_id(repo_id, None)
        .await
        .context("Failed to fetch project info")?
        .with_context(|| format!("Failed to fetch project info for ID {}", repo_id))?;
    let repo = state
        .github
        .client
        .repos_by_id(project_info.project.id)
        .get()
        .await
        .with_context(|| format!("Failed to fetch repo for ID {}", repo_id))?;
    let branch = repo.default_branch.as_deref().unwrap_or("main");

    let owner = repo.owner.context("Repository has no owner")?;
    if project_info.project.owner != owner.login || project_info.project.repo != repo.name {
        tracing::info!(
            "Migrating project from {}/{} to {}/{}",
            project_info.project.owner,
            project_info.project.repo,
            owner.login,
            repo.name
        );
        state
            .db
            .update_project_owner_repo(project_info.project.id, &owner.login, &repo.name)
            .await?;
        project_info = state
            .db
            .get_project_info_by_id(repo_id, None)
            .await
            .context("Failed to fetch project info")?
            .with_context(|| format!("Failed to fetch project info for ID {}", repo_id))?;
    }

    let project = &project_info.project;
    tracing::debug!("Refreshing project {}/{}", project.owner, project.repo);
    let client = state.github.client_for(&project.owner).await?;

    let workflow_ids = if let Some(workflow_id) = &project.workflow_id {
        vec![workflow_id.clone()]
    } else {
        let workflows = client
            .workflows(&project.owner, &project.repo)
            .list()
            .send()
            .await
            .context("Failed to fetch workflows")?;
        workflows.items.into_iter().map(|w| w.path).collect()
    };
    if workflow_ids.is_empty() {
        tracing::warn!("No workflows found for {}/{}", project.owner, project.repo);
        return Ok(());
    }
    for workflow_id in workflow_ids {
        let workflow_id =
            workflow_id.strip_prefix(".github/workflows/").unwrap_or(workflow_id.as_str());
        let mut runs = vec![];
        let mut page = 1u32;
        'outer: loop {
            let result = client
                .workflows(&project.owner, &project.repo)
                .list_runs(workflow_id)
                .branch(branch)
                .event("push")
                .status("completed")
                .exclude_pull_requests(true)
                .page(page)
                .send()
                .await;
            let items = match result {
                Ok(result) if result.items.is_empty() => break,
                Ok(result) => result.items,
                Err(octocrab::Error::GitHub { source, .. })
                    if matches!(*source, GitHubError {
                        status_code: StatusCode::NOT_FOUND,
                        ..
                    }) =>
                {
                    break
                }
                Err(e) => {
                    return Err(e)
                        .with_context(|| format!("Failed to fetch workflows page {}", page));
                }
            };
            for run in items {
                if let Some(commit) = project_info.commit.as_ref() {
                    if run.head_sha == commit.sha {
                        break 'outer;
                    }
                }
                let run_id = run.id;
                runs.push(run);
                if run_id == RunId(stop_run_id) {
                    break 'outer;
                }
            }
            page += 1;
        }
        tracing::info!(
            "Fetched {} runs for project {}/{} {}",
            runs.len(),
            project.owner,
            project.repo,
            workflow_id
        );

        struct TaskResult {
            run_id: RunId,
            commit: Commit,
            result: Result<ProcessWorkflowRunResult>,
        }
        let sem = Arc::new(Semaphore::new(10));
        let mut set = JoinSet::new();
        for run in runs {
            let sem = sem.clone();
            let project = project.clone();
            let client = client.clone();
            let db = state.db.clone();
            let run_id = run.id;
            let commit = Commit::from(&run.head_commit);
            set.spawn(async move {
                let _permit = sem.acquire().await.unwrap();
                match db.report_exists(project.id, &commit.sha).await {
                    Ok(true) => {
                        return TaskResult {
                            run_id,
                            commit,
                            result: Ok(ProcessWorkflowRunResult { artifacts: vec![] }),
                        };
                    }
                    Ok(false) => {}
                    Err(e) => return TaskResult { run_id, commit, result: Err(e) },
                }
                let result = process_workflow_run(&client, &project, run.id).await;
                TaskResult { run_id, commit, result }
            });
        }
        let mut found_artifacts = false;
        while let Some(join_result) = set.join_next().await {
            match join_result {
                Ok(TaskResult {
                    run_id,
                    commit,
                    result: Ok(ProcessWorkflowRunResult { artifacts }),
                }) => {
                    tracing::debug!(
                        "Processed workflow run {} ({}) (artifacts {})",
                        run_id,
                        commit.sha,
                        artifacts.len()
                    );
                    for artifact in artifacts {
                        let start = std::time::Instant::now();
                        state
                            .db
                            .insert_report(&project, &commit, &artifact.version, *artifact.report)
                            .await?;
                        let duration = start.elapsed();
                        tracing::info!(
                            "Inserted report {} ({}) in {}ms",
                            artifact.version,
                            commit.sha,
                            duration.as_millis()
                        );
                        found_artifacts = true;
                    }
                }
                Ok(TaskResult { run_id, commit, result: Err(e) }) => {
                    tracing::error!(
                        "Failed to process workflow run {} ({}): {:?}",
                        run_id,
                        commit.sha,
                        e
                    );
                }
                Err(e) => {
                    tracing::error!("Failed to process workflow run: {:?}", e);
                }
            }
        }

        if found_artifacts {
            if project.workflow_id.is_none() {
                state.db.update_project_workflow_id(project.id, workflow_id).await?;
            }
            break;
        }
    }

    Ok(())
}

pub struct ProcessWorkflowRunResult {
    pub artifacts: Vec<ProcessArtifactResult>,
}

pub struct ProcessArtifactResult {
    pub version: String,
    pub report: Box<Report>,
}

pub async fn process_workflow_run(
    client: &Octocrab,
    project: &Project,
    run_id: RunId,
) -> Result<ProcessWorkflowRunResult> {
    let artifacts = client
        .all_pages(
            client
                .actions()
                .list_workflow_run_artifacts(&project.owner, &project.repo, run_id)
                .send()
                .await
                .context("Failed to fetch artifacts")?
                .value
                .unwrap_or_default(),
        )
        .await?;
    tracing::debug!("Run {} (artifacts {})", run_id, artifacts.len());
    let mut result = ProcessWorkflowRunResult { artifacts: vec![] };
    if artifacts.is_empty() {
        return Ok(result);
    }
    static REGEX: OnceLock<Regex> = OnceLock::new();
    let regex = REGEX
        .get_or_init(|| Regex::new(r"^(?P<version>[A-z0-9_.\-]+)[_-]report(?:[_-].*)?$").unwrap());
    static MAPS_REGEX: OnceLock<Regex> = OnceLock::new();
    let maps_regex =
        MAPS_REGEX.get_or_init(|| Regex::new(r"^(?P<version>[A-z0-9_\-]+)_maps$").unwrap());
    let sem = Arc::new(Semaphore::new(3));
    let mut set = JoinSet::new();
    struct TaskResult {
        artifact_name: String,
        result: DownloadArtifactResult,
    }
    for artifact in &artifacts {
        let artifact_name = artifact.name.clone();
        let version =
            if let Some(version) = regex.captures(&artifact_name).and_then(|c| c.name("version")) {
                version.as_str().to_string()
            } else if artifact_name == "progress" || artifact_name == "progress.json" {
                // bfbb compatibility
                if let Some(version) = artifacts.iter().find_map(|a| {
                    maps_regex
                        .captures(&a.name)
                        .and_then(|c| c.name("version"))
                        .map(|m| m.as_str().to_string())
                }) {
                    version
                } else {
                    continue;
                }
            } else {
                continue;
            };
        let sem = sem.clone();
        let client = client.clone();
        let project = project.clone();
        let artifact_id = artifact.id;
        set.spawn(async move {
            let _permit = sem.acquire().await.unwrap();
            TaskResult {
                artifact_name,
                result: download_artifact(client, project, artifact_id, version).await,
            }
        });
    }
    while let Some(join_result) = set.join_next().await {
        match join_result {
            Ok(TaskResult { artifact_name: name, result: Ok(reports) }) => {
                if reports.is_empty() {
                    tracing::warn!("No report found in artifact {}", name);
                } else {
                    for (version, report) in reports {
                        tracing::info!("Processed artifact {} ({})", name, version);
                        result.artifacts.push(ProcessArtifactResult { version, report });
                    }
                }
            }
            Ok(TaskResult { artifact_name: name, result: Err(e) }) => {
                tracing::error!("Failed to process artifact {}: {:?}", name, e);
            }
            Err(e) => {
                tracing::error!("Failed to process artifact: {:?}", e);
            }
        }
    }
    Ok(result)
}

type DownloadArtifactResult = Result<Vec<(String, Box<Report>)>>;

async fn download_artifact(
    client: Octocrab,
    project: Project,
    artifact_id: ArtifactId,
    version: String,
) -> DownloadArtifactResult {
    let bytes = client
        .actions()
        .download_artifact(&project.owner, &project.repo, artifact_id, ArchiveFormat::Zip)
        .await?;
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let Some(path) = file.enclosed_name() else {
            continue;
        };
        if path.file_stem() == Some(OsStr::new("report"))
            || path.file_stem() == Some(OsStr::new("progress"))
        {
            let mut contents = Vec::with_capacity(file.size() as usize);
            file.read_to_end(&mut contents)?;
            let mut report = Box::new(Report::parse(&contents)?);
            report.migrate()?;
            // Split combined reports into individual reports
            if version.eq_ignore_ascii_case("combined") {
                return Ok(report
                    .split()
                    .into_iter()
                    .map(|(version, report)| (version, Box::new(report)))
                    .collect());
            }
            return Ok(vec![(version, report)]);
        }
    }
    Ok(vec![])
}
