import React, { useState } from "react";
import { KeyRound } from "lucide-react";
import { formatMessage } from "@symphony/shared-types";
import {
  Badge,
  Button,
  ConfirmDialog,
  Dialog,
  DialogBody,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Field,
  Input,
  Spinner,
  cn,
  useNotify
} from "@symphony/ui";
import { useMessages, useT } from "../../lib/i18n.js";
import { useDeleteCredential, useSaveCredential } from "../../lib/queries.js";
import { providerMessages } from "./messages.js";

/**
 * "Set key…" dialog. The secret is held only while typing in the password input; it is never
 * logged or cached, is sent once to the runtime vault, and is cleared the moment it is submitted
 * (or the dialog closes).
 */
export const SetCredentialDialog: React.FC<{ keyName: string; open: boolean; onOpenChange: (open: boolean) => void }> = ({
  keyName,
  open,
  onOpenChange
}) => {
  const m = useMessages(providerMessages);
  const t = useT();
  const notify = useNotify();
  const save = useSaveCredential();
  const [secret, setSecret] = useState("");
  const [invalid, setInvalid] = useState(false);

  const close = () => {
    setSecret("");
    setInvalid(false);
    onOpenChange(false);
  };

  const submit = (event: React.FormEvent) => {
    event.preventDefault();
    // The dialog is portalled; keep the submit from bubbling to any form around the trigger.
    event.stopPropagation();
    if (save.isPending) return;
    const value = secret;
    // Cleared immediately: the value is not retained anywhere in the UI after submission.
    setSecret("");
    if (!value.trim() || /[\r\n\0]/.test(value)) {
      setInvalid(true);
      return;
    }
    setInvalid(false);
    save.mutate(
      { key: keyName, value },
      {
        onSuccess: () => {
          notify("success", m.saved, formatMessage(m.storedKey, { key: keyName }));
          close();
        }
      }
    );
  };

  return (
    <Dialog open={open} onOpenChange={(next) => (!next && !save.isPending ? close() : undefined)}>
      <DialogContent closeLabel={t.common.close}>
        <form onSubmit={submit} autoComplete="off" className="flex min-h-0 flex-1 flex-col">
          <DialogHeader>
            <DialogTitle>{formatMessage(m.setKeyTitle, { key: keyName })}</DialogTitle>
            <DialogDescription>{m.setKeyDescription}</DialogDescription>
          </DialogHeader>
          <DialogBody>
            <div className="space-y-3 py-1">
              <Field label={m.secretValue} required error={invalid ? m.secretInvalid : undefined}>
                <Input
                  type="password"
                  icon={<KeyRound />}
                  value={secret}
                  name={`secret-${keyName}`}
                  autoComplete="new-password"
                  spellCheck={false}
                  onChange={(e) => setSecret(e.target.value)}
                />
              </Field>
            </div>
          </DialogBody>
          <DialogFooter>
              <Button variant="secondary" disabled={save.isPending} onClick={close}>
                {t.common.cancel}
              </Button>
              <Button
                variant="default"
                type="submit"
                disabled={secret.length === 0 || save.isPending}
              >
                {save.isPending ? <Spinner className="text-current" /> : null}
                {save.isPending ? m.storing : m.store}
              </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
};

/** One vault key: configured state plus Set/Replace and Remove (with confirmation). */
export const CredentialRow: React.FC<{
  keyName: string;
  configured: boolean;
  description?: string;
  className?: string;
}> = ({ keyName, configured, description, className }) => {
  const m = useMessages(providerMessages);
  const notify = useNotify();
  const remove = useDeleteCredential();
  const [setOpen, setSetOpen] = useState(false);
  const [confirmOpen, setConfirmOpen] = useState(false);

  return (
    <div className={cn("flex flex-wrap items-center gap-2", className)}>
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <code className="font-mono text-xs text-foreground">{keyName}</code>
          <Badge tone={configured ? "success" : "neutral"}>
            {configured ? m.configured : m.notConfigured}
          </Badge>
        </div>
        {description ? <p className="mt-0.5 text-xs text-muted-foreground">{description}</p> : null}
      </div>
      <Button size="sm" onClick={() => setSetOpen(true)} aria-label={`${configured ? m.replaceKey : m.setKey} ${keyName}`}>
        {configured ? m.replaceKey : m.setKey}
      </Button>
      {configured ? (
        <Button
          size="sm"
          variant="subtle"
          disabled={remove.isPending}
          onClick={() => setConfirmOpen(true)}
          aria-label={`${m.remove} ${keyName}`}
        >
          {m.remove}
        </Button>
      ) : null}

      {setOpen ? <SetCredentialDialog keyName={keyName} open={setOpen} onOpenChange={setSetOpen} /> : null}
      <ConfirmDialog
        open={confirmOpen}
        onOpenChange={setConfirmOpen}
        title={formatMessage(m.removeTitle, { key: keyName })}
        description={m.removeDescription}
        confirmLabel={m.remove}
        destructive
        pending={remove.isPending}
        onConfirm={() =>
          remove.mutate(keyName, {
            onSuccess: () => {
              notify("success", m.saved, formatMessage(m.removedKey, { key: keyName }));
              setConfirmOpen(false);
            }
          })
        }
      />
    </div>
  );
};
