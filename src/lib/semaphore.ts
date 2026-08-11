/**
 * Minimal async semaphore — caps how many jobs run concurrently.
 * Used to keep parallel Sharp/exifr work from saturating CPU and memory
 * when a large uncached album is hit by its first visitor.
 */
export class Semaphore {
  private queue: Array<() => void> = [];
  private active = 0;

  constructor(private readonly limit: number) {}

  private acquire(): Promise<() => void> {
    return new Promise((resolve) => {
      const tryAcquire = () => {
        if (this.active < this.limit) {
          this.active++;
          let released = false;
          resolve(() => {
            if (released) return;
            released = true;
            this.active--;
            const next = this.queue.shift();
            if (next) next();
          });
        } else {
          this.queue.push(tryAcquire);
        }
      };
      tryAcquire();
    });
  }

  async run<T>(fn: () => Promise<T>): Promise<T> {
    const release = await this.acquire();
    try {
      return await fn();
    } finally {
      release();
    }
  }
}

/** Shared cap for image-processing jobs (thumbnails, metadata, blur previews). */
export const imageJobSemaphore = new Semaphore(4);
