"""Strict admission of fresh TLAPS Toolbox proof results and source locations."""
from dataclasses import dataclass
from pathlib import Path
import re

import tlaps_source


class EvidenceError(ValueError):
    pass


@dataclass(frozen=True)
class Obligation:
    start: int
    end: int
    status: str
    prover: str
    reason: str
    already: str


@dataclass(frozen=True)
class Evidence:
    count: int
    obligations: dict[int, Obligation]

    @classmethod
    def parse(cls, output: str) -> 'Evidence':
        blocks = re.findall(r'(?ms)^@!!BEGIN\n(.*?)^@!!END\s*$', output)
        if output.count('@!!BEGIN') != len(blocks):
            raise EvidenceError('truncated TLAPS Toolbox block')
        count = None
        obligations = {}
        for block in blocks:
            fields = {}
            for line in block.splitlines():
                if line.startswith('@!!') and ':' in line:
                    key, value = line[3:].split(':', 1)
                    if key in fields:
                        raise EvidenceError('duplicate Toolbox field: ' + key)
                    fields[key] = value
            if fields.get('type') == 'obligationsnumber':
                if count is not None or not fields.get('count', '').isdigit():
                    raise EvidenceError('missing or repeated obligation count')
                count = int(fields['count'])
            elif fields.get('type') == 'obligation':
                try:
                    identity = int(fields['id'])
                    start, column, end, end_column = map(int, fields['loc'].split(':'))
                    if min(identity, start, column, end_column) < 1 or end < start:
                        raise ValueError('invalid proof location')
                    result = Obligation(start, end, fields['status'], fields.get('prover', ''),
                                        fields.get('reason', ''), fields.get('already', ''))
                except (KeyError, ValueError) as error:
                    raise EvidenceError('invalid obligation identity/location') from error
                previous = obligations.get(identity)
                if previous and (previous.start, previous.end) != (start, end):
                    raise EvidenceError('an obligation changed source location')
                obligations[identity] = result
        if count is None or count != len(obligations):
            raise EvidenceError('obligation inventory does not match its count')
        return cls(count, obligations)

    def positive(self, status: int, minimum: int, source: Path, names: tuple[str, ...]) -> None:
        if status or self.count < minimum:
            raise EvidenceError('positive prover exit or obligation floor failed')
        if any(item.status not in {'proved', 'trivial'} or item.already != 'false'
               for item in self.obligations.values()):
            raise EvidenceError('missing, failed, omitted or cached proof result')
        ranges = tlaps_source.theorem_ranges(source)
        for name in names:
            if name not in ranges:
                raise EvidenceError('required theorem missing: ' + name)
            start, end = ranges[name]
            if not any(start <= item.start <= item.end <= end for item in self.obligations.values()):
                raise EvidenceError('required theorem executed no obligations: ' + name)

    def mutant(self, status: int, source: Path, expected: str) -> None:
        if not status or not self.count:
            raise EvidenceError('mutant did not fail a nonempty proof run')
        ranges = tlaps_source.theorem_ranges(source)
        if expected not in ranges:
            raise EvidenceError('mutant theorem missing')
        start, end = ranges[expected]
        failures = [item for item in self.obligations.values() if item.status == 'failed']
        if not failures:
            raise EvidenceError('mutant has no actual failed obligation')
        for item in self.obligations.values():
            if item.status not in {'proved', 'trivial', 'failed'} or item.already != 'false':
                raise EvidenceError('incomplete or cached mutant proof run')
        for item in failures:
            if not (start <= item.start <= item.end <= end):
                raise EvidenceError('failure belongs to a different theorem')
            if item.reason != 'false' or not item.prover:
                raise EvidenceError('timeout/tool failure is not semantic calibration')

