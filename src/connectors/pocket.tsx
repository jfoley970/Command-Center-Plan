import { ago } from "../atera";
import { PocketSetup, usePocket } from "../widgets/PocketCommands";
import { registerConnector } from "./registry";

registerConnector({
  id: "pocket",
  name: "Pocket recorder",
  group: "Devices",
  description: "Say the wake phrase on a recording and it becomes a todo, a reminder or an agent request.",
  useStatus: () => {
    const { snap, error } = usePocket();
    if (error) return { state: "attention", detail: error };
    if (!snap || (!snap.has_key && !snap.has_webhook_secret)) return { state: "off", detail: "Add a Pocket API key" };
    if (snap.error) return { state: "attention", detail: snap.error };
    const waiting = snap.commands.filter((c) => c.status === "pending").length;
    return {
      state: "connected",
      detail: [
        `Listening for “${snap.wake}”`,
        waiting > 0 && `${waiting} waiting`,
        snap.has_webhook_secret && "webhook on",
        snap.checked_at && `checked ${ago(snap.checked_at)}`,
      ]
        .filter(Boolean)
        .join(" · "),
    };
  },
  Panel: () => <PocketSetup />,
});
