/**
 * Pi Durable (`@earendil-works/pi-durable`) as a Symphony cloud agent.
 *
 * Every model turn, tool call and result is committed to a SQLite file outside the repository
 * before it is acted on, so a sandbox that was stopped or crashed mid-run resumes from those
 * checkpoints: `harness.resume()` re-runs only what was interrupted (calls that are not
 * replay-safe are reported to the model as interrupted instead of being run twice).
 *
 * The coding tools are Pi Durable's own (`read`, `write`, `edit`, `bash`). A hook after each tool
 * commits and pushes what it changed, so every edit reaches the task branch.
 */
import { BACKGROUND_CONTEXT as ctx } from "@earendil-works/chord/context";
import { createModels } from "@earendil-works/pi-ai/models";
import { anthropicProvider } from "@earendil-works/pi-ai/providers/anthropic";
import { openaiProvider } from "@earendil-works/pi-ai/providers/openai";
import {
  createRegistry,
  defineExtension,
  GenerationTask,
  Harness,
  hook,
  ToolTask,
} from "@earendil-works/pi-durable";
import { NodeExecutionEnv } from "@earendil-works/pi-durable/env/node";
import { openNodeSqliteStorage } from "@earendil-works/pi-durable/storage/sqlite/node";
import { CodingTools } from "@earendil-works/pi-durable/tools";
import { mkdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { autocommit, clip, config, emit } from "./lib.ts";

const DEFAULT_MODEL = "anthropic/claude-sonnet-5-5";
const MUTATING_TOOLS = new Set(["bash", "write", "edit"]);

const INSTRUCTIONS = [
  `You are working in a git repository on branch \`${config.branch}\`, on the task ${config.issue}.`,
  "Use the tools to read and change files and to run commands. Every change you make is committed and pushed automatically after each tool call, so you never need to run git commit or git push yourself.",
  "Work until the task is done, verify it, then reply with a short summary of what you changed.",
].join("\n");

function modelRef(): { provider: string; modelId: string } {
  const spec = config.model ?? DEFAULT_MODEL;
  const slash = spec.indexOf("/");
  if (slash <= 0) throw new Error(`model must be "provider/model-id", got "${spec}"`);
  return { provider: spec.slice(0, slash), modelId: spec.slice(slash + 1) };
}

/** Text of a tool result, whatever shape the content parts have. */
function resultText(result: unknown): string {
  const content = (result as { content?: unknown } | undefined)?.content;
  if (!Array.isArray(content)) return "";
  return content
    .map((part) => (part && typeof part === "object" && "text" in part ? String((part as { text: unknown }).text) : ""))
    .join("");
}

const reporting = defineExtension({
  name: "symphony-reporting",
  hooks: [
    // Assistant text goes to the timeline as soon as the provider message is complete.
    hook(GenerationTask, {
      afterResponse(message) {
        const text = (message.content ?? [])
          .filter((part: { type: string }) => part.type === "text")
          .map((part: { type: string; text?: string }) => part.text ?? "")
          .join("\n")
          .trim();
        if (text) emit({ type: "output", text: clip(text) });
      },
    }),
    // After a tool ran: report it, and push what it changed.
    hook(ToolTask, {
      async afterTool(call, result) {
        const failed = (result as { isError?: boolean }).isError === true;
        const args = (call.arguments ?? {}) as Record<string, unknown>;
        emit({
          type: "tool",
          callId: call.id,
          name: call.name,
          kind: call.name === "bash" ? "command" : call.name === "read" ? "provider_tool" : "file_change",
          status: failed ? "failed" : "succeeded",
          input: args,
          output: clip(resultText(result)),
        });
        if (!failed && MUTATING_TOOLS.has(call.name)) {
          const path = typeof args.path === "string" ? ` ${args.path}` : "";
          await autocommit(`${config.issue}: ${call.name}${path}`);
        }
        return undefined;
      },
    }),
  ],
});

/** Input and output tokens the harness has used so far, summed over its models (all turns, persisted). */
async function tokenTotals(harness: Harness): Promise<{ input: number; output: number }> {
  const usage = (await harness.usage(ctx)) as { models?: Record<string, { input?: number; output?: number }> };
  let input = 0;
  let output = 0;
  for (const bucket of Object.values(usage.models ?? {})) {
    input += Number(bucket.input ?? 0);
    output += Number(bucket.output ?? 0);
  }
  return { input, output };
}

/** Runs the turn; `true` when the model answered. */
export async function runPiDurableAgent(): Promise<boolean> {
  const prompt = readFileSync(config.promptFile, "utf8");
  const ref = modelRef();

  const models = createModels();
  if (ref.provider === "anthropic") models.setProvider(anthropicProvider());
  else if (ref.provider === "openai") models.setProvider(openaiProvider());
  else throw new Error(`Pi Durable provider "${ref.provider}" is not set up in this template (anthropic, openai)`);

  const registry = createRegistry();
  registry.install(CodingTools);
  registry.install(reporting);

  mkdirSync(config.stateDir, { recursive: true });
  const harness = await Harness.open(
    await openNodeSqliteStorage(join(config.stateDir, "pi-durable.sqlite")),
    {
      models,
      registry,
      env: (target) => new NodeExecutionEnv({ cwd: target.cwd ?? config.repoDir }),
    },
    ctx,
  );

  try {
    // Initial settings apply only when the root conversation is first created: a resumed sandbox
    // keeps the conversation it had.
    const root = await harness.root(ctx, {
      agent: {
        model: ref,
        instructions: INSTRUCTIONS,
        cwd: config.repoDir,
        extensions: [CodingTools, reporting],
      },
    });
    // The harness total covers every turn this sandbox ever ran, but the bridge reports the usage of
    // this turn only (Symphony adds it to the task's totals), so measure from here. Work an earlier,
    // interrupted process had already paid for is not counted again; at worst it is not counted at all.
    const usageAtStart = await tokenTotals(harness);
    // Recover tasks an interruption left unfinished (a model request, a tool call).
    harness.resume();

    // The same request id returns the same submission, so a rerun of an interrupted turn waits for
    // the recovered run instead of asking the model again. A submission that already ended, answered
    // ("done") or not ("unanswered"), belongs to an earlier run: this prompt is new work and gets a
    // new id, otherwise a retry would just replay the earlier outcome.
    const base = `${config.taskId}-${config.turnId}`;
    let submission = await root.submit({ type: "input", content: prompt, requestId: base }, ctx);
    const existing = (await submission.status(ctx)).status;
    if (existing === "done" || existing === "unanswered") {
      submission = await root.submit({ type: "input", content: prompt, requestId: `${base}-${Date.now()}` }, ctx);
    }
    const settled = await submission.wait(ctx);

    const usageNow = await tokenTotals(harness);
    emit({
      type: "usage",
      inputTokens: Math.max(0, usageNow.input - usageAtStart.input),
      outputTokens: Math.max(0, usageNow.output - usageAtStart.output),
    });

    return settled.status === "done";
  } finally {
    await harness.close(ctx);
  }
}
