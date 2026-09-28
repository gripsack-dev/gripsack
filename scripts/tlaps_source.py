"""TLA source locations for proof admission, excluding comments and strings."""
from dataclasses import dataclass
from pathlib import Path
import re

IDENTIFIER = re.compile(r'[A-Za-z_][A-Za-z0-9_]*')


class SourceError(ValueError):
    pass


@dataclass(frozen=True)
class SourceToken:
    value: str
    line: int


def source_tokens(source: str) -> list[SourceToken]:
    """Inventory lexer only; the pinned prover still parses and checks TLA."""
    result = []
    offset = 0
    line = 1
    depth = 0
    while offset < len(source):
        if depth:
            if source.startswith('(*', offset):
                depth += 1
                offset += 2
            elif source.startswith('*)', offset):
                depth -= 1
                offset += 2
            else:
                line += source[offset] == '\n'
                offset += 1
        elif source.startswith('\\*', offset):
            newline = source.find('\n', offset)
            offset = len(source) if newline < 0 else newline
        elif source.startswith('(*', offset):
            depth = 1
            offset += 2
        elif source[offset] == '"':
            offset += 1
            while offset < len(source) and source[offset] != '"':
                if source[offset] == '\n':
                    raise SourceError('newline inside TLA string')
                offset += 2 if source[offset] == '\\' else 1
            if offset >= len(source):
                raise SourceError('unterminated TLA string')
            offset += 1
        elif source[offset].isspace():
            line += source[offset] == '\n'
            offset += 1
        else:
            identifier = IDENTIFIER.match(source, offset)
            if identifier:
                result.append(SourceToken(identifier.group(), line))
                offset = identifier.end()
            else:
                result.append(SourceToken(source[offset], line))
                offset += 1
    if depth:
        raise SourceError('unterminated TLA comment')
    return result


def module_dependencies(source: str) -> tuple[str, ...]:
    words = source_tokens(source)
    result = []
    for index, token in enumerate(words):
        if token.value not in {'EXTENDS', 'INSTANCE'}:
            continue
        cursor = index + 1
        while True:
            if cursor >= len(words) or not IDENTIFIER.fullmatch(words[cursor].value):
                raise SourceError('invalid TLA import')
            result.append(words[cursor].value)
            cursor += 1
            if token.value == 'INSTANCE' or cursor >= len(words) or words[cursor].value != ',':
                break
            cursor += 1
    return tuple(dict.fromkeys(result))


def theorem_ranges(source: Path) -> dict[str, tuple[int, int]]:
    """Locate actual declaration tokens; prover events must fall in their spans."""
    text = source.read_text()
    words = source_tokens(text)
    declarations = []
    for index, token in enumerate(words):
        if token.value not in {'THEOREM', 'LEMMA'}:
            continue
        if index + 1 >= len(words) or not IDENTIFIER.fullmatch(words[index + 1].value):
            raise SourceError('proof inventory requires named theorem declarations')
        declarations.append((words[index + 1].value, token.line))
    result = {}
    for index, (name, start) in enumerate(declarations):
        if name in result:
            raise SourceError('duplicate theorem declaration: ' + name)
        end = declarations[index + 1][1] - 1 if index + 1 < len(declarations) else len(text.splitlines())
        result[name] = (start, end)
    return result
