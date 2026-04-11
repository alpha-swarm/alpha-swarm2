import { Link } from "react-router";

function ConnectionDot({ connected }: { connected: boolean }) {
  const color = connected ? "bg-emerald-400" : "bg-red-400";
  return <div className={`w-1.5 h-1.5 rounded-full ${color}`} />;
}

export function TopBar({ connected }: { connected: boolean }) {
  return (
    <header className="flex items-center justify-between px-4 py-2.5 border-b bg-card/50">
      <Link to="/" className="text-sm font-bold tracking-tight hover:opacity-80">alpha-swarm</Link>
      <ConnectionDot connected={connected} />
    </header>
  );
}
