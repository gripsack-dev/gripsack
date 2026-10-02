// Package protocol owns the bounded v3 core/bridge wire, not build policy.
package protocol

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
)

const (
	ProtocolVersion       = 3
	MaxFrameBytes         = 4 * 1024 * 1024
	MaxDefinitionBytes    = 2 * 1024 * 1024
	MaxPlanNodes          = 256
	MaxLogChunkBytes      = 64 * 1024
	MaxLogEvents          = 4096
	ExpectedDaemonVersion = "v0.33.0"
)

type Identity struct {
	Session string `json:"session"`
	Attempt uint64 `json:"attempt"`
	Epoch   uint64 `json:"epoch"`
}

type WorkerBinding struct {
	Instance string `json:"instance"`
	Epoch    uint64 `json:"epoch"`
}

func (worker WorkerBinding) Validate() error {
	if !ValidDigest(worker.Instance) || worker.Epoch == 0 {
		return fmt.Errorf("owned worker instance and epoch are required")
	}
	return nil
}

type ExecutionIdentity struct {
	Identity
	Worker WorkerBinding `json:"worker"`
}

func (id Identity) Validate() error {
	if !SafeAtom(id.Session) || id.Attempt == 0 || id.Epoch == 0 {
		return fmt.Errorf("session, attempt and epoch must be admitted nonempty identities")
	}
	return nil
}
func SafeAtom(value string) bool {
	if len(value) == 0 || len(value) > 64 {
		return false
	}
	for _, c := range []byte(value) {
		if !(c >= 'a' && c <= 'z' || c >= 'A' && c <= 'Z' || c >= '0' && c <= '9' || c == '-' || c == '_') {
			return false
		}
	}
	return true
}
func ValidDigest(value string) bool {
	if len(value) != 64 {
		return false
	}
	for _, c := range []byte(value) {
		if !(c >= 'a' && c <= 'f' || c >= '0' && c <= '9') {
			return false
		}
	}
	return true
}
func Digest(bytes []byte) string { sum := sha256.Sum256(bytes); return hex.EncodeToString(sum[:]) }

type Platform struct {
	OS           string `json:"os"`
	Architecture string `json:"architecture"`
}

func (p Platform) Validate() error {
	if p.OS != "linux" || p.Architecture != "amd64" && p.Architecture != "arm64" {
		return fmt.Errorf("unsupported execution platform %q/%q", p.OS, p.Architecture)
	}
	return nil
}

type LowerRequest struct {
	ProtocolVersion uint32 `json:"protocol_version"`
	Identity
	Plan BuildPlan `json:"plan"`
}
type SourceBinding struct {
	Name   string `json:"name"`
	Digest string `json:"digest"`
}
type ExecuteRequest struct {
	ProtocolVersion uint32 `json:"protocol_version"`
	Identity
	Worker           WorkerBinding   `json:"worker"`
	Definition       []byte          `json:"definition"`
	DefinitionDigest string          `json:"definition_digest"`
	Exporter         ExporterPlan    `json:"exporter"`
	ExporterDigest   string          `json:"exporter_digest"`
	Sources          []SourceBinding `json:"sources"`
	Platform         Platform        `json:"platform"`
}
type NodeWitness struct {
	Node   uint32 `json:"node"`
	Vertex string `json:"vertex"`
	Output int64  `json:"output"`
}
type Lowered struct {
	Definition []byte        `json:"definition"`
	Witness    []NodeWitness `json:"witness"`
	Exporter   ExporterPlan  `json:"exporter"`
}
type Prepared struct {
	Identity
	Lowered Lowered `json:"lowered"`
}
type Accepted struct {
	Identity
	Worker        WorkerBinding `json:"worker"`
	DaemonVersion string        `json:"daemon_version"`
	Platform      Platform      `json:"platform"`
}
type Event struct {
	Identity
	Worker    WorkerBinding `json:"worker"`
	Vertex    string        `json:"vertex"`
	Chunk     []byte        `json:"chunk"`
	Truncated bool          `json:"truncated"`
}
type Failed struct {
	Identity
	Worker   *WorkerBinding `json:"worker,omitempty"`
	Code     string         `json:"code"`
	Message  string         `json:"message"`
	Vertices []string       `json:"vertices"`
}

// Exactly one outgoing variant is populated by the effect owner.
type Response struct {
	Prepared  *Prepared          `json:"Prepared,omitempty"`
	Accepted  *Accepted          `json:"Accepted,omitempty"`
	Event     *Event             `json:"Event,omitempty"`
	Exported  *ExecutionIdentity `json:"Exported,omitempty"`
	Done      *ExecutionIdentity `json:"Done,omitempty"`
	Failed    *Failed            `json:"Failed,omitempty"`
	Cancelled *ExecutionIdentity `json:"Cancelled,omitempty"`
}

func (r LowerRequest) Validate() error {
	if r.ProtocolVersion != ProtocolVersion {
		return fmt.Errorf("protocol version must be %d", ProtocolVersion)
	}
	if err := r.Identity.Validate(); err != nil {
		return err
	}
	return r.Plan.Validate()
}
func (r ExecuteRequest) Validate() error {
	if r.ProtocolVersion != ProtocolVersion {
		return fmt.Errorf("protocol version must be %d", ProtocolVersion)
	}
	if err := r.Identity.Validate(); err != nil {
		return err
	}
	if err := r.Worker.Validate(); err != nil {
		return err
	}
	if err := r.Platform.Validate(); err != nil {
		return err
	}
	if len(r.Definition) == 0 || len(r.Definition) > MaxDefinitionBytes || !ValidDigest(r.DefinitionDigest) || Digest(r.Definition) != r.DefinitionDigest {
		return fmt.Errorf("definition bytes do not match the checked digest/bound")
	}
	exporterDigest, err := r.Exporter.Digest()
	if err != nil {
		return err
	}
	if !ValidDigest(r.ExporterDigest) || exporterDigest != r.ExporterDigest {
		return fmt.Errorf("exporter options do not match their checked digest")
	}
	if len(r.Sources) > MaxPlanNodes {
		return fmt.Errorf("source binding count exceeds plan bound")
	}
	seen := make(map[string]string, len(r.Sources))
	for _, source := range r.Sources {
		if !SafeAtom(source.Name) || !ValidDigest(source.Digest) {
			return fmt.Errorf("invalid captured source binding")
		}
		if previous, exists := seen[source.Name]; exists && previous != source.Digest {
			return fmt.Errorf("conflicting captured source binding")
		}
		seen[source.Name] = source.Digest
	}
	return nil
}

// Required fields must exist even where Go's zero value would look valid.
func requireFields(data []byte, fields ...string) error {
	var raw map[string]json.RawMessage
	if err := json.Unmarshal(data, &raw); err != nil {
		return err
	}
	for _, field := range fields {
		if value, present := raw[field]; !present || string(value) == "null" {
			return fmt.Errorf("missing or null required field %q", field)
		}
	}
	return nil
}
func (r *LowerRequest) UnmarshalJSON(data []byte) error {
	if err := requireFields(data, "protocol_version", "session", "attempt", "epoch", "plan"); err != nil {
		return err
	}
	type plain LowerRequest
	return DecodeStrict(data, (*plain)(r))
}
func (r *ExecuteRequest) UnmarshalJSON(data []byte) error {
	if err := requireFields(data, "protocol_version", "session", "attempt", "epoch", "worker", "definition", "definition_digest", "exporter", "exporter_digest", "sources", "platform"); err != nil {
		return err
	}
	type plain ExecuteRequest
	return DecodeStrict(data, (*plain)(r))
}
