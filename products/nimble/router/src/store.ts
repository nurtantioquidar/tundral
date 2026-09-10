// Durable state and telemetry in one SQLite file (bun:sqlite): sessions, the cache-affinity
// ledger, requests, routing decisions, feedback, and the per-(model, task_type) statistics the
// score reads. No prompts, no message contents, no keys — ids, counts, durations, hashes, kinds.

import { Database } from "bun:sqlite";

export interface SessionState {
  session_id: string;
  current_model: string | null;
  prefix_hash: string | null;
  stable_tokens: number;
  last_request_at: number;
  requests: number;
  switches: number;
  recent_failures: string[];
  /** Attempt number whose failure evidence already produced a decisive escalation. */
  escalated_attempt: number | null;
  /** Attempt number whose `previous_failure` was already counted in recent_failures. */
  failure_attempt: number | null;
}

export interface CacheObservation {
  provider: string;
  model: string;
  prefix_hash: string;
  last_served_at: number;
  prefix_tokens_seen: number;
}

export interface RequestRecord {
  id: string;
  at: number;
  session_id: string;
  requested_model: string;
  selected_model: string;
  provider: string;
  routed: boolean;
  status: number;
  latency_ms: number;
  input_tokens: number | null;
  cached_tokens: number | null;
  output_tokens: number | null;
  cost_microusd: number | null;
  price_verified: boolean;
  prefix_hash: string | null;
  decision_id: string | null;
  error_kind: string | null;
  task_type: string | null;
}

export interface DecisionRecord {
  id: string;
  at: number;
  request_id: string;
  session_id: string;
  chosen: string;
  reason: string;
  components: unknown;
  model_changed: boolean;
  cache_affinity: number;
  estimated_cached_tokens: number;
}

export interface FeedbackRecord {
  session_id: string;
  request_id: string | null;
  at: number;
  task_type: string | null;
  task_success: boolean | null;
  verification_result: string | null;
  failure_type: string | null;
  attempt_count: number | null;
  cost_microusd: number | null;
  model: string | null;
}

export interface ModelTaskStats {
  model: string;
  task_type: string;
  n: number;
  successes: number;
  cost_sum: number;
  attempts_sum: number;
  output_tokens_sum: number;
  output_samples: number;
}

export class Store {
  readonly db: Database;

  constructor(path = ":memory:") {
    this.db = new Database(path, { create: true });
    this.db.exec("PRAGMA journal_mode = WAL");
    this.migrate();
    this.addColumnIfMissing("sessions", "escalated_attempt", "INTEGER");
    this.addColumnIfMissing("sessions", "failure_attempt", "INTEGER");
  }

  /** Additive migration for databases created before a column existed. */
  private addColumnIfMissing(table: string, column: string, type: string): void {
    const columns = this.db.query<{ name: string }, []>(`PRAGMA table_info(${table})`).all();
    if (!columns.some((c) => c.name === column)) this.db.exec(`ALTER TABLE ${table} ADD COLUMN ${column} ${type}`);
  }

  private migrate(): void {
    this.db.exec(`
      CREATE TABLE IF NOT EXISTS sessions (
        session_id TEXT PRIMARY KEY,
        current_model TEXT,
        prefix_hash TEXT,
        stable_tokens INTEGER NOT NULL DEFAULT 0,
        last_request_at INTEGER NOT NULL,
        requests INTEGER NOT NULL DEFAULT 0,
        switches INTEGER NOT NULL DEFAULT 0,
        recent_failures TEXT NOT NULL DEFAULT '[]',
        escalated_attempt INTEGER,
        failure_attempt INTEGER
      );
      CREATE TABLE IF NOT EXISTS cache_ledger (
        provider TEXT NOT NULL,
        model TEXT NOT NULL,
        prefix_hash TEXT NOT NULL,
        last_served_at INTEGER NOT NULL,
        prefix_tokens_seen INTEGER NOT NULL,
        PRIMARY KEY (provider, model, prefix_hash)
      );
      CREATE TABLE IF NOT EXISTS requests (
        id TEXT PRIMARY KEY,
        at INTEGER NOT NULL,
        session_id TEXT NOT NULL,
        requested_model TEXT NOT NULL,
        selected_model TEXT NOT NULL,
        provider TEXT NOT NULL,
        routed INTEGER NOT NULL,
        status INTEGER NOT NULL,
        latency_ms INTEGER NOT NULL,
        input_tokens INTEGER,
        cached_tokens INTEGER,
        output_tokens INTEGER,
        cost_microusd INTEGER,
        price_verified INTEGER NOT NULL,
        prefix_hash TEXT,
        decision_id TEXT,
        error_kind TEXT,
        task_type TEXT
      );
      CREATE INDEX IF NOT EXISTS requests_session ON requests(session_id, at);
      CREATE TABLE IF NOT EXISTS decisions (
        id TEXT PRIMARY KEY,
        at INTEGER NOT NULL,
        request_id TEXT NOT NULL,
        session_id TEXT NOT NULL,
        chosen TEXT NOT NULL,
        reason TEXT NOT NULL,
        components TEXT NOT NULL,
        model_changed INTEGER NOT NULL,
        cache_affinity REAL NOT NULL,
        estimated_cached_tokens INTEGER NOT NULL
      );
      CREATE INDEX IF NOT EXISTS decisions_session ON decisions(session_id, at);
      CREATE TABLE IF NOT EXISTS feedback (
        session_id TEXT NOT NULL,
        request_id TEXT NOT NULL DEFAULT '',
        at INTEGER NOT NULL,
        task_type TEXT,
        task_success INTEGER,
        verification_result TEXT,
        failure_type TEXT,
        attempt_count INTEGER,
        cost_microusd INTEGER,
        model TEXT,
        PRIMARY KEY (session_id, request_id)
      );
      CREATE TABLE IF NOT EXISTS model_task_stats (
        model TEXT NOT NULL,
        task_type TEXT NOT NULL,
        n INTEGER NOT NULL DEFAULT 0,
        successes INTEGER NOT NULL DEFAULT 0,
        cost_sum INTEGER NOT NULL DEFAULT 0,
        attempts_sum INTEGER NOT NULL DEFAULT 0,
        output_tokens_sum INTEGER NOT NULL DEFAULT 0,
        output_samples INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (model, task_type)
      );
    `);
  }

  close(): void {
    this.db.close();
  }

  // --- sessions ---------------------------------------------------------

  getSession(sessionId: string): SessionState | null {
    const row = this.db
      .query<
        {
          session_id: string;
          current_model: string | null;
          prefix_hash: string | null;
          stable_tokens: number;
          last_request_at: number;
          requests: number;
          switches: number;
          recent_failures: string;
          escalated_attempt: number | null;
          failure_attempt: number | null;
        },
        [string]
      >("SELECT * FROM sessions WHERE session_id = ?")
      .get(sessionId);
    if (!row) return null;
    return {
      ...row,
      recent_failures: JSON.parse(row.recent_failures) as string[],
      escalated_attempt: row.escalated_attempt ?? null,
      failure_attempt: row.failure_attempt ?? null,
    };
  }

  putSession(state: SessionState): void {
    this.db
      .query(
        `INSERT INTO sessions (session_id, current_model, prefix_hash, stable_tokens, last_request_at, requests, switches, recent_failures, escalated_attempt, failure_attempt)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(session_id) DO UPDATE SET current_model = excluded.current_model, prefix_hash = excluded.prefix_hash,
           stable_tokens = excluded.stable_tokens, last_request_at = excluded.last_request_at, requests = excluded.requests,
           switches = excluded.switches, recent_failures = excluded.recent_failures,
           escalated_attempt = excluded.escalated_attempt, failure_attempt = excluded.failure_attempt`,
      )
      .run(
        state.session_id,
        state.current_model,
        state.prefix_hash,
        state.stable_tokens,
        state.last_request_at,
        state.requests,
        state.switches,
        JSON.stringify(state.recent_failures.slice(-5)),
        state.escalated_attempt,
        state.failure_attempt,
      );
  }

  // --- cache ledger -----------------------------------------------------

  getCacheObservation(provider: string, model: string, prefixHash: string): CacheObservation | null {
    return (
      this.db
        .query<CacheObservation, [string, string, string]>(
          "SELECT provider, model, prefix_hash, last_served_at, prefix_tokens_seen FROM cache_ledger WHERE provider = ? AND model = ? AND prefix_hash = ?",
        )
        .get(provider, model, prefixHash) ?? null
    );
  }

  observeCache(observation: CacheObservation): void {
    this.db
      .query(
        `INSERT INTO cache_ledger (provider, model, prefix_hash, last_served_at, prefix_tokens_seen) VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(provider, model, prefix_hash) DO UPDATE SET last_served_at = excluded.last_served_at,
           prefix_tokens_seen = MAX(cache_ledger.prefix_tokens_seen, excluded.prefix_tokens_seen)`,
      )
      .run(observation.provider, observation.model, observation.prefix_hash, observation.last_served_at, observation.prefix_tokens_seen);
  }

  // --- requests + decisions ---------------------------------------------

  recordRequest(record: RequestRecord): void {
    this.db
      .query(
        `INSERT OR REPLACE INTO requests (id, at, session_id, requested_model, selected_model, provider, routed, status, latency_ms,
           input_tokens, cached_tokens, output_tokens, cost_microusd, price_verified, prefix_hash, decision_id, error_kind, task_type)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      )
      .run(
        record.id,
        record.at,
        record.session_id,
        record.requested_model,
        record.selected_model,
        record.provider,
        record.routed ? 1 : 0,
        record.status,
        record.latency_ms,
        record.input_tokens,
        record.cached_tokens,
        record.output_tokens,
        record.cost_microusd,
        record.price_verified ? 1 : 0,
        record.prefix_hash,
        record.decision_id,
        record.error_kind,
        record.task_type,
      );
  }

  recordDecision(record: DecisionRecord): void {
    this.db
      .query(
        `INSERT OR REPLACE INTO decisions (id, at, request_id, session_id, chosen, reason, components, model_changed, cache_affinity, estimated_cached_tokens)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      )
      .run(
        record.id,
        record.at,
        record.request_id,
        record.session_id,
        record.chosen,
        record.reason,
        JSON.stringify(record.components),
        record.model_changed ? 1 : 0,
        record.cache_affinity,
        record.estimated_cached_tokens,
      );
  }

  recentDecisions(sessionId: string | null, limit = 20): DecisionRecord[] {
    const rows = sessionId
      ? this.db
          .query<Record<string, unknown>, [string, number]>("SELECT * FROM decisions WHERE session_id = ? ORDER BY at DESC LIMIT ?")
          .all(sessionId, limit)
      : this.db.query<Record<string, unknown>, [number]>("SELECT * FROM decisions ORDER BY at DESC LIMIT ?").all(limit);
    return rows.map((row) => ({
      id: row.id as string,
      at: row.at as number,
      request_id: row.request_id as string,
      session_id: row.session_id as string,
      chosen: row.chosen as string,
      reason: row.reason as string,
      components: JSON.parse(row.components as string),
      model_changed: Boolean(row.model_changed),
      cache_affinity: row.cache_affinity as number,
      estimated_cached_tokens: row.estimated_cached_tokens as number,
    }));
  }

  lastRequestForSession(sessionId: string): RequestRecord | null {
    const row = this.db
      .query<Record<string, unknown>, [string]>("SELECT * FROM requests WHERE session_id = ? ORDER BY at DESC LIMIT 1")
      .get(sessionId);
    return row ? rowToRequest(row) : null;
  }

  /** Sum of Nimble's own priced requests for a session (verified cards only). */
  sessionCostMicroUsd(sessionId: string): { cost: number; verified: boolean } {
    const row = this.db
      .query<{ cost: number | null; unverified: number }, [string]>(
        "SELECT SUM(cost_microusd) AS cost, SUM(CASE WHEN price_verified = 0 THEN 1 ELSE 0 END) AS unverified FROM requests WHERE session_id = ? AND status = 200",
      )
      .get(sessionId);
    return { cost: row?.cost ?? 0, verified: (row?.unverified ?? 0) === 0 };
  }

  getRequest(id: string): RequestRecord | null {
    const row = this.db.query<Record<string, unknown>, [string]>("SELECT * FROM requests WHERE id = ?").get(id);
    return row ? rowToRequest(row) : null;
  }

  // --- feedback + stats -------------------------------------------------

  /** Returns false when this (session, request) feedback was already applied. */
  /**
   * Record a task outcome. Idempotent per (session, request), with one
   * exception: a record whose `task_success` is null (the client had no
   * oracle) is upgraded in place when a later report for the same request
   * carries a verdict — the benchmark's verifier, for instance. Returns what
   * happened so the caller can say so.
   */
  recordFeedback(record: FeedbackRecord): "recorded" | "upgraded" | "duplicate" {
    const existing = this.db
      .query<{ task_success: number | null }, [string, string]>(
        "SELECT task_success FROM feedback WHERE session_id = ? AND request_id = ?",
      )
      .get(record.session_id, record.request_id ?? "");
    if (existing) {
      if (existing.task_success !== null || record.task_success === null) return "duplicate";
      this.db
        .query(
          `UPDATE feedback SET at = ?, task_type = COALESCE(?, task_type), task_success = ?, verification_result = ?,
             failure_type = COALESCE(?, failure_type), attempt_count = COALESCE(?, attempt_count),
             cost_microusd = COALESCE(?, cost_microusd) WHERE session_id = ? AND request_id = ?`,
        )
        .run(
          record.at,
          record.task_type,
          record.task_success ? 1 : 0,
          record.verification_result,
          record.failure_type,
          record.attempt_count,
          record.cost_microusd,
          record.session_id,
          record.request_id ?? "",
        );
      this.bumpStats(record);
      return "upgraded";
    }
    this.db
      .query(
        `INSERT INTO feedback (session_id, request_id, at, task_type, task_success, verification_result, failure_type, attempt_count, cost_microusd, model)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      )
      .run(
        record.session_id,
        record.request_id ?? "",
        record.at,
        record.task_type,
        record.task_success === null ? null : record.task_success ? 1 : 0,
        record.verification_result,
        record.failure_type,
        record.attempt_count,
        record.cost_microusd,
        record.model,
      );
    this.bumpStats(record);
    return "recorded";
  }

  private bumpStats(record: FeedbackRecord): void {
    if (!record.model || record.task_success === null) return;
    this.db
      .query(
        `INSERT INTO model_task_stats (model, task_type, n, successes, cost_sum, attempts_sum)
         VALUES (?, ?, 1, ?, ?, ?)
         ON CONFLICT(model, task_type) DO UPDATE SET n = n + 1, successes = successes + excluded.successes,
           cost_sum = cost_sum + excluded.cost_sum, attempts_sum = attempts_sum + excluded.attempts_sum`,
      )
      .run(record.model, record.task_type ?? "unknown", record.task_success ? 1 : 0, record.cost_microusd ?? 0, record.attempt_count ?? 1);
  }

  observeOutput(model: string, taskType: string | null, outputTokens: number): void {
    this.db
      .query(
        `INSERT INTO model_task_stats (model, task_type, output_tokens_sum, output_samples) VALUES (?, ?, ?, 1)
         ON CONFLICT(model, task_type) DO UPDATE SET output_tokens_sum = output_tokens_sum + excluded.output_tokens_sum,
           output_samples = output_samples + 1`,
      )
      .run(model, taskType ?? "unknown", outputTokens);
  }

  stats(model: string, taskType: string | null): ModelTaskStats | null {
    return (
      this.db
        .query<ModelTaskStats, [string, string]>("SELECT * FROM model_task_stats WHERE model = ? AND task_type = ?")
        .get(model, taskType ?? "unknown") ?? null
    );
  }

  allStats(taskType?: string): ModelTaskStats[] {
    return taskType
      ? this.db.query<ModelTaskStats, [string]>("SELECT * FROM model_task_stats WHERE task_type = ? ORDER BY model").all(taskType)
      : this.db.query<ModelTaskStats, []>("SELECT * FROM model_task_stats ORDER BY task_type, model").all();
  }
}

function rowToRequest(row: Record<string, unknown>): RequestRecord {
  return {
    id: row.id as string,
    at: row.at as number,
    session_id: row.session_id as string,
    requested_model: row.requested_model as string,
    selected_model: row.selected_model as string,
    provider: row.provider as string,
    routed: Boolean(row.routed),
    status: row.status as number,
    latency_ms: row.latency_ms as number,
    input_tokens: (row.input_tokens as number | null) ?? null,
    cached_tokens: (row.cached_tokens as number | null) ?? null,
    output_tokens: (row.output_tokens as number | null) ?? null,
    cost_microusd: (row.cost_microusd as number | null) ?? null,
    price_verified: Boolean(row.price_verified),
    prefix_hash: (row.prefix_hash as string | null) ?? null,
    decision_id: (row.decision_id as string | null) ?? null,
    error_kind: (row.error_kind as string | null) ?? null,
    task_type: (row.task_type as string | null) ?? null,
  };
}
