import { useState, useEffect } from "react";
import { resources } from "@/lib/mcp";
import type { AgentRun } from "@/types/swarm";

const REFRESH_INTERVAL_MS = 3_000;

export function useLiveAgents() {
  const [agents, setAgents] = useState<AgentRun[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const fetchLive = async () => {
      try {
        setAgents(await resources.live());
        setError(null);
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        setLoading(false);
      }
    };
    fetchLive();
    const interval = setInterval(fetchLive, REFRESH_INTERVAL_MS);
    return () => clearInterval(interval);
  }, []);

  return { agents, loading, error };
}
