import React from "react";
import { Button, Dialog, DialogBody, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@symphony/ui";
import { useMessages, useT } from "../../lib/i18n.js";
import { useUiStore } from "../../store/uiStore.js";
import { shortcutsMessages } from "./messages.js";

type ItemKey = keyof (typeof shortcutsMessages)["en"]["items"];

/** Rows of the help dialog; the key combinations (alternatives separated) mirror `useGlobalShortcuts`. */
const GROUPS: ReadonlyArray<{ group: "general" | "layout"; items: ReadonlyArray<{ item: ItemKey; combos: string[][] }> }> = [
  {
    group: "general",
    items: [
      { item: "palette", combos: [["Ctrl", "K"]] },
      { item: "newTask", combos: [["Ctrl", "N"]] },
      { item: "projects", combos: [["Ctrl", "P"]] },
      { item: "refresh", combos: [["Ctrl", "R"]] },
      { item: "help", combos: [["F1"], ["Ctrl", "/"]] }
    ]
  },
  {
    group: "layout",
    items: [
      { item: "sidebar", combos: [["Ctrl", "B"]] },
      { item: "activity", combos: [["Ctrl", "J"]] }
    ]
  }
];

/** Keyboard shortcut help: F1, Ctrl+/ , the command palette or Settings → Application. */
export const ShortcutsDialog: React.FC = () => {
  const m = useMessages(shortcutsMessages);
  const t = useT();
  const open = useUiStore((s) => s.shortcutsOpen);
  const setOpen = useUiStore((s) => s.setShortcutsOpen);
  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogContent aria-label={m.title} showClose={false}>
        <DialogHeader>
          <DialogTitle>{m.title}</DialogTitle>
          <DialogDescription>{m.description}</DialogDescription>
        </DialogHeader>
        <DialogBody>
          <div className="space-y-4 pb-2">
            {GROUPS.map(({ group, items }) => (
              <section key={group} aria-label={m.groups[group]}>
                <h3 className="mb-1 text-xs font-semibold uppercase tracking-wide text-muted-foreground">{m.groups[group]}</h3>
                <ul className="divide-y divide-border rounded-lg border border-border">
                  {items.map(({ item, combos }) => (
                    <li key={item} className="flex items-center justify-between gap-4 px-3 py-1.5 text-sm">
                      <span>{m.items[item]}</span>
                      <span className="flex shrink-0 items-center gap-1">
                        {combos.map((keys, index) => (
                          <React.Fragment key={keys.join("+")}>
                            {index > 0 ? <span className="px-0.5 text-xs text-muted-foreground">{m.or}</span> : null}
                            {keys.map((key) => (
                              <kbd key={key} className="rounded border border-border bg-surface px-1.5 font-mono text-xs">
                                {key}
                              </kbd>
                            ))}
                          </React.Fragment>
                        ))}
                      </span>
                    </li>
                  ))}
                </ul>
              </section>
            ))}
          </div>
        </DialogBody>
        <DialogFooter>
          <Button variant="default" onClick={() => setOpen(false)}>
            {t.common.close}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
};
