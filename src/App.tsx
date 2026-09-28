import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import "./App.css";

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

interface PageRecord {
  page: number;
  source: "native" | "ocr";
  confidence: number;
  status: "completed" | "failed";
  warnings: string[];
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
  | { type: "page_completed"; page: number; total: number; status: "completed" | "failed" };

function formatBytes(bytes: number): string {
  const gb = bytes / 1024 ** 3;
  return `${gb.toFixed(1)} GB`;
}

function App() {
  const [hardware, setHardware] = useState<HardwareProfile | null>(null);
  const [ocrStatus, setOcrStatus] = useState<string>("checking...");
  const [sources, setSources] = useState<DocumentSource[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [processing, setProcessing] = useState(false);
  const [progress, setProgress] = useState<{ page: number; total: number } | null>(null);
  const [results, setResults] = useState<JobState[]>([]);

  useEffect(() => {
    invoke<HardwareProfile>("get_hardware_profile").then(setHardware);
    invoke<string>("ocr_runtime_status").then(setOcrStatus);
  }, []);

  useEffect(() => {
    const unlisten = listen<JobEvent>("job://progress", (event) => {
      const payload = event.payload;
      if (payload.type === "page_started" || payload.type === "page_completed") {
        setProgress({ page: payload.page, total: payload.total });
      }
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
      const jobResults: JobState[] = [];
      for (const source of sources) {
        const stem = source.path.split(/[/\\]/).pop()?.replace(/\.[^.]+$/, "") ?? "document";
        const outputDir = `${outputRoot}/${stem}`;
        setProgress({ page: 0, total: 0 });
        const state = await invoke<JobState>("process_document", {
          source,
          outputDir,
        });
        jobResults.push(state);
      }
      setResults(jobResults);
    } catch (e) {
      setError(String(e));
    } finally {
      setProcessing(false);
      setProgress(null);
    }
  }

  return (
    <main className="container">
      <h1>DocuMind</h1>
      <p className="subtitle">
        Local-first OCR & document intelligence — nothing you drop here leaves this machine.
      </p>

      <section className="drop-zone">
        <p>Drop PDF / Images Here</p>
        <div className="row">
          <button onClick={() => importPath(false)}>Select Files</button>
          <button onClick={() => importPath(true)}>Select Folder</button>
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
          <div className="row" style={{ marginTop: "0.75rem" }}>
            <button onClick={processAll} disabled={processing || ocrStatus !== "installed"}>
              {processing ? "Processing..." : "Process"}
            </button>
          </div>
          {processing && progress && progress.total > 0 && (
            <p>
              Page {progress.page} / {progress.total}
            </p>
          )}
        </section>
      )}

      {results.length > 0 && (
        <section className="status-panel">
          <h2>Results</h2>
          {results.map((r) => (
            <div key={r.document_id} style={{ marginBottom: "0.75rem" }}>
              <p>
                <strong>{r.document_path}</strong> — {r.status}
              </p>
              <p>
                {r.completed_pages}/{r.total_pages} pages completed
                {r.failed_pages.length > 0 && `, ${r.failed_pages.length} failed`}
              </p>
            </div>
          ))}
        </section>
      )}

      <section className="status-panel">
        <h2>Local Components</h2>
        <p>
          OCR Engine: <strong>{ocrStatus === "installed" ? "✓ Ready" : `✗ ${ocrStatus}`}</strong>
        </p>
        <p>Processing: ● Offline</p>
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
    </main>
  );
}

export default App;
