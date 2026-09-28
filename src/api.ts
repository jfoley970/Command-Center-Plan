// Typed wrappers around the Rust commands in src-tauri/src/lib.rs.
import { invoke } from "@tauri-apps/api/core";

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

export type Project = {
  id: number;
  name: string;
  description: string;
  color: string;
  archived: boolean;
  created_at: string;
};

export type Agent = {
  id: number;
  name: string;
  description: string;
  system_prompt: string;
  model: string;
  created_at: string;
};

export type AgentRun = {
  id: number;
  agent_id: number;
  agent_name: string;
  input: string;
  output: string;
  status: "running" | "done" | "error" | "refused";
  model: string;
  input_tokens: number;
  output_tokens: number;
  started_at: string;
  finished_at: string | null;
};

export type ModelOption = { id: string; label: string };

export const api = {
  listTodos: () => invoke<Todo[]>("list_todos"),
  addTodo: (todo: { title: string; notes?: string; priority?: number; due_at?: string | null; project_id?: number | null }) =>
    invoke<Todo>("add_todo", { todo }),
  setTodoDone: (id: number, done: boolean) => invoke<void>("set_todo_done", { id, done }),
  setTodoPriority: (id: number, priority: number) => invoke<void>("set_todo_priority", { id, priority }),
  deleteTodo: (id: number) => invoke<void>("delete_todo", { id }),

  listReminders: () => invoke<Reminder[]>("list_reminders"),
  addReminder: (reminder: { title: string; remind_at: string; repeat: Repeat; project_id?: number | null }) =>
    invoke<Reminder>("add_reminder", { reminder }),
  deleteReminder: (id: number) => invoke<void>("delete_reminder", { id }),
  snoozeReminder: (id: number, minutes: number) => invoke<void>("snooze_reminder", { id, minutes }),

  listProjects: () => invoke<Project[]>("list_projects"),
  saveProject: (project: Omit<Project, "id" | "created_at"> & { id?: number }) =>
    invoke<Project>("save_project", { project }),
  deleteProject: (id: number) => invoke<void>("delete_project", { id }),

  getMailSetup: () => invoke<{ client_id: string; tenant_id: string }>("get_mail_setup"),
  connectMicrosoft: (clientId: string, tenantId: string) =>
    invoke<MailAccount>("connect_microsoft", { clientId, tenantId }),
  listMailAccounts: () => invoke<MailAccount[]>("list_mail_accounts"),
  listEmails: (accountId: number) => invoke<Email[]>("list_emails", { accountId }),
  syncMail: (accountId: number) => invoke<number>("sync_mail", { accountId }),
  disconnectMail: (accountId: number) => invoke<void>("disconnect_mail", { accountId }),
  listSuggestions: () => invoke<Suggestion[]>("list_suggestions"),
  acceptSuggestion: (suggestion: { id: number; kind: string; title: string; due_at: string | null; project_id: number | null }) =>
    invoke<void>("accept_suggestion", { suggestion }),
  dismissSuggestion: (id: number) => invoke<void>("dismiss_suggestion", { id }),

  listModels: () => invoke<ModelOption[]>("list_models"),
  listAgents: () => invoke<Agent[]>("list_agents"),
  saveAgent: (agent: Omit<Agent, "id" | "created_at"> & { id?: number }) =>
    invoke<Agent>("save_agent", { agent }),
  deleteAgent: (id: number) => invoke<void>("delete_agent", { id }),
  listRuns: (agentId?: number, limit?: number) =>
    invoke<AgentRun[]>("list_runs", { agentId: agentId ?? null, limit: limit ?? null }),
  runAgent: (agentId: number, input: string) => invoke<AgentRun>("run_agent", { agentId, input }),

  hasApiKey: () => invoke<boolean>("has_api_key"),
  setApiKey: (key: string) => invoke<void>("set_api_key", { key }),
};

export function errorText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
}
