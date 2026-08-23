/** Frontend contract tests (plan/0003 §5): emit shape + spans. */

import assert from "node:assert/strict";
import { describe, it, beforeEach } from "node:test";
import {
  clearGraph,
  clearResources,
  configStep,
  define,
  dep,
  emitIr,
  fetchStep,
  githubRelease,
  installStep,
  Module,
  module,
  moduleIf,
  resource,
  runStep,
  service,
  shellStep,
  symlink,
  tarball,
  trackedCopy,
} from "../src/index.js";

beforeEach(() => {
  clearGraph();
  clearResources();
});

describe("emitIr", () => {
  it("emits IR v1 shape", () => {
    module("helix", {
      fetch: githubRelease({
        repo: "helix-editor/helix",
        asset: "helix-{version}-x86_64-linux.tar.xz",
      }),
      install: { "bin/hx": symlink("~/.local/bin/hx") },
      config: { "config.toml": trackedCopy("~/.config/helix/config.toml") },
      depends: [dep("git")],
      activate: [service("syncthing")],
    });
    module("git", { fetch: tarball("https://example.invalid/git.tar.xz") });

    const ir = JSON.parse(emitIr(["gui"]));

    assert.equal(ir.ir_version, 1);
    assert.deepEqual(ir.host.tags, ["gui"]);
    assert.ok(ir.host.os);
    assert.ok(ir.host.arch);

    const helix = ir.modules.helix;
    assert.equal(helix.fetch.kind, "github_release");
    assert.equal(helix.fetch.repo, "helix-editor/helix");
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
    module("x", { fetch: tarball("https://example.invalid/x.tar.xz") });
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
    assert.equal(ir.modules.helix.fetch, undefined);
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

  it("declared resources emit and validate", () => {
    resource("company.lock");
    module("tool", {
      steps: [shellStep("./sync.sh", "sync", { resources: ["company.lock"] })],
    });
    const ir = JSON.parse(emitIr());
    assert.deepEqual(ir.resources, [{ name: "company.lock" }]);
    assert.deepEqual(ir.modules.tool.steps[0].resources, ["company.lock"]);
  });

  it("undeclared resources throw at eval", () => {
    assert.throws(
      () => shellStep("true", "x", { resources: ["cargo.lokc"] }),
      /unknown resource 'cargo\.lokc'/,
    );
  });

  it("builtin resources pass eval", () => {
    const s = shellStep("cargo install hx", "build", { resources: ["cargo-lock"] });
    assert.deepEqual(s.resources, ["cargo-lock"]);
  });

  it("class modules chain the pipeline", () => {
    class Helix extends Module {
      override fetch() {
        return fetchStep(tarball("https://example.invalid/h.tar.xz"));
      }
      override install() {
        return installStep({ "bin/hx": symlink("~/.local/bin/hx") });
      }
      override config() {
        return configStep({ "config.toml": trackedCopy("~/.config/helix/config.toml") });
      }
    }
    define(Helix);
    const ir = JSON.parse(emitIr());
    const steps = ir.modules.helix.steps;
    assert.deepEqual(
      steps.map((s: { id: string }) => s.id),
      ["fetch", "install", "config"],
    );
    assert.equal(steps[0].needs, undefined);
    assert.deepEqual(steps[1].needs, ["fetch"]);
    assert.deepEqual(steps[2].needs, ["install"]);
    assert.equal(steps[2].phase, "config");
    assert.match(ir.modules.helix.span.file, /index\.test\.(ts|js)$/);
  });

  it("marks build-only deps", () => {
    module("helix-src", {
      fetch: tarball("https://example.invalid/helix.tar.xz"),
      depends: [dep("rust", "build")],
    });
    const ir = JSON.parse(emitIr());
    assert.equal(ir.modules["helix-src"].depends[0].edge, "build");
  });
});

describe("runStep", () => {
  it("emits structured argv actions with outputs", () => {
    module("built", {
      steps: [
        runStep(["make", "install"], "make-install", {
          needs: ["fetch"],
          outputs: ["bin/hx"],
        }),
      ],
    });
    const ir = JSON.parse(emitIr());
    const action = ir.modules.built.steps[0].action;
    assert.equal(action.kind, "run");
    assert.deepEqual(action.argv, ["make", "install"]);
    assert.deepEqual(action.outputs, ["bin/hx"]);
  });
});

describe("registry safety", () => {
  it("duplicate module names throw at eval", () => {
    module("dup", { fetch: tarball("https://example.invalid/a.tar.xz") });
    assert.throws(
      () => module("dup", { fetch: tarball("https://example.invalid/b.tar.xz") }),
      /duplicate module 'dup'/,
    );
  });
});

describe("when", () => {
  it("filters modules by host facts", () => {
    moduleIf("steam", { fetch: tarball("https://example.invalid/s.tar.xz") }, { os: "plan9" });
    module("git", { fetch: tarball("https://example.invalid/g.tar.xz") });
    const ir = JSON.parse(emitIr());
    assert.equal(ir.modules.steam, undefined);
    assert.ok(ir.modules.git);
  });
});
