import { useMemo } from "react";
import { Button, MessageBar, MessageBarBody, Spinner } from "@fluentui/react-components";
import { RefreshCw, ShieldCheck } from "lucide-react";
import { errorMessage } from "@symphony/api-client";
import { ALL_PROVIDERS } from "@symphony/shared-types";
import { AiEmptyView, ErrorState, LoadingRows, PageHeader } from "@symphony/ui";
import { useMessages } from "../../lib/i18n.js";
import { useCredentials, useProviders, useRefreshProviders, useSettings } from "../../lib/queries.js";
import { ProviderCard } from "./ProviderCard.js";
import { providerMessages } from "./messages.js";

/** Route `/providers`: detected agent CLIs, auth state, capabilities, default models and API keys. */
export default function ProvidersPage() {
  const m = useMessages(providerMessages);
  const providers = useProviders();
  const settings = useSettings();
  const credentials = useCredentials();
  const refresh = useRefreshProviders();

  const ordered = useMemo(() => {
    const list = providers.data ?? [];
    return [...list].sort((a, b) => ALL_PROVIDERS.indexOf(a.provider) - ALL_PROVIDERS.indexOf(b.provider));
  }, [providers.data]);

  const vault = credentials.data ?? null;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <PageHeader
        title={m.title}
        description={m.description}
        actions={
          <Button
            icon={refresh.isPending ? <Spinner size="tiny" /> : <RefreshCw className="size-4" />}
            disabled={refresh.isPending}
            onClick={() => refresh.mutate()}
          >
            {refresh.isPending ? m.redetecting : m.redetect}
          </Button>
        }
      />
      <div className="min-h-0 flex-1 overflow-auto">
        <div className="mx-auto flex max-w-6xl flex-col gap-3 p-4">
          <MessageBar intent="info" layout="multiline" icon={<ShieldCheck className="size-4" />}>
            <MessageBarBody>{m.credentialsNote}</MessageBarBody>
          </MessageBar>
          {credentials.isError ? (
            <ErrorState
              intent="warning"
              title={m.credentialsLoadError}
              message={errorMessage(credentials.error)}
              onRetry={() => void credentials.refetch()}
            />
          ) : null}

          {providers.isPending ? (
            <LoadingRows rows={8} label={m.title} />
          ) : providers.isError ? (
            <ErrorState title={m.loadError} message={errorMessage(providers.error)} onRetry={() => void providers.refetch()} />
          ) : ordered.length === 0 ? (
            <AiEmptyView title={m.empty} description={m.emptyDetail} />
          ) : (
            ordered.map((provider) => (
              <ProviderCard key={provider.provider} provider={provider} settings={settings.data} credentials={vault} />
            ))
          )}
        </div>
      </div>
    </div>
  );
}
