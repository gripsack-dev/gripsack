"""Frozen theorem floors and complete local-module dependency admission."""
from dataclasses import dataclass
import json
from pathlib import Path

from tlaps_evidence import EvidenceError
from tlaps_source import IDENTIFIER, module_dependencies, theorem_ranges

STANDARD_MODULES = frozenset({'Integers', 'Naturals', 'FiniteSets', 'Sequences', 'TLAPS'})


@dataclass(frozen=True)
class ProofUnit:
    module: str
    minimum_obligations: int
    theorems: tuple[str, ...]


@dataclass(frozen=True)
class ProofCatalog:
    roots: tuple[str, ...]
    units: tuple[ProofUnit, ...]
    sources: tuple[Path, ...]


def unique_fields(pairs):
    result = {}
    for name, value in pairs:
        if name in result:
            raise EvidenceError('duplicate proof catalog field: ' + name)
        result[name] = value
    return result


def names(value, label: str) -> tuple[str, ...]:
    if (not isinstance(value, list) or not value
            or any(not isinstance(item, str) or not IDENTIFIER.fullmatch(item) for item in value)
            or len(set(value)) != len(value)):
        raise EvidenceError('invalid or duplicate proof catalog ' + label)
    return tuple(value)


def load_catalog(path: Path, specs: Path) -> ProofCatalog:
    try:
        document = json.loads(path.read_text(), object_pairs_hook=unique_fields)
    except (json.JSONDecodeError, RecursionError) as error:
        raise EvidenceError('invalid proof catalog JSON') from error
    if (not isinstance(document, dict) or set(document) != {'version', 'roots', 'units'}
            or type(document['version']) is not int or document['version'] != 1
            or not isinstance(document['units'], dict)):
        raise EvidenceError('invalid proof catalog schema')
    roots = names(document['roots'], 'roots')
    registered = {}
    for module, row in document['units'].items():
        if (not IDENTIFIER.fullmatch(module) or not isinstance(row, dict)
                or set(row) != {'minimum_obligations', 'theorems'}
                or type(row['minimum_obligations']) is not int or row['minimum_obligations'] < 1):
            raise EvidenceError('invalid proof catalog unit: ' + module)
        registered[module] = ProofUnit(module, row['minimum_obligations'], names(row['theorems'], 'theorems'))
    sources = []
    units = []
    visited = set()
    visiting = set()

    def visit(module):
        if module in STANDARD_MODULES:
            if (specs / (module + '.tla')).exists():
                raise EvidenceError('local source shadows pinned standard module: ' + module)
            return
        if module in visiting:
            raise EvidenceError('proof dependency cycle: ' + module)
        if module in visited:
            return
        source = specs / (module + '.tla')
        if not source.is_file():
            raise EvidenceError('missing local proof dependency: ' + module)
        visiting.add(module)
        for dependency in module_dependencies(source.read_text()):
            visit(dependency)
        visiting.remove(module)
        visited.add(module)
        sources.append(source)
        declarations = theorem_ranges(source)
        if declarations:
            if module not in registered:
                raise EvidenceError('unregistered imported proof module: ' + module)
            unit = registered[module]
            if set(declarations) != set(unit.theorems):
                raise EvidenceError('named theorem inventory differs from source: ' + module)
            units.append(unit)

    for module in roots:
        visit(module)
    if {unit.module for unit in units} != set(registered):
        raise EvidenceError('proof catalog contains missing or unreachable theorem units')
    if not units:
        raise EvidenceError('empty generalized proof inventory')
    return ProofCatalog(roots, tuple(units), tuple(sources))
