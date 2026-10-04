// Package client owns the pinned upstream RPC/session boundary. It submits
// received, digest-checked operations; it never calls the lowering package.
package client

import (
	"context"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"sort"
	"strings"

	bk "github.com/moby/buildkit/client"
	"github.com/moby/buildkit/client/llb"
	"github.com/moby/buildkit/solver/pb"
	"github.com/opencontainers/go-digest"
	"github.com/tonistiigi/fsutil"
	fstypes "github.com/tonistiigi/fsutil/types"
	"gripsack.dev/buildkit-bridge/protocol"
)

type Paths struct{ Address, Inputs, Output string }

type SolveFailure struct {
	Cause    error
	Vertices []string
}

func (failure *SolveFailure) Error() string { return failure.Cause.Error() }
func (failure *SolveFailure) Unwrap() error { return failure.Cause }

func Execute(ctx context.Context, request protocol.ExecuteRequest, paths Paths, output io.Writer) error {
	if err := request.Validate(); err != nil {
		return err
	}
	identity := protocol.ExecutionIdentity{Identity: request.Identity, Worker: request.Worker}
	if err := admitPaths(paths); err != nil {
		return err
	}
	var decoded pb.Definition
	if err := decoded.UnmarshalVT(request.Definition); err != nil {
		return fmt.Errorf("checked definition decode: %w", err)
	}
	if len(decoded.Def) == 0 || len(decoded.Def) > protocol.MaxPlanNodes+1 {
		return fmt.Errorf("invalid definition vertex count")
	}
	known := make(map[digest.Digest]struct{}, len(decoded.Def))
	for _, operation := range decoded.Def {
		known[digest.FromBytes(operation)] = struct{}{}
	}
	sources, err := declaredSources(&decoded, request.Sources, paths.Inputs)
	if err != nil {
		return err
	}
	connection, err := bk.New(ctx, paths.Address)
	if err != nil {
		return err
	}
	defer connection.Close()
	info, err := connection.Info(ctx)
	if err != nil {
		return err
	}
	if info.BuildkitVersion.Version != protocol.ExpectedDaemonVersion {
		return fmt.Errorf("daemon %q is not pinned version %q", info.BuildkitVersion.Version, protocol.ExpectedDaemonVersion)
	}
	workers, err := connection.ListWorkers(ctx)
	if err != nil {
		return err
	}
	compatible := false
	for _, worker := range workers {
		for _, platform := range worker.Platforms {
			compatible = compatible || platform.OS == request.Platform.OS && platform.Architecture == request.Platform.Architecture && platform.Variant == ""
		}
	}
	if !compatible {
		return fmt.Errorf("daemon has no admitted execution platform")
	}
	if err := protocol.WriteFrame(output, protocol.Response{Accepted: &protocol.Accepted{
		Identity: request.Identity, Worker: request.Worker, DaemonVersion: info.BuildkitVersion.Version, Platform: request.Platform,
	}}); err != nil {
		return err
	}
	var definition llb.Definition
	definition.FromPB(&decoded)
	export, archive, err := exportEntry(request, paths.Output)
	if err != nil {
		return err
	}
	if archive != nil {
		defer archive.Close()
	}
	// FromPB preserves all op byte slices; no generator, resolver, auth/SSH
	// provider, frontend, entitlements, remote cache or ambient options are added.
	options := bk.SolveOpt{
		LocalMounts:          sources,
		CompatibilityVersion: compatibilityVersion,
		Exports:              []bk.ExportEntry{export},
	}
	solveContext, cancel := context.WithCancel(ctx)
	defer cancel()
	statuses := make(chan *bk.SolveStatus)
	finished := make(chan error, 1)
	go func() { _, err := connection.Solve(solveContext, &definition, options, statuses); finished <- err }()
	logEvents := 0
	logBytes := 0
	var outputError error
	failed := make(map[digest.Digest]struct{})
	// Keep draining after the log cap. Terminal/control frames do not compete
	// for space in a progress queue, and cancellation has its own context.
	for status := range statuses {
		for _, vertex := range status.Vertexes {
			if _, admitted := known[vertex.Digest]; admitted && vertex.Error != "" {
				failed[vertex.Digest] = struct{}{}
			}
		}
		for _, record := range status.Logs {
			for data := record.Data; len(data) != 0 && logEvents < protocol.MaxLogEvents-1 && logBytes < maxForwardedLogBytes; {
				count := min(len(data), protocol.MaxLogChunkBytes, maxForwardedLogBytes-logBytes)
				event := protocol.Event{Identity: request.Identity, Worker: request.Worker, Vertex: record.Vertex.String(), Chunk: data[:count], Truncated: count < len(data)}
				if outputError == nil {
					outputError = protocol.WriteFrame(output, protocol.Response{Event: &event})
					if outputError != nil {
						cancel()
					}
				}
				logEvents++
				logBytes += count
				data = data[count:]
			}
		}
	}
	solveError := <-finished
	if outputError != nil {
		return outputError
	}
	if logEvents >= protocol.MaxLogEvents-1 || logBytes >= maxForwardedLogBytes {
		if err := protocol.WriteFrame(output, protocol.Response{Event: &protocol.Event{Identity: request.Identity, Worker: request.Worker, Chunk: []byte{}, Truncated: true}}); err != nil {
			return err
		}
	}
	if ctx.Err() != nil {
		return protocol.WriteFrame(output, protocol.Response{Cancelled: &identity})
	}
	if solveError != nil {
		vertices := make([]string, 0, len(failed))
		for vertex := range failed {
			vertices = append(vertices, vertex.String())
		}
		sort.Strings(vertices)
		return &SolveFailure{Cause: solveError, Vertices: vertices}
	}
	if archive != nil {
		if err := archive.complete(); err != nil {
			return err
		}
	}
	// A completed export precedes Done; neither is host-store publication.
	if err := protocol.WriteFrame(output, protocol.Response{Exported: &identity}); err != nil {
		return err
	}
	return protocol.WriteFrame(output, protocol.Response{Done: &identity})
}

// Leaves room under the core's 16 MiB stdout budget for base64, frame overhead,
// and terminal messages even under sustained process stdout/stderr pressure.
const maxForwardedLogBytes = 8 * 1024 * 1024

func admitPaths(paths Paths) error {
	socket, ok := strings.CutPrefix(paths.Address, "unix://")
	if !ok || !filepath.IsAbs(socket) {
		return fmt.Errorf("only an explicit private Unix worker socket is supported")
	}
	for _, directory := range []string{filepath.Dir(socket), paths.Inputs, paths.Output} {
		if !filepath.IsAbs(directory) {
			return fmt.Errorf("session directories must be absolute")
		}
		canonical, err := filepath.EvalSymlinks(directory)
		if err != nil {
			return err
		}
		if canonical != directory {
			return fmt.Errorf("session directory cannot alias another path")
		}
		info, err := os.Lstat(directory)
		if err != nil {
			return err
		}
		if !info.IsDir() || info.Mode().Perm()&0077 != 0 {
			return fmt.Errorf("session root must be a private directory")
		}
	}
	info, err := os.Lstat(socket)
	if err != nil {
		return err
	}
	if info.Mode()&os.ModeSocket == 0 {
		return fmt.Errorf("worker endpoint is not a Unix socket")
	}
	entries, err := os.ReadDir(paths.Output)
	if err != nil {
		return err
	}
	if len(entries) != 0 {
		return fmt.Errorf("export staging must be empty")
	}
	return nil
}

func declaredSources(definition *pb.Definition, bindings []protocol.SourceBinding, root string) (map[string]fsutil.FS, error) {
	allowed := make(map[string]string, len(bindings))
	for _, binding := range bindings {
		allowed[binding.Name] = binding.Digest
	}
	used := make(map[string]bool, len(bindings))
	for _, bytes := range definition.Def {
		var operation pb.Op
		if err := operation.UnmarshalVT(bytes); err != nil {
			return nil, err
		}
		if source := operation.GetSource(); source != nil {
			name, local := strings.CutPrefix(source.Identifier, "local://")
			if !local {
				continue
			}
			digest, exists := allowed[name]
			if !exists || source.Attrs[pb.AttrSharedKeyHint] != digest || source.Attrs[pb.AttrLocalUniqueID] != digest {
				return nil, fmt.Errorf("LLB requests an unauthorized captured source")
			}
			used[name] = true
		}
	}
	if len(used) != len(allowed) {
		return nil, fmt.Errorf("unused source authority was attached to execute")
	}
	mounts := make(map[string]fsutil.FS, len(allowed))
	for name := range allowed {
		directory := filepath.Join(root, name)
		info, err := os.Lstat(directory)
		if err != nil {
			return nil, err
		}
		if !info.IsDir() || info.Mode()&os.ModeSymlink != 0 {
			return nil, fmt.Errorf("captured input must be a non-symlink directory")
		}
		filesystem, err := fsutil.NewFS(directory)
		if err != nil {
			return nil, err
		}
		// Portable tree identity includes bytes, link targets and the executable
		// bit, not host timestamps, ownership, xattrs or other permission bits.
		filesystem, err = fsutil.NewFilterFS(filesystem, &fsutil.FilterOpt{Map: portableMetadata})
		if err != nil {
			return nil, err
		}
		mounts[name] = filesystem
	}
	return mounts, nil
}

func portableMetadata(_ string, stat *fstypes.Stat) fsutil.MapResult {
	stat.Uid, stat.Gid, stat.ModTime = 0, 0, 0
	stat.Xattrs = nil
	mode := os.FileMode(stat.Mode)
	switch {
	case mode.IsDir():
		stat.Mode = uint32(os.ModeDir | 0755)
	case mode.IsRegular():
		stat.Mode = 0644
		if mode&0111 != 0 {
			stat.Mode = 0755
		}
	case mode&os.ModeSymlink != 0:
		stat.Mode = uint32(os.ModeSymlink | 0777)
	}
	return fsutil.MapResultKeep
}
