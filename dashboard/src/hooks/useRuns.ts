import { useState, useEffect, useCallback } from "react";
import { resources } from "@/lib/mcp";
import { onNatsMessage } from "@/lib/nats";
import type { AgentRun } from "@/types/swarm";

const INITIAL_LOAD_MS = 1_000;
const SLOW_POLL_MS = 30_000;

export function useRuns(project: string) {
  const [runs, setRuns] = useState<AgentRun[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const fetchRuns = useCallback(async () => {
    if (!project) return;
    try {
      setRuns(await resources.projectRuns(project));
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [project]);

  useEffect(() => {
    if (!project) return;
    fetchRuns();
    // Slow poll as fallback — NATS events trigger fast refreshes
    const interval = setInterval(fetchRuns, SLOW_POLL_MS);
    // NATS events trigger immediate refresh
    const unsub = onNatsMessage((subject) => {
      if (subject.includes(project) || subject.includes("agent") || subject.includes("swarm")) {
        setTimeout(fetchRuns, INITIAL_LOAD_MS);
      }
    });
    return () => { clearInterval(interval); unsub(); };
  }, [project, fetchRuns]);

  return { runs, loading, error };
}
