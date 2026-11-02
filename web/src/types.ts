// The shapes `ordo web` sends; see src/bin/ordo/web/view.rs.

export type Level = "note" | "warn" | "verdict";
export type Delta = "new" | "changed" | "moved" | null;

export interface Finding {
  source: string;
  name: string;
  message: string;
  level: Level;
  row: number | null;
  original_level: Level | null;
}

export interface Edge {
  label: string;
  target: number | null;
  needs: boolean;
}

export interface Item {
  path: string;
  old: [number, number];
  new: [number, number];
  category: string;
  rationale: string;
  what: string[];
  details: string[];
  notes: string[];
  edges: Edge[];
  symbol: string | null;
  bucket: string;
  grouped_by: string | null;
  noise: boolean;
  comment: boolean;
  symbols: string[];
  enclosing: string | null;
  findings: Finding[];
  level: Level | null;
  uses_at: { name: string; rows: number[] }[];
  executed: [number, number] | null;
  cluster: number | null;
  wave: number | null;
  reviewed: boolean;
  delta: Delta;
  note: string | null;
}

export type Row =
  | { kind: "file"; path: string; n: number }
  | { kind: "gap" }
  | { kind: "item"; i: number; opens: boolean }
  | { kind: "folded"; items: number[] };

export interface Bind {
  prefix: string | null;
  key: string;
  action: string;
}

export interface Command {
  name: string;
  args: string;
  help: string;
}

export interface Review {
  state: "ready";
  version: number;
  rev: string;
  subject: string | null;
  uncommitted: boolean;
  mode: "ledger" | "hunks";
  show_groups: boolean;
  strategy: string;
  watch: "off" | "hint" | "auto";
  only_wave: number | null;
  wave_intents: Record<string, string>;
  stale: string | null;
  path_filter: string | null;
  comments_only: boolean;
  show_all: boolean;
  view: number[];
  rows: Row[];
  items: Item[];
  coverage: { done: number; total: number; both: number; edges: number };
  levels: Partial<Record<Level, number>>;
  delta: { first_run: boolean; new: number; changed: number; moved: number; gone: number };
  rules: { catalog: boolean; own: number; includes: string[]; disabled: string[]; report: string[] };
  file_churn: Record<string, number>;
  sel: number;
  keys: Bind[];
  commands: Command[];
}

export type Slot =
  | { state: "loading" | "empty"; status: string; version: number; rev: string }
  | Review;

export type Spans = [string, string][];

export interface FileHunk {
  i: number;
  old: [number, number];
  new: [number, number];
  removed: ([number, number][] | null)[];
  added: ([number, number][] | null)[];
}

export interface LineComment {
  start: number;
  end: number;
  text: string;
  stale: boolean;
}

export interface FileData {
  path: string;
  missing?: boolean;
  old: string[];
  new: string[];
  old_spans: Spans[] | null;
  new_spans: Spans[] | null;
  hunks: FileHunk[];
  comments: LineComment[];
}

export interface ItemFacts {
  i: number;
  cascade: number[];
  out_of_order: number[];
  churn: string | null;
  line_churn: { commits: number; capped: boolean; last: { author: string; date: string } | null } | null;
  comments: LineComment[];
}

export interface Popup {
  title: string;
  lines: string[];
}

export interface ActResult {
  version: number;
  error?: string;
  popup?: Popup;
  notice?: string;
  navigate?: string;
  clipboard?: string;
}

export type Theme = Record<string, string | null>;
