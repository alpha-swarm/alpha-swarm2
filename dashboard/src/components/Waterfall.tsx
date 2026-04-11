import { useState } from "react";
import type { PhaseTimingRecord } from "@/types/swarm";

interface WaterfallPhase {
  label: string;
  duration_ms: number;
  offset_ms: number;
  color: string;
}

const PHASE_COLORS: Record<string, string> = {
  embedding: "#8b5cf6",
  rag: "#3b82f6",
  planning: "#f59e0b",
  agent_execution: "#10b981",
  quality_gate: "#ef4444",
};

const PHASE_ORDER = ["embedding", "rag", "planning", "agent_execution", "quality_gate"];

const MIN_BAR_WIDTH_PCT = 0.5;
const LABEL_THRESHOLD_PCT = 15;

function buildPhases(timings: PhaseTimingRecord): WaterfallPhase[] {
  const phases: WaterfallPhase[] = [];
  let offset = 0;

  for (const key of PHASE_ORDER) {
    const ms = (timings as unknown as Record<string, number>)[`${key}_ms`] ?? 0;
    if (ms > 0) {
      phases.push({
        label: key.replace("_", " "),
        duration_ms: ms,
        offset_ms: offset,
        color: PHASE_COLORS[key] ?? "#6b7280",
      });
    }
    offset += ms;
  }

  return phases;
}

function WaterfallBar({ phase, total, isHovered, onHover, onLeave }: {
  phase: WaterfallPhase;
  total: number;
  isHovered: boolean;
  onHover: () => void;
  onLeave: () => void;
}) {
  const leftPct = (phase.offset_ms / total) * 100;
  const widthPct = Math.max((phase.duration_ms / total) * 100, MIN_BAR_WIDTH_PCT);

  return (
    <div
      className="relative h-7 group"
      onMouseEnter={onHover}
      onMouseLeave={onLeave}
    >
      <div className="absolute inset-0 rounded bg-muted/30" />
      <div
        className="absolute top-0 h-full rounded transition-opacity"
        style={{
          left: `${leftPct}%`,
          width: `${widthPct}%`,
          backgroundColor: phase.color,
          opacity: isHovered ? 1 : 0.8,
        }}
      />
      <div className="absolute inset-0 flex items-center px-2 text-[11px] font-medium pointer-events-none">
        <span
          className="truncate"
          style={{
            marginLeft: `${leftPct}%`,
            color: widthPct > LABEL_THRESHOLD_PCT ? "#fff" : "var(--foreground)",
            textShadow: widthPct > LABEL_THRESHOLD_PCT ? "0 1px 2px rgba(0,0,0,0.3)" : "none",
          }}
        >
          {phase.label}
        </span>
      </div>
    </div>
  );
}

function WaterfallTooltip({ phase, total }: { phase: WaterfallPhase; total: number }) {
  return (
    <div className="flex items-center gap-3 text-xs bg-popover border rounded-md px-3 py-2 shadow-sm">
      <div className="w-2.5 h-2.5 rounded-sm" style={{ backgroundColor: phase.color }} />
      <span className="font-medium">{phase.label}</span>
      <span className="text-muted-foreground">
        {(phase.duration_ms / 1000).toFixed(2)}s
      </span>
      <span className="text-muted-foreground">
        ({((phase.duration_ms / total) * 100).toFixed(0)}%)
      </span>
      <span className="text-muted-foreground ml-auto">
        starts at {(phase.offset_ms / 1000).toFixed(1)}s
      </span>
    </div>
  );
}

interface WaterfallProps {
  timings: PhaseTimingRecord;
  totalMs?: number;
}

export function Waterfall({ timings, totalMs }: WaterfallProps) {
  const [hovered, setHovered] = useState<WaterfallPhase | null>(null);
  const phases = buildPhases(timings);
  const total = totalMs ?? phases.reduce((s, p) => s + p.duration_ms, 0);

  if (total === 0) return null;

  return (
    <div className="space-y-1.5 w-full overflow-hidden">
      <div className="flex items-center justify-between text-xs text-muted-foreground">
        <span>0s</span>
        <span>{(total / 1000).toFixed(1)}s total</span>
      </div>
      <div className="space-y-1">
        {phases.map((phase) => (
          <WaterfallBar
            key={phase.label}
            phase={phase}
            total={total}
            isHovered={hovered === phase}
            onHover={() => setHovered(phase)}
            onLeave={() => setHovered(null)}
          />
        ))}
      </div>
      {hovered && <WaterfallTooltip phase={hovered} total={total} />}
    </div>
  );
}
