/** Cargo over an immutable source tree and a digest-pinned official Rust image.
 * The source includes Cargo.lock and its vendored dependency/configuration tree.
 * No helper step resolves dependencies, downloads tools, or invokes a shell. */
import type { Fetch } from "../fetch.ts";
import type { Span } from "../module.ts";
import type { PackageNode, RecipeNode, WorkspaceArg, WorkspaceOutput, WorkspacePlatform, WorkspaceStepValue } from "./ir.ts";
import { rejectUnknownFields } from "../fields.ts";
import { exec, lit, outputPath, sourcePath } from "./commands.ts";
import { pkg, recipe } from "./outputs.ts";
import { asPlatform } from "./target.ts";
import { asName, asRecord, asSelector, freezeDeep, nodeSpan } from "./validate.ts";

export interface CargoPackageSpec {
  source: Fetch;
  /** Digest-pinned official Rust image, including Cargo/rustup and native linker tools. */
  toolchain: string;
  /** Linux architecture and explicit gnu/musl ABI; no implicit cross compilation. */
  target: WorkspacePlatform;
  /** Public command name -> Cargo binary target name. */
  binaries: Record<string, string>;
  runtime?: string[];
  checks?: string[];
  span?: Span;
}

/** Spread the returned producer/package pair into workspace.outputs. */
export function cargoPackage(
  name: string,
  spec: CargoPackageSpec,
): readonly [WorkspaceOutput<RecipeNode>, WorkspaceOutput<PackageNode>] {
  const what = `cargoPackage("${name}")`;
  asName(name, `${what}: name`);
  asRecord(spec, what);
  rejectUnknownFields(what, spec, ["source", "toolchain", "target", "binaries", "runtime", "checks", "span"]);
  const span = nodeSpan(spec.span, what);
  const target = asPlatform(spec.target, `${what}: target`);
  if (target.os !== "linux" || (target.abi !== "gnu" && target.abi !== "musl")) {
    throw new Error(`${what}: select a Linux target with an explicit gnu or musl ABI`);
  }
  const triple = `${target.arch}-unknown-linux-${target.abi}`;
  const commands: Record<string, string> = Object.create(null);
  const binaries = new Set<string>();
  for (const [command, value] of Object.entries(asRecord(spec.binaries, `${what}: binaries`))) {
    asName(command, `${what}: command name`);
    const binary = asSelector(value, `${what}: binary target`);
    if (binary === "." || binary.includes("/")) throw new Error(`${what}: a Cargo binary target is one path component`);
    commands[command] = `bin/${binary}`;
    binaries.add(binary);
  }
  if (binaries.size === 0) throw new Error(`${what}: declare at least one exported binary target`);
  const environment: Record<string, WorkspaceArg> = {
    CARGO_HOME: outputPath(".cargo-home"),
    CARGO_TARGET_DIR: outputPath(".cargo-target"),
    RUSTUP_HOME: lit("/usr/local/rustup"),
    RUSTC: lit("/usr/local/cargo/bin/rustc"),
    RUSTDOC: lit("/usr/local/cargo/bin/rustdoc"),
  };
  const common = ["--release", "--frozen", "--offline", "--target", triple].map(lit);
  const steps: WorkspaceStepValue[] = [
    // Testing is in the ordered producer path, so it cannot be pruned from the
    // exported state even if it creates no application file.
    exec({ span, argv: [lit("/usr/local/cargo/bin/cargo"), lit("test"), lit("--all-targets"), ...common], env: environment, cwd: sourcePath() }),
    exec({ span, argv: [lit("/usr/local/cargo/bin/cargo"), lit("build"), lit("--bins"), ...common], env: environment, cwd: sourcePath() }),
  ];
  for (const binary of binaries) {
    steps.push(exec({ span, argv: [lit("install"), lit("-D"), outputPath(`.cargo-target/${triple}/release/${binary}`), outputPath(`bin/${binary}`)] }));
  }
  steps.push(exec({ span, argv: [lit("rm"), lit("-rf"), outputPath(".cargo-target"), outputPath(".cargo-home")] }));
  const producer = `${name}.build`;
  return freezeDeep([
    recipe(producer, { source: spec.source, target, span, output_kind: "tree", steps,
      execution: { kind: "isolated_linux", worker: "buildkit", platform: target, toolchain: { reference: spec.toolchain } },
      ...(spec.checks ? { checks: spec.checks } : {}),
    }),
    pkg(name, { producer, commands, target, span, layout: { kind: "relocatable" }, ...(spec.runtime ? { runtime: spec.runtime } : {}) }),
  ] as const);
}
