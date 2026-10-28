import { useMemo } from "react";
import { LogModeSwitch, PageHeader, Tabs, TabsContent, TabsList, TabsTrigger, cn } from "@symphony/ui";
import { shellMessages } from "../../app/messages.js";
import { useMessages } from "../../lib/i18n.js";
import { useApprovals, useRecentEvents } from "../../lib/queries.js";
import { mergeEventHistory, useEventStore } from "../../runtime/eventStore.js";
import { useUiStore, type DrawerTab } from "../../store/uiStore.js";
import {
  AGENT_KINDS,
  ApprovalsTab,
  EventFeed,
  FocusedFeed,
  ProblemsTab,
  TERMINAL_KINDS,
  ValidationsTab
} from "./ActivityPanels.js";

/** Global activity: live runtime events and the cross-task approval, validation and problem lists. */
export default function ActivityPage() {
  const m = useMessages(shellMessages);
  const tab = useUiStore((s) => s.drawerTab);
  const setTab = useUiStore((s) => s.setDrawerTab);
  const logMode = useUiStore((s) => s.logMode);
  const setLogMode = useUiStore((s) => s.setLogMode);
  const live = useEventStore((s) => s.events);
  // Persisted history (survives reloads) merged with the live stream, deduplicated by id.
  const history = useRecentEvents(200);
  const events = useMemo(() => mergeEventHistory(history.data, live), [history.data, live]);
  const approvals = useApprovals(true);
  const pending = approvals.data?.length ?? 0;

  const tabs: { key: DrawerTab; label: string; badge?: number }[] = [
    { key: "events", label: m.drawer.events },
    { key: "agent", label: m.drawer.agent },
    { key: "terminal", label: m.drawer.terminal },
    { key: "validations", label: m.drawer.validations },
    { key: "approvals", label: m.drawer.approvals, badge: pending },
    { key: "problems", label: m.drawer.problems }
  ];
  const showMode = tab === "events" || tab === "agent" || tab === "terminal";

  return (
    <section aria-label={m.drawer.label} className="flex h-full min-h-0 flex-col">
      <PageHeader title={m.drawer.label} actions={showMode ? <LogModeSwitch value={logMode} onChange={setLogMode} /> : null} />
      <Tabs value={tab} onValueChange={(v) => setTab(v as DrawerTab)} className="min-h-0 flex-1">
        <TabsList aria-label={m.drawer.label}>
          {tabs.map((x) => (
            <TabsTrigger key={x.key} value={x.key}>
              {x.label}
              {x.badge ? (
                <span className={cn("inline-flex h-4 min-w-4 items-center justify-center rounded-full bg-warning px-1 text-[10px] font-semibold text-background")}>{x.badge}</span>
              ) : null}
            </TabsTrigger>
          ))}
        </TabsList>
        <TabsContent value="events">
          <EventFeed events={events} showIdentifier emptyTitle={m.drawer.noEvents} emptyDetail={m.drawer.noEventsDetail} />
        </TabsContent>
        <TabsContent value="agent">
          <FocusedFeed kinds={AGENT_KINDS} emptyTitle={m.drawer.noEvents} />
        </TabsContent>
        <TabsContent value="terminal">
          <FocusedFeed kinds={TERMINAL_KINDS} emptyTitle={m.drawer.noCommands} />
        </TabsContent>
        <TabsContent value="validations">
          <ValidationsTab />
        </TabsContent>
        <TabsContent value="approvals">
          <ApprovalsTab />
        </TabsContent>
        <TabsContent value="problems">
          <ProblemsTab />
        </TabsContent>
      </Tabs>
    </section>
  );
}
