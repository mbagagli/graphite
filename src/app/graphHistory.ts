/** Own bounded session history and reject superseded exploration results. */
export class GraphHistory<T> {
  private entries: T[] = [];
  private serial = 0;
  get depth() {
    return this.entries.length;
  }
  begin() {
    return ++this.serial;
  }
  isCurrent(serial: number) {
    return serial === this.serial;
  }
  commit(serial: number, previous: T) {
    if (!this.isCurrent(serial)) return false;
    this.entries = [...this.entries.slice(-19), previous];
    return true;
  }
  back() {
    this.begin();
    return this.entries.pop();
  }
  clear() {
    this.begin();
    this.entries = [];
  }
}
