// Typed wrappers around the Atera commands in crates/cc-core/src/commands.rs.
import { call } from "./transport";

export type Severity = "critical" | "warning" | "information";

export type AteraAlert = {
  id: number;
  title: string;
  message: string;
  severity: Severity;
  category: string;
  created: string | null;
  snoozed_until: string | null;
  ticket_id: number | null;
  device: string;
  customer_id: number | null;
  customer: string;
};

export type HiddenCustomer = { id: number; name: string };

export type AteraSnapshot = {
  has_key: boolean;
  alerts: AteraAlert[];
  hidden_count: number;
  hidden_customers: HiddenCustomer[];
  fetched_at: string | null;
  error: string | null;
};

export const atera = {
  alerts: () => call<AteraSnapshot>("atera_alerts"),
  refresh: () => call<AteraSnapshot>("atera_refresh"),
  setKey: (key: string) => call<AteraSnapshot>("atera_set_key", { key }),
  setCustomerHidden: (customer: HiddenCustomer, hidden: boolean) =>
    call<void>("atera_set_customer_hidden", { customer, hidden }),
};

/** "just now", "5m ago", "3h ago", "2d ago". */
export function ago(iso: string | null): string {
  if (!iso) return "";
  const mins = Math.max(0, Math.round((Date.now() - new Date(iso).getTime()) / 60_000));
  if (mins < 1) return "just now";
  if (mins < 60) return `${mins}m ago`;
  const h = Math.round(mins / 60);
  return h < 48 ? `${h}h ago` : `${Math.round(h / 24)}d ago`;
}
