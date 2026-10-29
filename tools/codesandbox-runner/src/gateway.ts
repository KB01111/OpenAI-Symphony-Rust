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

/** Workspace folder of CodeSandbox sandboxes when the client does not report one. */
const FALLBACK_WORKSPACE = "/project/sandbox";

class SdkHandle implements SandboxHandle {
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
    let listener: (chunk: string) => void = () => {};
    let received = "";
    cmd.onOutput((chunk) => {
      received += chunk;
      listener(chunk);
    });
    // The SDK only delivers output printed after the subscription; the full output it returns on
    // completion also holds what the command printed before it. Hand that over too.
    const replayMissed = (full: string) => {
      if (full.length > received.length && full.endsWith(received)) {
        listener(full.slice(0, full.length - received.length));
      }
    };
    return {
      onOutput: (l) => {
        listener = l;
      },
      wait: async () => {
        try {
          replayMissed(await cmd.waitUntilComplete());
          return 0;
        } catch (error) {
          const failure = error as { exitCode?: unknown; output?: unknown };
          if (typeof failure.exitCode === "number") {
            if (typeof failure.output === "string") replayMissed(failure.output);
            return failure.exitCode;
          }
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
      } catch {
        // Deleted or expired: the run continues in a fresh sandbox from the pushed branch.
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
    } catch {
      return null;
    }
  }
}
