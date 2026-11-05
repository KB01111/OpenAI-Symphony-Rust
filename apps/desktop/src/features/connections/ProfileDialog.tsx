import React, { useState } from "react";
import { KeyRound } from "lucide-react";
import { formatMessage, type ModelService, type ProviderProfile } from "@symphony/shared-types";
import {
  Button,
  Dialog,
  DialogBody,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Field,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Spinner,
  Switch,
  useNotify
} from "@symphony/ui";
import { useMessages, useT } from "../../lib/i18n.js";
import { useSaveConnectionProfile, useSaveCredential, useTestConnectionProfile } from "../../lib/queries.js";
import { SERVICES, baseUrlProblem, nextProfileName, serviceInfo } from "./connectionLogic.js";
import { connectionMessages } from "./messages.js";

const NO_FALLBACK = "__none__";

/**
 * Add a connection (service + API key) or edit one. Saving stores the key, saves the profile and
 * tests the connection; the model is picked on the profile's card once its models are listed. The key
 * is held only while typing, is cleared the moment the form is submitted and is sent once to the vault.
 */
export const ProfileDialog: React.FC<{
  /** The profile being edited, or `null` to add one. */
  profile: ProviderProfile | null;
  profiles: ReadonlyArray<ProviderProfile>;
  keyConfigured: (credentialKey: string) => boolean;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  closeLabel: string;
}> = ({ profile, profiles, keyConfigured, open, onOpenChange, closeLabel }) => {
  const m = useMessages(connectionMessages);
  const t = useT();
  const notify = useNotify();
  const saveKey = useSaveCredential();
  const saveProfile = useSaveConnectionProfile();
  const test = useTestConnectionProfile();

  const editing = profile !== null;
  const [serviceId, setServiceId] = useState<ModelService>(profile?.service ?? SERVICES[0]!.id);
  const service = serviceInfo(serviceId)!;
  const [name, setName] = useState(profile?.name ?? "");
  const [secret, setSecret] = useState("");
  const [baseUrl, setBaseUrl] = useState(profile?.base_url ?? "");
  const [advanced, setAdvanced] = useState(Boolean(profile?.base_url || profile?.fallback_profile_id));
  const [fallback, setFallback] = useState(profile?.fallback_profile_id ?? NO_FALLBACK);
  const [isDefault, setIsDefault] = useState(profile?.is_default ?? profiles.length === 0);
  const [submitted, setSubmitted] = useState(false);
  const [busy, setBusy] = useState(false);
  const [keyInvalid, setKeyInvalid] = useState(false);
  // Once saved, a retry (after a failed test, say) updates this profile instead of creating another.
  const [savedId, setSavedId] = useState<string | undefined>(profile?.id);

  const stored = keyConfigured(service.credentialKey);
  const urlProblem = baseUrlProblem(service, baseUrl);
  const urlError =
    urlProblem === "invalid"
      ? m.baseUrlInvalid
      : urlProblem === "insecure"
        ? m.baseUrlInsecure
        : urlProblem === "host"
          ? formatMessage(m.baseUrlHost, { host: service.host })
          : undefined;
  const effectiveName = name.trim() || (editing ? "" : nextProfileName(service, profiles));
  const needsKey = !stored && secret.trim().length === 0;
  const fallbackOptions = profiles.filter((p) => p.id !== profile?.id);

  const close = () => {
    setSecret("");
    onOpenChange(false);
  };

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    event.stopPropagation();
    if (busy) return;
    setSubmitted(true);
    if (urlProblem || needsKey || !effectiveName) return;
    const value = secret;
    // Cleared immediately: the key is not retained anywhere in the UI after submission.
    setSecret("");
    if (/[\r\n\0]/.test(value)) {
      setKeyInvalid(true);
      return;
    }
    setKeyInvalid(false);
    setBusy(true);
    try {
      if (value.trim()) {
        await saveKey.mutateAsync({ key: service.credentialKey, value });
      }
      const saved = await saveProfile.mutateAsync({
        id: savedId,
        input: {
          name: effectiveName,
          service: service.id,
          base_url: baseUrl.trim() || null,
          model: profile?.model ?? null,
          fallback_profile_id: fallback === NO_FALLBACK ? null : fallback,
          is_default: isDefault
        }
      });
      setSavedId(saved.id);
      const result = await test.mutateAsync(saved.id);
      const status = result.profile.status;
      notify(
        status === "verified" ? "success" : "warning",
        formatMessage(
          status === "verified" ? m.connectedVerified : status === "limited" ? m.connectedLimited : m.connectedError,
          { name: saved.name }
        ),
        result.profile.status_detail ?? undefined
      );
      onOpenChange(false);
    } catch {
      // Each mutation reports its own failure with a toast; the dialog stays open to retry.
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(next) => (!next && !busy ? close() : undefined)}>
      <DialogContent closeLabel={closeLabel}>
        <form onSubmit={submit} autoComplete="off" className="flex min-h-0 flex-1 flex-col">
          <DialogHeader>
            <DialogTitle>{editing ? formatMessage(m.editTitle, { name: profile.name }) : m.addTitle}</DialogTitle>
            <DialogDescription>{m.addDescription}</DialogDescription>
          </DialogHeader>
          <DialogBody>
            <div className="space-y-3 py-1">
              <Field label={m.serviceLabel}>
                <Select value={serviceId} disabled={editing} onValueChange={(v) => {
                    // A key typed for one service must never be stored under another.
                    setServiceId(v as ModelService);
                    setSecret("");
                    setKeyInvalid(false);
                  }}>
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {SERVICES.map((s) => (
                      <SelectItem key={s.id} value={s.id}>
                        {m.services[s.id]}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </Field>
              <Field
                label={m.keyLabel}
                hint={stored ? m.keyHintStored : m.keyHintNew}
                error={keyInvalid ? m.keyInvalid : submitted && needsKey ? m.keyRequired : undefined}
                required={!stored}
              >
                <Input
                  type="password"
                  icon={<KeyRound />}
                  value={secret}
                  name={`secret-${service.credentialKey}`}
                  autoComplete="new-password"
                  spellCheck={false}
                  onChange={(e) => {
                    setSecret(e.target.value);
                    setKeyInvalid(false);
                  }}
                />
              </Field>
              <Field label={m.nameLabel} error={submitted && !effectiveName ? m.nameRequired : undefined}>
                <Input
                  value={name}
                  placeholder={editing ? undefined : nextProfileName(service, profiles)}
                  onChange={(e) => setName(e.target.value)}
                />
              </Field>
              <div className="flex items-start gap-2.5">
                <Switch id="connection-default" checked={isDefault} onCheckedChange={setIsDefault} className="mt-0.5" />
                <div className="min-w-0">
                  <Label htmlFor="connection-default" className="text-sm font-normal">
                    {m.defaultLabel}
                  </Label>
                  <p className="text-xs text-muted-foreground">{m.defaultHint}</p>
                </div>
              </div>
              <Button type="button" variant="link" size="sm" aria-expanded={advanced} onClick={() => setAdvanced((v) => !v)}>
                {advanced ? m.hideAdvanced : m.advanced}
              </Button>
              {advanced ? (
                <div className="space-y-3">
                  <Field
                    label={m.baseUrlLabel}
                    hint={formatMessage(m.baseUrlHint, { url: service.baseUrl })}
                    error={urlError}
                  >
                    <Input
                      value={baseUrl}
                      placeholder={service.baseUrl}
                      spellCheck={false}
                      onChange={(e) => setBaseUrl(e.target.value)}
                    />
                  </Field>
                  <Field label={m.fallbackLabel} hint={m.fallbackHint}>
                    <Select value={fallback} onValueChange={setFallback}>
                      <SelectTrigger>
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value={NO_FALLBACK}>{m.fallbackNone}</SelectItem>
                        {fallbackOptions.map((p) => (
                          <SelectItem key={p.id} value={p.id}>
                            {p.name}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </Field>
                </div>
              ) : null}
            </div>
          </DialogBody>
          <DialogFooter>
            <Button variant="secondary" type="button" disabled={busy} onClick={close}>
              {t.common.cancel}
            </Button>
            <Button variant="default" type="submit" disabled={busy}>
              {busy ? <Spinner className="text-current" /> : null}
              {busy ? m.connecting : editing ? m.save : m.connect}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
};
