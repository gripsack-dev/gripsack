from gripsack import module, github_release, dep

helix = module(
    "helix",
    fetch=github_release(repo="helix-editor/helix", asset="h.tar.xz"),
    depends=[dep("nvim")],
)
