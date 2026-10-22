export interface LogEntry {
  id: number;
  timestamp: string;
  level: "info" | "warn" | "error" | "debug";
  category: "AUDIO" | "IPC" | "UI" | "SYSTEM" | "PERF";
  message: string;
  details?: unknown;
}

class DebugLogger {
  private logs: LogEntry[] = [];
  private listeners: Set<() => void> = new Set();
  private nextId = 1;
  private maxLogs = 500;
  private _enabled = true;

  constructor() {
    try {
      const saved = localStorage.getItem("podloader_debug_enabled");
      if (saved !== null) {
        this._enabled = saved === "true";
      }
    } catch {
      // LocalStorage unavailable in certain test environments
    }
  }

  public get enabled(): boolean {
    return this._enabled;
  }

  public setEnabled(val: boolean) {
    this._enabled = val;
    try {
      localStorage.setItem("podloader_debug_enabled", String(val));
    } catch {}
    this.emitChange();
  }

  public getLogs(): LogEntry[] {
    return this.logs;
  }

  public clear() {
    this.logs = [];
    this.emitChange();
  }

  public log(
    level: LogEntry["level"],
    category: LogEntry["category"],
    message: string,
    details?: unknown,
  ) {
    const now = new Date();
    const timeStr = `${now.toTimeString().split(" ")[0]}.${String(
      now.getMilliseconds(),
    ).padStart(3, "0")}`;

    const entry: LogEntry = {
      id: this.nextId++,
      timestamp: timeStr,
      level,
      category,
      message,
      details,
    };

    // Mirror to browser console
    const prefix = `[${timeStr}] [${category}] ${message}`;
    if (level === "error") {
      console.error(prefix, details !== undefined ? details : "");
    } else if (level === "warn") {
      console.warn(prefix, details !== undefined ? details : "");
    } else {
      console.log(prefix, details !== undefined ? details : "");
    }

    if (!this._enabled) return;

    this.logs.push(entry);
    if (this.logs.length > this.maxLogs) {
      this.logs.shift();
    }
    this.emitChange();
  }

  public audio(message: string, details?: unknown) {
    this.log("info", "AUDIO", message, details);
  }

  public audioWarn(message: string, details?: unknown) {
    this.log("warn", "AUDIO", message, details);
  }

  public audioError(message: string, details?: unknown) {
    this.log("error", "AUDIO", message, details);
  }

  public ipc(message: string, details?: unknown) {
    this.log("info", "IPC", message, details);
  }

  public ipcError(message: string, details?: unknown) {
    this.log("error", "IPC", message, details);
  }

  public ui(message: string, details?: unknown) {
    this.log("info", "UI", message, details);
  }

  public system(message: string, details?: unknown) {
    this.log("info", "SYSTEM", message, details);
  }

  public perf(message: string, details?: unknown) {
    this.log("info", "PERF", message, details);
  }

  public subscribe(listener: () => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  private emitChange() {
    for (const listener of this.listeners) {
      try {
        listener();
      } catch (err) {
        console.error("DebugLogger listener error:", err);
      }
    }
  }

  public exportText(): string {
    return this.logs
      .map((l) => {
        const det = l.details ? ` | ${JSON.stringify(l.details)}` : "";
        return `[${l.timestamp}] [${l.level.toUpperCase()}] [${l.category}] ${l.message}${det}`;
      })
      .join("\n");
  }
}

export const logger = new DebugLogger();
