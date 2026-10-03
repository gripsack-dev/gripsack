package protocol

import (
	"encoding/json"
	"fmt"
	"path"
	"strings"
)

type BuildPlan struct {
	Platform Platform     `json:"platform"`
	Nodes    []Node       `json:"nodes"`
	Root     uint32       `json:"root"`
	Exporter ExporterPlan `json:"exporter"`
}
type ImageNode struct {
	Kind      string `json:"kind"`
	Reference string `json:"reference"`
}
type LocalNode struct {
	Kind   string `json:"kind"`
	Name   string `json:"name"`
	Digest string `json:"digest"`
}
type FileNode struct {
	Kind  string  `json:"kind"`
	Input *uint32 `json:"input"`
	Path  string  `json:"path"`
	Data  []byte  `json:"data"`
	Mode  uint32  `json:"mode"`
}
type DirectoryNode struct {
	Kind  string  `json:"kind"`
	Input *uint32 `json:"input"`
	Path  string  `json:"path"`
	Mode  uint32  `json:"mode"`
}
type CopyNode struct {
	Kind        string  `json:"kind"`
	Input       *uint32 `json:"input"`
	Source      uint32  `json:"source"`
	SourcePath  string  `json:"source_path"`
	Destination string  `json:"destination"`
	Contents    bool    `json:"contents"`
}
type InstallNode struct {
	CopyNode
	UID uint32 `json:"uid"`
	GID uint32 `json:"gid"`
}
type Mount struct {
	Source      *uint32 `json:"source"`
	Destination string  `json:"destination"`
	Readonly    bool    `json:"readonly"`
}
type ProcessNode struct {
	Kind   string   `json:"kind"`
	Root   uint32   `json:"root"`
	Argv   []string `json:"argv"`
	Env    []string `json:"env"`
	Cwd    string   `json:"cwd"`
	Mounts []Mount  `json:"mounts"`
	Output string   `json:"output"`
}
type Node struct {
	Image     *ImageNode
	Local     *LocalNode
	File      *FileNode
	Directory *DirectoryNode
	Copy      *CopyNode
	Install   *InstallNode
	Process   *ProcessNode
}

func (node *Node) UnmarshalJSON(data []byte) error {
	var tag struct {
		Kind string `json:"kind"`
	}
	if err := json.Unmarshal(data, &tag); err != nil {
		return err
	}
	var target any
	var fields []string
	*node = Node{}
	switch tag.Kind {
	case "image":
		node.Image = &ImageNode{}
		target = node.Image
		fields = []string{"reference"}
	case "local":
		node.Local = &LocalNode{}
		target = node.Local
		fields = []string{"name", "digest"}
	case "file":
		node.File = &FileNode{}
		target = node.File
		fields = []string{"path", "data", "mode"}
	case "directory":
		node.Directory = &DirectoryNode{}
		target = node.Directory
		fields = []string{"path", "mode"}
	case "copy":
		node.Copy = &CopyNode{}
		target = node.Copy
		fields = []string{"source", "source_path", "destination", "contents"}
	case "install":
		node.Install = &InstallNode{}
		target = node.Install
		fields = []string{"source", "source_path", "destination", "contents", "uid", "gid"}
	case "process":
		node.Process = &ProcessNode{}
		target = node.Process
		fields = []string{"root", "argv", "env", "cwd", "mounts", "output"}
	default:
		return fmt.Errorf("unsupported production node kind %q", tag.Kind)
	}
	if err := requireFields(data, fields...); err != nil {
		return err
	}
	return DecodeStrict(data, target)
}
func (p *BuildPlan) UnmarshalJSON(data []byte) error {
	if err := requireFields(data, "platform", "nodes", "root", "exporter"); err != nil {
		return err
	}
	type plain BuildPlan
	return DecodeStrict(data, (*plain)(p))
}
func (m *Mount) UnmarshalJSON(data []byte) error {
	if err := requireFields(data, "destination", "readonly"); err != nil {
		return err
	}
	type plain Mount
	return DecodeStrict(data, (*plain)(m))
}
func AbsolutePath(value string) bool {
	return strings.HasPrefix(value, "/") && !strings.ContainsRune(value, 0) && path.Clean(value) == value
}
func overlaps(left, right string) bool {
	return left == right || strings.HasPrefix(left, right+"/") || strings.HasPrefix(right, left+"/")
}
func (n Node) inputs(visit func(uint32) error) error {
	var err error
	optional := func(value *uint32) {
		if value != nil && err == nil {
			err = visit(*value)
		}
	}
	switch {
	case n.File != nil:
		optional(n.File.Input)
	case n.Directory != nil:
		optional(n.Directory.Input)
	case n.Copy != nil:
		optional(n.Copy.Input)
		if err == nil {
			err = visit(n.Copy.Source)
		}
	case n.Install != nil:
		optional(n.Install.Input)
		if err == nil {
			err = visit(n.Install.Source)
		}
	case n.Process != nil:
		err = visit(n.Process.Root)
		for _, mount := range n.Process.Mounts {
			optional(mount.Source)
		}
	}
	return err
}
func (p BuildPlan) Validate() error {
	if err := p.Platform.Validate(); err != nil {
		return err
	}
	if err := p.Exporter.Validate(); err != nil {
		return err
	}
	if len(p.Nodes) == 0 || len(p.Nodes) > MaxPlanNodes || int(p.Root) >= len(p.Nodes) {
		return fmt.Errorf("invalid plan node/root bound")
	}
	sourceNames := map[string]string{}
	for index, node := range p.Nodes {
		if err := node.inputs(func(input uint32) error {
			if int(input) >= index {
				return fmt.Errorf("node %d has a forward/missing/cyclic input", index)
			}
			return nil
		}); err != nil {
			return err
		}
		if err := node.validate(); err != nil {
			return fmt.Errorf("node %d: %w", index, err)
		}
		if node.Local != nil {
			source := node.Local
			if previous, exists := sourceNames[source.Name]; exists && previous != source.Digest {
				return fmt.Errorf("conflicting captured source name")
			}
			sourceNames[source.Name] = source.Digest
		}
	}
	reachable := make([]bool, len(p.Nodes))
	reachable[p.Root] = true
	for i := len(p.Nodes) - 1; i >= 0; i-- {
		if reachable[i] {
			_ = p.Nodes[i].inputs(func(input uint32) error { reachable[input] = true; return nil })
		}
	}
	for i, present := range reachable {
		if !present {
			return fmt.Errorf("node %d is unreachable from selected output", i)
		}
	}
	return nil
}
func (n Node) validate() error {
	switch {
	case n.Image != nil:
		name, digest, ok := strings.Cut(n.Image.Reference, "@sha256:")
		if !ok || !strings.Contains(name, "/") || strings.ContainsAny(name, "@ \t\r\n") || !ValidDigest(digest) {
			return fmt.Errorf("image must be normalized and digest-pinned")
		}
	case n.Local != nil:
		if !SafeAtom(n.Local.Name) || !ValidDigest(n.Local.Digest) {
			return fmt.Errorf("invalid captured source name/digest")
		}
	case n.File != nil:
		f := n.File
		if !AbsolutePath(f.Path) || f.Path == "/" || f.Mode > 0777 || len(f.Data) > MaxDefinitionBytes {
			return fmt.Errorf("invalid literal file")
		}
	case n.Directory != nil:
		if !AbsolutePath(n.Directory.Path) || n.Directory.Mode > 0777 {
			return fmt.Errorf("invalid directory")
		}
	case n.Copy != nil:
		if !AbsolutePath(n.Copy.SourcePath) || !AbsolutePath(n.Copy.Destination) {
			return fmt.Errorf("invalid copy selector/destination")
		}
	case n.Install != nil:
		if !AbsolutePath(n.Install.SourcePath) || !AbsolutePath(n.Install.Destination) {
			return fmt.Errorf("invalid image install selector/destination")
		}
	case n.Process != nil:
		return n.Process.validate()
	default:
		return fmt.Errorf("missing production node")
	}
	return nil
}
func (p ProcessNode) validate() error {
	if len(p.Argv) == 0 || !AbsolutePath(p.Cwd) {
		return fmt.Errorf("process needs an executable and an absolute cwd")
	}
	program := p.Argv[0]
	if !AbsolutePath(program) && (program == "" || program == "." || program == ".." || strings.Contains(program, "/")) {
		return fmt.Errorf("executable must be absolute or a bare name in the explicit image-local PATH")
	}
	for _, arg := range p.Argv {
		if strings.ContainsRune(arg, 0) {
			return fmt.Errorf("NUL in process argv")
		}
	}
	previous, hasPath := "", false
	for _, binding := range p.Env {
		key, value, ok := strings.Cut(binding, "=")
		if !ok || !envKey(key) || key <= previous || strings.ContainsRune(value, 0) {
			return fmt.Errorf("environment must have sorted unique valid keys")
		}
		previous = key
		hasPath = hasPath || key == "PATH"
	}
	if !hasPath {
		return fmt.Errorf("an explicit PATH binding is required; use PATH= for no search path")
	}
	outputs := 0
	for i, mount := range p.Mounts {
		if !AbsolutePath(mount.Destination) || mount.Destination == "/" {
			return fmt.Errorf("invalid non-root mount")
		}
		if !mount.Readonly {
			if mount.Destination != p.Output {
				return fmt.Errorf("only output mount may be writable")
			}
			outputs++
		}
		for _, earlier := range p.Mounts[:i] {
			if overlaps(earlier.Destination, mount.Destination) {
				return fmt.Errorf("overlapping mounts")
			}
		}
	}
	if outputs != 1 || len(p.Mounts) >= MaxPlanNodes {
		return fmt.Errorf("process needs exactly one writable output and bounded mounts")
	}
	return nil
}
func envKey(key string) bool {
	if len(key) == 0 {
		return false
	}
	for i := range len(key) {
		c := key[i]
		if !(c >= 'a' && c <= 'z' || c >= 'A' && c <= 'Z' || c == '_' || i > 0 && c >= '0' && c <= '9') {
			return false
		}
	}
	return true
}
