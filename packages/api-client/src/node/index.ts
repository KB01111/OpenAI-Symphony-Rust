/**
 * Node-only helpers (Vite dev-server discovery plugin, integration tests). Not part of the
 * browser entry point because it uses the filesystem.
 */
import { readFile } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";

export interface RuntimeDiscoveryRecord {
  base_url: string;
  token: string;
  pid: number;
  api_version: number;
  owner: string;
}

/** Same location as `dirs::data_local_dir()/symphony-desktop` used by the Rust runtime. */
export function defaultDataDir(): string {
  if (process.env.SYMPHONY_DATA_DIR) return process.env.SYMPHONY_DATA_DIR;
  if (process.platform === "win32") {
    return join(process.env.LOCALAPPDATA ?? join(homedir(), "AppData", "Local"), "symphony-desktop");
  }
  if (process.platform === "darwin") {
    return join(homedir(), "Library", "Application Support", "symphony-desktop");
  }
  return join(process.env.XDG_DATA_HOME ?? join(homedir(), ".local", "share"), "symphony-desktop");
}

function processAlive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return (error as NodeJS.ErrnoException).code === "EPERM";
  }
}

/** Read `<dataDir>/runtime.json` if its owning process is alive. */
export async function readRuntimeDiscovery(dataDir = defaultDataDir()): Promise<RuntimeDiscoveryRecord | null> {
  try {
    const record = JSON.parse(await readFile(join(dataDir, "runtime.json"), "utf8")) as RuntimeDiscoveryRecord;
    return processAlive(record.pid) ? record : null;
  } catch {
    return null;
  }
}
