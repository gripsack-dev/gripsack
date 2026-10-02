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
)

// probeGraph runs Epic B's probe 2: an LLB graph where two outputs
// share one build dependency. It demonstrates (and records) reuse,
// cancellation, bounded logs and attribution of a deliberate failure
// to its submitted operation.
func probeGraph(ctx context.Context, c *client.Client, pins map[string]string, outDir string) error {
	r := newReport("probe2-graph")
	alpine := pins["ALPINE_IMAGE"]

	dep := llb.Image(alpine).
		Run(llb.Shlex(`sh -c 'printf payload > /work/dep.bin'`), llb.WithCustomName("shared-dependency")).
		AddMount("/work", llb.Scratch())
	consumer := func(name, tag string) llb.State {
		return llb.Image(alpine).
			Run(
				llb.Shlexf(`sh -c 'cp /dep/dep.bin /out/%s; printf %s >> /out/%s'`, tag, tag, tag),
				llb.AddMount("/dep", dep, llb.Readonly),
				llb.WithCustomName(name),
			).
			AddMount("/out", llb.Scratch())
	}
	outA, outB := consumer("consumer-a", "a.bin"), consumer("consumer-b", "b.bin")

	solve1 := func(name string, state llb.State) (*solveStats, error) {
		opt, err := exportLocal(filepath.Join(outDir, "graph", strings.TrimSuffix(name, ".bin")))
		if err != nil {
			return nil, err
		}
		return solve(ctx, c, state, opt)
	}

	stats, err := solve1("a.bin", outA)
	if err != nil {
		return r.fail("first solve of A: %v", err)
	}
	r.Facts["solve-a1-cached"] = strings.Join(cachedList(stats), ",")

	stats, err = solve1("a.bin", outA)
	if err != nil {
		return r.fail("second solve of A (reuse): %v", err)
	}
	r.Facts["solve-a2-cached"] = strings.Join(cachedList(stats), ",")
	if !stats.cached["shared-dependency"] || !stats.cached["consumer-a"] {
		return r.fail("expected shared-dependency and consumer-a CACHED on identical re-solve, got %v", stats.cached)
	}
	r.Notes = append(r.Notes, "identical re-solve of A hit cache for the shared dependency and the consumer")

	stats, err = solve1("b.bin", outB)
	if err != nil {
		return r.fail("solve of B (shared dependency reuse): %v", err)
	}
	r.Facts["solve-b-cached"] = strings.Join(cachedList(stats), ",")
	if !stats.cached["shared-dependency"] {
		return r.fail("expected shared-dependency CACHED while consumer-b rebuilt, got %v", stats.cached)
	}
	if stats.cached["consumer-b"] {
		return r.fail("consumer-b must not be cached on its first solve")
	}
	r.Notes = append(r.Notes, "sibling output B reused the shared dependency while building its own consumer")

	for _, tag := range []string{"a.bin", "b.bin"} {
		path := filepath.Join(outDir, "graph", strings.TrimSuffix(tag, ".bin"), tag)
		data, err := os.ReadFile(path)
		if err != nil {
			return r.fail("exported output missing: %v", err)
		}
		if want := "payload" + tag; string(data) != want {
			return r.fail("output %s = %q, want %q", path, data, want)
		}
	}
	r.Notes = append(r.Notes, "both exports carry the shared dependency payload plus their own suffix")

	// Cancellation: a long op cancelled mid-solve must surface the
	// cancellation and leave the worker usable.
	slow := llb.Image(alpine).
		Run(llb.Shlex(`sh -c 'sleep 300'`), llb.WithCustomName("slow-op")).
		Root()
	cancelCtx, cancel := context.WithTimeout(ctx, 3*time.Second)
	defer cancel()
	cancelStart := time.Now()
	_, slowErr := solve(cancelCtx, c, slow, client.SolveOpt{})
	if slowErr == nil {
		return r.fail("cancellation probe: solve unexpectedly succeeded")
	}
	lowered := strings.ToLower(slowErr.Error())
	signal := strings.Contains(lowered, "context") || strings.Contains(lowered, "cancel") || strings.Contains(lowered, "deadline")
	if !signal {
		return r.fail("cancellation probe: error carries no cancellation signal: %v", slowErr)
	}
	r.Facts["cancellation-latency"] = time.Since(cancelStart).String()
	r.Notes = append(r.Notes, "cancelled mid-solve; error: "+slowErr.Error())

	health := llb.Image(alpine).Run(llb.Shlex(`sh -c 'true'`), llb.WithCustomName("worker-health-check")).Root()
	if _, err := solve(ctx, c, health, client.SolveOpt{}); err != nil {
		return r.fail("worker unhealthy after cancellation: %v", err)
	}
	r.Notes = append(r.Notes, "worker served a follow-up solve after cancellation")

	// Bounded failure attribution: the error and the captured logs must
	// name the failing operation, and log capture stays capped.
	bad := llb.Image(alpine).
		Run(llb.Shlex(`sh -c 'echo DELIBERATE_FAILURE_MARKER >&2; exit 7'`), llb.WithCustomName("deliberate-failure")).
		Root()
	failStats, failErr := solve(ctx, c, bad, client.SolveOpt{})
	if failErr == nil {
		return r.fail("failure probe: solve unexpectedly succeeded")
	}
	logTail := failStats.logs["deliberate-failure"]
	if !strings.Contains(string(logTail), "DELIBERATE_FAILURE_MARKER") {
		return r.fail("failure probe: vertex logs lack the marker (logs: %q, err: %v)", logTail, failErr)
	}
	for name, buf := range failStats.logs {
		if len(buf) > logCap {
			return r.fail("failure probe: log capture for %s exceeded %d bytes", name, logCap)
		}
	}
	r.Facts["failure-attribution"] = fmt.Sprintf("error=%q logs-name=deliberate-failure", failErr.Error())
	r.Notes = append(r.Notes, "deliberate failure attributed to its operation with bounded logs")

	r.Duration = time.Since(parseStarted(r.StartedAt)).String()
	return r.save(outDir)
}

func cachedList(stats *solveStats) []string {
	names := make([]string, 0, len(stats.cached))
	for name, cached := range stats.cached {
		if cached {
			names = append(names, name)
		}
	}
	sort.Strings(names)
	return names
}

func parseStarted(rfc3339 string) time.Time {
	t, err := time.Parse(time.RFC3339, rfc3339)
	if err != nil {
		return time.Now()
	}
	return t
}
