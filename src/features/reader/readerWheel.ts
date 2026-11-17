const WHEEL_GESTURE_GAP_MS = 180;
const PAGE_TURN_THRESHOLD = 70;

/** Allow one boundary page turn per wheel or trackpad gesture. */
export class ReaderWheelGate {
  private lastEvent = -Infinity;
  private accumulated = 0;
  private blocked = false;

  block() {
    this.blocked = true;
    this.accumulated = 0;
  }

  shouldTurn(delta: number, atBoundary: boolean, now = performance.now()) {
    if (now - this.lastEvent > WHEEL_GESTURE_GAP_MS) {
      this.blocked = false;
      this.accumulated = 0;
    }
    this.lastEvent = now;
    if (!atBoundary || this.blocked || !Number.isFinite(delta)) {
      this.accumulated = 0;
      return false;
    }
    if (Math.sign(delta) !== Math.sign(this.accumulated)) {
      this.accumulated = 0;
    }
    this.accumulated += Math.sign(delta) * Math.min(40, Math.abs(delta));
    if (Math.abs(this.accumulated) < PAGE_TURN_THRESHOLD) return false;
    this.block();
    return true;
  }
}
