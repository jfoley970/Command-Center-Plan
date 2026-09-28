import { PLANNED, registerConnector } from "./registry";

// Placeholders so the roadmap is visible in Settings; each gets its own file when built.
registerConnector({ id: "esxi", name: "VMware ESXi", group: "Infrastructure", description: "VM power state, host health and datastore space.", useStatus: () => PLANNED });
registerConnector({ id: "pocket", name: "Pocket recorder", group: "Devices", description: "Turns recording action items into todos and reminders.", useStatus: () => PLANNED });
registerConnector({ id: "calendar", name: "Calendar", group: "Email", description: "Today's meetings on the timeline.", useStatus: () => PLANNED });
registerConnector({ id: "voice", name: "Voice", group: "AI", description: "Push-to-talk commands.", useStatus: () => PLANNED });
