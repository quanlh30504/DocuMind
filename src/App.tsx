import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { openPath } from "@tauri-apps/plugin-opener";
import pkg from "../package.json";
import "./App.css";

interface SystemDependency {
  name: string;
  package: string;
  installed: boolean;
}

interface HardwareProfile {
  platform: string;
  cpu_cores: number;
  ram_bytes: number;
  disk_free_bytes: number;
  gpu: { name: string; vram_bytes: number } | null;
}

interface DocumentSource {
  path: string;
  kind: "pdf" | "image";
}

interface CorrectionRecord {
  original: string;
  corrected: string;
}

interface PageRecord {
  page: number;
  source: "native" | "ocr";
  confidence: number;
  status: "completed" | "failed";
  warnings: string[];
  suggested_corrections: CorrectionRecord[];
  flagged_words: string[];
}

interface JobState {
  document_id: string;
  document_path: string;
  total_pages: number;
  completed_pages: number;
  failed_pages: number[];
  status: "COMPLETED" | "COMPLETED_WITH_WARNINGS" | "INTERRUPTED";
  pages: PageRecord[];
}

type JobEvent =
  | { type: "page_started"; page: number; total: number }
  | { type: "page_completed"; page: number; total: number; status: "completed" | "failed" }
  | { type: "page_skipped"; page: number; total: number };

type PageLogEntry = { page: number; status: "running" | "completed" | "failed" | "skipped" };

const LANGUAGE_LABELS: Record<string, string> = {
  eng: "English",
  vie: "Vietnamese",
};

function formatBytes(bytes: number): string {
  const gb = bytes / 1024 ** 3;
  return `${gb.toFixed(1)} GB`;
}

function basename(path: string): string {
  return path.split(/[/\\]/).pop() ?? path;
}

function dedupeCorrections(items: CorrectionRecord[]): CorrectionRecord[] {
  const seen = new Map<string, CorrectionRecord>();
  for (const item of items) seen.set(item.original, item);
  return Array.from(seen.values());
}

function App() {
  const [hardware, setHardware] = useState<HardwareProfile | null>(null);
  const [ocrStatus, setOcrStatus] = useState<string>("checking...");
  const [languages, setLanguages] = useState<string[]>([]);
  const [selectedLang, setSelectedLang] = useState<string>("auto");
  const [sources, setSources] = useState<DocumentSource[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [processing, setProcessing] = useState(false);
  const [currentDoc, setCurrentDoc] = useState<{ index: number; total: number; path: string } | null>(null);
  const [pageTotal, setPageTotal] = useState(0);
  const [pageLog, setPageLog] = useState<PageLogEntry[]>([]);
  const [results, setResults] = useState<{ state: JobState; outputDir: string }[]>([]);
  const [systemDeps, setSystemDeps] = useState<SystemDependency[]>([]);
  const [installing, setInstalling] = useState(false);
  const [installMessage, setInstallMessage] = useState<string | null>(null);

  async function refreshDependencyStatus() {
    invoke<string>("ocr_runtime_status").then(setOcrStatus);
    invoke<string[]>("ocr_languages").then(setLanguages);
    invoke<SystemDependency[]>("check_system_dependencies").then(setSystemDeps);
  }

  useEffect(() => {
    invoke<HardwareProfile>("get_hardware_profile").then(setHardware);
    refreshDependencyStatus();
  }, []);

  async function installMissingDependencies() {
    setInstalling(true);
    setInstallMessage(null);
    try {
      const result = await invoke<string>("install_system_dependencies");
      setInstallMessage(result || "Installed successfully.");
    } catch (e) {
      setInstallMessage(String(e));
    } finally {
      setInstalling(false);
      refreshDependencyStatus();
    }
  }

  useEffect(() => {
    const unlisten = listen<JobEvent>("job://progress", (event) => {
      const payload = event.payload;
      setPageTotal(payload.total);
      setPageLog((log) => {
        const status =
          payload.type === "page_started" ? "running" : payload.type === "page_skipped" ? "skipped" : payload.status;
        const idx = log.findIndex((e) => e.page === payload.page);
        const entry: PageLogEntry = { page: payload.page, status };
        if (idx === -1) return [...log, entry];
        const copy = [...log];
        copy[idx] = entry;
        return copy;
      });
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  async function importPath(directory: boolean) {
    setError(null);
    const selection = await open({ directory, multiple: false });
    if (!selection) return;
    try {
      const found = await invoke<DocumentSource[]>("discover_documents", {
        root: selection,
      });
      setSources(found);
      setResults([]);
    } catch (e) {
      setError(String(e));
    }
  }

  async function processAll() {
    if (sources.length === 0) return;
    setError(null);
    const outputRoot = await open({ directory: true, title: "Choose output folder" });
    if (!outputRoot) return;

    setProcessing(true);
    setResults([]);
    try {
      const jobResults: { state: JobState; outputDir: string }[] = [];
      for (let i = 0; i < sources.length; i++) {
        const source = sources[i];
        setCurrentDoc({ index: i + 1, total: sources.length, path: source.path });
        setPageTotal(0);
        setPageLog([]);

        const stem = basename(source.path).replace(/\.[^.]+$/, "") || "document";
        const outputDir = `${outputRoot}/${stem}`;
        const state = await invoke<JobState>("process_document", {
          source,
          outputDir,
          lang: selectedLang,
        });
        jobResults.push({ state, outputDir });
      }
      setResults(jobResults);
    } catch (e) {
      setError(String(e));
    } finally {
      setProcessing(false);
      setCurrentDoc(null);
    }
  }

  const completedCount = pageLog.filter((e) => e.status !== "running").length;
  const progressPct = pageTotal > 0 ? Math.round((completedCount / pageTotal) * 100) : 0;

  return (
    <main className="container">
      <h1>DocuMind</h1>
      <p className="subtitle">
        Local-first OCR & document intelligence — nothing you drop here leaves this machine.
      </p>

      <section className="drop-zone">
        <p>Drop PDF / Images Here</p>
        <div className="row">
          <button onClick={() => importPath(false)} disabled={processing}>
            Select Files
          </button>
          <button onClick={() => importPath(true)} disabled={processing}>
            Select Folder
          </button>
        </div>
      </section>

      {error && <p className="error">{error}</p>}

      {sources.length > 0 && (
        <section>
          <h2>Discovered ({sources.length})</h2>
          <ul className="source-list">
            {sources.map((s) => (
              <li key={s.path}>
                <span className="kind">{s.kind}</span> {s.path}
              </li>
            ))}
          </ul>

          <div className="row lang-row">
            <label htmlFor="lang-select">OCR language:</label>
            <select
              id="lang-select"
              value={selectedLang}
              onChange={(e) => setSelectedLang(e.target.value)}
              disabled={processing}
            >
              <option value="auto">Auto-detect{languages.length > 0 ? ` (${languages.map((l) => LANGUAGE_LABELS[l] ?? l).join(" + ")})` : ""}</option>
              {languages.map((l) => (
                <option key={l} value={l}>
                  {LANGUAGE_LABELS[l] ?? l}
                </option>
              ))}
            </select>
          </div>

          <div className="row" style={{ marginTop: "0.75rem" }}>
            <button onClick={processAll} disabled={processing || ocrStatus !== "installed"}>
              {processing ? "Processing..." : "Process"}
            </button>
          </div>
        </section>
      )}

      {processing && currentDoc && (
        <section className="status-panel processing-panel">
          <h2>
            Processing {currentDoc.index}/{currentDoc.total}: {basename(currentDoc.path)}
          </h2>
          <div className="progress-bar-track">
            <div className="progress-bar-fill" style={{ width: `${progressPct}%` }} />
          </div>
          <p className="progress-label">
            {pageTotal > 0 ? `${completedCount} / ${pageTotal} pages (${progressPct}%)` : "Starting…"}
          </p>
          <ul className="page-log">
            {pageLog
              .slice()
              .reverse()
              .slice(0, 8)
              .map((e) => (
                <li key={e.page} className={`page-log-${e.status}`}>
                  {e.status === "running" && "⏳"}
                  {e.status === "completed" && "✓"}
                  {e.status === "skipped" && "⏭"}
                  {e.status === "failed" && "✗"}
                  {" "}Page {e.page}
                </li>
              ))}
          </ul>
        </section>
      )}

      {results.length > 0 && (
        <section className="status-panel">
          <h2>Results</h2>
          {results.map(({ state: r, outputDir }) => {
            const suggestions = dedupeCorrections(r.pages.flatMap((p) => p.suggested_corrections));
            const flaggedWords = Array.from(new Set(r.pages.flatMap((p) => p.flagged_words)));
            const spellcheckUnavailable = Array.from(
              new Set(
                r.pages
                  .flatMap((p) => p.warnings)
                  .filter((w) => w.startsWith("spellcheck_unavailable:"))
                  .map((w) => w.split(":")[1])
              )
            );
            const lowCoverage = Array.from(
              new Set(
                r.pages
                  .flatMap((p) => p.warnings)
                  .filter((w) => w.startsWith("spellcheck_low_coverage:"))
                  .map((w) => w.split(":")[1])
              )
            );
            return (
              <div key={r.document_id} style={{ marginBottom: "0.75rem" }}>
                <p>
                  <strong>{basename(r.document_path)}</strong> —{" "}
                  {r.status === "COMPLETED" && "✓ Completed"}
                  {r.status === "COMPLETED_WITH_WARNINGS" && "⚠ Completed with warnings"}
                  {r.status === "INTERRUPTED" && "✗ Interrupted"}
                </p>
                <p>
                  {r.completed_pages}/{r.total_pages} pages completed
                  {r.failed_pages.length > 0 && `, ${r.failed_pages.length} failed (${r.failed_pages.join(", ")})`}
                </p>
                {spellcheckUnavailable.length > 0 && (
                  <p className="warning-note">
                    ⚠ No dictionary installed for: {spellcheckUnavailable.join(", ")} — spelling checks skipped for those pages.
                  </p>
                )}
                {lowCoverage.length > 0 && (
                  <p className="warning-note">
                    ⚠ Dictionary for {lowCoverage.join(", ")} has limited word coverage on this machine — words are flagged, not suggested, to avoid guessing wrong.
                  </p>
                )}
                {suggestions.length > 0 && (
                  <details className="flagged-words" open>
                    <summary>{suggestions.length} suggested correction(s) — review before using, not applied automatically</summary>
                    <ul className="suggestion-list">
                      {suggestions.map((c) => (
                        <li key={c.original}>
                          <span className="orig">{c.original}</span> → <span className="sugg">{c.corrected}</span>
                        </li>
                      ))}
                    </ul>
                  </details>
                )}
                {flaggedWords.length > 0 && (
                  <details className="flagged-words">
                    <summary>{flaggedWords.length} word(s) flagged as possibly wrong (no suggestion found)</summary>
                    <p className="flagged-word-list">{flaggedWords.join(", ")}</p>
                  </details>
                )}
                <button onClick={() => openPath(outputDir).catch((e) => setError(String(e)))}>
                  Open Output Folder
                </button>
              </div>
            );
          })}
          <p className="ai-note">
            Note: the items above come from dictionary matching only (no AI model installed) — nearest-neighbor
            word lookup with no grammar or context. It can be wrong (two real words can be equally close to a
            typo), so suggestions are shown for you to review, never written into the output files automatically.
            A local AI model (planned, not yet built) would be needed for correction that understands context
            well enough to apply automatically.
          </p>
        </section>
      )}

      <section className="status-panel">
        <h2>Local Components</h2>
        <p>
          OCR Engine: <strong>{ocrStatus === "installed" ? "✓ Ready" : `✗ ${ocrStatus}`}</strong>
        </p>
        <p>
          Languages:{" "}
          <strong>{languages.length > 0 ? languages.map((l) => LANGUAGE_LABELS[l] ?? l).join(", ") : "none installed"}</strong>
        </p>
        <p>Processing: ● Offline</p>

        <ul className="dep-list">
          {systemDeps.map((d) => (
            <li key={d.package} className={d.installed ? "dep-ok" : "dep-missing"}>
              {d.installed ? "✓" : "✗"} {d.name}
              {!d.installed && <span className="dep-pkg"> ({d.package})</span>}
            </li>
          ))}
        </ul>

        {systemDeps.some((d) => !d.installed) && (
          <div className="dep-install">
            <button onClick={installMissingDependencies} disabled={installing}>
              {installing ? "Installing… (check for a password prompt)" : "Install Missing Dependencies"}
            </button>
            {installMessage && <p className="install-message">{installMessage}</p>}
          </div>
        )}
      </section>

      <section className="status-panel">
        <h2>Detected Hardware</h2>
        {hardware ? (
          <ul>
            <li>Platform: {hardware.platform}</li>
            <li>CPU cores: {hardware.cpu_cores}</li>
            <li>RAM: {formatBytes(hardware.ram_bytes)}</li>
            <li>Free disk: {formatBytes(hardware.disk_free_bytes)}</li>
            <li>GPU: {hardware.gpu ? hardware.gpu.name : "not detected (CPU-only assumed)"}</li>
          </ul>
        ) : (
          <p>Detecting...</p>
        )}
      </section>

      <footer className="app-footer">
        DocuMind v{pkg.version} — by Quan Nguyen
      </footer>
    </main>
  );
}

export default App;
