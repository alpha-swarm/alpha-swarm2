import { useState, useEffect, useCallback } from "react";

export interface StreamEvent {
  type: string;
  data: Record<string, unknown>;
  timestamp: number;
}

const MAX_EVENTS = 100;
const RECONNECT_MS = 3_000;

export function useEventStream() {
  const [events, setEvents] = useState<StreamEvent[]>([]);
  const [connected, setConnected] = useState(false);

  const addEvent = useCallback((type: string, data: Record<string, unknown>) => {
    setEvents((prev) => {
      const next = [{ type, data, timestamp: Date.now() }, ...prev];
      return next.length > MAX_EVENTS ? next.slice(0, MAX_EVENTS) : next;
    });
  }, []);

  useEffect(() => {
    let es: EventSource | null = null;
    let reconnectTimer: ReturnType<typeof setTimeout>;

    function connect() {
      es = new EventSource("/api/events");
      es.onopen = () => setConnected(true);
      es.onerror = () => {
        setConnected(false);
        es?.close();
        reconnectTimer = setTimeout(connect, RECONNECT_MS);
      };
      es.addEventListener("status", (e) => addEvent("status", JSON.parse(e.data)));
      es.addEventListener("agent_started", (e) => addEvent("agent_started", JSON.parse(e.data)));
      es.addEventListener("agent_finished", (e) => addEvent("agent_finished", JSON.parse(e.data)));
      es.addEventListener("agent_failed", (e) => addEvent("agent_failed", JSON.parse(e.data)));
    }

    connect();
    return () => { es?.close(); clearTimeout(reconnectTimer); };
  }, [addEvent]);

  return { events, connected };
}
