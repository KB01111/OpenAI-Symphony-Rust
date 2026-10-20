/**
 * Coalesces high-frequency items (agent stream events) into batches delivered at most once per
 * `intervalMs`, so React state updates and cache invalidations happen per frame-ish window rather
 * than per event. While `slowMode` is on (window hidden), the interval stretches to `hiddenIntervalMs`.
 */
export class Batcher<T> {
  private queue: T[] = [];
  private timer: ReturnType<typeof setTimeout> | null = null;
  private slow = false;

  constructor(
    private readonly flush: (items: T[]) => void,
    private readonly intervalMs = 80,
    private readonly hiddenIntervalMs = 2_000
  ) {}

  push(item: T): void {
    this.queue.push(item);
    if (!this.timer) {
      this.timer = setTimeout(() => this.drain(), this.slow ? this.hiddenIntervalMs : this.intervalMs);
    }
  }

  setSlowMode(slow: boolean): void {
    this.slow = slow;
    if (!slow && this.timer) {
      clearTimeout(this.timer);
      this.timer = null;
      this.drain();
    }
  }

  drain(): void {
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
    if (this.queue.length === 0) return;
    const items = this.queue;
    this.queue = [];
    this.flush(items);
  }

  dispose(): void {
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
    this.queue = [];
  }
}
