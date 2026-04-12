import { useState, useEffect, useCallback } from "react";
import { resources } from "@/lib/mcp";
import { onNatsMessage } from "@/lib/nats";
import type { AgentRun } from "@/types/swarm";

const SLOW_POLL_MS = 30_000;

export function useLiveAgents() {
  const [agents, setAgents] = useState<AgentRun[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const fetchLive = useCallback(async () => {
    try {
      setAgents(await resources.live());
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchLive();
    const interval = setInterval(fetchLive, SLOW_POLL_MS);
    const unsub = onNatsMessage((subject) => {
      if (subject.includes("agent") || subject.includes("progress")) {
        setTimeout(fetchLive, 500);
      }
    });
    return () => { clearInterval(interval); unsub(); };
  }, [fetchLive]);

  return { agents, loading, error };
}
