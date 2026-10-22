import { useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { isTauri, nativeHost } from "@symphony/api-client";
import { useClient } from "../../runtime/RuntimeProvider.js";
import { qk } from "../../lib/queryKeys.js";

/** `GET /runtime` — service name, version, API version and auth requirement. */
export function useRuntimeInfo() {
  const client = useClient();
  return useQuery({ queryKey: qk.runtime, queryFn: ({ signal }) => client.runtime(signal), staleTime: 60_000 });
}

export interface DiagnosticsBundle {
  url: string | null;
  bytes: number;
}

/**
 * `GET /diagnostics/export` (already redacted by the runtime) turned into a downloadable Blob.
 * The object URL is revoked when replaced or when the component unmounts.
 */
export function useExportDiagnostics() {
  const client = useClient();
  const [bundle, setBundle] = useState<DiagnosticsBundle | null>(null);

  useEffect(
    () => () => {
      if (bundle?.url) URL.revokeObjectURL?.(bundle.url);
    },
    [bundle]
  );

  const mutation = useMutation({
    mutationFn: () => client.exportDiagnostics(),
    onSuccess: (data) => {
      const json = JSON.stringify(data, null, 2);
      const blob = new Blob([json], { type: "application/json" });
      const url = typeof URL.createObjectURL === "function" ? URL.createObjectURL(blob) : null;
      setBundle({ url, bytes: blob.size });
    }
  });

  return { ...mutation, bundle };
}

/** `GET /backups` — the local backups, newest first. */
export function useBackups() {
  const client = useClient();
  return useQuery({ queryKey: qk.backups, queryFn: ({ signal }) => client.listBackups(signal) });
}

/** `POST /backups` — create a backup, then refresh the list. */
export function useCreateBackup() {
  const client = useClient();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => client.createBackup(),
    onSuccess: () => qc.invalidateQueries({ queryKey: qk.backups })
  });
}

/** `GET /remote-access` — persisted remote settings plus the listener's effective state. */
export function useRemoteAccess() {
  const client = useClient();
  return useQuery({
    queryKey: qk.remoteAccess,
    queryFn: ({ signal }) => client.getRemoteAccess(signal),
    refetchInterval: 15_000
  });
}

/** Generate (or rotate) the remote access token. The token is only in the mutation result. */
export function useRotateRemoteToken() {
  const client = useClient();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => client.rotateRemoteAccessToken(),
    onSuccess: (issued) => qc.setQueryData(qk.remoteAccess, issued.status)
  });
}

export function useRevokeRemoteToken() {
  const client = useClient();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => client.revokeRemoteAccessToken(),
    onSuccess: (status) => qc.setQueryData(qk.remoteAccess, status)
  });
}

/**
 * Desktop host IPC configuration (`get_local_api_config`), for host-side problems the HTTP API
 * cannot report, such as an unpublished discovery file. `null` outside the desktop host.
 */
export function useNativeHostConfig() {
  return useQuery({
    queryKey: ["native-host-config"],
    queryFn: () => (isTauri() ? nativeHost.getLocalApiConfig() : Promise.resolve(null)),
    staleTime: Infinity
  });
}
