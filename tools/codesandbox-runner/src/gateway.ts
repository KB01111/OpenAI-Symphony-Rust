import { CodeSandbox, VMTier, type Sandbox, type SandboxClient } from "@codesandbox/sdk";
import type { GitCredentials, VmTierName } from "./protocol.ts";

/** A command running in the sandbox. */
export interface RunningCommand {
  onOutput(listener: (chunk: string) => void): void;
  /** Resolves with the exit code; a non-zero exit is a result, not an error. */
  wait(): Promise<number>;
  kill(): Promise<void>;
}

/** What the bridge needs from a connected sandbox. */
export interface SandboxHandle {
  readonly id: string;
  /** `true` when an existing sandbox was resumed rather than created from the template. */
  readonly resumed: boolean;
  readonly workspacePath: string;
  writeTextFile(path: string, content: string): Promise<void>;
  run(command: string, options: { cwd?: string; env?: Record<string, string>; name?: string }): Promise<RunningCommand>;
  hibernate(): Promise<void>;
  shutdown(): Promise<void>;
  dispose(): void;
}

export interface OpenOptions {
  apiKey: string;
  templateId: string;
  /** Resume this sandbox; a sandbox that no longer exists is replaced by a new one. */
  sandboxId?: string | null;
  vmTier: VmTierName;
  hibernationTimeoutSecs: number;
  title: string;
  git: GitCredentials;
  env: Record<string, string>;
}

/** The CodeSandbox SDK as the bridge uses it; tests replace it. */
export interface SandboxGateway {
  open(options: OpenOptions): Promise<SandboxHandle>;
  /** Connect to an existing sandbox for `abort` / `stop`; `null` when it does not exist. */
  attach(apiKey: string, sandboxId: string): Promise<SandboxHandle | null>;
}

const TIERS: Record<VmTierName, VMTier> = {
  pico: VMTier.Pico,
  nano: VMTier.Nano,
  micro: VMTier.Micro,
  small: VMTier.Small,
  medium: VMTier.Medium,
  large: VMTier.Large,
  xlarge: VMTier.XLarge,
};

/**
 * Whether a resume / connect failure means the sandbox is gone (deleted or expired), as opposed to a
 * transient or permission problem. The SDK reports an HTTP 404 as `<operation>: Sandbox not found`;
 * everything else (`Unauthorized`, `Bad gateway`, rate limits, network errors) says something else.
 */
export function isSandboxMissing(error: unknown): boolean {
  return /sandbox not found/i.test(error instanceof Error ? error.message : String(error));
}

/** Workspace folder of CodeSandbox sandboxes when the client does not report one. */
const FALLBACK_WORKSPACE = "/project/sandbox";

/** The SDK adapter. Exported for the tests, which give it a fake SDK client. */
export class SdkHandle implements SandboxHandle {
  constructor(
    private readonly sdk: CodeSandbox,
    private readonly sandbox: Sandbox,
    private readonly client: SandboxClient,
    readonly resumed: boolean,
  ) {}

  get id(): string {
    return this.sandbox.id;
  }

  get workspacePath(): string {
    return this.client.workspacePath || FALLBACK_WORKSPACE;
  }

  async writeTextFile(path: string, content: string): Promise<void> {
    await this.client.fs.writeTextFile(path, content, { create: true, overwrite: true });
  }

  async run(
    command: string,
    options: { cwd?: string; env?: Record<string, string>; name?: string },
  ): Promise<RunningCommand> {
    const cmd = await this.client.commands.runBackground(command, options);
    let listener: ((chunk: string) => void) | undefined;
    // Output that arrives before the caller has subscribed is held and handed over, first, when it
    // does: the caller sees every chunk the SDK delivered, once, in the order it was delivered.
    // (The SDK's own record of the output, from `waitUntilComplete`, joins chunks with newlines of
    // its own and keeps only the last thousand, so it cannot be spliced into this stream.)
    let early: string[] = [];
    cmd.onOutput((chunk) => {
      if (listener) listener(chunk);
      else early.push(chunk);
    });
    return {
      onOutput: (l) => {
        listener = l;
        const held = early;
        early = [];
        for (const chunk of held) l(chunk);
      },
      wait: async () => {
        try {
          await cmd.waitUntilComplete();
          return 0;
        } catch (error) {
          const exitCode = (error as { exitCode?: unknown }).exitCode;
          if (typeof exitCode === "number") return exitCode;
          throw error;
        }
      },
      kill: () => cmd.kill(),
    };
  }

  async hibernate(): Promise<void> {
    await this.sdk.sandboxes.hibernate(this.sandbox.id);
  }

  async shutdown(): Promise<void> {
    await this.sdk.sandboxes.shutdown(this.sandbox.id);
  }

  dispose(): void {
    this.client.dispose();
  }
}

export class CodeSandboxGateway implements SandboxGateway {
  async open(options: OpenOptions): Promise<SandboxHandle> {
    const sdk = new CodeSandbox(options.apiKey);
    let sandbox: Sandbox | undefined;
    let resumed = false;
    if (options.sandboxId) {
      try {
        sandbox = await sdk.sandboxes.resume(options.sandboxId);
        resumed = true;
      } catch (error) {
        // Only a sandbox that no longer exists is replaced (the run then continues in a fresh one from
        // the pushed branch). A transient or permission failure must not strand the old sandbox, with
        // its checkpoint and unpushed edits, behind a new one: it is reported, and the retry resumes.
        if (!isSandboxMissing(error)) throw error;
        sandbox = undefined;
      }
    }
    if (!sandbox) {
      sandbox = await sdk.sandboxes.create({
        id: options.templateId,
        title: options.title,
        privacy: "private",
        tags: ["symphony"],
        vmTier: TIERS[options.vmTier],
        hibernationTimeoutSeconds: options.hibernationTimeoutSecs,
      });
    } else {
      await sandbox.updateHibernationTimeout(options.hibernationTimeoutSecs);
    }
    // git credentials and env go into the session: credentials in `$HOME/.private`, never in the
    // repository or on a command line.
    const client = await sandbox.connect({
      git: {
        provider: "github.com",
        username: "x-access-token",
        accessToken: options.git.token,
        email: options.git.email,
        name: options.git.name,
      },
      env: options.env,
    });
    return new SdkHandle(sdk, sandbox, client, resumed);
  }

  async attach(apiKey: string, sandboxId: string): Promise<SandboxHandle | null> {
    const sdk = new CodeSandbox(apiKey);
    try {
      const sandbox = await sdk.sandboxes.resume(sandboxId);
      const client = await sandbox.connect();
      return new SdkHandle(sdk, sandbox, client, true);
    } catch (error) {
      // Gone: nothing to stop. Anything else must not look like that, or an `abort` that could not
      // reach the sandbox would leave the agent running and pushing.
      if (isSandboxMissing(error)) return null;
      throw error;
    }
  }
}
