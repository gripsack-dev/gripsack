/** Assertion helper for the structured diagnostic transport (A1-06):
 *  authoring failures are DiagnosticError values whose labels carry
 *  the source spans — tests assert the code and the labeled facts,
 *  not message wording. `asDiagnostic` is the same extraction the
 *  driver's envelope path uses. */

import assert from "node:assert/strict";
import { asDiagnostic, type FrontendDiagnostic } from "../src/diagnostic.ts";

export function thrownDiagnostic(
  fn: () => unknown,
  code: string,
): FrontendDiagnostic {
  try {
    fn();
  } catch (error) {
    const diagnostic = asDiagnostic(error);
    assert.ok(diagnostic, `expected a structured ${code} diagnostic, got: ${error}`);
    assert.equal(diagnostic.code, code);
    assert.equal(diagnostic.severity, "error");
    return diagnostic;
  }
  assert.fail("expected the declaration to be rejected");
}
