//! Read-only Atera RMM alerts. The backend polls the REST API (Atera only sends
//! webhooks for tickets, and only on higher plans), keeps the latest list in memory,
//! and pops an OS notification for new critical alerts.

use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::Arc;

use crate::secrets::{Secrets, ATERA_KEY};
use crate::Core;

const API_URL: &str = "https://app.atera.com/api/v3/alerts";
/// Atera caps a page at 50 items.
const PAGE_SIZE: u32 = 50;
/// Enough for any realistic open-alert backlog while staying far under the rate limit.
const MAX_PAGES: u32 = 20;
const POLL_SECS: u64 = 120;

// ---------- API types ----------

#[derive(Deserialize)]
struct Page {
    #[serde(default)]
    items: Vec<RawAlert>,
    #[serde(rename = "totalPages", default)]
    total_pages: u32,
}

/// Field names follow Atera's v3 alert object. Everything is optional so a
/// missing or renamed field degrades one column instead of the whole widget.
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawAlert {
    #[serde(rename = "AlertID")]
    alert_id: i64,
    title: Option<String>,
    severity: Option<String>,
    created: Option<String>,
    snoozed_end_date: Option<String>,
    archived: Option<bool>,
    #[serde(rename = "TicketID")]
    ticket_id: Option<i64>,
    alert_message: Option<String>,
    device_name: Option<String>,
    #[serde(rename = "CustomerID")]
    customer_id: Option<i64>,
    customer_name: Option<String>,
    alert_category_id: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct Alert {
    pub id: i64,
    pub title: String,
    pub message: String,
    /// "critical", "warning" or "information".
    pub severity: String,
    pub category: String,
    /// RFC 3339 UTC.
    pub created: Option<String>,
    pub snoozed_until: Option<String>,
    pub ticket_id: Option<i64>,
    pub device: String,
    pub customer_id: Option<i64>,
    pub customer: String,
}

#[derive(Serialize, Clone, Default)]
pub struct Snapshot {
    pub has_key: bool,
    pub alerts: Vec<Alert>,
    /// Alerts from hidden customers, counted but not shown.
    pub hidden_count: usize,
    pub hidden_customers: Vec<HiddenCustomer>,
    pub fetched_at: Option<String>,
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct HiddenCustomer {
    pub id: i64,
    pub name: String,
}

/// Atera sends timestamps without an offset; they are UTC.
fn to_rfc3339(s: &str) -> Option<String> {
    if let Ok(d) = DateTime::parse_from_rfc3339(s) {
        return Some(d.with_timezone(&Utc).to_rfc3339());
    }
    NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f")
        .ok()
        .map(|n| n.and_utc().to_rfc3339())
}

fn normalize(r: RawAlert) -> Alert {
    let severity = match r.severity.as_deref().unwrap_or("").to_ascii_lowercase().as_str() {
        "critical" => "critical",
        "warning" => "warning",
        _ => "information",
    };
    let clean = |s: Option<String>| s.map(|v| v.trim().to_string()).unwrap_or_default();
    Alert {
        id: r.alert_id,
        title: clean(r.title),
        message: clean(r.alert_message),
        severity: severity.into(),
        category: clean(r.alert_category_id),
        created: r.created.as_deref().and_then(to_rfc3339),
        snoozed_until: r.snoozed_end_date.as_deref().and_then(to_rfc3339),
        ticket_id: r.ticket_id.filter(|t| *t > 0),
        device: clean(r.device_name),
        customer_id: r.customer_id,
        customer: clean(r.customer_name),
    }
}

async fn fetch_alerts(key: &str) -> Result<Vec<Alert>, String> {
    let client = reqwest::Client::new();
    let mut out = Vec::new();
    let mut page = 1;
    loop {
        let res = client
            .get(API_URL)
            .header("X-API-KEY", key)
            .header("Accept", "application/json")
            .query(&[("page", page), ("itemsInPage", PAGE_SIZE)])
            .send()
            .await
            .map_err(|e| format!("Could not reach Atera: {e}"))?;
        let status = res.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err("Atera rejected the API key. Check that it is active and can read alerts.".into());
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err("Atera is rate limiting requests. Will retry shortly.".into());
        }
        if !status.is_success() {
            return Err(format!("Atera returned {status}."));
        }
        let body: Page = res.json().await.map_err(|e| format!("Unexpected response from Atera: {e}"))?;
        out.extend(body.items.into_iter().filter(|a| a.archived != Some(true)).map(normalize));
        if page >= body.total_pages || page >= MAX_PAGES {
            break;
        }
        page += 1;
    }
    // Worst first, then newest.
    let rank = |s: &str| match s {
        "critical" => 0,
        "warning" => 1,
        _ => 2,
    };
    out.sort_by(|a, b| rank(&a.severity).cmp(&rank(&b.severity)).then(b.created.cmp(&a.created)));
    Ok(out)
}

// ---------- State ----------

pub struct AteraState {
    inner: Mutex<Inner>,
    hidden_path: PathBuf,
}

struct Inner {
    all: Vec<Alert>,
    fetched_at: Option<String>,
    error: Option<String>,
    hidden: Vec<HiddenCustomer>,
    /// Critical alert ids already notified; None until the first successful poll,
    /// so the backlog at startup doesn't fire a burst of notifications.
    seen_critical: Option<HashSet<i64>>,
}

impl AteraState {
    pub fn new(data_dir: &std::path::Path) -> Self {
        let hidden_path = data_dir.join("atera-hidden-customers.json");
        let hidden = std::fs::read_to_string(&hidden_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self {
            inner: Mutex::new(Inner { all: vec![], fetched_at: None, error: None, hidden, seen_critical: None }),
            hidden_path,
        }
    }

    pub fn snapshot(&self, secrets: &Secrets) -> Result<Snapshot, String> {
        let has_key = secrets.get(ATERA_KEY)?.is_some();
        let i = self.inner.lock().unwrap();
        let is_hidden = |a: &Alert| a.customer_id.is_some_and(|id| i.hidden.iter().any(|h| h.id == id));
        let alerts: Vec<Alert> = i.all.iter().filter(|a| !is_hidden(a)).cloned().collect();
        Ok(Snapshot {
            has_key,
            hidden_count: i.all.len() - alerts.len(),
            alerts,
            hidden_customers: i.hidden.clone(),
            fetched_at: i.fetched_at.clone(),
            error: i.error.clone(),
        })
    }

    pub fn set_hidden(&self, customer: HiddenCustomer, hidden: bool) -> Result<(), String> {
        let mut i = self.inner.lock().unwrap();
        i.hidden.retain(|h| h.id != customer.id);
        if hidden {
            i.hidden.push(customer);
        }
        let json = serde_json::to_string_pretty(&i.hidden).map_err(|e| e.to_string())?;
        std::fs::write(&self.hidden_path, json).map_err(|e| format!("Could not save hidden customers: {e}"))
    }

    fn clear(&self) {
        let mut i = self.inner.lock().unwrap();
        i.all.clear();
        i.fetched_at = None;
        i.error = None;
        i.seen_critical = None;
    }
}

/// Fetches alerts now, stores them, notifies about new critical ones and tells the UI.
pub async fn refresh(core: &Core) -> Result<Snapshot, String> {
    let state = &core.atera;
    let Some(key) = core.secrets.get(ATERA_KEY)? else {
        state.clear();
        core.changed("atera-changed");
        return state.snapshot(&core.secrets);
    };
    let result = fetch_alerts(&key).await;
    let fresh: Vec<Alert> = {
        let mut i = state.inner.lock().unwrap();
        match result {
            Ok(alerts) => {
                let now = Utc::now().to_rfc3339();
                let hidden: HashSet<i64> = i.hidden.iter().map(|h| h.id).collect();
                let active_critical: Vec<&Alert> = alerts
                    .iter()
                    .filter(|a| a.severity == "critical")
                    .filter(|a| !a.customer_id.is_some_and(|c| hidden.contains(&c)))
                    .filter(|a| a.snoozed_until.as_deref().map_or(true, |s| s < now.as_str()))
                    .collect();
                let fresh = match &i.seen_critical {
                    Some(seen) => active_critical.iter().filter(|a| !seen.contains(&a.id)).map(|a| (*a).clone()).collect(),
                    None => vec![],
                };
                let seen = i.seen_critical.get_or_insert_with(HashSet::new);
                seen.extend(active_critical.iter().map(|a| a.id));
                i.all = alerts;
                i.fetched_at = Some(now);
                i.error = None;
                fresh
            }
            Err(e) => {
                i.error = Some(e);
                vec![]
            }
        }
    };
    for a in fresh.iter().take(3) {
        let where_ = [a.customer.as_str(), a.device.as_str()].iter().filter(|s| !s.is_empty()).cloned().collect::<Vec<_>>().join(" · ");
        let body = if where_.is_empty() { a.title.clone() } else { format!("{}\n{}", a.title, where_) };
        core.notify("Atera critical alert", body);
    }
    if fresh.len() > 3 {
        core.notify("Atera critical alerts", format!("{} more new critical alerts", fresh.len() - 3));
    }
    core.changed("atera-changed");
    state.snapshot(&core.secrets)
}

pub async fn poll_loop(core: Arc<Core>) {
    loop {
        if let Err(e) = refresh(&core).await {
            eprintln!("atera poll failed: {e}");
        }
        tokio::time::sleep(std::time::Duration::from_secs(POLL_SECS)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_atera_page() {
        let json = r#"{"items":[{"AlertID":7,"Title":"Disk C: low","Severity":"Critical","Created":"2026-09-27T10:15:00.123",
            "Archived":false,"TicketID":0,"DeviceName":"SRV01","CustomerID":3,"CustomerName":"Acme","AlertCategoryID":"Disk",
            "SnoozedEndDate":null,"Unknown":"ignored"},{"AlertID":8,"Archived":true}],"totalPages":1,"page":1}"#;
        let page: Page = serde_json::from_str(json).unwrap();
        let alerts: Vec<Alert> = page.items.into_iter().filter(|a| a.archived != Some(true)).map(normalize).collect();
        assert_eq!(alerts.len(), 1);
        let a = &alerts[0];
        assert_eq!(a.severity, "critical");
        assert_eq!(a.ticket_id, None);
        assert_eq!(a.created.as_deref(), Some("2026-09-27T10:15:00.123+00:00"));
        assert_eq!(a.customer, "Acme");
    }
}
