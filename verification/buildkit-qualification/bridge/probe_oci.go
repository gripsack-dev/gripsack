package main

import (
	"context"
	"os"
	"path/filepath"
	"strconv"
	"time"

	"github.com/moby/buildkit/client"
	"github.com/moby/buildkit/client/llb"
)

// probeOCI runs Epic B's probe 4: assemble and export a minimal OCI
// image. Independent verification of digests/DiffIDs/config and the
// runnable-on-a-separate-runtime check live in the driver (python
// verifier + docker load), which does not share this program's code.
// The driver exports twice from two FRESH workers: that is the clean
// reproduction comparison, distinguished from any cache reuse.
func probeOCI(ctx context.Context, c *client.Client, pins map[string]string, outDir string) error {
	r := newReport("probe4-oci")
	alpine := pins["ALPINE_IMAGE"]

	content := llb.Image(alpine).
		Run(
			llb.Shlex(`sh -c 'printf "gripsack b0 oci fixture\n" > /work/hello.txt; sha256sum /work/hello.txt > /work/hello.txt.sha256'`),
			llb.WithCustomName("oci-content (fixed timestamps)"),
		).
		AddMount("/work", llb.Scratch())
	// The image carries the pinned alpine runtime base so the exported
	// result is genuinely runnable by an independent runtime (probe 4).
	state := llb.Image(alpine).
		File(llb.Copy(content, "/hello.txt", "/hello.txt", llb.WithCreatedTime(fixedEpoch))).
		File(llb.Copy(content, "/hello.txt.sha256", "/hello.txt.sha256", llb.WithCreatedTime(fixedEpoch)))

	dest := filepath.Join(outDir, "oci-layout.tar")
	opt, err := exportOCITar(dest)
	if err != nil {
		return r.fail("export opt: %v", err)
	}
	if _, err := solve(ctx, c, state, opt); err != nil {
		return r.fail("solve: %v", err)
	}
	info, err := os.Stat(dest)
	if err != nil {
		return r.fail("stat tar: %v", err)
	}
	r.Facts["oci-tar-bytes"] = strconv.FormatInt(info.Size(), 10)
	r.Facts["base-image"] = alpine
	r.Facts["content-timestamps"] = "fixed epoch (llb.WithCreatedTime)"
	r.Notes = append(r.Notes,
		"digest/config verification, two-fresh-worker reproduction and separate-runtime execution are performed by the independent driver verifier",
	)
	r.Duration = time.Since(parseStarted(r.StartedAt)).String()
	return r.save(outDir)
}
