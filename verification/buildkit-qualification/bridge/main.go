// B0-01 qualification bridge (handover Epic B): a minimal Go client
// over upstream BuildKit client/LLB APIs driving the required probe
// graphs against a pinned, disposable buildkitd worker.
//
// This is deliberately NOT the production tools/buildkit-bridge
// (B1/B2): no protocol, no store integration, no activation logic. It
// exists to qualify the selected backend and record versions,
// configuration and observed behavior.
package main

import (
	"bufio"
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/moby/buildkit/client"
	"github.com/moby/buildkit/client/llb"
	"github.com/moby/buildkit/session/filesync"
	"github.com/opencontainers/go-digest"
	"github.com/opencontainers/image-spec/specs-go/v1"
)

// probeReport is the machine-readable outcome of one probe: the facts
// B0 must record (cache behavior, attribution, timings) so the
// qualification evidence is reviewable without re-running.
type probeReport struct {
	Probe     string            `json:"probe"`
	StartedAt string            `json:"started_at"`
	Duration  string            `json:"duration"`
	Facts     map[string]string `json:"facts"`
	Notes     []string          `json:"notes,omitempty"`
}

func newReport(probe string) *probeReport {
	return &probeReport{Probe: probe, StartedAt: time.Now().UTC().Format(time.RFC3339), Facts: map[string]string{}}
}

func (r *probeReport) fail(format string, args ...any) error {
	return fmt.Errorf("%s: %s", r.Probe, fmt.Sprintf(format, args...))
}

func (r *probeReport) save(outDir string) error {
	bytes, err := json.MarshalIndent(r, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(filepath.Join(outDir, r.Probe+".json"), append(bytes, '\n'), 0o644)
}

type solveStats struct {
	cached   map[string]bool          // vertex name -> cached on this solve
	failed   []string                 // vertex names carrying errors
	logs     map[string][]byte        // name -> capped log tail
	digestOf map[digest.Digest]string // vertex digest -> name (logs arrive keyed by digest)
	started  time.Time
	ended    time.Time
}

// bounded-log policy; buildkit additionally enforces its own caps.
const logCap = 64 << 10

func (s *solveStats) appendLog(vertexDigest digest.Digest, data []byte) {
	name := s.digestOf[vertexDigest]
	if name == "" {
		name = vertexDigest.String() // re-keyed when the vertex record arrives
	}
	buf := s.logs[name]
	if room := logCap - len(buf); room < len(data) {
		data = data[:max(room, 0)]
	}
	s.logs[name] = append(buf, data...)
}

func (s *solveStats) resolveLogs() {
	for vertexDigest, name := range s.digestOf {
		if name == vertexDigest.String() {
			continue
		}
		if data, ok := s.logs[vertexDigest.String()]; ok {
			s.logs[name] = append(s.logs[name], data...)
			delete(s.logs, vertexDigest.String())
		}
	}
}

// solve drives one Solve and collects progress facts; the solve error
// is returned verbatim so callers assert on it while stats stay valid.
func solve(ctx context.Context, c *client.Client, state llb.State, opt client.SolveOpt) (*solveStats, error) {
	def, err := state.Marshal(ctx, llb.Platform(v1.Platform{OS: "linux", Architecture: "amd64"}))
	if err != nil {
		return nil, fmt.Errorf("marshal: %w", err)
	}
	stats := &solveStats{
		cached: map[string]bool{}, logs: map[string][]byte{}, digestOf: map[digest.Digest]string{},
		started: time.Now(),
	}
	var solveErr error
	done := make(chan struct{})
	ch := make(chan *client.SolveStatus)
	go func() {
		defer close(done)
		_, err := c.Solve(ctx, def, opt, ch)
		solveErr = err
	}()
	for status := range ch {
		for _, v := range status.Vertexes {
			stats.digestOf[v.Digest] = v.Name
			if v.Cached {
				stats.cached[v.Name] = true
			}
			if v.Error != "" {
				stats.failed = append(stats.failed, v.Name+" ("+v.Error+")")
			}
			if v.Completed != nil {
				cached := ""
				if v.Cached {
					cached = " CACHED"
				}
				fmt.Fprintf(os.Stderr, "vertex %s completed%s\n", v.Name, cached)
			}
		}
		for _, l := range status.Logs {
			stats.appendLog(l.Vertex, l.Data)
			os.Stderr.Write(l.Data)
		}
	}
	<-done
	stats.ended = time.Now()
	stats.resolveLogs()
	return stats, solveErr
}

// exportLocal returns a SolveOpt exporting the solved state to dir.
func exportLocal(dir string) (client.SolveOpt, error) {
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return client.SolveOpt{}, err
	}
	return client.SolveOpt{
		Exports: []client.ExportEntry{{
			Type:      client.ExporterLocal,
			OutputDir: dir,
		}},
	}, nil
}

// exportOCITar returns a SolveOpt exporting an OCI layout tar to dest.
func exportOCITar(dest string) (client.SolveOpt, error) {
	f, err := os.Create(dest)
	if err != nil {
		return client.SolveOpt{}, err
	}
	return client.SolveOpt{
		Exports: []client.ExportEntry{{
			Type: client.ExporterOCI,
			Output: filesync.FileOutputFunc(func(map[string]string) (io.WriteCloser, error) {
				return f, nil
			}),
		}},
	}, nil
}

// loadPins reads the KEY=VALUE pins file; every image the graphs use
// comes from here, never a literal in probe code.
func loadPins(path string) (map[string]string, error) {
	f, err := os.Open(path)
	if err != nil {
		return nil, err
	}
	defer f.Close()
	pins := map[string]string{}
	scanner := bufio.NewScanner(f)
	for scanner.Scan() {
		line := strings.TrimSpace(scanner.Text())
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}
		key, value, ok := strings.Cut(line, "=")
		if !ok {
			return nil, fmt.Errorf("malformed pins line %q", line)
		}
		pins[key] = value
	}
	return pins, scanner.Err()
}

func main() {
	addr := flag.String("addr", "tcp://127.0.0.1:1234", "buildkitd address")
	out := flag.String("out", "results", "results directory")
	pinsPath := flag.String("pins", "../pins.env", "pins file")
	probe := flag.String("probe", "", "probe to run: graph | executable | oci | policy")
	flag.Parse()
	if *probe == "" {
		fatalf("-probe is required (graph | executable | oci | policy)")
	}
	pins, err := loadPins(*pinsPath)
	if err != nil {
		fatalf("pins: %v", err)
	}
	ctx := context.Background()
	c, err := client.New(ctx, *addr)
	if err != nil {
		fatalf("connect %s: %v", *addr, err)
	}
	defer c.Close()
	if err := os.MkdirAll(*out, 0o755); err != nil {
		fatalf("out dir: %v", err)
	}
	var runErr error
	switch *probe {
	case "graph":
		runErr = probeGraph(ctx, c, pins, *out)
	case "executable":
		runErr = probeExecutable(ctx, c, pins, *out)
	case "oci":
		runErr = probeOCI(ctx, c, pins, *out)
	case "policy":
		runErr = probePolicy(ctx, c, pins, *out)
	default:
		fatalf("unknown probe %q", *probe)
	}
	if runErr != nil {
		fatalf("%v", runErr)
	}
	fmt.Fprintf(os.Stderr, "probe %s: PASS\n", *probe)
}

func fatalf(format string, args ...any) {
	fmt.Fprintf(os.Stderr, "bridge: "+format+"\n", args...)
	os.Exit(1)
}
