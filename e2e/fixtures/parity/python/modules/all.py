"""Parity fixture (python) — every IR field, so absence-class drift
has no shadow to hide in (plan/0007: the conformance test).
"""
from gripsack import (
    brew, dep, desktop_entry, file_fetch, fonts, github_release, merge, module, pixi,
    plugin_fetch, service, symlink, tarball, template, tracked_copy, tree, verify_binary,
    verify_deployed, verify_file, verify_shell, when,
)
from gripsack.entries import Ownership
from gripsack.resources import resource

resource("parity.lock")

module(
    "core",
    fetch=github_release(
        repo="starship/starship",
        asset="starship-{version}-x86_64-linux.tar.gz",
        version="v1.20.0",
        base_url="https://ghe.example.com/api/v3",
    ),
    install={"starship": symlink("~/.local/bin/starship")},
    config={
        "configs/demo/a.toml": tracked_copy("~/.config/demo/a.toml"),
        "configs/demo/id.toml": template("~/.config/demo/id.toml", vars={"email": "a@b.c"}),
        "configs/demo/block.sh": merge("~/.bashrc", marker="#"),
    },
    depends=[dep("tools")],
    env={"PARITY_HOME": "{store}", "PATH+": "{store}/bin"},
    verify=verify_binary("starship", args=["--version"]),
    retries=2,
    lint="helix",
    activate=[fonts(), service("parity", user=True), desktop_entry()],
)

module(
    "tools",
    fetch=tarball("https://example.invalid/tools.tar.gz", sha256="ab" * 32),
    config={**tree("configs/demo", "~/.config/demo", mode=Ownership.OWNED)},
    verify=verify_shell("test -f bin/tools"),
    when=when(tags=["gui"]),
)

module(
    "extras",
    fetch=file_fetch("payloads/x.tar.gz"),
    verify=verify_deployed("~/.config/demo/a.toml"),
)

module(
    "brewed",
    fetch=brew("jq", version="1.8.0"),
)

module(
    "pixied",
    fetch=pixi("ripgrep", version="15.0.0"),
)

module(
    "plugged",
    fetch=plugin_fetch("apt", package="htop", version="3.3.0"),
    verify=verify_file("bin/htop"),
)
