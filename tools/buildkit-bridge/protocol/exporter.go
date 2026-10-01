package protocol

import (
	"crypto/sha256"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"hash"
	"strconv"
	"strings"
)

const MaxImageConfigBytes = 64 * 1024
const maxConfigEntries = 4096
const ociProfile = "gripsack-oci-v1\x00compatibility=30\x00media=oci\x00compression=gzip:6\x00force=true\x00epoch=0\x00rewrite=true\x00tar=true\x00"

type ExporterPlan struct {
	Kind   string       `json:"kind"`
	Config *ImageConfig `json:"config,omitempty"`
}
type ImageConfig struct {
	Entrypoint []string `json:"entrypoint"`
	Args       []string `json:"args"`
	Env        []string `json:"env"`
	Cwd        string   `json:"cwd"`
	User       string   `json:"user"`
}

func (e *ExporterPlan) UnmarshalJSON(data []byte) error {
	var tag struct {
		Kind string `json:"kind"`
	}
	if err := json.Unmarshal(data, &tag); err != nil {
		return err
	}
	*e = ExporterPlan{}
	switch tag.Kind {
	case "local":
		if err := DecodeStrict(data, &tag); err != nil {
			return err
		}
		e.Kind = tag.Kind
		return nil
	case "oci":
		if err := requireFields(data, "kind", "config"); err != nil {
			return err
		}
		type plain ExporterPlan
		return DecodeStrict(data, (*plain)(e))
	default:
		return fmt.Errorf("unsupported exporter %q", tag.Kind)
	}
}
func (c *ImageConfig) UnmarshalJSON(data []byte) error {
	if err := requireFields(data, "entrypoint", "args", "env", "cwd", "user"); err != nil {
		return err
	}
	type plain ImageConfig
	return DecodeStrict(data, (*plain)(c))
}
func (e ExporterPlan) Validate() error {
	switch e.Kind {
	case "local":
		if e.Config != nil {
			return fmt.Errorf("local exporter cannot carry image configuration")
		}
		return nil
	case "oci":
		if e.Config == nil {
			return fmt.Errorf("OCI exporter requires image configuration")
		}
		return e.Config.validate()
	default:
		return fmt.Errorf("unsupported exporter %q", e.Kind)
	}
}
func (e ExporterPlan) Digest() (string, error) {
	if err := e.Validate(); err != nil {
		return "", err
	}
	if e.Kind == "local" {
		return Digest([]byte(`{"kind":"local"}`)), nil
	}
	h := sha256.New()
	h.Write([]byte(ociProfile))
	for _, values := range [][]string{e.Config.Entrypoint, e.Config.Args, e.Config.Env} {
		writeLength(h, uint64(len(values)))
		for _, value := range values {
			writeString(h, value)
		}
	}
	writeString(h, e.Config.Cwd)
	writeString(h, e.Config.User)
	return hex.EncodeToString(h.Sum(nil)), nil
}
func writeLength(h hash.Hash, length uint64) {
	var bytes [8]byte
	binary.LittleEndian.PutUint64(bytes[:], length)
	h.Write(bytes[:])
}
func writeString(h hash.Hash, value string) {
	writeLength(h, uint64(len(value)))
	h.Write([]byte(value))
}
func (c ImageConfig) validate() error {
	if !AbsolutePath(c.Cwd) {
		return fmt.Errorf("image working directory must be normalized and absolute")
	}
	uid, gid, found := strings.Cut(c.User, ":")
	if !found || !numericID(uid) || !numericID(gid) {
		return fmt.Errorf("image user must be canonical numeric uid:gid")
	}
	if len(c.Entrypoint) != 0 && !AbsolutePath(c.Entrypoint[0]) {
		return fmt.Errorf("image entrypoint must be absolute")
	}
	bytes := len(c.Cwd) + len(c.User)
	if bytes > MaxImageConfigBytes {
		return fmt.Errorf("image configuration exceeds its byte bound")
	}
	for _, values := range [][]string{c.Entrypoint, c.Args, c.Env} {
		if len(values) > maxConfigEntries {
			return fmt.Errorf("image configuration entry count exceeds its bound")
		}
		for _, value := range values {
			if strings.ContainsRune(value, 0) || len(value) > MaxImageConfigBytes-bytes {
				return fmt.Errorf("image configuration contains NUL or exceeds its byte bound")
			}
			bytes += len(value)
		}
	}
	previous := ""
	for _, entry := range c.Env {
		key, _, found := strings.Cut(entry, "=")
		if !found || !envKey(key) || key <= previous {
			return fmt.Errorf("image environment keys must be valid, unique and sorted")
		}
		previous = key
	}
	return nil
}
func numericID(value string) bool {
	if value == "" || len(value) > 1 && value[0] == '0' {
		return false
	}
	for i := range len(value) {
		if value[i] < '0' || value[i] > '9' {
			return false
		}
	}
	_, err := strconv.ParseUint(value, 10, 32)
	return err == nil
}
