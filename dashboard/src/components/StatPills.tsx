import type { DashboardStats } from "@/types/swarm";

function Pill({ label, value, color }: { label: string; value: number; color: string }) {
  return (
    <span className="text-xs">
      <span className="text-muted-foreground/60">{label} </span>
      <span className={`font-mono font-semibold tabular-nums ${color}`}>{value}</span>
    </span>
  );
}

function Dot() {
  return <span className="text-border/40 mx-1.5">&middot;</span>;
}

export function StatPills({ stats }: { stats: DashboardStats }) {
  return (
    <div className="flex items-center flex-wrap">
      <Pill label="Active" value={stats.active} color="text-amber-400" />
      <Dot />
      <Pill label="Passed" value={stats.passed} color="text-emerald-400" />
      <Dot />
      <Pill label="Failed" value={stats.failed} color="text-red-400" />
      <Dot />
      <Pill label="Pending" value={stats.pending} color="text-muted-foreground" />
      <Dot />
      <Pill label="Total" value={stats.total_runs} color="text-foreground" />
    </div>
  );
}
