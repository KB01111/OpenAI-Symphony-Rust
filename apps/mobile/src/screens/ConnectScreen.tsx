import React, { useState } from "react";
import { Button, Checkbox, Field, Input, MessageBar, MessageBarBody, MessageBarTitle, Spinner } from "@fluentui/react-components";
import { SymphonyClient } from "@symphony/api-client";
import { formatMessage } from "@symphony/shared-types";
import { classifyConnectionError, normalizeBaseUrl, type ConnectionSettings } from "../connection.js";
import { useStrings } from "../i18n.js";

export interface ConnectScreenProps {
  initial: ConnectionSettings | null;
  /** Shown when the previous session ended because the runtime rejected the token. */
  rejected?: boolean;
  onConnected: (settings: ConnectionSettings) => void;
}

type Problem = { title: string; body?: string } | null;

/** Runtime URL + token form. Verifies the token against a protected endpoint before connecting. */
export const ConnectScreen: React.FC<ConnectScreenProps> = ({ initial, rejected, onConnected }) => {
  const { m, t } = useStrings();
  const [url, setUrl] = useState(initial?.baseUrl ?? "");
  const [token, setToken] = useState(initial?.token ?? "");
  const [remember, setRemember] = useState(initial?.remember ?? false);
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<Problem>(rejected ? { title: m.tokenRejected, body: m.tokenRejectedBody } : null);
  const [urlError, setUrlError] = useState<string | null>(null);

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    if (busy) return;
    const normalized = normalizeBaseUrl(url);
    if (!normalized.ok) {
      setUrlError(normalized.problem === "empty" ? m.urlEmpty : normalized.problem === "unsupported_scheme" ? m.urlScheme : m.urlInvalid);
      return;
    }
    setUrlError(null);
    setProblem(null);
    setBusy(true);
    try {
      const client = new SymphonyClient({ baseUrl: normalized.url, token: token.trim() || null, timeoutMs: 10_000 });
      // `/state` is protected, so this proves the token — `/health` would not.
      await client.getState();
      onConnected({ baseUrl: normalized.url, token: token.trim(), remember });
    } catch (error) {
      const kind = classifyConnectionError(error);
      if (kind === "unauthorized") setProblem({ title: m.tokenRejected, body: m.tokenRejectedBody });
      else if (kind === "network") setProblem({ title: m.unreachable, body: formatMessage(m.unreachableBody, { url: normalized.url }) });
      else setProblem({ title: m.connectFailed, body: error instanceof Error ? error.message : undefined });
    } finally {
      setBusy(false);
    }
  };

  return (
    <main className="mx-auto flex min-h-full w-full max-w-md flex-col justify-center gap-4 p-4">
      <header className="space-y-1">
        <h1 className="font-display text-xl font-semibold text-foreground">{m.connectTitle}</h1>
        <p className="text-sm text-muted-foreground">{m.connectDescription}</p>
      </header>
      {problem ? (
        <MessageBar intent="error">
          <MessageBarBody>
            <MessageBarTitle>{problem.title}</MessageBarTitle>
            {problem.body}
          </MessageBarBody>
        </MessageBar>
      ) : null}
      <form className="flex flex-col gap-3 rounded-md border border-border bg-surface p-4" onSubmit={(e) => void submit(e)} noValidate>
        <Field label={t.mobile.hostUrl} hint={m.urlHint} validationMessage={urlError ?? undefined} validationState={urlError ? "error" : "none"} required>
          <Input
            type="url"
            inputMode="url"
            autoComplete="url"
            autoCapitalize="off"
            value={url}
            onChange={(_, d) => setUrl(d.value)}
          />
        </Field>
        <Field label={t.mobile.accessToken} hint={t.mobile.tokenStoredNotice}>
          <Input type="password" autoComplete="current-password" value={token} onChange={(_, d) => setToken(d.value)} />
        </Field>
        <Field hint={m.rememberHint}>
          <Checkbox checked={remember} onChange={(_, d) => setRemember(d.checked === true)} label={t.mobile.rememberToken} />
        </Field>
        <Button type="submit" appearance="primary" disabled={busy} icon={busy ? <Spinner size="tiny" /> : undefined}>
          {busy ? m.connecting : t.mobile.connect}
        </Button>
      </form>
    </main>
  );
};
