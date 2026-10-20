import React from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { SymphonyClient } from "@symphony/api-client";
import { SymphonyDesignProvider } from "@symphony/ui";
import { RuntimeProvider } from "../runtime/RuntimeProvider.js";
import { useUiStore } from "../store/uiStore.js";

export function createQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: {
        retry: (count, error) => count < 2 && (error as { kind?: string })?.kind !== "not_found",
        refetchOnWindowFocus: false,
        staleTime: 2_000
      },
      mutations: { retry: false }
    }
  });
}

const DesignBridge: React.FC<{ children: React.ReactNode }> = ({ children }) => {
  const themeMode = useUiStore((s) => s.themeMode);
  const density = useUiStore((s) => s.density);
  const locale = useUiStore((s) => s.locale);
  return (
    <SymphonyDesignProvider themeMode={themeMode} density={density} locale={locale}>
      {children}
    </SymphonyDesignProvider>
  );
};

export interface AppProvidersProps {
  children: React.ReactNode;
  queryClient?: QueryClient;
  client?: SymphonyClient;
}

export const AppProviders: React.FC<AppProvidersProps> = ({ children, queryClient, client }) => {
  const [qc] = React.useState(() => queryClient ?? createQueryClient());
  return (
    <QueryClientProvider client={qc}>
      <DesignBridge>
        <RuntimeProvider client={client}>{children}</RuntimeProvider>
      </DesignBridge>
    </QueryClientProvider>
  );
};
