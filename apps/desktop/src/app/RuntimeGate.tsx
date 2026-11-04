import React from "react";
import { Button, ErrorState, Spinner } from "@symphony/ui";
import { useRuntime } from "../runtime/RuntimeProvider.js";
import { useMessages } from "../lib/i18n.js";
import { shellMessages } from "./messages.js";

/** Renders children only once the runtime client is discovered; otherwise a startup state. */
export const RuntimeGate: React.FC<{ children: React.ReactNode }> = ({ children }) => {
  const { phase, error, retry, client } = useRuntime();
  const m = useMessages(shellMessages);
  // Children (and every hook using `useClient`) only render with a live client.
  if (phase === "ready" && client) return <>{children}</>;
  return (
    <main className="flex h-full items-center justify-center bg-background p-6">
      <div className="w-full max-w-md space-y-4 text-center">
        {phase === "discovering" ? (
          <>
            <Spinner className="mx-auto size-6" label={m.runtime.connecting} />
            <p className="text-sm text-muted-foreground">{m.runtime.connectingDetail}</p>
            {error ? <p className="font-mono text-xs text-subtle-foreground">{error}</p> : null}
          </>
        ) : (
          <>
            <ErrorState title={m.runtime.failed} message={`${m.runtime.failedDetail} ${error ?? ""}`} onRetry={retry} />
            <Button variant="subtle" onClick={retry}>
              {m.runtime.connecting}
            </Button>
          </>
        )}
      </div>
    </main>
  );
};
