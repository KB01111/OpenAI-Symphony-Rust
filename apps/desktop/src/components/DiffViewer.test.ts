import { describe, expect, it } from "vitest";
import { parseUnifiedDiff } from "./DiffViewer.js";

describe("parseUnifiedDiff", () => {
  it("keeps hunk content that looks like file headers", () => {
    const diff = [
      "diff --git a/notes.md b/notes.md",
      "--- a/notes.md",
      "+++ b/notes.md",
      "@@ -1,2 +1,2 @@",
      "--- old rule",
      "+++ new rule",
      " unchanged",
      "diff --git a/b.txt b/b.txt",
      "new file mode 100644",
      "--- /dev/null",
      "+++ b/b.txt",
      "@@ -0,0 +1 @@",
      "+hello"
    ].join("\n");
    const [first, second] = parseUnifiedDiff(diff);
    expect(first).toMatchObject({ path: "notes.md", additions: 1, deletions: 1 });
    expect(first!.lines).toEqual(["@@ -1,2 +1,2 @@", "--- old rule", "+++ new rule", " unchanged"]);
    expect(second).toMatchObject({ path: "b.txt", additions: 1, deletions: 0 });
  });
});
