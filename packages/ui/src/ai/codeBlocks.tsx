import React, { useState } from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkBreaks from "remark-breaks";
import remarkGfm from "remark-gfm";
import { CheckIcon, CopyIcon } from "lucide-react";
import { cn } from "../lib/utils.js";
import { useUiStrings } from "../i18n.js";

/** Small icon button that copies text and confirms briefly. */
export const CopyButton: React.FC<{ value: string; label: string; className?: string }> = ({ value, label, className }) => {
  const [copied, setCopied] = useState(false);
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      onClick={() => {
        void navigator.clipboard?.writeText(value).then(() => {
          setCopied(true);
          window.setTimeout(() => setCopied(false), 1500);
        });
      }}
      className={cn(
        "inline-flex size-6 items-center justify-center rounded-md text-muted-foreground outline-none hover:bg-surface-hover hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/60",
        className
      )}
    >
      {copied ? <CheckIcon className="size-3.5 text-success" aria-hidden /> : <CopyIcon className="size-3.5" aria-hidden />}
    </button>
  );
};

/** Code block with a named copy button. Scrolls horizontally inside itself, never the page. */
export const CodeBlock: React.FC<{ children: string; language?: string; className?: string }> = ({ children, language, className }) => {
  const { ui } = useUiStrings();
  return (
    <div className={cn("group/code relative min-w-0 overflow-hidden rounded-lg border border-border bg-code", className)}>
      {language && language !== "text" ? (
        <span className="pointer-events-none absolute left-3 top-1.5 text-[10px] uppercase tracking-wide text-subtle-foreground">{language}</span>
      ) : null}
      <CopyButton value={children} label={ui.copyCode} className="absolute right-1.5 top-1.5 bg-code" />
      <pre
        tabIndex={0}
        className={cn("max-h-80 overflow-auto px-3 pb-2.5 font-mono text-xs leading-relaxed text-foreground", language && language !== "text" ? "pt-6" : "pt-2.5")}
      >
        <code>{children}</code>
      </pre>
    </div>
  );
};

const markdownComponents: Components = {
  pre: ({ children }) => {
    const props = React.isValidElement<{ children?: unknown; className?: unknown }>(children) ? children.props : null;
    const code = typeof props?.children === "string" ? props.children.replace(/\n$/, "") : "";
    const language = typeof props?.className === "string" ? (props.className.match(/language-(.*)$/)?.[1] ?? "text") : "text";
    return <CodeBlock language={language}>{code}</CodeBlock>;
  },
  code: ({ className, children, ...rest }) => (
    <code className={cn("rounded bg-muted px-1 py-0.5 font-mono text-[0.85em]", className)} {...rest}>
      {children}
    </code>
  ),
  a: ({ children, href }) => (
    <a href={href} target="_blank" rel="noreferrer noopener" className="text-accent underline-offset-2 hover:underline">
      {children}
    </a>
  ),
  ul: ({ children }) => <ul className="my-1.5 list-disc space-y-0.5 ps-5">{children}</ul>,
  ol: ({ children }) => <ol className="my-1.5 list-decimal space-y-0.5 ps-5">{children}</ol>,
  p: ({ children }) => <p className="my-1.5 first:mt-0 last:mb-0">{children}</p>,
  h1: ({ children }) => <h3 className="mb-1 mt-3 font-display text-base font-semibold">{children}</h3>,
  h2: ({ children }) => <h3 className="mb-1 mt-3 font-display text-base font-semibold">{children}</h3>,
  h3: ({ children }) => <h4 className="mb-1 mt-2 font-semibold">{children}</h4>,
  blockquote: ({ children }) => <blockquote className="my-2 border-s-2 border-border-strong ps-3 text-muted-foreground">{children}</blockquote>,
  table: ({ children }) => (
    <div className="my-2 overflow-x-auto">
      <table className="w-full text-sm">{children}</table>
    </div>
  ),
  th: ({ children }) => <th className="border-b border-border px-2 py-1 text-left font-medium">{children}</th>,
  td: ({ children }) => <td className="border-b border-border px-2 py-1">{children}</td>
};

export interface MarkdownProps {
  children: string;
  breakNewLines?: boolean;
  /** Accepted for call-site clarity; raw HTML is never rendered either way. */
  skipHtml?: boolean;
  className?: string;
}

/** Markdown (GFM) rendered with Symphony tokens. Raw HTML is never rendered. */
export const Markdown: React.FC<MarkdownProps> = ({ children, breakNewLines = false, className }) => (
  <div className={cn("min-w-0 break-words", className)}>
    <ReactMarkdown remarkPlugins={breakNewLines ? [remarkGfm, remarkBreaks] : [remarkGfm]} components={markdownComponents}>
      {children}
    </ReactMarkdown>
  </div>
);
