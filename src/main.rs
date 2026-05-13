use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use serde::Deserialize;
use std::process::Command;

#[derive(Parser)]
#[command(name = "ghelpr", about = "GitHub PR helper")]
struct Cli {
    /// Repository owner (inferred from git remote if omitted)
    #[arg(short, long, global = true)]
    owner: Option<String>,

    /// Repository name (inferred from git remote if omitted)
    #[arg(short, long, global = true)]
    repo: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Show review comments for a pull request
    Comments {
        /// Pull request number
        pr: u64,

        /// Include all comments (resolved and unresolved)
        #[arg(short, long, default_value_t = false)]
        all: bool,

        /// Include full details (diff hunks, review info, line positions, etc.)
        #[arg(short, long, default_value_t = false)]
        full: bool,
    },

    /// Show Copilot premium request quota and usage
    Quota {
        /// Output raw JSON response
        #[arg(long, default_value_t = false)]
        json: bool,
    },
}

// ---------------------------------------------------------------------------
// Auth
// ---------------------------------------------------------------------------

fn get_token() -> Result<String> {
    // 1. GH_TOKEN env var
    if let Ok(token) = std::env::var("GH_TOKEN") {
        if !token.is_empty() {
            return Ok(token);
        }
    }
    // 2. GITHUB_TOKEN env var
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        if !token.is_empty() {
            return Ok(token);
        }
    }
    // 3. gh auth token
    let output = Command::new("gh")
        .args(["auth", "token"])
        .output()
        .context("Failed to run `gh auth token`. Is the GitHub CLI installed?")?;
    if output.status.success() {
        let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !token.is_empty() {
            return Ok(token);
        }
    }
    bail!(
        "No GitHub token found. Set GH_TOKEN or GITHUB_TOKEN, \
         or log in with `gh auth login`."
    )
}

fn get_session_cookie() -> Result<String> {
    if let Ok(cookie) = std::env::var("GITHUB_SESSION_COOKIE") {
        if !cookie.is_empty() {
            return Ok(cookie);
        }
    }
    bail!(
        "No GitHub session cookie found.\n\n\
         Set the GITHUB_SESSION_COOKIE environment variable to your github.com `user_session` cookie value.\n\n\
         To get it:\n  \
         1. Open https://github.com/settings/copilot/features in your browser\n  \
         2. Open DevTools (F12) > Application > Cookies > github.com\n  \
         3. Copy the value of the `user_session` cookie\n  \
         4. Set it:  export GITHUB_SESSION_COOKIE=\"<value>\"\n\n\
         The cookie typically lasts ~2 weeks and is refreshed as you use GitHub."
    )
}

// ---------------------------------------------------------------------------
// Git remote parsing
// ---------------------------------------------------------------------------

/// Parse owner/repo from the `origin` remote URL.
///
/// Handles:
///   git@github.com:owner/repo.git
///   https://github.com/owner/repo.git
///   https://github.com/owner/repo
fn parse_owner_repo_from_remote() -> Result<(String, String)> {
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()
        .context("Failed to run `git remote get-url origin`. Are you in a git repo?")?;
    if !output.status.success() {
        bail!("Could not get the origin remote URL. Use --owner and --repo.");
    }
    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    parse_owner_repo(&url)
}

fn parse_owner_repo(url: &str) -> Result<(String, String)> {
    // SSH: git@github.com:owner/repo.git
    if let Some(rest) = url.strip_prefix("git@github.com:") {
        let rest = rest.strip_suffix(".git").unwrap_or(rest);
        return split_owner_repo(rest);
    }
    // HTTPS: https://github.com/owner/repo(.git)
    let prefixes = ["https://github.com/", "http://github.com/"];
    for prefix in prefixes {
        if let Some(rest) = url.strip_prefix(prefix) {
            let rest = rest.strip_suffix(".git").unwrap_or(rest);
            return split_owner_repo(rest);
        }
    }
    bail!("Cannot parse owner/repo from remote URL: {url}")
}

fn split_owner_repo(s: &str) -> Result<(String, String)> {
    let parts: Vec<&str> = s.splitn(3, '/').collect();
    if parts.len() < 2 || parts[0].is_empty() || parts[1].is_empty() {
        bail!("Cannot parse owner/repo from: {s}");
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}

// ---------------------------------------------------------------------------
// GraphQL queries
// ---------------------------------------------------------------------------

const REVIEW_THREADS_QUERY_SUMMARY: &str = r#"
query($owner: String!, $repo: String!, $pr: Int!, $cursor: String) {
  repository(owner: $owner, name: $repo) {
    pullRequest(number: $pr) {
      reviewThreads(first: 100, after: $cursor) {
        nodes {
          id
          isResolved
          isOutdated
          path
          line
          startLine
          comments(first: 100) {
            nodes {
              body
              createdAt
              author { login }
            }
            pageInfo {
              hasNextPage
              endCursor
            }
          }
        }
        pageInfo {
          hasNextPage
          endCursor
        }
      }
    }
  }
}
"#;

const REVIEW_THREADS_QUERY_FULL: &str = r#"
query($owner: String!, $repo: String!, $pr: Int!, $cursor: String) {
  repository(owner: $owner, name: $repo) {
    pullRequest(number: $pr) {
      reviewThreads(first: 100, after: $cursor) {
        nodes {
          id
          isResolved
          isOutdated
          path
          line
          startLine
          originalLine
          originalStartLine
          diffSide
          startDiffSide
          subjectType
          resolvedBy { login }
          comments(first: 100) {
            nodes {
              databaseId
              body
              createdAt
              updatedAt
              url
              state
              outdated
              diffHunk
              path
              line
              startLine
              originalLine
              originalStartLine
              author { login }
              replyTo { databaseId }
              pullRequestReview {
                state
                body
                author { login }
              }
            }
            pageInfo {
              hasNextPage
              endCursor
            }
          }
        }
        pageInfo {
          hasNextPage
          endCursor
        }
      }
    }
  }
}
"#;

// ---------------------------------------------------------------------------
// GraphQL response types (full, superset - optional fields handle both modes)
// ---------------------------------------------------------------------------

#[derive(Deserialize, Debug)]
struct GqlResponse {
    data: Option<GqlData>,
    errors: Option<Vec<GqlError>>,
}

#[derive(Deserialize, Debug)]
struct GqlError {
    message: String,
}

#[derive(Deserialize, Debug)]
struct GqlData {
    repository: GqlRepository,
}

#[derive(Deserialize, Debug)]
struct GqlRepository {
    #[serde(rename = "pullRequest")]
    pull_request: GqlPullRequest,
}

#[derive(Deserialize, Debug)]
struct GqlPullRequest {
    #[serde(rename = "reviewThreads")]
    review_threads: GqlConnection<GqlReviewThread>,
}

#[derive(Deserialize, Debug)]
struct GqlConnection<T> {
    nodes: Vec<T>,
    #[serde(rename = "pageInfo")]
    page_info: GqlPageInfo,
}

#[derive(Deserialize, Debug)]
struct GqlPageInfo {
    #[serde(rename = "hasNextPage")]
    has_next_page: bool,
    #[serde(rename = "endCursor")]
    end_cursor: Option<String>,
}

#[derive(Deserialize, Debug)]
struct GqlReviewThread {
    id: String,
    #[serde(rename = "isResolved")]
    is_resolved: bool,
    #[serde(rename = "isOutdated")]
    is_outdated: bool,
    path: String,
    line: Option<u64>,
    #[serde(rename = "startLine")]
    start_line: Option<u64>,
    #[serde(rename = "originalLine")]
    original_line: Option<u64>,
    #[serde(rename = "originalStartLine")]
    original_start_line: Option<u64>,
    #[serde(rename = "diffSide")]
    diff_side: Option<String>,
    #[serde(rename = "startDiffSide")]
    start_diff_side: Option<String>,
    #[serde(rename = "subjectType")]
    subject_type: Option<String>,
    #[serde(rename = "resolvedBy")]
    resolved_by: Option<GqlAuthor>,
    comments: GqlConnection<GqlComment>,
}

#[derive(Deserialize, Debug)]
struct GqlComment {
    #[serde(rename = "databaseId")]
    database_id: Option<u64>,
    body: String,
    #[serde(rename = "createdAt")]
    created_at: String,
    #[serde(rename = "updatedAt")]
    updated_at: Option<String>,
    url: Option<String>,
    state: Option<String>,
    outdated: Option<bool>,
    #[serde(rename = "diffHunk")]
    diff_hunk: Option<String>,
    path: Option<String>,
    line: Option<u64>,
    #[serde(rename = "startLine")]
    start_line: Option<u64>,
    #[serde(rename = "originalLine")]
    original_line: Option<u64>,
    #[serde(rename = "originalStartLine")]
    original_start_line: Option<u64>,
    author: Option<GqlAuthor>,
    #[serde(rename = "replyTo")]
    reply_to: Option<GqlReplyRef>,
    #[serde(rename = "pullRequestReview")]
    pull_request_review: Option<GqlReview>,
}

#[derive(Deserialize, Debug, serde::Serialize)]
struct GqlAuthor {
    login: String,
}

#[derive(Deserialize, Debug)]
struct GqlReplyRef {
    #[serde(rename = "databaseId")]
    database_id: Option<u64>,
}

#[derive(Deserialize, Debug)]
struct GqlReview {
    state: Option<String>,
    body: Option<String>,
    author: Option<GqlAuthor>,
}

// ---------------------------------------------------------------------------
// Output types (separate for summary vs full)
// ---------------------------------------------------------------------------

#[derive(serde::Serialize)]
struct SummaryThread {
    path: String,
    line: Option<u64>,
    start_line: Option<u64>,
    is_resolved: bool,
    is_outdated: bool,
    comments: Vec<SummaryComment>,
}

#[derive(serde::Serialize)]
struct SummaryComment {
    author: String,
    created_at: String,
    body: String,
}

#[derive(serde::Serialize)]
struct FullThread {
    id: String,
    path: String,
    line: Option<u64>,
    start_line: Option<u64>,
    original_line: Option<u64>,
    original_start_line: Option<u64>,
    is_resolved: bool,
    is_outdated: bool,
    diff_side: Option<String>,
    start_diff_side: Option<String>,
    subject_type: Option<String>,
    resolved_by: Option<String>,
    comments: Vec<FullComment>,
}

#[derive(serde::Serialize)]
struct FullComment {
    database_id: Option<u64>,
    author: String,
    created_at: String,
    updated_at: Option<String>,
    url: Option<String>,
    state: Option<String>,
    outdated: Option<bool>,
    diff_hunk: Option<String>,
    path: Option<String>,
    line: Option<u64>,
    start_line: Option<u64>,
    original_line: Option<u64>,
    original_start_line: Option<u64>,
    reply_to_id: Option<u64>,
    review_state: Option<String>,
    review_body: Option<String>,
    review_author: Option<String>,
    body: String,
}

// ---------------------------------------------------------------------------
// Conversion
// ---------------------------------------------------------------------------

fn to_summary(thread: &GqlReviewThread) -> SummaryThread {
    SummaryThread {
        path: thread.path.clone(),
        line: thread.line,
        start_line: thread.start_line,
        is_resolved: thread.is_resolved,
        is_outdated: thread.is_outdated,
        comments: thread
            .comments
            .nodes
            .iter()
            .map(|c| SummaryComment {
                author: c
                    .author
                    .as_ref()
                    .map(|a| a.login.clone())
                    .unwrap_or_else(|| "ghost".to_string()),
                created_at: c.created_at.clone(),
                body: c.body.clone(),
            })
            .collect(),
    }
}

fn to_full(thread: &GqlReviewThread) -> FullThread {
    FullThread {
        id: thread.id.clone(),
        path: thread.path.clone(),
        line: thread.line,
        start_line: thread.start_line,
        original_line: thread.original_line,
        original_start_line: thread.original_start_line,
        is_resolved: thread.is_resolved,
        is_outdated: thread.is_outdated,
        diff_side: thread.diff_side.clone(),
        start_diff_side: thread.start_diff_side.clone(),
        subject_type: thread.subject_type.clone(),
        resolved_by: thread
            .resolved_by
            .as_ref()
            .map(|a| a.login.clone()),
        comments: thread
            .comments
            .nodes
            .iter()
            .map(|c| FullComment {
                database_id: c.database_id,
                author: c
                    .author
                    .as_ref()
                    .map(|a| a.login.clone())
                    .unwrap_or_else(|| "ghost".to_string()),
                created_at: c.created_at.clone(),
                updated_at: c.updated_at.clone(),
                url: c.url.clone(),
                state: c.state.clone(),
                outdated: c.outdated,
                diff_hunk: c.diff_hunk.clone(),
                path: c.path.clone(),
                line: c.line,
                start_line: c.start_line,
                original_line: c.original_line,
                original_start_line: c.original_start_line,
                reply_to_id: c.reply_to.as_ref().and_then(|r| r.database_id),
                review_state: c
                    .pull_request_review
                    .as_ref()
                    .and_then(|r| r.state.clone()),
                review_body: c
                    .pull_request_review
                    .as_ref()
                    .and_then(|r| r.body.clone()),
                review_author: c
                    .pull_request_review
                    .as_ref()
                    .and_then(|r| r.author.as_ref().map(|a| a.login.clone())),
                body: c.body.clone(),
            })
            .collect(),
    }
}

// ---------------------------------------------------------------------------
// API call
// ---------------------------------------------------------------------------

async fn fetch_review_threads(
    client: &reqwest::Client,
    token: &str,
    owner: &str,
    repo: &str,
    pr: u64,
    full: bool,
) -> Result<Vec<GqlReviewThread>> {
    let query = if full {
        REVIEW_THREADS_QUERY_FULL
    } else {
        REVIEW_THREADS_QUERY_SUMMARY
    };

    let mut all_threads = Vec::new();
    let mut cursor: Option<String> = None;

    loop {
        let variables = serde_json::json!({
            "owner": owner,
            "repo": repo,
            "pr": pr as i64,
            "cursor": cursor,
        });

        let body = serde_json::json!({
            "query": query,
            "variables": variables,
        });

        let resp = client
            .post("https://api.github.com/graphql")
            .bearer_auth(token)
            .header("User-Agent", "ghelpr")
            .json(&body)
            .send()
            .await
            .context("GraphQL request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            bail!("GitHub API returned {status}: {text}");
        }

        let gql: GqlResponse =
            resp.json().await.context("Failed to parse GraphQL response")?;

        if let Some(errors) = gql.errors {
            let msgs: Vec<_> = errors.iter().map(|e| e.message.as_str()).collect();
            bail!("GraphQL errors: {}", msgs.join("; "));
        }

        let data = gql.data.context("No data in GraphQL response")?;
        let threads = data.repository.pull_request.review_threads;

        all_threads.extend(threads.nodes);

        if threads.page_info.has_next_page {
            cursor = threads.page_info.end_cursor;
        } else {
            break;
        }
    }

    Ok(all_threads)
}

// ---------------------------------------------------------------------------
// Copilot entitlement types
// ---------------------------------------------------------------------------

#[derive(Deserialize, Debug, serde::Serialize)]
struct CopilotEntitlement {
    #[serde(rename = "licenseType")]
    license_type: Option<String>,
    quotas: Option<CopilotQuotas>,
    plan: Option<String>,
    trial: Option<CopilotTrial>,
}

#[derive(Deserialize, Debug, serde::Serialize)]
struct CopilotQuotas {
    limits: Option<CopilotLimits>,
    remaining: Option<CopilotRemaining>,
    #[serde(rename = "resetDate")]
    reset_date: Option<String>,
    #[serde(rename = "overagesEnabled")]
    overages_enabled: Option<bool>,
}

#[derive(Deserialize, Debug, serde::Serialize)]
struct CopilotLimits {
    #[serde(rename = "premiumInteractions")]
    premium_interactions: Option<u64>,
}

#[derive(Deserialize, Debug, serde::Serialize)]
struct CopilotRemaining {
    #[serde(rename = "premiumInteractions")]
    premium_interactions: Option<u64>,
    #[serde(rename = "chatPercentage")]
    chat_percentage: Option<f64>,
    #[serde(rename = "premiumInteractionsPercentage")]
    premium_interactions_percentage: Option<f64>,
}

#[derive(Deserialize, Debug, serde::Serialize)]
struct CopilotTrial {
    eligible: Option<bool>,
}

// ---------------------------------------------------------------------------
// Copilot entitlement API call
// ---------------------------------------------------------------------------

async fn fetch_copilot_quota(
    client: &reqwest::Client,
    session_cookie: &str,
) -> Result<CopilotEntitlement> {
    let resp = client
        .get("https://github.com/github-copilot/chat/entitlement")
        .header("Accept", "application/json")
        .header("X-Requested-With", "XMLHttpRequest")
        .header("github-verified-fetch", "true")
        .header("User-Agent", "Mozilla/5.0 (compatible; ghelpr)")
        .header("Cookie", format!("user_session={session_cookie}"))
        .send()
        .await
        .context("Failed to fetch Copilot entitlement")?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        if status.as_u16() == 404 {
            bail!(
                "GitHub returned 404. Your session cookie is likely expired or invalid.\n\
                 Refresh it from your browser and update GITHUB_SESSION_COOKIE."
            );
        }
        bail!("GitHub returned {status}: {text}");
    }

    resp.json()
        .await
        .context("Failed to parse Copilot entitlement response")
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Comments {
            pr,
            all,
            full,
        } => {
            let (resolved_owner, resolved_repo) = match (cli.owner, cli.repo) {
                (Some(o), Some(r)) => (o, r),
                (None, None) => parse_owner_repo_from_remote()?,
                _ => bail!("Specify both --owner and --repo, or neither (to infer from git)."),
            };

            let token = get_token()?;
            let client = reqwest::Client::new();

            let all_threads =
                fetch_review_threads(&client, &token, &resolved_owner, &resolved_repo, pr, full)
                    .await?;

            let threads: Vec<_> = if all {
                all_threads
            } else {
                all_threads
                    .into_iter()
                    .filter(|t| !t.is_resolved)
                    .collect()
            };

            if full {
                let output: Vec<_> = threads.iter().map(to_full).collect();
                let json =
                    serde_json::to_string_pretty(&output).expect("Failed to serialize to JSON");
                println!("{json}");
            } else {
                let output: Vec<_> = threads.iter().map(to_summary).collect();
                let json =
                    serde_json::to_string_pretty(&output).expect("Failed to serialize to JSON");
                println!("{json}");
            }
        }

        Commands::Quota { json } => {
            let session_cookie = get_session_cookie()?;
            let client = reqwest::Client::new();
            let entitlement = fetch_copilot_quota(&client, &session_cookie).await?;

            if json {
                let output = serde_json::to_string_pretty(&entitlement)
                    .expect("Failed to serialize to JSON");
                println!("{output}");
            } else {
                let plan = entitlement.plan.as_deref().unwrap_or("unknown");

                if let Some(quotas) = &entitlement.quotas {
                    let limit = quotas
                        .limits
                        .as_ref()
                        .and_then(|l| l.premium_interactions);
                    let remaining = quotas
                        .remaining
                        .as_ref()
                        .and_then(|r| r.premium_interactions);
                    let pct = quotas
                        .remaining
                        .as_ref()
                        .and_then(|r| r.premium_interactions_percentage);
                    let reset = quotas.reset_date.as_deref().unwrap_or("unknown");
                    let overages = quotas.overages_enabled.unwrap_or(false);

                    println!("Copilot Premium Requests");
                    println!("  Plan:      {plan}");
                    if let (Some(rem), Some(lim)) = (remaining, limit) {
                        let used = lim.saturating_sub(rem);
                        println!("  Used:      {used} / {lim}");
                    }
                    if let Some(rem) = remaining {
                        if let Some(p) = pct {
                            println!("  Remaining: {rem} ({p:.1}%)");
                        } else {
                            println!("  Remaining: {rem}");
                        }
                    }
                    println!("  Resets:    {reset}");
                    println!(
                        "  Overages:  {}",
                        if overages { "enabled" } else { "disabled" }
                    );
                } else {
                    println!("Plan: {plan}");
                    println!("No quota information available.");
                }
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ssh_url() {
        let (owner, repo) = parse_owner_repo("git@github.com:lkurcak/ghelpr.git").unwrap();
        assert_eq!(owner, "lkurcak");
        assert_eq!(repo, "ghelpr");
    }

    #[test]
    fn test_parse_ssh_url_no_suffix() {
        let (owner, repo) = parse_owner_repo("git@github.com:lkurcak/ghelpr").unwrap();
        assert_eq!(owner, "lkurcak");
        assert_eq!(repo, "ghelpr");
    }

    #[test]
    fn test_parse_https_url() {
        let (owner, repo) =
            parse_owner_repo("https://github.com/lkurcak/ghelpr.git").unwrap();
        assert_eq!(owner, "lkurcak");
        assert_eq!(repo, "ghelpr");
    }

    #[test]
    fn test_parse_https_url_no_suffix() {
        let (owner, repo) = parse_owner_repo("https://github.com/lkurcak/ghelpr").unwrap();
        assert_eq!(owner, "lkurcak");
        assert_eq!(repo, "ghelpr");
    }
}
