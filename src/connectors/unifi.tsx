import { ago } from "../atera";
import { useLiveSnapshot } from "../live";
import { unifi } from "../unifi";
import { UnifiSetup } from "../widgets/UnifiFleet";
import { registerConnector } from "./registry";

registerConnector({
  id: "unifi",
  name: "UniFi Site Manager",
  group: "Network",
  description: "Health of every UniFi site on your account, with alerts when a console, gateway or device drops.",
  useStatus: () => {
    const { snap, error } = useLiveSnapshot(unifi.fleet, "unifi-changed");
    if (error) return { state: "attention", detail: error };
    if (!snap?.has_key) return { state: "off", detail: "Add a Site Manager API key" };
    if (snap.error) return { state: "attention", detail: snap.error };
    const down = snap.sites.filter((s) => s.health === "down").length;
    return {
      state: "connected",
      detail: [
        `${snap.sites.length} site${snap.sites.length === 1 ? "" : "s"}`,
        down > 0 && `${down} down`,
        snap.fetched_at && `updated ${ago(snap.fetched_at)}`,
      ]
        .filter(Boolean)
        .join(" · "),
    };
  },
  Panel: () => <UnifiSetup />,
});
