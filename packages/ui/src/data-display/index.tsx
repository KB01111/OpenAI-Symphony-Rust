import React, { useEffect, useMemo, useRef, useState } from "react";
import { Tooltip } from "@fluentui/react-components";
import { useVirtualizer } from "@tanstack/react-virtual";
import { formatMessage, getTranslations, type HardwareSnapshot } from "@symphony/shared-types";
import { BatteryCharging, BatteryMedium, Cpu, Flame, MemoryStick, Plug } from "lucide-react";
import { cn } from "../lib/utils.js";
import { useDesign } from "../providers/SymphonyDesignProvider.js";
import { useUiStrings } from "../i18n.js";

export interface MetricTileProps {
  label: string;
  value: React.ReactNode;
  hint?: React.ReactNode;
  tone?: "neutral" | "accent" | "success" | "warning" | "danger";
  icon?: React.ReactNode;
  onClick?: () => void;
}

const TONE_TEXT = {
  neutral: "text-foreground",
  accent: "text-accent",
  success: "text-success",
  warning: "text-warning",
  danger: "text-destructive"
} as const;

export const MetricTile: React.FC<MetricTileProps> = ({ label, value, hint, tone = "neutral", icon, onClick }) => {
  const Comp = onClick ? "button" : "div";
  return (
    <Comp
      onClick={onClick}
      className={cn(
        "flex min-w-0 flex-col gap-1 rounded-md border border-border bg-surface px-3 py-2.5 text-start",
        onClick && "transition-colors hover:bg-surface-hover"
      )}
    >
      <span className="flex items-center gap-1.5 text-xs text-muted-foreground">
        {icon}
        {label}
      </span>
      <span className={cn("font-display text-xl font-semibold tabular-nums", TONE_TEXT[tone])}>{value}</span>
      {hint ? <span className="truncate text-xs text-subtle-foreground">{hint}</span> : null}
    </Comp>
  );
};

export const KeyValueList: React.FC<{ items: ReadonlyArray<{ label: string; value: React.ReactNode; mono?: boolean }>; className?: string }> = ({
  items,
  className
}) => (
  // Side by side when there is room; in a narrow panel the label sits above its value, so long
  // values (paths, branches, model ids) keep a readable width instead of wrapping per character.
  <div className={cn("@container", className)}>
    <dl className="grid grid-cols-1 gap-x-3 gap-y-0.5 text-sm @[17rem]:grid-cols-[minmax(96px,max-content)_minmax(0,1fr)] @[17rem]:gap-y-1.5">
      {items.map((item) => (
        <React.Fragment key={item.label}>
          <dt className="text-xs text-muted-foreground @[17rem]:text-sm">{item.label}</dt>
          <dd className={cn("mb-1.5 min-w-0 [overflow-wrap:anywhere] text-foreground @[17rem]:mb-0", item.mono && "font-mono text-xs leading-5")}>
            {item.value ?? "—"}
          </dd>
        </React.Fragment>
      ))}
    </dl>
  </div>
);

function relativeTime(iso: string, now: number, locale: ReturnType<typeof getTranslations>): string {
  const seconds = Math.max(0, Math.round((now - new Date(iso).getTime()) / 1000));
  if (seconds < 5) return locale.common.justNow;
  if (seconds < 60) return formatMessage(locale.common.secondsAgo, { count: seconds });
  if (seconds < 3600) return formatMessage(locale.common.minutesAgo, { count: Math.floor(seconds / 60) });
  if (seconds < 86400) return formatMessage(locale.common.hoursAgo, { count: Math.floor(seconds / 3600) });
  return formatMessage(locale.common.daysAgo, { count: Math.floor(seconds / 86400) });
}

/** Shared 15 s clock so every relative timestamp updates together without per-item timers. */
const clockListeners = new Set<(now: number) => void>();
let clockTimer: ReturnType<typeof setInterval> | null = null;
function subscribeClock(fn: (now: number) => void) {
  clockListeners.add(fn);
  if (!clockTimer) clockTimer = setInterval(() => clockListeners.forEach((l) => l(Date.now())), 15_000);
  return () => {
    clockListeners.delete(fn);
    if (clockListeners.size === 0 && clockTimer) {
      clearInterval(clockTimer);
      clockTimer = null;
    }
  };
}

export function useNow(): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => subscribeClock(setNow), []);
  return now;
}

export const TimeAgo: React.FC<{ iso: string | null | undefined; className?: string }> = ({ iso, className }) => {
  const { locale } = useDesign();
  const now = useNow();
  if (!iso) return <span className={className}>—</span>;
  return (
    <time dateTime={iso} title={new Date(iso).toLocaleString(locale)} className={className}>
      {relativeTime(iso, now, getTranslations(locale))}
    </time>
  );
};

export function formatDuration(seconds: number, locale: ReturnType<typeof getTranslations>): string {
  const s = Math.max(0, Math.round(seconds));
  if (s < 60) return formatMessage(locale.common.durationSeconds, { count: s });
  if (s < 3600) return formatMessage(locale.common.durationMinutes, { minutes: Math.floor(s / 60), seconds: s % 60 });
  return formatMessage(locale.common.durationHours, { hours: Math.floor(s / 3600), minutes: Math.floor((s % 3600) / 60) });
}

/** Live duration since `since` (or a fixed number of seconds). */
export const Duration: React.FC<{ since?: string | null; seconds?: number; className?: string }> = ({ since, seconds, className }) => {
  const { locale } = useDesign();
  const now = useNow();
  const value = since ? (now - new Date(since).getTime()) / 1000 : (seconds ?? 0);
  return <span className={cn("tabular-nums", className)}>{formatDuration(value, getTranslations(locale))}</span>;
};

export function formatTokens(n: number, locale: string): string {
  return new Intl.NumberFormat(locale, { notation: n >= 100_000 ? "compact" : "standard" }).format(n);
}

export function formatCurrency(usd: number, locale: string): string {
  return new Intl.NumberFormat(locale, { style: "currency", currency: "USD", maximumFractionDigits: usd < 1 ? 4 : 2 }).format(usd);
}

export interface VirtualListProps<T> {
  items: ReadonlyArray<T>;
  getKey: (item: T, index: number) => string;
  renderItem: (item: T, index: number) => React.ReactNode;
  estimateSize?: number;
  className?: string;
  /** Keep the view pinned to the end while the user has not scrolled up (live logs). */
  followTail?: boolean;
  onFollowChange?: (following: boolean) => void;
  ariaLabel?: string;
  overscan?: number;
}

/**
 * Virtualized vertical list (TanStack Virtual) with optional tail-following: new items scroll into
 * view only while the operator is at the bottom; scrolling up pauses following.
 */
export function VirtualList<T>({
  items,
  getKey,
  renderItem,
  estimateSize = 56,
  className,
  followTail = false,
  onFollowChange,
  ariaLabel,
  overscan = 8
}: VirtualListProps<T>) {
  const parentRef = useRef<HTMLDivElement>(null);
  const [following, setFollowing] = useState(true);
  const virtualizer = useVirtualizer({
    count: items.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => estimateSize,
    overscan,
    getItemKey: (index) => getKey(items[index] as T, index)
  });

  useEffect(() => {
    if (followTail && following && items.length > 0) {
      virtualizer.scrollToIndex(items.length - 1, { align: "end" });
    }
  }, [items.length, followTail, following, virtualizer]);

  const onScroll = () => {
    const el = parentRef.current;
    if (!el || !followTail) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
    if (atBottom !== following) {
      setFollowing(atBottom);
      onFollowChange?.(atBottom);
    }
  };

  return (
    <div ref={parentRef} onScroll={onScroll} className={cn("relative overflow-auto", className)} role="list" aria-label={ariaLabel}>
      <div style={{ height: virtualizer.getTotalSize(), position: "relative", width: "100%" }}>
        {virtualizer.getVirtualItems().map((row) => (
          <div
            key={row.key}
            role="listitem"
            data-index={row.index}
            ref={virtualizer.measureElement}
            style={{ position: "absolute", top: 0, left: 0, width: "100%", transform: `translateY(${row.start}px)` }}
          >
            {renderItem(items[row.index] as T, row.index)}
          </div>
        ))}
      </div>
    </div>
  );
}

/** Compact host telemetry for the status bar. Data comes from the runtime snapshot only. */
export const HardwareTelemetryStrip: React.FC<{ hardware: HardwareSnapshot; className?: string }> = ({ hardware, className }) => {
  const { ui } = useUiStrings();
  const memory = useMemo(
    () => `${Math.round(hardware.available_memory_mb / 1024)} / ${Math.round(hardware.total_memory_mb / 1024)} GB`,
    [hardware.available_memory_mb, hardware.total_memory_mb]
  );
  const chip = "inline-flex items-center gap-1 px-1.5 tabular-nums";
  return (
    <div className={cn("flex items-center gap-2 text-xs text-muted-foreground", className)}>
      <Tooltip content={hardware.cpu_brand || ui.cpu} relationship="description">
        <span className={chip}>
          <Cpu className="size-3.5" aria-hidden />
          {ui.cpu} {hardware.cpu_usage_percent.toFixed(0)}%
        </span>
      </Tooltip>
      <Tooltip content={memory} relationship="description">
        <span className={cn(chip, hardware.memory_usage_percent > 85 && "text-warning")}>
          <MemoryStick className="size-3.5" aria-hidden />
          {ui.ram} {hardware.memory_usage_percent.toFixed(0)}%
        </span>
      </Tooltip>
      <span className={chip}>
        {hardware.on_battery ? <BatteryMedium className="size-3.5" aria-hidden /> : hardware.battery_percent != null ? <BatteryCharging className="size-3.5" aria-hidden /> : <Plug className="size-3.5" aria-hidden />}
        {hardware.on_battery
          ? hardware.battery_percent != null
            ? formatMessage(ui.battery, { percent: hardware.battery_percent })
            : ui.batteryUnknown
          : ui.acPower}
      </span>
      {hardware.thermal_throttling ? (
        <span className={cn(chip, "text-warning")}>
          <Flame className="size-3.5" aria-hidden />
          {ui.throttled}
        </span>
      ) : null}
      <Tooltip content={hardware.dispatch_delayed_reason ?? ui.maxAgents.replace("{count}", String(hardware.recommended_max_concurrency))} relationship="description">
        <span className={chip}>{formatMessage(ui.maxAgents, { count: hardware.effective_max_concurrency })}</span>
      </Tooltip>
    </div>
  );
};
