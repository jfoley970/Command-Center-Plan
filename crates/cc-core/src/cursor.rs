//! Cursor background agents. A run starts an agent in Cursor's cloud against a
//! GitHub repository; it keeps working after the request returns, so a loop
//! checks on running agents and records the summary and pull request when
//! they finish. The run continues even if this app closes.
//!
//! This uses Cursor's v0 agents API, which Cursor now marks legacy but still
//! serves; v1 renames the fields (repos[], autoCreatePR) and splits runs out.

use serde_json::{json, Value};
use std::sync::Arc;

use crate::secrets::CURSOR_KEY;
use crate::{db, Core};

const API: &str = "https://api.cursor.com/v0/agents";
const POLL_SECS: u64 = 30;

pub struct Launched {
    pub id: String,
    pub link: String,
}

/// Where a finished or failed agent ended up.
#[derive(Debug, PartialEq)]
pub enum Progress {
    Running,
    Done { summary: String, link: String },
    Failed(String),
}

fn launch_body(repo: &str, prompt: &str) -> Value {
    json!({
        "prompt": { "text": prompt },
        "source": { "repository": repo },
        "target": { "autoCreatePr": true },
    })
}

/// A repository can be given as "owner/repo" or a full GitHub URL.
pub fn normalize_repo(repo: &str) -> Result<String, String> {
    let r = repo.trim().trim_end_matches('/').trim_end_matches(".git");
    if r.starts_with("https://") {
        return Ok(r.to_string());
    }
    let parts: Vec<&str> = r.split('/').collect();
    if parts.len() == 2 && parts.iter().all(|p| !p.is_empty()) {
        return Ok(format!("https://github.com/{r}"));
    }
    Err("A Cursor agent needs a GitHub repository, like owner/repo.".into())
}

fn parse_progress(v: &Value) -> Progress {
    let link = v["target"]["prUrl"].as_str().or(v["target"]["url"].as_str()).unwrap_or_default().to_string();
    match v["status"].as_str().unwrap_or_default() {
        "FINISHED" => Progress::Done {
            summary: v["summary"].as_str().filter(|s| !s.trim().is_empty()).unwrap_or("Cursor finished.").to_string(),
            link,
        },
        "ERROR" => Progress::Failed(v["summary"].as_str().unwrap_or("Cursor reported an error.").to_string()),
        "EXPIRED" => Progress::Failed("The Cursor agent expired before it finished.".into()),
        "STOPPED" | "CANCELLED" => Progress::Failed("The Cursor agent was stopped before it finished.".into()),
        "FAILED" => Progress::Failed(v["summary"].as_str().unwrap_or("Cursor reported an error.").to_string()),
        _ => Progress::Running,
    }
}

async fn request(api_key: &str, req: reqwest::RequestBuilder) -> Result<Value, String> {
    let resp = req
        .basic_auth(api_key, None::<&str>)
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| format!("Could not reach Cursor: {e}"))?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        let msg = v["error"].as_str().or(v["error"]["message"].as_str()).or(v["message"].as_str()).unwrap_or("no details");
        return Err(match status.as_u16() {
            401 | 403 => "The Cursor key was rejected. Check it in Settings > Connections.".into(),
            code => format!("Cursor error {code}: {msg}"),
        });
    }
    Ok(v)
}

pub async fn launch(api_key: &str, repo: &str, prompt: &str) -> Result<Launched, String> {
    let repo = normalize_repo(repo)?;
    let v = request(api_key, reqwest::Client::new().post(API).json(&launch_body(&repo, prompt))).await?;
    let id = v["id"].as_str().ok_or("Cursor did not return an agent id.")?.to_string();
    let link = v["target"]["url"].as_str().unwrap_or_default().to_string();
    Ok(Launched { id, link })
}

pub async fn progress(api_key: &str, id: &str) -> Result<Progress, String> {
    let v = request(api_key, reqwest::Client::new().get(format!("{API}/{id}"))).await?;
    Ok(parse_progress(&v))
}

/// Checks running Cursor agents every 30 seconds.
pub async fn poll_loop(core: Arc<Core>) {
    loop {
        if let Err(e) = poll_once(&core).await {
            eprintln!("cursor check failed: {e}");
        }
        tokio::time::sleep(std::time::Duration::from_secs(POLL_SECS)).await;
    }
}

async fn poll_once(core: &Arc<Core>) -> Result<(), String> {
    let running = db::list_external_runs(&core.db.0.lock().unwrap()).map_err(|e| e.to_string())?;
    if running.is_empty() {
        return Ok(());
    }
    let Some(key) = core.secrets.get(CURSOR_KEY)? else {
        return Ok(());
    };
    for (run_id, agent_name, external_id) in running {
        let (status, output, link) = match progress(&key, &external_id).await {
            Ok(Progress::Running) => continue,
            Ok(Progress::Done { summary, link }) => ("done", summary, link),
            Ok(Progress::Failed(why)) => ("error", why, String::new()),
            Err(e) => {
                eprintln!("cursor agent {external_id}: {e}");
                continue;
            }
        };
        db::finish_external_run(&core.db.0.lock().unwrap(), run_id, status, &output, &link).map_err(|e| e.to_string())?;
        core.notify(format!("{agent_name} finished"), if status == "done" { "Cursor's work is ready to review." } else { "Cursor ran into a problem." });
        core.changed("runs-changed");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repos_accept_short_and_full_forms() {
        assert_eq!(normalize_repo("jfoley970/Command-Center-Plan").unwrap(), "https://github.com/jfoley970/Command-Center-Plan");
        assert_eq!(normalize_repo(" https://github.com/a/b.git ").unwrap(), "https://github.com/a/b");
        assert!(normalize_repo("").is_err());
        assert!(normalize_repo("just-a-name").is_err());
    }

    #[test]
    fn launch_asks_for_a_pull_request() {
        let b = launch_body("https://github.com/a/b", "fix it");
        assert_eq!(b["prompt"]["text"], "fix it");
        assert_eq!(b["source"]["repository"], "https://github.com/a/b");
        assert_eq!(b["target"]["autoCreatePr"], true);
    }

    #[test]
    fn progress_reads_status() {
        assert_eq!(parse_progress(&json!({ "status": "RUNNING" })), Progress::Running);
        assert_eq!(parse_progress(&json!({ "status": "CREATING" })), Progress::Running);
        assert_eq!(
            parse_progress(&json!({ "status": "FINISHED", "summary": "Fixed login", "target": { "url": "https://cursor.com/agents?id=1", "prUrl": "https://github.com/a/b/pull/7" } })),
            Progress::Done { summary: "Fixed login".into(), link: "https://github.com/a/b/pull/7".into() }
        );
        assert!(matches!(parse_progress(&json!({ "status": "EXPIRED" })), Progress::Failed(_)));
    }
}
