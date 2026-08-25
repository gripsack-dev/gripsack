"""Public API surface tests: __all__ must be importable (0005 §3).

E109 prescribes verify_deployed from the public API — an __all__ entry
without the import is a broken remedy, and nothing else catches it."""

import gripsack


def test_all_is_exported():
    missing = set(gripsack.__all__) - set(dir(gripsack))
    assert not missing, f"__all__ entries not importable: {sorted(missing)}"


def test_verify_deployed_is_public():
    from gripsack import verify_deployed

    assert verify_deployed("~/.config/zed/settings.json").to_ir() == {
        "kind": "file_deployed",
        "path": "~/.config/zed/settings.json",
    }
