/** Frontend contract tests (plan/0003 §5): emit shape + spans. */

import assert from "node:assert/strict";
import { describe, it, beforeEach } from "node:test";
import {
  clearGraph,
  dep,
  emitIr,
  fetchStep,
  githubRelease,
  module,
  service,
  shellStep,
  symlink,
  tarball,
  trackedCopy,
} from "../src/index.js";

beforeEach(() => clearGraph());

describe("emitIr", () => {
  it("emits IR v1 shape", () => {
    module("helix", {
      source: githubRelease({
        repo: "helix-editor/helix",
        asset: "helix-{version}-x86_64-linux.tar.xz",
      }),
      install: { "bin/hx": symlink("~/.local/bin/hx") },
      config: { "config.toml": trackedCopy("~/.config/helix/config.toml") },
      depends: [dep("git")],
      activate: [service("syncthing")],
    });
    module("git", { source: tarball("https://example.invalid/git.tar.xz") });

    const ir = JSON.parse(emitIr(["gui"]));

    assert.equal(ir.ir_version, 1);
    assert.deepEqual(ir.host.tags, ["gui"]);
    assert.ok(ir.host.os);
    assert.ok(ir.host.arch);

    const helix = ir.modules.helix;
    assert.equal(helix.source.kind, "github_release");
    assert.equal(helix.source.repo, "helix-editor/helix");
    assert.deepEqual(helix.install, [
      { from: "bin/hx", to: "~/.local/bin/hx", mode: "owned" },
    ]);
    assert.equal(helix.config[0].mode, "tracked_copy");
    assert.deepEqual(helix.depends, [{ module: "git", edge: "runtime" }]);
    assert.equal(helix.activate[0].kind, "service");
    assert.equal(helix.activate[0].trigger, "post_activate");

    // optional sections absent, not null — the IR is sparse by convention
    assert.equal(ir.modules.git.install, undefined);
    assert.equal(ir.modules.git.depends, undefined);
  });

  it("captures spans pointing at this file", () => {
    module("x", { source: tarball("https://example.invalid/x.tar.xz") });
    const ir = JSON.parse(emitIr());
    const span = ir.modules.x.span;
    assert.ok(span, "span present");
    // tests run compiled — the span points at the executing file
    assert.match(span.file, /index\.test\.(ts|js)$/);
    assert.ok(span.line > 0);
    assert.ok(span.col > 0);
  });

  it("dotfiles-only modules emit no source", () => {
    module("helix", {
      config: { "config.toml": trackedCopy("~/.config/helix/config.toml") },
    });
    const ir = JSON.parse(emitIr());
    assert.equal(ir.modules.helix.source, undefined);
    assert.equal(ir.modules.helix.config[0].mode, "tracked_copy");
  });

  it("emits explicit steps", () => {
    module("helix-patched", {
      steps: [
        fetchStep(tarball("https://example.invalid/helix.tar.xz")),
        shellStep("patch -p1 < fix.patch", "patch", { needs: ["fetch"] }),
      ],
    });
    const ir = JSON.parse(emitIr());
    const steps = ir.modules["helix-patched"].steps;
    assert.equal(steps[0].action.kind, "fetch");
    assert.equal(steps[0].phase, "fetch");
    assert.equal(steps[1].id, "patch");
    assert.equal(steps[1].action.kind, "custom_shell");
    assert.deepEqual(steps[1].needs, ["fetch"]);
  });

  it("marks build-only deps", () => {
    module("helix-src", {
      source: tarball("https://example.invalid/helix.tar.xz"),
      depends: [dep("rust", "build")],
    });
    const ir = JSON.parse(emitIr());
    assert.equal(ir.modules["helix-src"].depends[0].edge, "build");
  });
});
