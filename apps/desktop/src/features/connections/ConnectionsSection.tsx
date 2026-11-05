import React, { useState } from "react";
import { AlertTriangle, CheckCircle2, CircleDashed, KeyRound, XCircle } from "lucide-react";
import { errorMessage } from "@symphony/api-client";
import { formatMessage, type ProfileStatus, type ProviderProfile } from "@symphony/shared-types";
import {
  Badge,
  Button,
  Combobox,
  ConfirmDialog,
  ErrorState,
  Field,
  LoadingRows,
  Spinner,
  useNotify
} from "@symphony/ui";
import { useMessages, useT } from "../../lib/i18n.js";
import {
  useConnectionModels,
  useConnectionProfiles,
  useCredentials,
  useDeleteConnectionProfile,
  useSaveConnectionProfile,
  useTestConnectionProfile
} from "../../lib/queries.js";
import { SettingsSection } from "../settings/sections.js";
import { CredentialRow, SetCredentialDialog } from "../providers/CredentialControls.js";
import { providerMessages } from "../providers/messages.js";
import { ProfileDialog } from "./ProfileDialog.js";
import { SERVICES, modelProblem, serviceInfo, statusTone } from "./connectionLogic.js";
import { connectionMessages } from "./messages.js";

const STATUS_ICONS: Record<ProfileStatus, React.ReactNode> = {
  configured: <CircleDashed aria-hidden className="size-3" />,
  verified: <CheckCircle2 aria-hidden className="size-3" />,
  limited: <AlertTriangle aria-hidden className="size-3" />,
  error: <XCircle aria-hidden className="size-3" />
};

/** Status as icon plus text, so colour is never the only signal. */
export const ProfileStatusBadge: React.FC<{ status: ProfileStatus }> = ({ status }) => {
  const m = useMessages(connectionMessages);
  return (
    <Badge tone={statusTone(status)} className="gap-1">
      {STATUS_ICONS[status]}
      {m.status[status]}
    </Badge>
  );
};

const ProfileCard: React.FC<{
  profile: ProviderProfile;
  keyConfigured: boolean;
  onEdit: (profile: ProviderProfile) => void;
}> = ({ profile, keyConfigured, onEdit }) => {
  const m = useMessages(connectionMessages);
  const p = useMessages(providerMessages);
  const notify = useNotify();
  const service = serviceInfo(profile.service);
  const models = useConnectionModels(profile.id, keyConfigured);
  const test = useTestConnectionProfile();
  const save = useSaveConnectionProfile();
  const remove = useDeleteConnectionProfile();
  const [keyOpen, setKeyOpen] = useState(false);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [model, setModel] = useState(profile.model ?? "");
  const titleId = `connection-${profile.id}-title`;

  const savedModel = profile.model ?? "";
  const dirty = model.trim() !== savedModel;
  const invalidModel = modelProblem(model);
  const options = models.data?.models ?? [];
  const keyName = service?.credentialKey ?? profile.credential_ref ?? "";

  const saveModel = () => {
    if (!service || invalidModel) return;
    save.mutate(
      {
        id: profile.id,
        input: {
          name: profile.name,
          service: service.id,
          base_url: profile.base_url,
          model: model.trim() || null,
          fallback_profile_id: profile.fallback_profile_id,
          is_default: profile.is_default
        }
      },
      { onSuccess: () => notify("success", m.savedModel) }
    );
  };

  return (
    <section aria-labelledby={titleId} className="@container space-y-3 rounded-lg border border-border bg-surface-subtle p-3">
      <div className="flex flex-wrap items-center gap-2">
        <h3 id={titleId} className="min-w-0 truncate text-sm font-semibold text-foreground">
          {profile.name}
        </h3>
        <Badge tone="outline">{service ? m.services[service.id] : profile.service}</Badge>
        {profile.is_default ? <Badge tone="info">{m.defaultBadge}</Badge> : null}
        <span className="ml-auto" />
        <ProfileStatusBadge status={profile.status} />
      </div>
      {profile.status_detail ? <p className="text-xs text-muted-foreground">{profile.status_detail}</p> : null}

      <div className="flex flex-wrap items-center gap-2">
        <KeyRound aria-hidden className="size-3.5 text-muted-foreground" />
        <code className="font-mono text-xs text-foreground">{keyName}</code>
        <Badge tone={keyConfigured ? "success" : "neutral"}>{keyConfigured ? p.configured : p.notConfigured}</Badge>
        <Button size="sm" onClick={() => setKeyOpen(true)} aria-label={`${keyConfigured ? p.replaceKey : p.setKey} ${keyName}`}>
          {keyConfigured ? p.replaceKey : p.setKey}
        </Button>
      </div>

      <div className="grid gap-2 @md:grid-cols-[minmax(0,1fr)_auto] @md:items-end">
        <Field
          label={m.modelLabel}
          error={invalidModel ? m.modelInvalid : undefined}
          hint={
            models.isFetching
              ? m.modelsLoading
              : models.isError
                ? `${m.modelsFailed}: ${errorMessage(models.error)}`
                : options.length > 0
                  ? formatMessage(m.modelsCount, { count: options.length })
                  : models.data && models.data.status !== "verified"
                    ? models.data.detail
                    : undefined
          }
        >
          <Combobox value={model} options={options} onValueChange={setModel} placeholder={m.modelPlaceholder} />
        </Field>
        <Button size="sm" variant="secondary" disabled={!dirty || invalidModel || save.isPending} onClick={saveModel}>
          {m.saveModel}
        </Button>
      </div>

      <div className="flex flex-wrap items-center gap-2">
        <Button
          size="sm"
          variant="secondary"
          disabled={!keyConfigured || test.isPending}
          title={keyConfigured ? undefined : m.testNeedsKey}
          onClick={() => test.mutate(profile.id)}
        >
          {test.isPending ? <Spinner className="text-current" /> : null}
          {test.isPending ? m.testing : m.testConnection}
        </Button>
        <Button size="sm" variant="subtle" onClick={() => onEdit(profile)} aria-label={formatMessage(m.editAria, { name: profile.name })}>
          {m.edit}
        </Button>
        <Button
          size="sm"
          variant="subtle"
          disabled={remove.isPending}
          onClick={() => setConfirmOpen(true)}
          aria-label={formatMessage(m.removeAria, { name: profile.name })}
        >
          {m.remove}
        </Button>
      </div>

      {keyOpen ? <SetCredentialDialog keyName={keyName} open={keyOpen} onOpenChange={setKeyOpen} /> : null}
      <ConfirmDialog
        open={confirmOpen}
        onOpenChange={setConfirmOpen}
        title={formatMessage(m.removeTitle, { name: profile.name })}
        description={m.removeDescription}
        confirmLabel={m.remove}
        destructive
        pending={remove.isPending}
        onConfirm={() =>
          remove.mutate(profile.id, {
            onSuccess: () => {
              notify("success", m.removed);
              setConfirmOpen(false);
            }
          })
        }
      />
    </section>
  );
};

/** Settings → Model connections: the services OpenCode runs through, with their keys and models. */
export const ConnectionsSection: React.FC = () => {
  const m = useMessages(connectionMessages);
  const t = useT();
  const profiles = useConnectionProfiles();
  const credentials = useCredentials();
  const [dialog, setDialog] = useState<{ profile: ProviderProfile | null } | null>(null);
  const configured = new Set(credentials.data?.configured ?? []);
  // Service keys left in the vault with no connection to manage them.
  const orphanKeys = SERVICES.filter(
    (s) => configured.has(s.credentialKey) && !(profiles.data ?? []).some((p) => p.service === s.id)
  );

  return (
    <SettingsSection id="connections" intro={m.intro}>
      {profiles.isPending || credentials.isPending ? (
        <LoadingRows rows={2} />
      ) : profiles.isError ? (
        <ErrorState title={m.loadError} message={errorMessage(profiles.error)} onRetry={() => void profiles.refetch()} />
      ) : credentials.isError ? (
        // Not an empty key list: that would show every connection as having no key.
        <ErrorState
          title={m.credentialsLoadError}
          message={errorMessage(credentials.error)}
          onRetry={() => void credentials.refetch()}
        />
      ) : (
        <div className="space-y-3">
          {profiles.data.length === 0 ? (
            <div className="rounded-lg border border-dashed border-border p-4 text-center">
              <p className="text-sm font-medium text-foreground">{m.emptyTitle}</p>
              <p className="mt-0.5 text-xs text-muted-foreground">{m.emptyDetail}</p>
            </div>
          ) : (
            profiles.data.map((profile) => (
              <ProfileCard
                key={profile.id}
                profile={profile}
                keyConfigured={configured.has(serviceInfo(profile.service)?.credentialKey ?? profile.credential_ref ?? "")}
                onEdit={(p) => setDialog({ profile: p })}
              />
            ))
          )}
          <Button onClick={() => setDialog({ profile: null })}>{m.add}</Button>
          {orphanKeys.length > 0 ? (
            <div className="space-y-2 border-t border-border pt-3">
              <h3 className="text-xs font-semibold text-foreground">{m.orphanTitle}</h3>
              <p className="text-xs text-muted-foreground">{m.orphanHint}</p>
              <ul className="divide-y divide-border">
                {orphanKeys.map((s) => (
                  <li key={s.id} className="py-2 first:pt-0 last:pb-0">
                    <CredentialRow keyName={s.credentialKey} configured description={m.services[s.id]} />
                  </li>
                ))}
              </ul>
            </div>
          ) : null}
        </div>
      )}
      {dialog ? (
        <ProfileDialog
          profile={dialog.profile}
          profiles={profiles.data ?? []}
          keyConfigured={(key) => configured.has(key)}
          open
          onOpenChange={(open) => (open ? undefined : setDialog(null))}
          closeLabel={t.common.close}
        />
      ) : null}
    </SettingsSection>
  );
};
