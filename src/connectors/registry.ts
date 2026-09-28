// Every API, agent and service the app talks to registers itself here, and
// Settings > Connections lists whatever is registered. A new integration adds
// one file in this folder and one import in index.ts.
import type { ComponentType } from "react";
import type { Page } from "../App";

export type ConnectorState = "connected" | "attention" | "off" | "planned";

export type ConnectorStatus = {
  state: ConnectorState;
  /** One line under the name, such as "2 inboxes · 14 unread" or the last error. */
  detail: string;
};

export type Connector = {
  id: string;
  name: string;
  group: "AI" | "Email" | "Monitoring" | "Network" | "Infrastructure" | "Devices";
  /** What the app does with it, in a sentence. */
  description: string;
  /** A React hook, so it can subscribe to live status. */
  useStatus: () => ConnectorStatus;
  /** Setup and management, shown when the row is expanded. */
  Panel?: ComponentType<{ go: (p: Page) => void }>;
};

const connectors: Connector[] = [];

export function registerConnector(c: Connector) {
  const i = connectors.findIndex((x) => x.id === c.id);
  if (i >= 0) connectors[i] = c;
  else connectors.push(c);
}

export function allConnectors(): readonly Connector[] {
  return connectors;
}

export const PLANNED: ConnectorStatus = { state: "planned", detail: "Coming later" };
