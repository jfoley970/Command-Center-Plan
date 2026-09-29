import { useCallback, useEffect, useState } from "react";
import { listen } from "./transport";
import { errorText } from "./api";

/** Loads a backend snapshot, reloads it when `event` fires, and once a minute so "5m ago" labels stay current. */
export function useLiveSnapshot<T>(load: () => Promise<T>, event: string) {
  const [snap, setSnap] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(() => {
    load()
      .then((s) => {
        setSnap(s);
        setError(null);
      })
      .catch((e) => setError(errorText(e)));
  }, [load]);

  useEffect(() => {
    reload();
    const un = listen(event, reload);
    const t = setInterval(reload, 60_000);
    return () => {
      clearInterval(t);
      un.then((f) => f());
    };
  }, [reload, event]);

  return { snap, error, setError, reload };
}
