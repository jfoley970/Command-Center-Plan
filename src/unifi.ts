// Typed wrappers around the UniFi commands in crates/cc-core/src/commands.rs.
import { call } from "./transport";

export type Health = "ok" | "warning" | "down";

export type UnifiDevice = {
  id: string;
  name: string;
  model: string;
  ip: string;
  status: string;
  update_available: boolean;
};

export type UnifiSite = {
  id: string;
  host_id: string;
  name: string;
  console: string;
  console_model: string;
  console_online: boolean;
  console_version: string;
  health: Health;
  issues: string[];
  devices_total: number;
  devices_offline: number;
  pending_updates: number;
  clients: number;
  wan_uptime: number | null;
  isp: string;
  critical_notifications: number;
  problem_devices: UnifiDevice[];
};

export type HiddenSite = { id: string; name: string };

export type UnifiSnapshot = {
  has_key: boolean;
  sites: UnifiSite[];
  hidden_sites: HiddenSite[];
  fetched_at: string | null;
  error: string | null;
};

export const unifi = {
  fleet: () => call<UnifiSnapshot>("unifi_fleet"),
  refresh: () => call<UnifiSnapshot>("unifi_refresh"),
  setKey: (key: string) => call<UnifiSnapshot>("unifi_set_key", { key }),
  setSiteHidden: (site: HiddenSite, hidden: boolean) => call<void>("unifi_set_site_hidden", { site, hidden }),
};
