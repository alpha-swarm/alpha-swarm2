import type { AttemptRecord } from "@/types/swarm";

interface AttemptRowProps {
  attempt: AttemptRecord;
  compact?: boolean;
}

function QualityGateBadge({ passed }: { passed: boolean }) {
  return (
    <span className={passed ? "text-green-500" : "text-destructive"}>
      QG: {passed ? "pass" : "fail"}
    </span>
  );
}

export function AttemptRow({ attempt: a, compact }: AttemptRowProps) {
  const padding = compact ? "p-2 mb-1" : "p-3 mb-2";

  return (
    <div className={`text-xs border rounded ${padding}`}>
      <div className={`flex ${compact ? "gap-2" : "gap-3"} text-muted-foreground ${compact ? "" : "mb-1"}`}>
        <span className={compact ? "" : "font-semibold"}>#{a.attempt}</span>
        <span className="font-mono">{a.model}</span>
        <span>{(a.duration_ms / 1000).toFixed(1)}s</span>
        <span>{a.tokens_input}in{compact ? "/" : " / "}{a.tokens_output}out</span>
        {a.quality_passed !== null && <QualityGateBadge passed={a.quality_passed} />}
      </div>
      {!compact && a.prompt_preview && (
        <details className="mt-1">
          <summary className="text-muted-foreground cursor-pointer">Prompt preview</summary>
          <pre className="bg-muted/50 rounded p-2 mt-1 whitespace-pre-wrap font-mono">{a.prompt_preview}</pre>
        </details>
      )}
      {!compact && a.response_preview && (
        <details className="mt-1">
          <summary className="text-muted-foreground cursor-pointer">Response preview</summary>
          <pre className="bg-muted/50 rounded p-2 mt-1 whitespace-pre-wrap font-mono">{a.response_preview}</pre>
        </details>
      )}
      {a.error && <div className="text-destructive mt-1">{a.error}</div>}
    </div>
  );
}
