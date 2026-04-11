import { Badge } from "@/components/ui/badge";
import type { NatsEvent } from "@/hooks/useNatsEvents";

function eventType(subject: string) {
  const parts = subject.split(".");
  return parts.slice(2).join(".");
}

function EventDot({ subject }: { subject: string }) {
  const type = eventType(subject);
  const color = type.includes("started") ? "bg-blue-400" : type.includes("finished") ? "bg-emerald-400" : type.includes("failed") ? "bg-red-400" : type.includes("progress") ? "bg-amber-400" : "bg-gray-400";
  return <div className={`w-1.5 h-1.5 rounded-full shrink-0 ${color}`} />;
}

function EventRow({ event }: { event: NatsEvent }) {
  const age = Math.round((Date.now() - event.timestamp) / 1000);
  const type = eventType(event.subject);
  const label = String(event.data.task ?? event.data.action ?? event.data.goal ?? type).slice(0, 60);
  return (
    <div className="flex items-center gap-1.5 py-0.5 text-[10px]">
      <EventDot subject={event.subject} />
      <Badge variant="outline" className="text-[8px] px-1 py-0 h-3.5 shrink-0">{type}</Badge>
      <span className="truncate flex-1 text-muted-foreground/60">{label}</span>
      <span className="text-muted-foreground/30 shrink-0 tabular-nums">{age}s</span>
    </div>
  );
}

export function ActivityFeed({ events }: { events: NatsEvent[] }) {
  if (events.length === 0) return null;
  return (
    <div className="max-h-32 overflow-auto space-y-0">
      {events.slice(0, 30).map((e, i) => <EventRow key={i} event={e} />)}
    </div>
  );
}
