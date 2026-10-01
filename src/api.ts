// Typed wrappers around the backend commands in crates/cc-core/src/commands.rs.
import { call } from "./transport";

export type Todo = {
  id: number;
  title: string;
  notes: string;
  priority: 1 | 2 | 3;
  due_at: string | null;
  done: boolean;
  created_at: string;
  done_at: string | null;
  project_id: number | null;
  email_id: number | null;
};

export type Repeat = "none" | "daily" | "weekly";

export type Reminder = {
  id: number;
  title: string;
  remind_at: string;
  repeat: Repeat;
  fired: boolean;
  created_at: string;
  project_id: number | null;
  email_id: number | null;
};

export type MailAccount = {
  id: number;
  email: string;
  display_name: string;
  client_id: string;
  tenant_id: string;
  summary: string;
  summary_at: string | null;
  last_sync_at: string | null;
  last_error: string | null;
  unread: number;
  pending: number;
};

export type Email = {
  id: number;
  account_id: number;
  subject: string;
  from_name: string;
  from_addr: string;
  received_at: string;
  preview: string;
  is_read: boolean;
  web_link: string;
  analyzed: boolean;
};

export type Suggestion = {
  id: number;
  account_id: number;
  email_id: number | null;
  kind: "todo" | "reminder";
  title: string;
  notes: string;
  due_at: string | null;
  priority: 1 | 2 | 3;
  project_id: number | null;
  status: string;
  created_at: string;
  email_subject: string | null;
  email_from: string | null;
  email_link: string | null;
};

export type FlagLink = {
  account_id: number;
  todo_id: number | null;
  subject: string;
  sender: string;
  web_link: string;
  flagged: boolean;
};

export type TodoEmail = {
  subject: string;
  from_name: string;
  from_addr: string;
  received_at: string | null;
  preview: string;
  web_link: string;
};

export type Project = {
  id: number;
  name: string;
  description: string;
  color: string;
  archived: boolean;
  created_at: string;
  parent_id: number | null;
};

export type Agent = {
  id: number;
  name: string;
  description: string;
  system_prompt: string;
  model: string;
  created_at: string;
  provider: ProviderId;
  /** GitHub repository a Cursor agent works on. */
  repo: string;
  /** Whether runs include the day's open todos and reminders. */
  include_context: boolean;
};

export type ProviderId = "claude" | "chatgpt" | "grok" | "cursor" | "copilot";

export type Provider = {
  id: ProviderId;
  name: string;
  connected: boolean;
  /** Works on a repository and finishes later (Cursor). */
  background: boolean;
  /** Why it can't be used from the app yet, if it can't. */
  unavailable: string | null;
  models: ModelOption[];
};

export type AgentRun = {
  id: number;
  agent_id: number;
  agent_name: string;
  provider: ProviderId;
  input: string;
  output: string;
  status: "running" | "done" | "error" | "refused";
  model: string;
  input_tokens: number;
  output_tokens: number;
  started_at: string;
  finished_at: string | null;
  /** Where to see the result outside the app, such as a pull request. */
  link: string;
};

export type ModelOption = { id: string; label: string };

export const api = {
  listTodos: () => call<Todo[]>("list_todos"),
  addTodo: (todo: { title: string; notes?: string; priority?: number; due_at?: string | null; project_id?: number | null }) =>
    call<Todo>("add_todo", { todo }),
  setTodoDone: (id: number, done: boolean) => call<void>("set_todo_done", { id, done }),
  setTodoPriority: (id: number, priority: number) => call<void>("set_todo_priority", { id, priority }),
  setTodoNotes: (id: number, notes: string) => call<void>("set_todo_notes", { id, notes }),
  todoEmail: (id: number) => call<TodoEmail | null>("todo_email", { id }),
  deleteTodo: (id: number) => call<void>("delete_todo", { id }),

  listReminders: () => call<Reminder[]>("list_reminders"),
  addReminder: (reminder: { title: string; remind_at: string; repeat: Repeat; project_id?: number | null }) =>
    call<Reminder>("add_reminder", { reminder }),
  deleteReminder: (id: number) => call<void>("delete_reminder", { id }),
  snoozeReminder: (id: number, minutes: number) => call<void>("snooze_reminder", { id, minutes }),

  listProjects: () => call<Project[]>("list_projects"),
  saveProject: (project: Omit<Project, "id" | "created_at"> & { id?: number }) =>
    call<Project>("save_project", { project }),
  deleteProject: (id: number) => call<void>("delete_project", { id }),

  getMailSetup: () => call<{ client_id: string; tenant_id: string }>("get_mail_setup"),
  connectMicrosoft: (clientId: string, tenantId: string) =>
    call<MailAccount>("connect_microsoft", { clientId, tenantId }),
  listMailAccounts: () => call<MailAccount[]>("list_mail_accounts"),
  listEmails: (accountId: number) => call<Email[]>("list_emails", { accountId }),
  syncMail: (accountId: number) => call<number>("sync_mail", { accountId }),
  disconnectMail: (accountId: number) => call<void>("disconnect_mail", { accountId }),
  listFlagLinks: () => call<FlagLink[]>("list_flag_links"),
  listSuggestions: () => call<Suggestion[]>("list_suggestions"),
  acceptSuggestion: (suggestion: { id: number; kind: string; title: string; due_at: string | null; project_id: number | null }) =>
    call<void>("accept_suggestion", { suggestion }),
  dismissSuggestion: (id: number) => call<void>("dismiss_suggestion", { id }),

  listModels: () => call<ModelOption[]>("list_models"),
  listAgents: () => call<Agent[]>("list_agents"),
  saveAgent: (agent: Omit<Agent, "id" | "created_at"> & { id?: number }) =>
    call<Agent>("save_agent", { agent }),
  deleteAgent: (id: number) => call<void>("delete_agent", { id }),
  listRuns: (agentId?: number, limit?: number) =>
    call<AgentRun[]>("list_runs", { agentId: agentId ?? null, limit: limit ?? null }),
  runAgent: (agentId: number, input: string) => call<AgentRun>("run_agent", { agentId, input }),
  listProviders: () => call<Provider[]>("list_providers"),
  setProviderKey: (provider: ProviderId, key: string) => call<void>("set_provider_key", { provider, key }),
  /** Asks a provider directly, using its first agent (made on first use). */
  askProvider: (provider: ProviderId, input: string) => call<AgentRun>("ask_provider", { provider, input }),

  hasApiKey: () => call<boolean>("has_api_key"),
  setApiKey: (key: string) => call<void>("set_api_key", { key }),
};

export function errorText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
}
