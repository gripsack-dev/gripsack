package main

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"

	"github.com/moby/buildkit/client"
	"github.com/moby/buildkit/client/llb"
	"github.com/tonistiigi/fsutil"
)

// probePolicy runs Epic B's probe 5: declared-input transport and
// output export must not expose ambient credentials or arbitrary host
// paths. A canary tree is offered as the local source with a strict
// include pattern; the graph observes exactly what arrived, its own
// environment, and the container root — the exported observations are
// asserted here, in Go, against the denylists.
func probePolicy(ctx context.Context, c *client.Client, pins map[string]string, outDir string) error {
	r := newReport("probe5-policy")
	alpine := pins["ALPINE_IMAGE"]

	inputDir := filepath.Join(outDir, "policy-input")
	if err := os.MkdirAll(filepath.Join(inputDir, "subdir"), 0o755); err != nil {
		return r.fail("input dir: %v", err)
	}
	declared := []byte("declared input\n")
	files := map[string][]byte{
		"allowed.txt":       declared,
		"secret-canary.txt": []byte("HOST_SECRET_CANARY_MUST_NOT_ARRIVE"),
		"subdir/nested.txt": []byte("nested canary must not arrive"),
	}
	for name, data := range files {
		if err := os.WriteFile(filepath.Join(inputDir, name), data, 0o644); err != nil {
			return r.fail("input file %s: %v", name, err)
		}
	}

	observed := llb.Image(alpine).
		Run(
			llb.Shlex(`sh -c 'ls -A /input > /obs/visible.txt; env | sort > /obs/env.txt; ls -A / > /obs/root.txt; ls -A /root >> /obs/root.txt 2>&1 || true'`),
			llb.AddMount("/input",
				llb.Local("context", llb.IncludePatterns([]string{"allowed.txt"}), llb.WithCustomName("declared input (include: allowed.txt)")),
				llb.Readonly),
			llb.WithCustomName("observe-declared-input-and-env"),
		).
		AddMount("/obs", llb.Scratch())

	opt, err := exportLocal(filepath.Join(outDir, "policy"))
	if err != nil {
		return r.fail("export opt: %v", err)
	}
	fs, err := fsutil.NewFS(inputDir)
	if err != nil {
		return r.fail("local input fs: %v", err)
	}
	opt.LocalMounts = map[string]fsutil.FS{"context": fs}
	if _, err := solve(ctx, c, observed, opt); err != nil {
		return r.fail("solve: %v", err)
	}

	read := func(name string) string {
		data, err := os.ReadFile(filepath.Join(outDir, "policy", name))
		if err != nil {
			r.fail("observation %s missing: %v", name, err)
			return ""
		}
		return string(data)
	}

	visible := read("visible.txt")
	if strings.TrimSpace(visible) != "allowed.txt" {
		return r.fail("declared-input leak: /input contains %q, want exactly allowed.txt", visible)
	}
	r.Facts["input-visible"] = strings.TrimSpace(visible)

	// Ambient-credential denylist: anything registry/auth/token shaped
	// that the host could have injected must not appear in the op env.
	denyPrefixes := []string{"DOCKER_", "KUBERNETES_", "AWS_", "GITHUB_", "GOOGLE_", "GIT_"}
	env := read("env.txt")
	var leaked []string
	for _, line := range strings.Split(env, "\n") {
		for _, prefix := range denyPrefixes {
			if strings.HasPrefix(strings.ToUpper(line), prefix) {
				leaked = append(leaked, line)
			}
		}
	}
	if strings.Contains(env, "HOST_SECRET_CANARY_MUST_NOT_ARRIVE") {
		leaked = append(leaked, "secret-canary content present in environment")
	}
	sort.Strings(leaked)
	if len(leaked) > 0 {
		return r.fail("ambient credential exposure: %s", strings.Join(leaked, ", "))
	}
	r.Facts["env-lines-observed"] = fmt.Sprintf("%d", len(strings.Split(strings.TrimSpace(env), "\n")))

	// Arbitrary host paths: the container root must look like a
	// container, not a mount of the developer's filesystem.
	root := read("root.txt")
	for _, hostTell := range []string{"tarek", "home/tarek", "workspace"} {
		if strings.Contains(root, hostTell) {
			return r.fail("host path exposure: container root mentions %q:\n%s", hostTell, root)
		}
	}
	r.Facts["container-root-listing"] = strings.ReplaceAll(strings.TrimSpace(root), "\n", " ")
	r.Notes = append(r.Notes,
		"include-pattern transport carried only allowed.txt; canary files and nested subtree absent",
		"op environment contains no registry/auth/token-shaped host credentials",
	)
	r.Duration = time.Since(parseStarted(r.StartedAt)).String()
	return r.save(outDir)
}
