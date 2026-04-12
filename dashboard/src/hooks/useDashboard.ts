import { useState, useEffect, useCallback } from "react";
import { resources } from "@/lib/mcp";
import { onNatsMessage } from "@/lib/nats";
import type { DashboardStats } from "@/types/swarm";

const SLOW_POLL_MS = 30_000;

export function useDashboard() {
  const [stats, setStats] = useState<DashboardStats | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const fetchStats = useCallback(async () => {
    try {
      const data = await resources.dashboard();
      setStats(data[0] ?? null);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchStats();
    const interval = setInterval(fetchStats, SLOW_POLL_MS);
    const unsub = onNatsMessage(() => setTimeout(fetchStats, 1_000));
    return () => { clearInterval(interval); unsub(); };
  }, [fetchStats]);

  return { stats, loading, error };
}
