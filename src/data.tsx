// One shared copy of the app's data, refreshed after any change or backend event.
import { createContext, useCallback, useContext, useEffect, useState, type ReactNode } from "react";
import { listen } from "./transport";
import { api, errorText, type Agent, type AgentRun, type FlagLink, type Provider, type MailAccount, type Project, type Reminder, type Suggestion, type Todo } from "./api";

type Data = {
  todos: Todo[];
  reminders: Reminder[];
  projects: Project[];
  mailAccounts: MailAccount[];
  suggestions: Suggestion[];
  flagLinks: FlagLink[];
  agents: Agent[];
  runs: AgentRun[];
  providers: Provider[];
  /** Whether the Claude key is saved. */
  hasKey: boolean;
  error: string | null;
  refresh: () => Promise<void>;
  setError: (e: string | null) => void;
  /** Runs an action, reports any error in the banner, then refreshes. */
  act: <T>(fn: () => Promise<T>) => Promise<T | undefined>;
};

const DataContext = createContext<Data | null>(null);

export function DataProvider({ children }: { children: ReactNode }) {
  const [todos, setTodos] = useState<Todo[]>([]);
  const [reminders, setReminders] = useState<Reminder[]>([]);
  const [projects, setProjects] = useState<Project[]>([]);
  const [mailAccounts, setMailAccounts] = useState<MailAccount[]>([]);
  const [suggestions, setSuggestions] = useState<Suggestion[]>([]);
  const [flagLinks, setFlagLinks] = useState<FlagLink[]>([]);
  const [agents, setAgents] = useState<Agent[]>([]);
  const [runs, setRuns] = useState<AgentRun[]>([]);
  const [providers, setProviders] = useState<Provider[]>([]);
  const [hasKey, setHasKey] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const [t, r, p, m, s, f, a, ru, k, pr] = await Promise.all([
        api.listTodos(),
        api.listReminders(),
        api.listProjects(),
        api.listMailAccounts(),
        api.listSuggestions(),
        api.listFlagLinks(),
        api.listAgents(),
        api.listRuns(undefined, 50),
        api.hasApiKey(),
        api.listProviders(),
      ]);
      setTodos(t);
      setReminders(r);
      setProjects(p);
      setMailAccounts(m);
      setSuggestions(s);
      setFlagLinks(f);
      setAgents(a);
      setRuns(ru);
      setHasKey(k);
      setProviders(pr);
    } catch (e) {
      setError(errorText(e));
    }
  }, []);

  const act = useCallback(
    async <T,>(fn: () => Promise<T>) => {
      try {
        setError(null);
        return await fn();
      } catch (e) {
        setError(errorText(e));
        return undefined;
      } finally {
        await refresh();
      }
    },
    [refresh],
  );

  useEffect(() => {
    refresh();
    const unlisten = Promise.all([
      listen("runs-changed", () => refresh()),
      listen("reminders-fired", () => refresh()),
      listen("reminders-changed", () => refresh()),
      listen("mail-changed", () => refresh()),
      listen("providers-changed", () => refresh()),
    ]);
    // Keeps relative labels like "overdue" current.
    const timer = setInterval(refresh, 60_000);
    return () => {
      clearInterval(timer);
      unlisten.then((fns) => fns.forEach((f) => f()));
    };
  }, [refresh]);

  return (
    <DataContext.Provider value={{ todos, reminders, projects, mailAccounts, suggestions, flagLinks, agents, runs, providers, hasKey, error, refresh, setError, act }}>
      {children}
    </DataContext.Provider>
  );
}

export function useData(): Data {
  const d = useContext(DataContext);
  if (!d) throw new Error("useData must be used inside DataProvider");
  return d;
}
