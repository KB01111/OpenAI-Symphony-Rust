import React from "react";
import { CopyButton } from "@openai/apps-sdk-ui/components/Button";
import { CodeBlockBase } from "@openai/apps-sdk-ui/components/CodeBlock";
import { Markdown as SdkMarkdownBase } from "@openai/apps-sdk-ui/components/Markdown";
import { useUiStrings } from "../i18n.js";

/**
 * The Apps SDK `CodeBlock` with a named copy button: the SDK's own button has an icon and no
 * accessible name, so screen readers announce an unlabelled "button". Same parts and look.
 */
export const CodeBlock: React.FC<{ children: string; language?: string; className?: string }> = ({ children, language, className }) => {
  const { ui } = useUiStrings();
  return (
    <CodeBlockBase className={className}>
      <CodeBlockBase.Code language={language}>{children}</CodeBlockBase.Code>
      <div className="absolute end-0 top-0 rounded-[inherit] bg-[var(--codeblock-background-color)] pe-2.5 pt-2.5">
        <CopyButton copyValue={children} variant="ghost" color="secondary" pill={false} size="md" uniform aria-label={ui.copyCode} />
      </div>
    </CodeBlockBase>
  );
};

/** Fenced code in Markdown, as the SDK parses it (`<pre><code class="language-x">`). */
const MarkdownPre: React.FC<{ children?: React.ReactNode; className?: string }> = ({ children, className }) => {
  const props = React.isValidElement<{ children?: unknown; className?: unknown }>(children) ? children.props : null;
  const code = typeof props?.children === "string" ? props.children.replace(/\n$/, "") : "";
  const language = typeof props?.className === "string" ? (props.className.match(/language-(.*)$/)?.[1] ?? "text") : "text";
  return (
    <CodeBlock language={language} className={className}>
      {code}
    </CodeBlock>
  );
};

type SdkMarkdownProps = React.ComponentProps<typeof SdkMarkdownBase>;

/** The Apps SDK `Markdown` whose copyable code blocks use the named copy button above. */
export const Markdown: React.FC<SdkMarkdownProps> = ({ components, copyableCodeBlocks = true, ...props }) => (
  <SdkMarkdownBase
    {...props}
    copyableCodeBlocks={copyableCodeBlocks}
    components={copyableCodeBlocks ? { pre: MarkdownPre, ...components } : components}
  />
);
