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
// Everything is optional so a missing or renamed field degrades one number
// instead of the whole widget.

#[derive(Deserialize)]
struct Envelope<T> {
    #[serde(default = "Vec::new")]
    data: Vec<T>,
    #[serde(rename = "nextToken")]
    next_token: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawHost {
    id: String,
    ip_address: Option<String>,
    is_blocked: Option<bool>,
    last_connection_state_change: Option<String>,
    reported_state: Option<RawReportedState>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawReportedState {
    hostname: Option<String>,
    name: Option<String>,
    state: Option<String>,
    version: Option<String>,
    hardware: Option<RawHardware>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawHardware {
    name: Option<String>,
    shortname: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawSite {
    site_id: String,
    host_id: String,
    meta: RawSiteMeta,
    statistics: RawStats,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawSiteMeta {
    desc: Option<String>,
    name: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawStats {
    counts: RawCounts,
    percentages: RawPercentages,
    isp_info: Option<RawIsp>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawCounts {
    total_device: Option<u32>,
    offline_device: Option<u32>,
    offline_gateway_device: Option<u32>,
    pending_update_device: Option<u32>,
    critical_notification: Option<u32>,
    wifi_client: Option<u32>,
    wired_client: Option<u32>,
    guest_client: Option<u32>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawPercentages {
    wan_uptime: Option<f64>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawIsp {
    name: Option<String>,
    organization: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawDeviceGroup {
    host_id: String,
    devices: Vec<RawDevice>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawDevice {
    id: String,
    mac: Option<String>,
    name: Option<String>,
    model: Option<String>,
    shortname: Option<String>,
    ip: Option<String>,
    status: Option<String>,
    version: Option<String>,
    firmware_status: Option<String>,
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
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err("UniFi rejected the API key. Create a new one at unifi.ui.com under API.".into());
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err("UniFi is rate limiting requests. Will retry shortly.".into());
        }
        if !status.is_success() {
            return Err(format!("UniFi Site Manager returned {status}."));
        }
        let body: Envelope<T> = res.json().await.map_err(|e| format!("Unexpected response from UniFi: {e}"))?;
        out.extend(body.data);
        match body.next_token.filter(|t| !t.is_empty()) {
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
    let devices: Vec<RawDeviceGroup> = get_all(&client, key, "/devices").await.unwrap_or_default();
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
        let h: Envelope<RawHost> = serde_json::from_str(HOSTS).unwrap();
        let s: Envelope<RawSite> = serde_json::from_str(SITES).unwrap();
        let d: Envelope<RawDeviceGroup> = serde_json::from_str(DEVICES).unwrap();
        assert!(h.next_token.as_deref() == Some(""));
        build(h.data, s.data, d.data)
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
}
