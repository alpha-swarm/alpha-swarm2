import { connect, type NatsConnection, type Subscription, StringCodec } from "nats.ws";

const NATS_WS_URL = "ws://localhost:4226";
const sc = StringCodec();

let conn: NatsConnection | null = null;
let connecting = false;
const listeners: Set<(subject: string, data: unknown) => void> = new Set();

async function ensureConnection(): Promise<NatsConnection> {
  if (conn) return conn;
  if (connecting) {
    await new Promise((r) => setTimeout(r, 500));
    return ensureConnection();
  }
  connecting = true;
  try {
    conn = await connect({ servers: NATS_WS_URL });
    conn.closed().then(() => { conn = null; });
    return conn;
  } finally {
    connecting = false;
  }
}

export function onNatsMessage(fn: (subject: string, data: unknown) => void) {
  listeners.add(fn);
  return () => { listeners.delete(fn); };
}

function dispatch(subject: string, data: unknown) {
  for (const fn of listeners) fn(subject, data);
}

let globalSub: Subscription | null = null;

export async function startSubscription() {
  const nc = await ensureConnection();
  if (globalSub) return;
  globalSub = nc.subscribe("alpha-swarm.>");
  (async () => {
    for await (const msg of globalSub) {
      try {
        const data = JSON.parse(sc.decode(msg.data));
        dispatch(msg.subject, data);
      } catch { /* ignore non-JSON */ }
    }
  })();
}

export async function isConnected(): Promise<boolean> {
  return conn !== null && !conn.isClosed();
}
