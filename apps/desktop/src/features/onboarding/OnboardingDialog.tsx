import { useRef, useState } from "react";
import { Button, Dialog, DialogActions, DialogBody, DialogContent, DialogSurface, DialogTitle, Spinner } from "@fluentui/react-components";
import { CheckCircle2, CircleDashed, FolderGit2 } from "lucide-react";
import { errorMessage } from "@symphony/api-client";
import { formatMessage } from "@symphony/shared-types";
import { ErrorState, LoadingRows, useNotify } from "@symphony/ui";
import { useMessages, useT } from "../../lib/i18n.js";
import { useProjects, useProviders, useSaveSettings, useSettings } from "../../lib/queries.js";
import { useUiStore } from "../../store/uiStore.js";
import { onboardingMessages } from "./messages.js";

const STEPS = ["welcome", "providers", "repository", "finish"] as const;
type Step = (typeof STEPS)[number];

/**
 * First-run guide, shown while `settings.onboarding_completed` is false. Finishing or skipping
 * persists the flag in runtime settings, which unmounts the dialog.
 */
export default function OnboardingDialog() {
  const m = useMessages(onboardingMessages);
  const t = useT();
  const notify = useNotify();
  const settings = useSettings();
  const providers = useProviders();
  const projects = useProjects();
  const save = useSaveSettings();
  const setAddProjectOpen = useUiStore((s) => s.setAddProjectOpen);
  const [step, setStep] = useState<Step>("welcome");
  const index = STEPS.indexOf(step);

  // Synchronous guard so a double click cannot submit twice before `isPending` renders.
  const submitting = useRef(false);
  const complete = () => {
    if (!settings.data || save.isPending || submitting.current) return;
    submitting.current = true;
    save.mutate(
      (latest) => ({ ...latest, onboarding_completed: true }),
      {
        onError: (error) => notify("error", m.saveFailed, errorMessage(error)),
        onSettled: () => {
          submitting.current = false;
        }
      }
    );
  };

  const detected = providers.data ?? [];
  const installedAgents = detected.filter((p) => p.provider !== "simulated");

  return (
    // The Add repository dialog opens on top of this one (nested Fluent dialogs), so progress
    // through the steps is kept while the user registers a repository.
    <Dialog open modalType="alert">
      <DialogSurface aria-describedby={undefined} className="max-w-xl">
        <DialogBody>
          <DialogTitle>
            <span className="block text-xs font-normal text-muted-foreground">
              {formatMessage(m.stepOf, { step: index + 1, total: STEPS.length })} · {m.steps[step]}
            </span>
            {m.title}
          </DialogTitle>
          <DialogContent className="space-y-3 text-sm">
            <ol className="flex gap-1" aria-hidden>
              {STEPS.map((s, i) => (
                <li key={s} className={`h-1 flex-1 rounded-full ${i <= index ? "bg-accent" : "bg-muted"}`} />
              ))}
            </ol>

            {step === "welcome" ? (
              <section className="space-y-2">
                <h2 className="font-display text-base font-semibold text-foreground">{m.welcome.heading}</h2>
                <p className="text-muted-foreground">{m.welcome.body}</p>
                <ul className="space-y-1">
                  {m.welcome.points.map((point) => (
                    <li key={point} className="flex gap-2 text-foreground">
                      <CheckCircle2 className="mt-0.5 size-4 shrink-0 text-success" aria-hidden />
                      {point}
                    </li>
                  ))}
                </ul>
              </section>
            ) : null}

            {step === "providers" ? (
              <section className="space-y-2">
                <h2 className="font-display text-base font-semibold text-foreground">{m.providers.heading}</h2>
                <p className="text-muted-foreground">{m.providers.body}</p>
                {providers.isLoading ? (
                  <LoadingRows rows={3} label={m.providers.loading} />
                ) : providers.isError ? (
                  <ErrorState title={m.providers.failed} message={errorMessage(providers.error)} onRetry={() => void providers.refetch()} retryLabel={t.common.retry} />
                ) : (
                  <ul className="divide-y divide-border rounded-md border border-border">
                    {installedAgents.map((p) => (
                      <li key={p.provider} className="flex items-center gap-2 px-3 py-2">
                        {p.installed ? (
                          <CheckCircle2 className="size-4 text-success" aria-hidden />
                        ) : (
                          <CircleDashed className="size-4 text-muted-foreground" aria-hidden />
                        )}
                        <span className="flex-1 text-foreground">{t.provider[p.provider]}</span>
                        <span className="text-xs text-muted-foreground">
                          {p.installed ? `${m.providers.installed}${p.version ? ` · ${p.version}` : ""} · ${t.authState[p.auth_state]}` : m.providers.missing}
                        </span>
                      </li>
                    ))}
                  </ul>
                )}
                {!providers.isLoading && !providers.isError && !installedAgents.some((p) => p.installed) ? (
                  <p className="text-xs text-muted-foreground">{m.providers.none}</p>
                ) : null}
              </section>
            ) : null}

            {step === "repository" ? (
              <section className="space-y-2">
                <h2 className="font-display text-base font-semibold text-foreground">{m.repository.heading}</h2>
                <p className="text-muted-foreground">{m.repository.body}</p>
                <Button appearance="primary" icon={<FolderGit2 className="size-4" />} onClick={() => setAddProjectOpen(true)}>
                  {m.repository.add}
                </Button>
                <div>
                  <h3 className="mb-1 text-xs font-medium text-muted-foreground">{m.repository.registered}</h3>
                  {(projects.data ?? []).length === 0 ? (
                    <p className="text-xs text-muted-foreground">{m.repository.none}</p>
                  ) : (
                    <ul className="space-y-1">
                      {(projects.data ?? []).map((p) => (
                        <li key={p.id} className="flex items-center gap-2">
                          <CheckCircle2 className="size-4 text-success" aria-hidden />
                          <span className="text-foreground">{p.name}</span>
                          <code className="truncate font-mono text-xs text-muted-foreground" title={p.repo_path}>
                            {p.repo_path}
                          </code>
                        </li>
                      ))}
                    </ul>
                  )}
                </div>
                <p className="text-xs text-muted-foreground">{m.repository.later}</p>
              </section>
            ) : null}

            {step === "finish" ? (
              <section className="space-y-2">
                <h2 className="font-display text-base font-semibold text-foreground">{m.finish.heading}</h2>
                <p className="text-muted-foreground">{m.finish.body}</p>
              </section>
            ) : null}
          </DialogContent>
          <DialogActions fluid className="justify-between">
            <Button appearance="subtle" disabled={save.isPending} onClick={complete}>
              {m.actions.skip}
            </Button>
            <span className="flex gap-2">
              {index > 0 ? (
                <Button disabled={save.isPending} onClick={() => setStep(STEPS[index - 1]!)}>
                  {m.actions.back}
                </Button>
              ) : null}
              {step === "finish" ? (
                <Button appearance="primary" disabled={save.isPending || !settings.data} icon={save.isPending ? <Spinner size="tiny" /> : undefined} onClick={complete}>
                  {save.isPending ? m.actions.finishing : m.actions.finish}
                </Button>
              ) : (
                <Button appearance="primary" onClick={() => setStep(STEPS[index + 1]!)}>
                  {m.actions.next}
                </Button>
              )}
            </span>
          </DialogActions>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  );
}
