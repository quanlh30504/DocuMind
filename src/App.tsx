import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
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

function formatBytes(bytes: number): string {
  const gb = bytes / 1024 ** 3;
  return `${gb.toFixed(1)} GB`;
}

function App() {
  const [hardware, setHardware] = useState<HardwareProfile | null>(null);
  const [ocrStatus, setOcrStatus] = useState<string>("checking...");
  const [sources, setSources] = useState<DocumentSource[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    invoke<HardwareProfile>("get_hardware_profile").then(setHardware);
    invoke<string>("ocr_runtime_status").then(setOcrStatus);
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
    } catch (e) {
      setError(String(e));
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
