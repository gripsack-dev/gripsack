package client

import (
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"sync"
	"time"

	bk "github.com/moby/buildkit/client"
	imagekeys "github.com/moby/buildkit/exporter/containerimage/exptypes"
	exportkeys "github.com/moby/buildkit/exporter/exptypes"
	ocispec "github.com/opencontainers/image-spec/specs-go/v1"
	"gripsack.dev/buildkit-bridge/protocol"
)

// This profile is mirrored by the core's independent OCI admission and the
// length-prefixed exporter digest. No user/ambient map is merged into it.
const compatibilityVersion = 30
const maxOCIArchiveBytes int64 = 8 * 1024 * 1024 * 1024
const ociArchiveName = "image.oci.tar"

func exportEntry(request protocol.ExecuteRequest, output string) (bk.ExportEntry, *archiveOutput, error) {
	if request.Exporter.Kind == "local" {
		return bk.ExportEntry{Type: bk.ExporterLocal, OutputDir: output}, nil, nil
	}
	config := request.Exporter.Config
	epoch := time.Unix(0, 0).UTC()
	image := ocispec.Image{
		Created:  &epoch,
		Platform: ocispec.Platform{OS: request.Platform.OS, Architecture: request.Platform.Architecture},
		Config:   ocispec.ImageConfig{Entrypoint: config.Entrypoint, Cmd: config.Args, Env: config.Env, WorkingDir: config.Cwd, User: config.User},
		RootFS:   ocispec.RootFS{Type: "layers", DiffIDs: nil},
	}
	configuration, err := json.Marshal(image)
	if err != nil {
		return bk.ExportEntry{}, nil, err
	}
	file, err := os.OpenFile(filepath.Join(output, ociArchiveName), os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0600)
	if err != nil {
		return bk.ExportEntry{}, nil, err
	}
	archive := &archiveOutput{file: file, remaining: maxOCIArchiveBytes}
	return bk.ExportEntry{
		Type: bk.ExporterOCI,
		Attrs: map[string]string{
			"tar": "true", "oci-mediatypes": "true", "oci-artifact": "false",
			"compression": "gzip", "compression-level": "6", "force-compression": "true",
			"rewrite-timestamp": "true", string(exportkeys.OptKeySourceDateEpoch): "0",
			// v0.33 OCI Resolve retains metadata attributes and Export merges them
			// into the solver result. No frontend or alternate LLB is needed.
			imagekeys.ExporterImageConfigKey: string(configuration),
		},
		Output: archive.open,
	}, archive, nil
}

// A single bounded file writer; repeated grants, writes after close and failed
// durability barriers cannot masquerade as a completed export.
type archiveOutput struct {
	mu         sync.Mutex
	file       *os.File
	remaining  int64
	granted    bool
	closed     bool
	closeError error
}

func (a *archiveOutput) open(_ map[string]string) (io.WriteCloser, error) {
	a.mu.Lock()
	defer a.mu.Unlock()
	if a.granted || a.closed {
		return nil, fmt.Errorf("OCI archive output was already granted or closed")
	}
	a.granted = true
	return a, nil
}
func (a *archiveOutput) Write(bytes []byte) (int, error) {
	a.mu.Lock()
	defer a.mu.Unlock()
	if a.closed {
		return 0, os.ErrClosed
	}
	if int64(len(bytes)) > a.remaining {
		return 0, fmt.Errorf("OCI archive exceeds its eight-GiB export bound")
	}
	count, err := a.file.Write(bytes)
	a.remaining -= int64(count)
	return count, err
}
func (a *archiveOutput) Close() error {
	a.mu.Lock()
	defer a.mu.Unlock()
	if !a.closed {
		a.closed = true
		a.closeError = errors.Join(a.file.Sync(), a.file.Close())
	}
	return a.closeError
}
func (a *archiveOutput) complete() error {
	a.mu.Lock()
	defer a.mu.Unlock()
	if !a.granted || !a.closed || a.remaining == maxOCIArchiveBytes {
		return fmt.Errorf("OCI solve did not finish its granted archive output")
	}
	return a.closeError
}
