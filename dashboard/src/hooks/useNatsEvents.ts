import { useState, useEffect } from "react";
import { startSubscription, onNatsMessage, isConnected } from "@/lib/nats";

export interface NatsEvent {
  subject: string;
  data: Record<string, unknown>;
  timestamp: number;
}

const MAX_EVENTS = 200;

export function useNatsEvents() {
  const [events, setEvents] = useState<NatsEvent[]>([]);
  const [connected, setConnected] = useState(false);

  useEffect(() => {
    startSubscription().then(() => setConnected(true)).catch(() => setConnected(false));
    const checkConn = setInterval(() => { isConnected().then(setConnected); }, 5_000);
    const unsub = onNatsMessage((subject, data) => {
      setEvents((prev) => {
        const next = [{ subject, data: data as Record<string, unknown>, timestamp: Date.now() }, ...prev];
        return next.length > MAX_EVENTS ? next.slice(0, MAX_EVENTS) : next;
      });
    });
    return () => { unsub(); clearInterval(checkConn); };
  }, []);

  return { events, connected };
}
