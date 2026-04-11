import { useNavigate } from "react-router";
import { Button } from "@/components/ui/button";

function ConnectionDot({ connected }: { connected: boolean }) {
  const color = connected ? "bg-emerald-400" : "bg-red-400";
  return <div className={`w-1.5 h-1.5 rounded-full ${color}`} />;
}

export function TopBar({ connected }: { connected: boolean }) {
  const navigate = useNavigate();
  return (
    <header className="flex items-center justify-between px-4 py-2.5 border-b bg-card/50">
      <span className="text-sm font-bold tracking-tight">alpha-swarm</span>
      <div className="flex items-center gap-3">
        <Button size="sm" variant="outline" className="h-7 text-xs" onClick={() => navigate("/submit")}>+ New Task</Button>
        <ConnectionDot connected={connected} />
      </div>
    </header>
  );
}
