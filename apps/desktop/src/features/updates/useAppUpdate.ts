import { isTauri, nativeHost } from "@symphony/api-client";
import { useQuery } from "@tanstack/react-query";
import { useUiStore } from "../../store/uiStore.js";

export const UPDATE_QUERY_KEY = ["app-update"] as const;
const CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000;

/**
 * The newest release as seen by the desktop host. Shared by the banner and Settings through the
 * query cache. With `auto` (the app shell) and the preference on, it checks at startup and every
 * few hours; otherwise it only runs when `refetch()` is called, so Settings never causes a
 * network request by merely being opened. Outside the desktop host there is nothing to check.
 */
export function useAppUpdate({ auto = false }: { auto?: boolean } = {}) {
  const autoCheck = useUiStore((s) => s.autoCheckUpdates);
  const enabled = isTauri() && auto && autoCheck;
  return useQuery({
    queryKey: UPDATE_QUERY_KEY,
    queryFn: () => nativeHost.checkForUpdates(),
    enabled,
    staleTime: CHECK_INTERVAL_MS,
    refetchInterval: enabled ? CHECK_INTERVAL_MS : false,
    refetchOnWindowFocus: false,
    retry: false
  });
}
