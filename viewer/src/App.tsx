import { useEffect, useMemo, useState } from "react";

type Receipt = {
  id: string;
  room_id: string;
  branch: string;
  commit_sha: string;
  verdict: "PASS" | "FAIL";
  reason: string | null;
  changed_files: string[];
  base_output: string;
  suite_output: string;
  duration_ms: number;
  created_at: string;
  countersigned_by: string | null;
  countersigned_at: string | null;
  landed_url: string | null;
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

function evidence(receipt: Receipt) {
  const text =
    receipt.verdict === "FAIL"
      ? receipt.suite_output || receipt.base_output
      : receipt.base_output;
  return text.trim();
}

function ReceiptSheet({ receipt, index }: { receipt: Receipt; index: number }) {
  const [open, setOpen] = useState(false);
  const pass = receipt.verdict === "PASS";
  const evidenceText = evidence(receipt);

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

      {!pass && receipt.reason && (
        <p className="reason">
          <span className="reason-label">Why it failed</span>
          {receipt.reason}
        </p>
      )}

      {evidenceText && (
        <div className="evidence">
          <button
            className="evidence-toggle mono"
            onClick={() => setOpen((value) => !value)}
            aria-expanded={open}
          >
            {open ? "− hide evidence" : "+ show evidence"}
          </button>
          {open && <pre className="mono">{evidenceText}</pre>}
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
              {pass ? "Awaiting a person's countersignature" : "Not eligible"}
            </p>
          </>
        )}
      </div>
    </article>
  );
}

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
        const response = await fetch("/api/receipts", { cache: "no-store" });
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
              ? `ledger live · ${updated}`
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
            runs the proof. A person countersigns before anything lands.
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
              <span className="mono">04</span> A person signs. It lands.
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
