import { useEffect, useMemo, useState } from "react";

type Receipt = {
  id: string;
  room_id: string;
  branch: string;
  commit_sha: string;
  base_sha: string;
  verdict: "PASS" | "FAIL";
  reason: string | null;
  changed_files: string[];
  base_output: string;
  suite_output: string;
  diffstat: string;
  diff: string;
  duration_ms: number;
  attempt: number;
  supersedes: string | null;
  created_at: string;
  countersigned_by: string | null;
  countersigned_at: string | null;
  landed_url: string | null;
  pr_state: string | null;
};

function Mark() {
  return (
    <svg className="mark" viewBox="0 0 32 32" aria-hidden="true">
      <rect width="32" height="32" rx="7" fill="#141312" />
      <path
        d="M5 18.5l4.2 4.2L20.5 9.4"
        fill="none"
        stroke="#D9481C"
        strokeWidth="3.2"
        strokeLinecap="square"
      />
      <path
        d="M10.5 18.5l4.2 4.2L26 9.4"
        fill="none"
        stroke="#F4F1E8"
        strokeWidth="3.2"
        strokeLinecap="square"
      />
    </svg>
  );
}

function shortSha(sha: string) {
  return sha.slice(0, 10);
}

function stampTime(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString(undefined, {
    year: "numeric",
    month: "short",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function Diff({ text }: { text: string }) {
  return (
    <pre className="diff mono">
      {text.split("\n").map((line, index) => {
        let className = "";
        if (line.startsWith("@@")) className = "diff-hunk";
        else if (line.startsWith("+") && !line.startsWith("+++")) className = "diff-add";
        else if (line.startsWith("-") && !line.startsWith("---")) className = "diff-del";
        return (
          <span key={index} className={className}>
            {line}
            {"\n"}
          </span>
        );
      })}
    </pre>
  );
}

function ReceiptSheet({ receipt, index }: { receipt: Receipt; index: number }) {
  const [evidenceOpen, setEvidenceOpen] = useState(false);
  const [tab, setTab] = useState<"suite" | "base">("suite");
  const [diffOpen, setDiffOpen] = useState(false);
  const pass = receipt.verdict === "PASS";

  const evidence =
    tab === "suite" ? receipt.suite_output : receipt.base_output;
  const hasSuite = receipt.suite_output.trim().length > 0;
  const hasBase = receipt.base_output.trim().length > 0;

  return (
    <article
      className={`sheet ${pass ? "sheet-pass" : "sheet-fail"}`}
      style={{ animationDelay: `${Math.min(index, 6) * 40}ms` }}
    >
      <div className="sheet-head">
        <div>
          <p className="eyebrow">
            Receipt <span className="mono">{receipt.id}</span>
          </p>
          <h2 className="branch mono">{receipt.branch}</h2>
          {(receipt.attempt > 1 || receipt.supersedes) && (
            <p className="chain mono">
              Attempt {receipt.attempt}
              {receipt.supersedes ? ` · supersedes ${receipt.supersedes}` : ""}
            </p>
          )}
        </div>
        <div className={`stamp ${pass ? "stamp-pass" : "stamp-fail"}`}>
          {receipt.verdict}
        </div>
      </div>

      <dl className="meta">
        <div>
          <dt>Commit</dt>
          <dd className="mono">{shortSha(receipt.commit_sha)}</dd>
        </div>
        <div>
          <dt>Changed</dt>
          <dd className="mono">{receipt.changed_files.join(" · ")}</dd>
        </div>
        <div>
          <dt>Took</dt>
          <dd className="mono">{(receipt.duration_ms / 1000).toFixed(1)}s</dd>
        </div>
        <div>
          <dt>Issued</dt>
          <dd className="mono">{stampTime(receipt.created_at)}</dd>
        </div>
      </dl>

      {receipt.diffstat && <p className="diffstat mono">{receipt.diffstat}</p>}

      {!pass && receipt.reason && (
        <p className="reason">
          <span className="reason-label">Why it failed</span>
          {receipt.reason.replaceAll("`", "")}
        </p>
      )}

      {(hasSuite || hasBase) && (
        <div className="evidence">
          <div className="tabs">
            <button
              className={`tab mono ${tab === "suite" ? "tab-active" : ""}`}
              onClick={() => {
                setTab("suite");
                setEvidenceOpen(true);
              }}
              disabled={!hasSuite}
            >
              Suite · branch
            </button>
            <button
              className={`tab mono ${tab === "base" ? "tab-active" : ""}`}
              onClick={() => {
                setTab("base");
                setEvidenceOpen(true);
              }}
              disabled={!hasBase}
            >
              Reproduction · base
            </button>
            <button
              className="evidence-toggle mono"
              onClick={() => setEvidenceOpen((value) => !value)}
              aria-expanded={evidenceOpen}
            >
              {evidenceOpen ? "− hide" : "+ show"}
            </button>
          </div>
          {evidenceOpen && evidence.trim() && (
            <pre className="evidence-pre mono">{evidence}</pre>
          )}
        </div>
      )}

      {receipt.diff.trim() && (
        <div className="diff-block">
          <button
            className="evidence-toggle mono"
            onClick={() => setDiffOpen((value) => !value)}
            aria-expanded={diffOpen}
          >
            {diffOpen ? "− hide diff" : "+ show diff"}
          </button>
          {diffOpen && <Diff text={receipt.diff} />}
        </div>
      )}

      <div className={`signature ${receipt.countersigned_by ? "signed" : ""}`}>
        {receipt.countersigned_by ? (
          <>
            <span className="signature-mark" aria-hidden="true">
              ✓
            </span>
            <div>
              <p className="eyebrow">Countersigned</p>
              <p className="signature-name">
                {receipt.countersigned_by}
                <span className="signature-time mono">
                  {receipt.countersigned_at
                    ? stampTime(receipt.countersigned_at)
                    : ""}
                </span>
                {receipt.pr_state && (
                  <span className="pr-state mono">{receipt.pr_state}</span>
                )}
              </p>
              {receipt.landed_url &&
                (receipt.landed_url.startsWith("http") ? (
                  <a
                    className="landed mono"
                    href={receipt.landed_url}
                    target="_blank"
                    rel="noreferrer"
                  >
                    {receipt.landed_url}
                  </a>
                ) : (
                  <p className="landed mono">{receipt.landed_url}</p>
                ))}
            </div>
          </>
        ) : (
          <>
            <span className="signature-line" aria-hidden="true" />
            <p className="eyebrow awaiting">
              {pass
                ? "Awaiting an approver's countersignature"
                : "Failed verification · nothing to sign"}
            </p>
          </>
        )}
      </div>
    </article>
  );
}

const RECEIPTS_URL =
  (import.meta.env.VITE_RECEIPTS_URL as string | undefined) || "/api/receipts";
const STATIC_LEDGER = RECEIPTS_URL.endsWith(".json");

export default function App() {
  const [receipts, setReceipts] = useState<Receipt[]>([]);
  const [status, setStatus] = useState<"loading" | "live" | "offline">(
    "loading",
  );
  const [updated, setUpdated] = useState<string>("");

  useEffect(() => {
    let cancelled = false;
    async function load() {
      try {
        const response = await fetch(RECEIPTS_URL, { cache: "no-store" });
        if (!response.ok) throw new Error(String(response.status));
        const data = (await response.json()) as Receipt[];
        if (cancelled) return;
        setReceipts(data);
        setStatus("live");
        setUpdated(new Date().toLocaleTimeString());
      } catch {
        if (!cancelled) setStatus("offline");
      }
    }
    load();
    const timer = setInterval(load, 5000);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, []);

  const stats = useMemo(() => {
    const passed = receipts.filter((receipt) => receipt.verdict === "PASS");
    return {
      total: receipts.length,
      passed: passed.length,
      signed: receipts.filter((receipt) => receipt.countersigned_by).length,
    };
  }, [receipts]);

  return (
    <div className="page">
      <div className="grain" aria-hidden="true" />
      <header className="masthead">
        <div className="brand">
          <Mark />
          <div>
            <p className="wordmark">Countersign</p>
            <p className="tagline">No agent ships alone.</p>
          </div>
        </div>
        <div className="status">
          <span className={`dot dot-${status}`} aria-hidden="true" />
          <span className="mono status-text">
            {status === "live"
              ? `${STATIC_LEDGER ? "ledger snapshot" : "ledger live"} · ${updated}`
              : status === "offline"
                ? "ledger unreachable"
                : "connecting"}
          </span>
        </div>
      </header>

      <main className="layout">
        <aside className="rail">
          <p className="rail-lede">
            Agents propose. An independent verifier reproduces the failure and
            runs the proof. An approver countersigns before anything lands.
          </p>
          <div className="ledger-stats">
            <div>
              <span className="stat-number">{stats.total}</span>
              <span className="stat-label">
                {stats.total === 1 ? "Receipt" : "Receipts"}
              </span>
            </div>
            <div>
              <span className="stat-number">{stats.passed}</span>
              <span className="stat-label">Passed</span>
            </div>
            <div>
              <span className="stat-number">{stats.signed}</span>
              <span className="stat-label">Signed</span>
            </div>
          </div>
          <ol className="sequence">
            <li>
              <span className="mono">01</span> A person reports the problem.
            </li>
            <li>
              <span className="mono">02</span> The fixer proposes a branch.
            </li>
            <li>
              <span className="mono">03</span> The verifier proves or refutes it.
            </li>
            <li>
              <span className="mono">04</span> An approver signs. It lands.
            </li>
          </ol>
          <p className="rail-foot mono">
            The room is the source of truth.
            <br />
            This ledger is its index.
          </p>
        </aside>

        <section className="ledger" aria-live="polite">
          {receipts.length === 0 && status !== "loading" && (
            <div className="empty">
              <p className="empty-title">No receipts yet.</p>
              <p>
                The room is waiting for its first proposal. When a branch is
                verified, its receipt appears here — with the exact evidence
                the verifier ran.
              </p>
            </div>
          )}
          {receipts.map((receipt, index) => (
            <ReceiptSheet key={receipt.id} receipt={receipt} index={index} />
          ))}
        </section>
      </main>

      <footer className="colophon">
        <span className="mono">Countersign · a Switch room for verified work</span>
        <span className="mono">propose → verify → countersign</span>
      </footer>
    </div>
  );
}
