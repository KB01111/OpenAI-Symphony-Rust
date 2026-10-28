import assert from "node:assert/strict";
import { test } from "node:test";
import { generatorTypeScriptProblem } from "./generate-api.mjs";

test("the API generator runs on TypeScript 6", () => {
  assert.equal(generatorTypeScriptProblem("6.0.3"), null);
  assert.equal(generatorTypeScriptProblem("6.1.0-beta"), null);
});

test("any other TypeScript major is refused with the reason", () => {
  for (const version of ["7.0.2", "5.9.3", "8.0.0"]) {
    const problem = generatorTypeScriptProblem(version);
    assert.match(problem, new RegExp(`TypeScript ${version.replace(/\./g, "\\.")}`));
    assert.match(problem, /TypeScript 6 compiler API/);
    assert.match(problem, /PRODUCTION_READINESS\.md/);
  }
});
