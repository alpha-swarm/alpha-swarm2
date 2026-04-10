import { useState, useEffect } from "react";
import { resources } from "../lib/mcp";
import type { DashboardStats } from "../types/swarm";

const REFRESH_INTERVAL_MS = 10_000;

export function useDashboard() {
  const [stats, setStats] = useState<DashboardStats | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const fetch = async () => {
      try {
        const data = await resources.dashboard();
        const parsed = Array.isArray(data) ? data[0]?.result?.[0] : data?.[0]?.result?.[0];
        setStats(parsed ?? null);
        setError(null);
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        setLoading(false);
      }
    };

    fetch();
    const interval = setInterval(fetch, REFRESH_INTERVAL_MS);
    return () => clearInterval(interval);
  }, []);

  return { stats, loading, error };
}
