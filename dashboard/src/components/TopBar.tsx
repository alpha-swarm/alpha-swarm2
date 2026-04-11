import { Link, useNavigate } from "react-router";
import { Button } from "@/components/ui/button";

function ConnectionDot({ connected }: { connected: boolean }) {
  const color = connected ? "bg-emerald-400" : "bg-red-400";
  return <div className={`w-1.5 h-1.5 rounded-full ${color}`} title={connected ? "Connected" : "Disconnected"} />;
}

function Logo() {
  return (
    <Link to="/" className="flex items-center gap-2 hover:opacity-80 transition-opacity">
      <span className="text-sm font-bold tracking-tight">alpha-swarm</span>
    </Link>
  );
}

interface TopBarProps {
  connected: boolean;
}

export function TopBar({ connected }: TopBarProps) {
  const navigate = useNavigate();
  return (
    <header className="flex items-center justify-between px-4 py-2.5 border-b bg-card/50">
      <Logo />
      <nav className="flex items-center gap-4 text-xs">
        <Link to="/projects" className="text-muted-foreground hover:text-foreground transition-colors">Projects</Link>
        <Button size="sm" variant="outline" className="h-7 text-xs" onClick={() => navigate("/submit")}>+ New Task</Button>
        <ConnectionDot connected={connected} />
      </nav>
    </header>
  );
}
