export interface Session {
  talker: string;
  displayName: string;
  kind: "friend" | "group" | "official" | "other";
  lastTimestamp: number;
}

export interface WordFreq {
  word: string;
  count: number;
}

export interface WxStatus {
  ready: boolean;
  mode: "real" | "mock";
  message: string;
  wxid?: string | null;
  /** Failing stage + raw error when real reading is unavailable. */
  detail?: string | null;
}

export interface ExportResult {
  path: string;
  dir: string;
  count: number;
}

export interface DiagnoseStep {
  name: string;
  ok: boolean;
  info: string;
}

export interface Diagnostics {
  steps: DiagnoseStep[];
}
