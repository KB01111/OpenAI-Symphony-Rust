import { useEffect, useState } from "react";
import { useMutation, useQuery } from "@tanstack/react-query";
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
