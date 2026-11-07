//! Minimal GitHub REST client (issues, pull requests, labels, repositories), blocking: called through `spawn_blocking` from the DB worker.

use crate::board::RemoteIssue;
use serde_json::{Value, json};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct GhConfig {
    pub token: String,
    /// API root, `https://api.github.com` or a GitHub Enterprise `https://host/api/v3`
    pub api: String,
}

/// Whether `host` is excluded from proxying by a NO_PROXY-style list ("a.com,.b.org,10.0.0.1,*").
fn no_proxy_matches(host: &str, list: &str) -> bool {
    let host = host.to_lowercase();
    list.split(',').map(|e| e.trim().to_lowercase()).filter(|e| !e.is_empty()).any(|e| {
        if e == "*" {
            return true;
        }
        let e = e.trim_start_matches("*.").trim_start_matches('.');
        host == e || host.ends_with(&format!(".{e}"))
    })
}

/// HTTP agent honouring HTTPS_PROXY / HTTP_PROXY / ALL_PROXY *and* NO_PROXY (ureq 2 ignores the latter).
fn agent(url: &str) -> ureq::Agent {
    let mut b = ureq::AgentBuilder::new().timeout(Duration::from_secs(25));
    let host = url.split("://").nth(1).unwrap_or(url).split(['/', ':']).next().unwrap_or("");
    let env =
        |k: &str| std::env::var(k).or_else(|_| std::env::var(k.to_lowercase())).ok().filter(|v| !v.trim().is_empty());
    let excluded = env("NO_PROXY").is_some_and(|l| no_proxy_matches(host, &l));
    let proxy =
        if url.starts_with("https") { env("HTTPS_PROXY") } else { env("HTTP_PROXY") }.or_else(|| env("ALL_PROXY"));
    if !excluded
        && let Some(p) = proxy
        && let Ok(p) = ureq::Proxy::new(p)
    {
        b = b.proxy(p);
    }
    b.build()
}

fn request(cfg: &GhConfig, method: &str, path: &str) -> ureq::Request {
    let url = format!("{}{path}", cfg.api.trim_end_matches('/'));
    agent(&url)
        .request(method, &url)
        .set("Authorization", &format!("Bearer {}", cfg.token.trim()))
        .set("Accept", "application/vnd.github+json")
        .set("X-GitHub-Api-Version", "2022-11-28")
        .set("User-Agent", "Mnemo")
}

fn error(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, resp) => {
            let body: Value = resp.into_json().unwrap_or(Value::Null);
            let msg = body.get("message").and_then(|m| m.as_str()).unwrap_or("").to_string();
            match code {
                401 => "Token GitHub invalide ou expiré".into(),
                403 if msg.to_lowercase().contains("rate limit") => {
                    "Limite de requêtes GitHub atteinte, réessayez plus tard".into()
                }
                403 => format!("Accès refusé par GitHub (droits du token ?) : {msg}"),
                404 => "Dépôt introuvable, ou le token n'y a pas accès".into(),
                410 => "Les issues sont désactivées sur ce dépôt".into(),
                422 => format!("GitHub a refusé la requête : {msg}"),
                c => format!("Erreur GitHub {c} : {msg}"),
            }
        }
        ureq::Error::Transport(t) => format!("GitHub injoignable : {t}"),
    }
}

fn valid_repo(repo: &str) -> Result<&str, String> {
    let r = repo.trim().trim_start_matches("https://github.com/").trim_end_matches(".git").trim_matches('/');
    let parts: Vec<&str> = r.split('/').collect();
    if parts.len() == 2
        && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_alphanumeric() || "-_.".contains(c)))
    {
        Ok(r)
    } else {
        Err(format!("Dépôt invalide « {repo} » (attendu : propriétaire/nom)"))
    }
}

/// Normalises "https://github.com/owner/name(.git)" or "owner/name" to "owner/name".
pub fn normalize_repo(repo: &str) -> Result<String, String> {
    valid_repo(repo).map(str::to_string)
}

/// Issue or pull request (the issues API returns both; a PR carries a `pull_request` object).
fn parse_issue(v: &Value) -> Option<RemoteIssue> {
    let pr = v.get("pull_request");
    Some(RemoteIssue {
        pull: pr.is_some(),
        draft: v.get("draft").and_then(|d| d.as_bool()).unwrap_or(false),
        merged: pr.and_then(|p| p.get("merged_at")).is_some_and(|m| !m.is_null()),
        number: v.get("number")?.as_i64()? as i32,
        title: v.get("title")?.as_str()?.to_string(),
        body: v.get("body").and_then(|b| b.as_str()).unwrap_or("").to_string(),
        open: v.get("state").and_then(|s| s.as_str()) != Some("closed"),
        labels: v
            .get("labels")
            .and_then(|l| l.as_array())
            .map(|a| a.iter().filter_map(|l| l.get("name").and_then(|n| n.as_str()).map(str::to_string)).collect())
            .unwrap_or_default(),
        url: v.get("html_url").and_then(|u| u.as_str()).unwrap_or("").to_string(),
        updated_at: v
            .get("updated_at")
            .and_then(|u| u.as_str())
            .and_then(|u| chrono::DateTime::parse_from_rfc3339(u).ok())
            .map(|d| d.with_timezone(&chrono::Utc))
            .unwrap_or_else(chrono::Utc::now),
    })
}

/// Login of the token's owner (connection test).
pub fn whoami(cfg: &GhConfig) -> Result<String, String> {
    let v: Value = request(cfg, "GET", "/user").call().map_err(error)?.into_json().map_err(|e| e.to_string())?;
    Ok(v.get("login").and_then(|l| l.as_str()).unwrap_or("?").to_string())
}

/// All issues and pull requests (open and closed), up to 1000.
pub fn list_issues(cfg: &GhConfig, repo: &str) -> Result<Vec<RemoteIssue>, String> {
    let repo = valid_repo(repo)?;
    let mut out = vec![];
    for page in 1..=10 {
        let v: Value = request(cfg, "GET", &format!("/repos/{repo}/issues"))
            .query("state", "all")
            .query("per_page", "100")
            .query("page", &page.to_string())
            .call()
            .map_err(error)?
            .into_json()
            .map_err(|e| e.to_string())?;
        let arr = v.as_array().cloned().unwrap_or_default();
        out.extend(arr.iter().filter_map(parse_issue));
        if arr.len() < 100 {
            break;
        }
    }
    Ok(out)
}

/// Labels of a repository: (name, colour as "rrggbb").
pub fn list_labels(cfg: &GhConfig, repo: &str) -> Result<Vec<(String, String)>, String> {
    let repo = valid_repo(repo)?;
    let mut out = vec![];
    for page in 1..=5 {
        let v: Value = request(cfg, "GET", &format!("/repos/{repo}/labels"))
            .query("per_page", "100")
            .query("page", &page.to_string())
            .call()
            .map_err(error)?
            .into_json()
            .map_err(|e| e.to_string())?;
        let arr = v.as_array().cloned().unwrap_or_default();
        out.extend(arr.iter().filter_map(|l| {
            Some((
                l.get("name")?.as_str()?.to_string(),
                l.get("color").and_then(|c| c.as_str()).unwrap_or("").to_string(),
            ))
        }));
        if arr.len() < 100 {
            break;
        }
    }
    Ok(out)
}

/// Repositories the token can see ("owner/name"), most recently updated first.
pub fn list_repos(cfg: &GhConfig) -> Result<Vec<String>, String> {
    let mut out = vec![];
    for page in 1..=5 {
        let v: Value = request(cfg, "GET", "/user/repos")
            .query("per_page", "100")
            .query("page", &page.to_string())
            .query("sort", "updated")
            .query("affiliation", "owner,collaborator,organization_member")
            .call()
            .map_err(error)?
            .into_json()
            .map_err(|e| e.to_string())?;
        let arr = v.as_array().cloned().unwrap_or_default();
        out.extend(arr.iter().filter_map(|r| r.get("full_name")?.as_str().map(str::to_string)));
        if arr.len() < 100 {
            break;
        }
    }
    Ok(out)
}

pub fn create_issue(
    cfg: &GhConfig,
    repo: &str,
    title: &str,
    body: &str,
    labels: &[String],
) -> Result<RemoteIssue, String> {
    let repo = valid_repo(repo)?;
    let v: Value = request(cfg, "POST", &format!("/repos/{repo}/issues"))
        .send_json(json!({ "title": title, "body": body, "labels": labels }))
        .map_err(error)?
        .into_json()
        .map_err(|e| e.to_string())?;
    parse_issue(&v).ok_or_else(|| "Réponse GitHub inattendue".into())
}

pub fn update_issue(
    cfg: &GhConfig,
    repo: &str,
    number: i32,
    title: &str,
    body: &str,
    open: Option<bool>,
    labels: &[String],
) -> Result<RemoteIssue, String> {
    let repo = valid_repo(repo)?;
    let mut patch = json!({ "title": title, "body": body, "labels": labels });
    // None : on ne touche pas à l'état (pull requests : ne jamais fermer une PR depuis Mnemo)
    if let Some(open) = open {
        patch["state"] = json!(if open { "open" } else { "closed" });
    }
    let v: Value = request(cfg, "PATCH", &format!("/repos/{repo}/issues/{number}"))
        .send_json(patch)
        .map_err(error)?
        .into_json()
        .map_err(|e| e.to_string())?;
    parse_issue(&v).ok_or_else(|| "Réponse GitHub inattendue".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repos() {
        assert_eq!(normalize_repo("owner/name").unwrap(), "owner/name");
        assert_eq!(normalize_repo("https://github.com/owner/my-repo.git").unwrap(), "owner/my-repo");
        assert!(normalize_repo("owner").is_err());
        assert!(normalize_repo("a/b/c").is_err());
        assert!(normalize_repo("a/b c").is_err());
    }

    #[test]
    fn no_proxy() {
        assert!(no_proxy_matches("127.0.0.1", "localhost,127.0.0.1"));
        assert!(no_proxy_matches("git.corp.example", ".corp.example"));
        assert!(no_proxy_matches("api.github.com", "*"));
        assert!(!no_proxy_matches("api.github.com", "github.io,localhost"));
        assert!(!no_proxy_matches("evilgithub.com", "github.com"));
    }

    #[test]
    fn parse() {
        let v = json!({"number": 3, "title": "T", "body": null, "state": "closed", "labels": [{"name": "bug"}],
                       "html_url": "u", "updated_at": "2024-01-02T03:04:05Z"});
        let i = parse_issue(&v).unwrap();
        assert_eq!((i.number, i.open, i.labels.len(), i.body.as_str(), i.pull), (3, false, 1, "", false));
        let pr = parse_issue(&json!({"number": 1, "title": "PR", "state": "closed", "pull_request": {"merged_at": "2024-01-02T03:04:05Z"}})).unwrap();
        assert!(pr.pull && pr.merged && !pr.draft);
        let draft =
            parse_issue(&json!({"number": 2, "title": "D", "draft": true, "pull_request": {"merged_at": null}}))
                .unwrap();
        assert!(draft.pull && draft.draft && !draft.merged);
    }
}
