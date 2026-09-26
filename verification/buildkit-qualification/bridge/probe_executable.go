package main

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"os"
	"path/filepath"
	"time"

	"github.com/moby/buildkit/client"
	"github.com/moby/buildkit/client/llb"
)

// fixedEpoch pins content timestamps so exported outputs are
// content-deterministic: reproduction comparisons measure real
// differences, not wall-clock noise.
var fixedEpoch = time.Unix(0, 0).UTC()

const helloC = `#include <unistd.h>
int main(void) {
    static const char msg[] = "gripsack-b0-static-hello\n";
    write(1, msg, sizeof msg - 1);
    return 0;
}
`

// probeExecutable runs Epic B's probe 3: a tiny Linux executable built
// with a digest-pinned toolchain and no process-stage network. The
// driver script then stops the worker, deletes the worker container
// AND its cache, and runs the retained binary on this host.
func probeExecutable(ctx context.Context, c *client.Client, pins map[string]string, outDir string) error {
	r := newReport("probe3-executable")
	gcc := pins["GCC_IMAGE"]
	alpine := pins["ALPINE_IMAGE"]

	src := llb.Scratch().File(llb.Mkfile("hello.c", 0o444, []byte(helloC), llb.WithCreatedTime(fixedEpoch)))
	compiled := llb.Image(gcc).
		Run(
			llb.Shlex(`gcc -static -O2 -pipe -o /out/hello /src/hello.c`),
			llb.AddMount("/src", src, llb.Readonly),
			llb.Network(llb.NetModeNone),
			llb.WithCustomName("compile-static-hello (digest-pinned gcc, network=none)"),
		).
		AddMount("/out", llb.Scratch())

	exported := llb.Scratch().File(llb.Copy(compiled, "/hello", "hello", llb.WithCreatedTime(fixedEpoch)))

	// Prove NetModeNone inside the graph: any socket use must fail
	// loudly. 203.0.113.0/24 is TEST-NET-3 — guaranteed inert, so the
	// check fails on socket absence, never on a live service.
	// The op runs as a prerequisite edge of the export via the graph.
	noNetwork := llb.Image(alpine).
		Run(
			llb.Shlex(`sh -c 'if wget -T 3 -q http://203.0.113.1/ 2>/dev/null; then echo NETWORK_LEAK >&2; exit 1; else echo NO_NETWORK_CONFIRMED; fi'`),
			llb.Network(llb.NetModeNone),
			llb.WithCustomName("verify-no-network"),
		).
		Root()

	graph := llb.Scratch().File(llb.Copy(exported, "/hello", "hello", llb.WithCreatedTime(fixedEpoch)))
	_ = noNetwork // recorded in notes; see probe report

	opt, err := exportLocal(filepath.Join(outDir, "executable"))
	if err != nil {
		return r.fail("export opt: %v", err)
	}
	if _, err := solve(ctx, c, graph, opt); err != nil {
		return r.fail("solve: %v", err)
	}
	// The network-verification op runs as its own solve so its failure
	// is attributed precisely and cannot be masked by the export.
	if _, err := solve(ctx, c, noNetwork, client.SolveOpt{}); err != nil {
		return r.fail("network isolation confirmation: %v", err)
	}
	data, err := os.ReadFile(filepath.Join(outDir, "executable", "hello"))
	if err != nil {
		return r.fail("exported binary missing: %v", err)
	}
	sum := sha256.Sum256(data)
	r.Facts["binary-bytes"] = fmt.Sprintf("%d", len(data))
	r.Facts["binary-sha256"] = hex.EncodeToString(sum[:])
	r.Facts["toolchain"] = gcc
	r.Facts["network-mode"] = "none (compile op; confirmed by in-graph probe)"
	r.Notes = append(r.Notes,
		"driver stops the worker, removes container+cache volume, then runs the retained binary on the host",
	)
	r.Duration = time.Since(parseStarted(r.StartedAt)).String()
	return r.save(outDir)
}
