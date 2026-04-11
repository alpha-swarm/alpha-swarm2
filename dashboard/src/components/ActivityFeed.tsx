import { Badge } from "@/components/ui/badge";
import type { StreamEvent } from "@/hooks/useEventStream";

const EVENT_COLORS: Record<string, string> = {
  agent_started: "bg-blue-500",
  agent_finished: "bg-green-500",
  agent_failed: "bg-red-500",
  status: "bg-gray-500",
};

function EventDot({ type }: { type: string }) {
  return <div className={`w-2 h-2 rounded-full ${EVENT_COLORS[type] ?? "bg-gray-400"}`} />;
}

function EventRow({ event }: { event: StreamEvent }) {
  const age = Math.round((Date.now() - event.timestamp) / 1000);
  const label = String(event.data.task ?? event.data.message ?? event.type).slice(0, 80);
  return (
    <div className="flex items-center gap-2 py-1.5 text-xs">
      <EventDot type={event.type} />
      <Badge variant="outline" className="text-[9px] shrink-0">{event.type}</Badge>
      <span className="truncate flex-1 text-muted-foreground">{label}</span>
      <span className="text-[10px] text-muted-foreground shrink-0">{age}s ago</span>
    </div>
  );
}

export function ActivityFeed({ events, connected }: { events: StreamEvent[]; connected: boolean }) {
  if (events.length === 0) {
    return <p className="text-xs text-muted-foreground">No events yet. {connected ? "Connected." : "Connecting..."}</p>;
  }
  return (
    <div>
      <div className="flex items-center gap-2 mb-2">
        <h3 className="text-sm font-semibold">Live Activity</h3>
        <div className={`w-1.5 h-1.5 rounded-full ${connected ? "bg-green-500" : "bg-red-500"}`} />
      </div>
      <div className="max-h-48 overflow-auto space-y-0.5">
        {events.slice(0, 20).map((e, i) => <EventRow key={i} event={e} />)}
      </div>
    </div>
  );
}
