// Package lower constructs upstream LLB without a daemon, resolver or host read.
// The core independently checks its serialized result before a separate execute.
package lower

import (
	"context"
	"fmt"
	"os"
	"strings"
	"time"

	"github.com/distribution/reference"
	"github.com/moby/buildkit/client/llb"
	ocispec "github.com/opencontainers/image-spec/specs-go/v1"
	"gripsack.dev/buildkit-bridge/protocol"
)

var fixedEpoch = time.Unix(0, 0).UTC()

func Prepare(ctx context.Context, plan protocol.BuildPlan) (*protocol.Lowered, error) {
	if err := plan.Validate(); err != nil {
		return nil, err
	}
	platform := llb.Platform(ocispec.Platform{OS: plan.Platform.OS, Architecture: plan.Platform.Architecture})
	constraints := llb.NewConstraints(platform)
	states := make([]llb.State, len(plan.Nodes))
	witness := make([]protocol.NodeWitness, len(plan.Nodes))
	input := func(index *uint32) llb.State {
		if index == nil {
			return llb.Scratch()
		}
		return states[*index]
	}
	for index, node := range plan.Nodes {
		if err := ctx.Err(); err != nil {
			return nil, err
		}
		var state llb.State
		switch {
		case node.Image != nil:
			image := node.Image.Reference
			normalized, err := reference.ParseNormalizedNamed(image)
			if err != nil || normalized.String() != image {
				return nil, fmt.Errorf("image is not a normalized pinned reference")
			}
			// No MetaResolver: inherited image Env/Cmd/WorkingDir is not input.
			state = llb.Image(image, platform)
		case node.Local != nil:
			source := node.Local
			state = llb.Local(source.Name, llb.SharedKeyHint(source.Digest), llb.LocalUniqueID(source.Digest))
		case node.File != nil:
			file := node.File
			state = input(file.Input).File(llb.Mkfile(file.Path, os.FileMode(file.Mode), file.Data, llb.WithCreatedTime(fixedEpoch)))
		case node.Directory != nil:
			directory := node.Directory
			state = input(directory.Input).File(llb.Mkdir(directory.Path, os.FileMode(directory.Mode), llb.WithParents(true), llb.WithCreatedTime(fixedEpoch)))
		case node.Copy != nil:
			copy := node.Copy
			state = input(copy.Input).File(llb.Copy(states[copy.Source], copy.SourcePath, copy.Destination, &llb.CopyInfo{
				CopyDirContentsOnly: copy.Contents, CreateDestPath: true, CreatedTime: &fixedEpoch,
			}))
		case node.Install != nil:
			install := node.Install
			state = input(install.Input).File(llb.Copy(states[install.Source], install.SourcePath, install.Destination, &llb.CopyInfo{
				CopyDirContentsOnly: install.Contents, CreateDestPath: true, CreatedTime: &fixedEpoch,
			}, llb.WithUIDGID(int(install.UID), int(install.GID))))
		case node.Process != nil:
			process := node.Process
			// Rebuild only State metadata, not its filesystem operation. Prior
			// argv/env/cwd or image config cannot leak into the next process.
			base := llb.NewState(states[process.Root].Output()).Dir(process.Cwd)
			for _, binding := range process.Env {
				key, value, _ := strings.Cut(binding, "=")
				base = base.AddEnv(key, value)
			}
			options := []llb.RunOption{llb.Args(process.Argv), llb.ReadonlyRootFS(), llb.Network(llb.NetModeNone), platform}
			for _, mount := range process.Mounts {
				if mount.Readonly {
					options = append(options, llb.AddMount(mount.Destination, input(mount.Source), llb.Readonly))
				} else {
					options = append(options, llb.AddMount(mount.Destination, input(mount.Source)))
				}
			}
			state = base.Run(options...).GetMount(process.Output)
		default:
			return nil, fmt.Errorf("unadmitted node %d", index)
		}
		states[index] = state
		output, err := state.Output().ToInput(ctx, constraints)
		if err != nil {
			return nil, fmt.Errorf("node %d witness: %w", index, err)
		}
		witness[index] = protocol.NodeWitness{Node: uint32(index), Vertex: output.Digest, Output: output.Index}
	}
	definition, err := states[plan.Root].Marshal(ctx, platform)
	if err != nil {
		return nil, err
	}
	bytes, err := definition.ToPB().MarshalVT()
	if err != nil {
		return nil, err
	}
	if len(bytes) > protocol.MaxDefinitionBytes {
		return nil, fmt.Errorf("definition exceeds byte bound")
	}
	return &protocol.Lowered{Definition: bytes, Witness: witness, Exporter: plan.Exporter}, nil
}
