import React, { useMemo, useState } from "react";
import { ChevronDown, ChevronRight, FileDiff } from "lucide-react";
import { VirtualList, cn } from "@symphony/ui";
import type { ChangedFileSummary } from "@symphony/shared-types";

export interface DiffFile {
  path: string;
  lines: string[];
  additions: number;
  deletions: number;
}

/** Split a unified diff (git format) into per-file sections. */
export function parseUnifiedDiff(diff: string): DiffFile[] {
  const files: DiffFile[] = [];
  let current: DiffFile | null = null;
  for (const line of diff.split("\n")) {
    if (line.startsWith("diff --git ")) {
      const match = /diff --git a\/(.+?) b\/(.+)$/.exec(line);
      current = { path: match?.[2] ?? line.slice(11), lines: [], additions: 0, deletions: 0 };
      files.push(current);
      continue;
    }
    if (!current) continue;
    if (line.startsWith("+++ ") || line.startsWith("--- ") || line.startsWith("index ") || line.startsWith("new file mode") || line.startsWith("deleted file mode")) {
      if (line.startsWith("+++ ") && line !== "+++ /dev/null") current.path = line.replace(/^\+\+\+ (b\/)?/, "");
      continue;
    }
    if (line.startsWith("+")) current.additions += 1;
    else if (line.startsWith("-")) current.deletions += 1;
    current.lines.push(line);
  }
  return files;
}

const LINE_CLASS = (line: string) =>
  line.startsWith("@@")
    ? "bg-info-subtle text-info"
    : line.startsWith("+")
      ? "bg-diff-add text-foreground"
      : line.startsWith("-")
        ? "bg-diff-remove text-foreground"
        : "text-muted-foreground";

const DiffFileSection: React.FC<{ file: DiffFile; defaultOpen: boolean; maxHeight: number }> = ({ file, defaultOpen, maxHeight }) => {
  const [open, setOpen] = useState(defaultOpen);
  return (
    <section className="overflow-hidden rounded-md border border-border">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        className="flex w-full items-center gap-2 bg-surface-sunken px-2 py-1.5 text-start text-xs hover:bg-surface-hover"
      >
        {open ? <ChevronDown className="size-3.5" aria-hidden /> : <ChevronRight className="size-3.5" aria-hidden />}
        <FileDiff className="size-3.5 text-muted-foreground" aria-hidden />
        <span className="min-w-0 flex-1 truncate font-mono text-foreground">{file.path}</span>
        <span className="font-mono text-success">+{file.additions}</span>
        <span className="font-mono text-destructive">−{file.deletions}</span>
      </button>
      {open ? (
        // Large files are windowed: only visible lines are rendered.
        <VirtualList
          items={file.lines}
          getKey={(_, i) => String(i)}
          estimateSize={18}
          className={cn("bg-code font-mono text-xs leading-[18px]", maxHeight === 480 ? "max-h-[480px]" : "max-h-[320px]")}
          ariaLabel={file.path}
          renderItem={(line, i) => (
            <div className={cn("flex whitespace-pre px-2", LINE_CLASS(line))}>
              <span className="w-10 shrink-0 select-none pe-2 text-end text-subtle-foreground">{i + 1}</span>
              <span>{line || " "}</span>
            </div>
          )}
        />
      ) : null}
    </section>
  );
};

export interface DiffViewerProps {
  diff: string;
  /** Optional summaries (status/line counts) from the runtime for files without diff text. */
  files?: ReadonlyArray<ChangedFileSummary>;
  truncated?: boolean;
  truncatedLabel?: string;
  emptyLabel: string;
  /** Files expanded initially (others start collapsed to keep large diffs cheap). */
  expandFirst?: number;
  className?: string;
}

/** Unified diff viewer with per-file collapsing and virtualized line rendering. */
export const DiffViewer: React.FC<DiffViewerProps> = ({ diff, truncated, truncatedLabel, emptyLabel, expandFirst = 3, className }) => {
  const files = useMemo(() => parseUnifiedDiff(diff), [diff]);
  if (files.length === 0) return <p className="p-3 text-sm text-muted-foreground">{emptyLabel}</p>;
  return (
    <div className={cn("space-y-2", className)}>
      {files.map((file, i) => (
        <DiffFileSection key={`${file.path}-${i}`} file={file} defaultOpen={i < expandFirst} maxHeight={480} />
      ))}
      {truncated && truncatedLabel ? <p className="text-xs text-warning">{truncatedLabel}</p> : null}
    </div>
  );
};
