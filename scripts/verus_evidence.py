"""Admit actual pinned Verus JSON, never text fragments from unrelated errors."""
from dataclasses import dataclass
import json
from pathlib import Path
import re


class EvidenceError(ValueError):
    pass


@dataclass(frozen=True)
class Evidence:
    results: dict
    functions: dict[str, bool]
    diagnostics: tuple[dict, ...]

    @classmethod
    def parse(cls, output: str) -> "Evidence":
        reports, diagnostics = [], []
        decoder = json.JSONDecoder()
        for start in re.finditer(r"(?m)^\{", output):
            try:
                value, _ = decoder.raw_decode(output, start.start())
            except json.JSONDecodeError as error:
                raise EvidenceError("truncated or malformed verifier JSON") from error
            if not isinstance(value, dict):
                continue
            if value.get("reason") == "compiler-message":
                if value.get("target", {}).get("name") == "gripsack_policy":
                    diagnostics.append(value["message"])
            if "verification-results" in value and any(
                name.startswith("gripsack_policy::") for name in value.get("func-details", {})
            ):
                reports.append(value)
        if len(reports) != 1:
            raise EvidenceError("expected exactly one fresh gripsack_policy verifier report")
        report = reports[0]
        results = report["verification-results"]
        if results.get("is-verifying-entire-crate") is not True:
            raise EvidenceError("subset verification cannot qualify the production crate")
        if results.get("encountered-vir-error") is not False:
            raise EvidenceError("a front-end/type/translation error is not a failed proof")
        functions = {}
        modules = report.get("times-ms", {}).get("smt", {}).get("smt-run-module-times", [])
        for module in modules:
            for function in module.get("function-breakdown", []):
                name = function["function"]
                if not name.startswith("gripsack_policy::"):
                    raise EvidenceError("foreign function in production verification report")
                if type(function.get("success")) is not bool:
                    raise EvidenceError("function has no actual solver verdict")
                name = name.removeprefix("gripsack_policy::")
                functions[name] = functions.get(name, True) and function["success"]
        if not functions:
            raise EvidenceError("no production function obligations executed")
        return cls(results, functions, tuple(diagnostics))

    def positive(self, returncode: int, minimum: int, families: dict[str, tuple[str, ...]]) -> None:
        if (returncode or self.results.get("success") is not True
                or self.results.get("encountered-error") is not False
                or self.results.get("errors") != 0
                or type(self.results.get("verified")) is not int
                or self.results["verified"] < minimum):
            raise EvidenceError("positive proof failed or fell below its obligation floor")
        for family, required in families.items():
            missing = [name for name in required if self.functions.get(name) is not True]
            if missing:
                raise EvidenceError(f"{family}: unverified named production functions: {missing}")

    def mutant(self, returncode: int, source: Path, function: str, workspace: Path) -> None:
        failed = {name for name, success in self.functions.items() if not success}
        if (returncode == 0 or self.results.get("success") is not False
                or self.results.get("errors", 0) < 1 or self.results.get("verified", 0) < 1):
            raise EvidenceError("mutant did not reach a nonempty failed verification")
        if failed != {function}:
            raise EvidenceError(f"wrong failing functions: {sorted(failed)}; expected {function}")
        admitted = []
        for diagnostic in self.diagnostics:
            if diagnostic.get("level") != "error":
                continue
            message = diagnostic.get("message", "")
            if message.startswith("aborting due to "):
                continue
            spans = diagnostic.get("spans", [])
            # Both file and proof-failure kind must belong to THIS diagnostic,
            # not independent greps joined from unrelated errors/warnings.
            proof_failure = "not satisfied" in message or message == "assertion failed"
            exact_file = any(
                (workspace / span["file_name"]).resolve() == source.resolve()
                for span in spans if span.get("is_primary")
            )
            if not proof_failure or not exact_file:
                raise EvidenceError(f"unattributed verifier error: {message}")
            admitted.append(diagnostic)
        if not admitted:
            raise EvidenceError("no named source diagnostic for the failed production contract")


def self_check() -> None:
    """Gate consumers must reject forged coverage and split diagnostic evidence."""
    good = Evidence(
        {"success": True, "encountered-error": False, "errors": 0, "verified": 72},
        {"ownership::plan_copy": True}, (),
    )
    good.positive(0, 72, {"ownership": ("ownership::plan_copy",)})
    workspace = Path(__file__).resolve().parent.parent
    source = workspace / "crates/gripsack-policy/src/lib.rs"
    failure = {"success": False, "errors": 1, "verified": 71}
    target_span = {"file_name": "crates/gripsack-policy/src/lib.rs", "is_primary": True}
    foreign_span = {"file_name": "crates/gripsack-policy/src/graph.rs", "is_primary": True}
    target_error = {"level": "error", "message": "postcondition not satisfied", "spans": [target_span]}

    def attribution(functions, diagnostics, status=1):
        Evidence(failure, functions, tuple(diagnostics)).mutant(status, source, "classify", workspace)

    attribution({"classify": False}, [target_error])
    bad_cases = (
        ("missing family", lambda: good.positive(0, 72, {"retention": ("retention::plan_delete",)})),
        ("zero/fewer obligations", lambda: good.positive(0, 73, {})),
        ("failed process", lambda: good.positive(1, 72, {})),
        ("missing report", lambda: Evidence.parse("verification results:: 72 verified, 0 errors")),
        ("truncated JSON", lambda: Evidence.parse('{"verification-results":')),
        ("unrelated function in target file", lambda: attribution(
            {"classify": True, "unrelated_lemma": False}, [target_error])),
        ("warning and foreign proof error", lambda: attribution(
            {"classify": False}, [
                {**target_error, "level": "warning"},
                {**target_error, "spans": [foreign_span]},
            ])),
        ("secondary target span only", lambda: attribution(
            {"classify": False}, [{**target_error, "spans": [
                {**target_span, "is_primary": False}, foreign_span,
            ]}])),
        ("unrelated compiler failure", lambda: attribution(
            {"classify": False}, [{**target_error, "message": "cannot find value"}])),
        ("successful mutant process", lambda: attribution(
            {"classify": False}, [target_error], 0)),
    )
    for name, check in bad_cases:
        try:
            check()
        except EvidenceError:
            continue
        raise EvidenceError(f"gate calibration accepted {name}")
    print(f"VERUS_GATE_NEGATIVES={len(bad_cases)}", flush=True)
