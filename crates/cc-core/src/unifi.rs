//! Read-only health for every UniFi site on one UI account, via the cloud Site
//! Manager API. It works from anywhere (no VPN into each site) but only refreshes
//! every few minutes, so the backend polls it, keeps the latest fleet in memory
//! and pops an OS notification when a site or device goes down.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::Arc;

use crate::secrets::{Secrets, UNIFI_KEY};
use crate::Core;

const API_BASE: &str = "https://api.ui.com/v1";
const PAGE_SIZE: &str = "200";
/// Site Manager data is itself refreshed every few minutes; polling faster buys nothing.
const POLL_SECS: u64 = 300;
/// Below this WAN uptime (percent over the API's window) a site is flagged.
const WAN_WARN_PCT: f64 = 99.0;

// ---------- API types ----------
// Everything is optional so a missing, null, renamed or oddly typed field
// degrades one number instead of the whole widget. `#[serde(default)]` alone
// only covers missing fields: a `null` or a decimal count would still fail the
// whole reply, so every field goes through one of the lenient readers below.

mod lenient {
    use serde::{Deserialize, Deserializer};
    use serde_json::Value;

    /// Any value, with null (or a value of the wrong shape) read as the default.
    pub fn or_default<'de, D: Deserializer<'de>, T: Default + for<'a> Deserialize<'a>>(d: D) -> Result<T, D::Error> {
        let v = Value::deserialize(d)?;
        Ok(serde_json::from_value(v).unwrap_or_default())
    }

    /// Text; numbers and booleans are turned into text, anything else is None.
    pub fn text<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
        Ok(match Value::deserialize(d)? {
            Value::String(s) => Some(s),
            Value::Number(n) => Some(n.to_string()),
            Value::Bool(b) => Some(b.to_string()),
            _ => None,
        })
    }

    /// Text that is never missing: absent or null become "".
    pub fn text_or_empty<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
        text(d).map(Option::unwrap_or_default)
    }

    /// A count: decimals are rounded, negatives and non-numbers are None.
    pub fn count<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u32>, D::Error> {
        Ok(number(Value::deserialize(d)?).filter(|n| *n >= 0.0).map(|n| n.round().min(u32::MAX as f64) as u32))
    }

    pub fn float<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
        Ok(number(Value::deserialize(d)?))
    }

    pub fn flag<'de, D: Deserializer<'de>>(d: D) -> Result<Option<bool>, D::Error> {
        Ok(match Value::deserialize(d)? {
            Value::Bool(b) => Some(b),
            Value::String(s) => s.parse().ok(),
            _ => None,
        })
    }

    fn number(v: Value) -> Option<f64> {
        match v {
            Value::Number(n) => n.as_f64(),
            Value::String(s) => s.trim().parse().ok(),
            _ => None,
        }
    }
}

/// One page of any list endpoint. Items stay raw JSON here so a single odd
/// item can be skipped instead of failing the page.
#[derive(Deserialize, Default)]
#[serde(default)]
struct Envelope {
    #[serde(deserialize_with = "lenient::or_default")]
    data: Vec<serde_json::Value>,
    #[serde(rename = "nextToken", deserialize_with = "lenient::text")]
    next_token: Option<String>,
}

/// Parses one page body. Returns the items that could be read and how many were skipped.
fn parse_page<T: for<'de> Deserialize<'de>>(path: &str, body: &str) -> Result<(Vec<T>, Option<String>, usize), String> {
    let page: Envelope = serde_json::from_str(body).map_err(|e| {
        let kind = if body.trim_start().starts_with('<') { " (got a web page, not JSON)" } else { "" };
        format!("Unexpected response from UniFi {path}{kind}: {e}")
    })?;
    let mut items = Vec::with_capacity(page.data.len());
    let mut skipped = 0;
    for v in page.data {
        match serde_json::from_value::<T>(v) {
            Ok(item) => items.push(item),
            Err(e) => {
                skipped += 1;
                eprintln!("unifi: skipped one item from {path}: {e}");
            }
        }
    }
    Ok((items, page.next_token, skipped))
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawHost {
    #[serde(deserialize_with = "lenient::text_or_empty")]
    id: String,
    #[serde(deserialize_with = "lenient::text")]
    ip_address: Option<String>,
    #[serde(deserialize_with = "lenient::flag")]
    is_blocked: Option<bool>,
    #[serde(deserialize_with = "lenient::text")]
    last_connection_state_change: Option<String>,
    #[serde(deserialize_with = "lenient::or_default")]
    reported_state: Option<RawReportedState>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawReportedState {
    #[serde(deserialize_with = "lenient::text")]
    hostname: Option<String>,
    #[serde(deserialize_with = "lenient::text")]
    name: Option<String>,
    #[serde(deserialize_with = "lenient::text")]
    state: Option<String>,
    #[serde(deserialize_with = "lenient::text")]
    version: Option<String>,
    #[serde(deserialize_with = "lenient::or_default")]
    hardware: Option<RawHardware>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawHardware {
    #[serde(deserialize_with = "lenient::text")]
    name: Option<String>,
    #[serde(deserialize_with = "lenient::text")]
    shortname: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawSite {
    #[serde(deserialize_with = "lenient::text_or_empty")]
    site_id: String,
    #[serde(deserialize_with = "lenient::text_or_empty")]
    host_id: String,
    #[serde(deserialize_with = "lenient::or_default")]
    meta: RawSiteMeta,
    #[serde(deserialize_with = "lenient::or_default")]
    statistics: RawStats,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawSiteMeta {
    #[serde(deserialize_with = "lenient::text")]
    desc: Option<String>,
    #[serde(deserialize_with = "lenient::text")]
    name: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawStats {
    #[serde(deserialize_with = "lenient::or_default")]
    counts: RawCounts,
    #[serde(deserialize_with = "lenient::or_default")]
    percentages: RawPercentages,
    #[serde(deserialize_with = "lenient::or_default")]
    isp_info: Option<RawIsp>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawCounts {
    #[serde(deserialize_with = "lenient::count")]
    total_device: Option<u32>,
    #[serde(deserialize_with = "lenient::count")]
    offline_device: Option<u32>,
    #[serde(deserialize_with = "lenient::count")]
    offline_gateway_device: Option<u32>,
    #[serde(deserialize_with = "lenient::count")]
    pending_update_device: Option<u32>,
    #[serde(deserialize_with = "lenient::count")]
    critical_notification: Option<u32>,
    #[serde(deserialize_with = "lenient::count")]
    wifi_client: Option<u32>,
    #[serde(deserialize_with = "lenient::count")]
    wired_client: Option<u32>,
    #[serde(deserialize_with = "lenient::count")]
    guest_client: Option<u32>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawPercentages {
    #[serde(deserialize_with = "lenient::float")]
    wan_uptime: Option<f64>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawIsp {
    #[serde(deserialize_with = "lenient::text")]
    name: Option<String>,
    #[serde(deserialize_with = "lenient::text")]
    organization: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawDeviceGroup {
    #[serde(deserialize_with = "lenient::text_or_empty")]
    host_id: String,
    #[serde(deserialize_with = "lenient::or_default")]
    devices: Vec<RawDevice>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawDevice {
    #[serde(deserialize_with = "lenient::text_or_empty")]
    id: String,
    #[serde(deserialize_with = "lenient::text")]
    mac: Option<String>,
    #[serde(deserialize_with = "lenient::text")]
    name: Option<String>,
    #[serde(deserialize_with = "lenient::text")]
    model: Option<String>,
    #[serde(deserialize_with = "lenient::text")]
    shortname: Option<String>,
    #[serde(deserialize_with = "lenient::text")]
    ip: Option<String>,
    #[serde(deserialize_with = "lenient::text")]
    status: Option<String>,
    #[serde(deserialize_with = "lenient::text")]
    version: Option<String>,
    #[serde(deserialize_with = "lenient::text")]
    firmware_status: Option<String>,
    #[serde(deserialize_with = "lenient::flag")]
    is_console: Option<bool>,
}

// ---------- What the UI sees ----------

#[derive(Serialize, Clone, Debug)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub model: String,
    pub ip: String,
    /// "online", "offline", or whatever else UniFi reports (for example "updating").
    pub status: String,
    pub update_available: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct Site {
    pub id: String,
    pub host_id: String,
    /// Console name, then the site name when a console runs more than one.
    pub name: String,
    pub console: String,
    pub console_model: String,
    pub console_online: bool,
    pub console_version: String,
    /// "ok", "warning" or "down".
    pub health: String,
    /// Plain-language reasons behind a warning or down state.
    pub issues: Vec<String>,
    pub devices_total: u32,
    pub devices_offline: u32,
    pub pending_updates: u32,
    pub clients: u32,
    pub wan_uptime: Option<f64>,
    pub isp: String,
    pub critical_notifications: u32,
    /// Devices that are not online, worst first. Empty when the device list is unavailable.
    pub problem_devices: Vec<Device>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct HiddenSite {
    pub id: String,
    pub name: String,
}

#[derive(Serialize, Clone, Default)]
pub struct Snapshot {
    pub has_key: bool,
    pub sites: Vec<Site>,
    pub hidden_sites: Vec<HiddenSite>,
    pub fetched_at: Option<String>,
    pub error: Option<String>,
}

// ---------- Fetching ----------

async fn get_all<T: for<'de> Deserialize<'de>>(client: &reqwest::Client, key: &str, path: &str) -> Result<Vec<T>, String> {
    let mut out = Vec::new();
    let mut token: Option<String> = None;
    // A generous cap; one account rarely has more than a few pages.
    for _ in 0..20 {
        let mut req = client
            .get(format!("{API_BASE}{path}"))
            .header("X-API-KEY", key)
            .header("Accept", "application/json")
            .query(&[("pageSize", PAGE_SIZE)]);
        if let Some(t) = &token {
            req = req.query(&[("nextToken", t)]);
        }
        let res = req.send().await.map_err(|e| format!("Could not reach UniFi Site Manager: {e}"))?;
        let status = res.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err("UniFi rejected the API key. Create a new one at unifi.ui.com under API.".into());
        }
        if status == reqwest::StatusCode::FORBIDDEN {
            return Err(format!("UniFi accepted the key but refused access to {path}. Make the key with the account that owns the sites."));
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err("UniFi is rate limiting requests. Will retry shortly.".into());
        }
        if !status.is_success() {
            return Err(format!("UniFi Site Manager returned {status} for {path}."));
        }
        let body = res.text().await.map_err(|e| format!("Could not read the reply from UniFi {path}: {e}"))?;
        let (items, next, _skipped) = parse_page::<T>(path, &body)?;
        out.extend(items);
        match next.filter(|t| !t.is_empty()) {
            Some(t) => token = Some(t),
            None => break,
        }
    }
    Ok(out)
}

async fn fetch_fleet(key: &str) -> Result<Vec<Site>, String> {
    let client = reqwest::Client::new();
    let hosts: Vec<RawHost> = get_all(&client, key, "/hosts").await?;
    let sites: Vec<RawSite> = get_all(&client, key, "/sites").await?;
    // Devices only add detail, so a failure here keeps the site tiles.
    let devices: Vec<RawDeviceGroup> = get_all(&client, key, "/devices").await.unwrap_or_else(|e| {
        eprintln!("unifi: device list unavailable: {e}");
        vec![]
    });
    Ok(build(hosts, sites, devices))
}

fn clean(s: &Option<String>) -> String {
    s.as_deref().map(str::trim).unwrap_or("").to_string()
}

fn build(hosts: Vec<RawHost>, sites: Vec<RawSite>, devices: Vec<RawDeviceGroup>) -> Vec<Site> {
    let hosts: HashMap<&str, &RawHost> = hosts.iter().map(|h| (h.id.as_str(), h)).collect();
    let mut by_host: HashMap<&str, Vec<&RawDevice>> = HashMap::new();
    for g in &devices {
        by_host.entry(g.host_id.as_str()).or_default().extend(g.devices.iter());
    }
    let sites_per_host = sites.iter().fold(HashMap::<&str, usize>::new(), |mut m, s| {
        *m.entry(s.host_id.as_str()).or_default() += 1;
        m
    });

    let mut out: Vec<Site> = sites
        .iter()
        .map(|s| {
            let host = hosts.get(s.host_id.as_str());
            let rs = host.and_then(|h| h.reported_state.as_ref());
            let console = rs
                .map(|r| clean(&r.name)).filter(|n| !n.is_empty())
                .or_else(|| rs.map(|r| clean(&r.hostname)).filter(|n| !n.is_empty()))
                .or_else(|| host.and_then(|h| h.ip_address.clone()))
                .unwrap_or_else(|| "UniFi console".into());
            let site_name = clean(&s.meta.desc);
            let site_name = if site_name.is_empty() { clean(&s.meta.name) } else { site_name };
            let multi = sites_per_host.get(s.host_id.as_str()).copied().unwrap_or(1) > 1;
            let name = if multi && !site_name.is_empty() && !site_name.eq_ignore_ascii_case("default") {
                format!("{console} · {site_name}")
            } else {
                console.clone()
            };
            // Hosts missing from /hosts can't be judged, so assume online rather than cry wolf.
            let console_online = match (host, rs.and_then(|r| r.state.as_deref())) {
                (Some(h), _) if h.is_blocked == Some(true) => false,
                (Some(_), Some(state)) => state.eq_ignore_ascii_case("connected"),
                _ => true,
            };

            let c = &s.statistics.counts;
            // The device list is per console, so it only lines up with a site when the console has one site.
            let problem_devices: Vec<Device> = if multi {
                vec![]
            } else {
                let mut list: Vec<Device> = by_host
                    .get(s.host_id.as_str())
                    .map(|ds| ds.iter().map(|d| to_device(d)).filter(|d| d.status != "online").collect())
                    .unwrap_or_default();
                list.sort_by(|a, b| (a.status != "offline").cmp(&(b.status != "offline")).then(a.name.cmp(&b.name)));
                list
            };
            let pending_from_list = by_host
                .get(s.host_id.as_str())
                .filter(|_| !multi)
                .map(|ds| ds.iter().filter(|d| to_device(d).update_available).count() as u32);

            let mut site = Site {
                id: s.site_id.clone(),
                host_id: s.host_id.clone(),
                name,
                console,
                console_model: rs.and_then(|r| r.hardware.as_ref()).map(|h| {
                    let n = clean(&h.name);
                    if n.is_empty() { clean(&h.shortname) } else { n }
                }).unwrap_or_default(),
                console_online,
                console_version: rs.map(|r| clean(&r.version)).unwrap_or_default(),
                health: String::new(),
                issues: vec![],
                devices_total: c.total_device.unwrap_or(0),
                devices_offline: c.offline_device.unwrap_or(0),
                pending_updates: c.pending_update_device.or(pending_from_list).unwrap_or(0),
                clients: c.wifi_client.unwrap_or(0) + c.wired_client.unwrap_or(0) + c.guest_client.unwrap_or(0),
                wan_uptime: s.statistics.percentages.wan_uptime,
                isp: s.statistics.isp_info.as_ref().map(|i| {
                    let n = clean(&i.name);
                    if n.is_empty() { clean(&i.organization) } else { n }
                }).unwrap_or_default(),
                critical_notifications: c.critical_notification.unwrap_or(0),
                problem_devices,
            };
            let gateway_down = c.offline_gateway_device.unwrap_or(0) > 0;
            judge(&mut site, gateway_down);
            site
        })
        .collect();

    // Worst first, then by name.
    let rank = |h: &str| match h {
        "down" => 0,
        "warning" => 1,
        _ => 2,
    };
    out.sort_by(|a, b| rank(&a.health).cmp(&rank(&b.health)).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    out
}

fn to_device(d: &RawDevice) -> Device {
    let name = clean(&d.name);
    let model = clean(&d.shortname);
    Device {
        id: if d.id.is_empty() { clean(&d.mac) } else { d.id.clone() },
        name: if name.is_empty() { clean(&d.mac) } else { name },
        model: if model.is_empty() { clean(&d.model) } else { model },
        ip: clean(&d.ip),
        status: d.status.as_deref().unwrap_or("unknown").to_ascii_lowercase(),
        update_available: d.firmware_status.as_deref().is_some_and(|f| f.eq_ignore_ascii_case("updateAvailable")),
    }
}

fn judge(s: &mut Site, gateway_down: bool) {
    let mut down = vec![];
    let mut warn = vec![];
    if !s.console_online {
        down.push("Console is offline".to_string());
    }
    if gateway_down {
        down.push("Gateway is offline".to_string());
    }
    if s.devices_offline > 0 {
        warn.push(format!("{} device{} offline", s.devices_offline, if s.devices_offline == 1 { "" } else { "s" }));
    }
    if let Some(up) = s.wan_uptime.filter(|u| *u < WAN_WARN_PCT) {
        warn.push(format!("WAN uptime {up:.1}%"));
    }
    if s.critical_notifications > 0 {
        warn.push(format!("{} critical notification{}", s.critical_notifications, if s.critical_notifications == 1 { "" } else { "s" }));
    }
    s.health = if !down.is_empty() { "down" } else if !warn.is_empty() { "warning" } else { "ok" }.into();
    down.extend(warn);
    s.issues = down;
}

// ---------- State ----------

pub struct UnifiState {
    inner: Mutex<Inner>,
    hidden_path: PathBuf,
}

struct Inner {
    all: Vec<Site>,
    fetched_at: Option<String>,
    error: Option<String>,
    hidden: Vec<HiddenSite>,
    /// Per site, the problems already notified; None until the first successful
    /// poll so the state at startup doesn't fire a burst of notifications.
    seen: Option<HashMap<String, HashSet<String>>>,
}

impl UnifiState {
    pub fn new(data_dir: &std::path::Path) -> Self {
        let hidden_path = data_dir.join("unifi-hidden-sites.json");
        let hidden = std::fs::read_to_string(&hidden_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self {
            inner: Mutex::new(Inner { all: vec![], fetched_at: None, error: None, hidden, seen: None }),
            hidden_path,
        }
    }

    pub fn snapshot(&self, secrets: &Secrets) -> Result<Snapshot, String> {
        let has_key = secrets.get(UNIFI_KEY)?.is_some();
        let i = self.inner.lock().unwrap();
        Ok(Snapshot {
            has_key,
            sites: i.all.iter().filter(|s| !i.hidden.iter().any(|h| h.id == s.id)).cloned().collect(),
            hidden_sites: i.hidden.clone(),
            fetched_at: i.fetched_at.clone(),
            error: i.error.clone(),
        })
    }

    pub fn set_hidden(&self, site: HiddenSite, hidden: bool) -> Result<(), String> {
        let mut i = self.inner.lock().unwrap();
        i.hidden.retain(|h| h.id != site.id);
        if hidden {
            i.hidden.push(site);
        }
        let json = serde_json::to_string_pretty(&i.hidden).map_err(|e| e.to_string())?;
        std::fs::write(&self.hidden_path, json).map_err(|e| format!("Could not save hidden sites: {e}"))
    }

    fn clear(&self) {
        let mut i = self.inner.lock().unwrap();
        i.all.clear();
        i.fetched_at = None;
        i.error = None;
        i.seen = None;
    }
}

/// Stable keys for the problems worth a notification at a site.
fn problem_keys(s: &Site) -> HashSet<String> {
    let mut keys = HashSet::new();
    if !s.console_online {
        keys.insert("console".into());
    }
    if s.issues.iter().any(|i| i == "Gateway is offline") {
        keys.insert("gateway".into());
    }
    for d in s.problem_devices.iter().filter(|d| d.status == "offline") {
        keys.insert(format!("device:{}", d.id));
    }
    // Without a device list, fall back to the count so a new outage still alerts.
    if s.problem_devices.is_empty() && s.devices_offline > 0 {
        keys.insert(format!("offline-count:{}", s.devices_offline));
    }
    keys
}

/// New problems since the last poll, one line per site.
fn new_problems(seen: &Option<HashMap<String, HashSet<String>>>, sites: &[&Site]) -> Vec<(String, String)> {
    let Some(seen) = seen else { return vec![] };
    let mut out = vec![];
    for s in sites {
        let now = problem_keys(s);
        let before = seen.get(&s.id);
        let fresh: Vec<&String> = now.iter().filter(|k| before.map_or(true, |b| !b.contains(*k))).collect();
        if fresh.is_empty() {
            continue;
        }
        let text = if fresh.iter().any(|k| *k == "console") {
            "Console went offline".to_string()
        } else if fresh.iter().any(|k| *k == "gateway") {
            "Gateway went offline".to_string()
        } else {
            let names: Vec<&str> = s
                .problem_devices
                .iter()
                .filter(|d| fresh.iter().any(|k| **k == format!("device:{}", d.id)))
                .map(|d| d.name.as_str())
                .collect();
            if names.is_empty() {
                format!("{} device{} offline", s.devices_offline, if s.devices_offline == 1 { "" } else { "s" })
            } else {
                format!("Offline: {}", names.join(", "))
            }
        };
        out.push((s.name.clone(), text));
    }
    out
}

/// Fetches the fleet now, stores it, notifies about new outages and tells the UI.
pub async fn refresh(core: &Core) -> Result<Snapshot, String> {
    let state = &core.unifi;
    let Some(key) = core.secrets.get(UNIFI_KEY)? else {
        state.clear();
        core.changed("unifi-changed");
        return state.snapshot(&core.secrets);
    };
    let result = fetch_fleet(&key).await;
    let fresh = {
        let mut i = state.inner.lock().unwrap();
        match result {
            Ok(sites) => {
                let hidden: HashSet<&str> = i.hidden.iter().map(|h| h.id.as_str()).collect();
                let visible: Vec<&Site> = sites.iter().filter(|s| !hidden.contains(s.id.as_str())).collect();
                let fresh = new_problems(&i.seen, &visible);
                i.seen = Some(visible.iter().map(|s| (s.id.clone(), problem_keys(s))).collect());
                i.all = sites;
                i.fetched_at = Some(Utc::now().to_rfc3339());
                i.error = None;
                fresh
            }
            Err(e) => {
                eprintln!("unifi: {e}");
                i.error = Some(e);
                vec![]
            }
        }
    };
    for (site, text) in fresh.iter().take(3) {
        core.notify(format!("UniFi: {site}"), text.clone());
    }
    if fresh.len() > 3 {
        core.notify("UniFi", format!("{} more sites have new problems", fresh.len() - 3));
    }
    core.changed("unifi-changed");
    state.snapshot(&core.secrets)
}

pub async fn poll_loop(core: Arc<Core>) {
    loop {
        if let Err(e) = refresh(&core).await {
            eprintln!("unifi poll failed: {e}");
        }
        tokio::time::sleep(std::time::Duration::from_secs(POLL_SECS)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOSTS: &str = r#"{"data":[
        {"id":"H1","ipAddress":"203.0.113.5","isBlocked":false,"reportedState":{"hostname":"udm-home","name":"Home UDM Pro","state":"connected","version":"4.3.6","hardware":{"name":"UniFi Dream Machine Pro","shortname":"UDMPRO"}}},
        {"id":"H2","reportedState":{"name":"Lake House","state":"disconnected"}}
    ],"httpStatusCode":200,"traceId":"t","nextToken":""}"#;
    const SITES: &str = r#"{"data":[
        {"siteId":"S1","hostId":"H1","meta":{"desc":"Default","name":"default"},"statistics":{"counts":{"totalDevice":6,"offlineDevice":1,"wifiClient":20,"wiredClient":5,"guestClient":1,"criticalNotification":0,"pendingUpdateDevice":2,"offlineGatewayDevice":0},"percentages":{"wanUptime":100},"ispInfo":{"name":"Comcast","organization":"Comcast Cable"}}},
        {"siteId":"S2","hostId":"H2","meta":{"desc":"Default"},"statistics":{"counts":{"totalDevice":3,"offlineDevice":3}}},
        {"siteId":"S3","hostId":"H3","meta":{"desc":"Office"},"statistics":{"counts":{"totalDevice":2,"offlineDevice":0},"percentages":{"wanUptime":99.9}}}
    ]}"#;
    const DEVICES: &str = r#"{"data":[{"hostId":"H1","hostName":"udm-home","devices":[
        {"id":"D1","mac":"aa","name":"Garage AP","shortname":"U6LR","ip":"10.0.0.9","status":"offline","firmwareStatus":"upToDate"},
        {"id":"D2","mac":"bb","name":"Core switch","status":"online","firmwareStatus":"updateAvailable"}
    ]}]}"#;

    fn fleet() -> Vec<Site> {
        let (h, next, _) = parse_page::<RawHost>("/hosts", HOSTS).unwrap();
        let (s, _, _) = parse_page::<RawSite>("/sites", SITES).unwrap();
        let (d, _, _) = parse_page::<RawDeviceGroup>("/devices", DEVICES).unwrap();
        assert!(next.as_deref() == Some(""));
        build(h, s, d)
    }

    #[test]
    fn builds_sites_worst_first() {
        let sites = fleet();
        assert_eq!(sites.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["S2", "S1", "S3"]);

        let lake = &sites[0];
        assert_eq!(lake.health, "down");
        assert_eq!(lake.name, "Lake House");
        assert!(!lake.console_online);

        let home = &sites[1];
        assert_eq!(home.health, "warning");
        assert_eq!(home.name, "Home UDM Pro");
        assert_eq!(home.console_model, "UniFi Dream Machine Pro");
        assert_eq!(home.clients, 26);
        assert_eq!(home.pending_updates, 2);
        assert_eq!(home.isp, "Comcast");
        assert_eq!(home.problem_devices.len(), 1);
        assert_eq!(home.problem_devices[0].name, "Garage AP");
        assert_eq!(home.issues, ["1 device offline"]);

        // Unknown host: named generically, not flagged down.
        assert_eq!(sites[2].health, "ok");
        assert_eq!(sites[2].name, "UniFi console");
    }

    #[test]
    fn notifies_only_new_problems_after_first_poll() {
        let sites = fleet();
        let refs: Vec<&Site> = sites.iter().collect();
        assert!(new_problems(&None, &refs).is_empty());

        let mut seen: HashMap<String, HashSet<String>> = HashMap::new();
        seen.insert("S1".into(), HashSet::new());
        let fresh = new_problems(&Some(seen.clone()), &refs);
        // S2 unseen, S1 has a new offline device, S3 is fine.
        assert_eq!(fresh.len(), 2);
        assert!(fresh.iter().any(|(n, t)| n == "Home UDM Pro" && t == "Offline: Garage AP"));
        assert!(fresh.iter().any(|(n, t)| n == "Lake House" && t == "Console went offline"));

        let all_seen: HashMap<_, _> = sites.iter().map(|s| (s.id.clone(), problem_keys(s))).collect();
        assert!(new_problems(&Some(all_seen), &refs).is_empty());
    }

    #[test]
    fn tolerates_nulls_odd_types_and_bad_items() {
        // Nulls where the code expects text or objects, decimal and negative
        // counts, a numeric version, a null data list on devices and one item
        // that is not an object at all.
        let hosts = r#"{"data":[
            {"id":"H1","ipAddress":null,"isBlocked":null,"reportedState":{"name":null,"hostname":"udm","state":"connected","version":4.3,"hardware":null}},
            {"id":null,"reportedState":null},
            "not an object"
        ],"nextToken":null}"#;
        let sites = r#"{"data":[
            {"siteId":"S1","hostId":"H1","meta":null,"statistics":{"counts":{"totalDevice":6.0,"offlineDevice":null,"wifiClient":-1,"wiredClient":"3"},"percentages":null,"ispInfo":null}},
            {"siteId":"S2","hostId":null,"meta":{"desc":null,"name":"Office"},"statistics":null}
        ]}"#;
        let devices = r#"{"data":null}"#;

        let (h, next, skipped) = parse_page::<RawHost>("/hosts", hosts).unwrap();
        assert_eq!((h.len(), skipped, next), (2, 1, None));
        let (s, _, _) = parse_page::<RawSite>("/sites", sites).unwrap();
        let (d, _, _) = parse_page::<RawDeviceGroup>("/devices", devices).unwrap();
        assert!(d.is_empty());

        let fleet = build(h, s, d);
        assert_eq!(fleet.len(), 2);
        let home = fleet.iter().find(|s| s.id == "S1").unwrap();
        assert_eq!(home.name, "udm");
        assert_eq!(home.console_version, "4.3");
        assert_eq!(home.devices_total, 6);
        assert_eq!(home.clients, 3);
        assert_eq!(home.health, "ok");
    }

    #[test]
    fn unreadable_reply_names_the_endpoint() {
        let Err(err) = parse_page::<RawSite>("/sites", "<html>maintenance</html>") else { panic!("should fail") };
        assert!(err.contains("/sites") && err.contains("web page"), "{err}");
    }
}
