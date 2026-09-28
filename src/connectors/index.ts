// Importing a connector module registers it. Order here is the order in Settings within each group.
import "./claude";
import "./microsoft365";
import "./atera";
import "./unifi";
import "./planned";

export { allConnectors, type Connector } from "./registry";
