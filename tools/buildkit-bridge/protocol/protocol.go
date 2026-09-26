// Package protocol is the Go half of the bounded Rust↔Go bridge wire
// contract (Epic B §4/§10.3, plan/0051 B1 fences). The Rust core owns
// gripsack-buildkit::protocol; this package mirrors the exact same
// frame format, message shapes and fence rules so both sides decode
// each other strictly. The JSON shapes are fixed by the Rust serde
// definitions (externally tagged enums; flattened BridgeEvent) and the
// shared conformance corpus under fuzz/corpus/buildkit_protocol is
// decoded by BOTH implementations.
//
// Rules mirrored from the Rust side:
//   - frames are LE u64 length + JSON body; the declared length is
//     checked against MaxFrameBytes BEFORE the body is read
//   - unknown fields reject (DisallowUnknownFields)
//   - log chunks are base64 and capped after decode
//   - negotiation accepts exactly ProtocolVersion
//   - the fence gate accepts exactly one terminal per session
package protocol

import (
	"bytes"
	"encoding/base64"
	"encoding/binary"
	"encoding/json"
	"errors"
	"fmt"
	"io"
)

// MaxFrameBytes is the absolute frame cap, checked against the header
// before any body allocation.
const MaxFrameBytes = 256 * 1024

// MaxLogChunkBytes caps a decoded log chunk.
const MaxLogChunkBytes = 64 * 1024

// ProtocolVersion is the one version this pair speaks.
const ProtocolVersion uint32 = 1

// Digest is a sha256 hex digest: exactly 64 lowercase hex characters.
type Digest string

func (d Digest) Validate() error {
	if len(d) != 64 {
		return fmt.Errorf("digest must be 64 lowercase hex characters, got %d", len(d))
	}
	for i := 0; i < len(d); i++ {
		c := d[i]
		if !(c >= '0' && c <= '9' || c >= 'a' && c <= 'f') {
			return fmt.Errorf("digest contains non-lowercase-hex character %q", c)
		}
	}
	return nil
}

// SessionID is an opaque bounded ASCII session identity.
type SessionID string

func (s SessionID) Validate() error {
	if len(s) == 0 || len(s) > 64 {
		return fmt.Errorf("session id must be 1..=64 ASCII characters, got %d", len(s))
	}
	for i := 0; i < len(s); i++ {
		c := s[i]
		if !(c >= 'a' && c <= 'z' || c >= 'A' && c <= 'Z' || c >= '0' && c <= '9' || c == '-' || c == '_') {
			return fmt.Errorf("session id contains invalid character %q", c)
		}
	}
	return nil
}

// ToBridge is a core→bridge message. Exactly one field is set; the
// JSON shape is the externally tagged enum the Rust side defines.
type ToBridge struct {
	Negotiate *Negotiate
	Submit    *Submit
	Cancel    *Cancel
}

type Negotiate struct {
	ProtocolVersion uint32   `json:"protocol_version"`
	Capabilities    []string `json:"capabilities"`
}

type Submit struct {
	Session       SessionID `json:"session"`
	Definition    Digest    `json:"definition"`
	DefinitionLen uint64    `json:"definition_len"`
	Exporter      Digest    `json:"exporter"`
}

type Cancel struct {
	Session SessionID `json:"session"`
}

// FromBridge is a bridge→core message. Exactly one field is set.
type FromBridge struct {
	NegotiateOK *NegotiateOK
	Accepted    *Accepted
	Event       *Event
	Exported    *Exported
	Done        *Terminal
	Failed      *Failed
	Cancelled   *Terminal
}

type NegotiateOK struct {
	ProtocolVersion uint32   `json:"protocol_version"`
	DaemonVersion   string   `json:"daemon_version"`
	Capabilities    []string `json:"capabilities"`
}

type Accepted struct {
	Session SessionID `json:"session"`
	Epoch   uint64    `json:"epoch"`
}

// Event carries a flattened BridgeEvent (Log or Progress), matching
// serde(flatten) on the Rust side: {"session":...,"epoch":...,"Log":{}}.
type Event struct {
	Session  SessionID
	Epoch    uint64
	Log      *LogEvent
	Progress *ProgressEvent
}

type LogEvent struct {
	Vertex    string `json:"vertex"`
	Chunk     []byte `json:"-"`
	Truncated bool   `json:"truncated"`
}

type ProgressEvent struct {
	Vertex  string `json:"vertex"`
	Message string `json:"message"`
}

type Exported struct {
	Session SessionID        `json:"session"`
	Epoch   uint64           `json:"epoch"`
	Outputs []ExportedOutput `json:"outputs"`
}

type ExportedOutput struct {
	Name   string `json:"name"`
	Digest Digest `json:"digest"`
	Size   uint64 `json:"size"`
}

type Terminal struct {
	Session SessionID `json:"session"`
	Epoch   uint64    `json:"epoch"`
}

type Failed struct {
	Session SessionID   `json:"session"`
	Epoch   uint64      `json:"epoch"`
	Code    FailureCode `json:"code"`
	Message string      `json:"message"`
}

type FailureCode string

const (
	FailureRejected   FailureCode = "rejected"
	FailureWorkerDied FailureCode = "worker_died"
	FailureExportFail FailureCode = "export_failed"
	FailureInvalidOut FailureCode = "invalid_output"
	FailureInternal   FailureCode = "internal"
)

// MarshalJSON renders the externally tagged shape with the flattened
// event kind, byte-identical in structure to the Rust encoder.
func (m ToBridge) MarshalJSON() ([]byte, error) {
	switch {
	case m.Negotiate != nil:
		return tagged("Negotiate", m.Negotiate)
	case m.Submit != nil:
		return tagged("Submit", m.Submit)
	case m.Cancel != nil:
		return tagged("Cancel", m.Cancel)
	}
	return nil, errors.New("ToBridge message is empty")
}

// UnmarshalJSON decodes the externally tagged shape strictly.
func (m *ToBridge) UnmarshalJSON(data []byte) error {
	tag, body, err := splitTagged(data)
	if err != nil {
		return err
	}
	strict := func(v any) error {
		dec := json.NewDecoder(bytes.NewReader(body))
		dec.DisallowUnknownFields()
		return dec.Decode(v)
	}
	switch tag {
	case "Negotiate":
		m.Negotiate = &Negotiate{}
		return strict(m.Negotiate)
	case "Submit":
		m.Submit = &Submit{}
		return strict(m.Submit)
	case "Cancel":
		m.Cancel = &Cancel{}
		return strict(m.Cancel)
	}
	return fmt.Errorf("unknown ToBridge tag %q", tag)
}

func (m FromBridge) MarshalJSON() ([]byte, error) {
	switch {
	case m.NegotiateOK != nil:
		return tagged("NegotiateOk", m.NegotiateOK)
	case m.Accepted != nil:
		return tagged("Accepted", m.Accepted)
	case m.Event != nil:
		kind, inner, err := m.Event.marshalKind()
		if err != nil {
			return nil, err
		}
		return taggedFlat("Event", m.Event.Session, m.Event.Epoch, kind, inner)
	case m.Exported != nil:
		return tagged("Exported", m.Exported)
	case m.Done != nil:
		return tagged("Done", m.Done)
	case m.Failed != nil:
		return tagged("Failed", m.Failed)
	case m.Cancelled != nil:
		return tagged("Cancelled", m.Cancelled)
	}
	return nil, errors.New("FromBridge message is empty")
}

func (m *FromBridge) UnmarshalJSON(data []byte) error {
	tag, body, err := splitTagged(data)
	if err != nil {
		return err
	}
	strict := func(v any) error {
		dec := json.NewDecoder(bytes.NewReader(body))
		dec.DisallowUnknownFields()
		return dec.Decode(v)
	}
	switch tag {
	case "NegotiateOk":
		m.NegotiateOK = &NegotiateOK{}
		return strict(m.NegotiateOK)
	case "Accepted":
		m.Accepted = &Accepted{}
		return strict(m.Accepted)
	case "Event":
		event, err := unmarshalEvent(body)
		if err != nil {
			return err
		}
		m.Event = event
		return nil
	case "Exported":
		m.Exported = &Exported{}
		return strict(m.Exported)
	case "Done":
		m.Done = &Terminal{}
		return strict(m.Done)
	case "Failed":
		m.Failed = &Failed{}
		return strict(m.Failed)
	case "Cancelled":
		m.Cancelled = &Terminal{}
		return strict(m.Cancelled)
	}
	return fmt.Errorf("unknown FromBridge tag %q", tag)
}

func (e Event) marshalKind() (string, any, error) {
	switch {
	case e.Log != nil:
		return "Log", struct {
			Vertex    string `json:"vertex"`
			Chunk     string `json:"chunk"`
			Truncated bool   `json:"truncated"`
		}{e.Log.Vertex, base64.StdEncoding.EncodeToString(e.Log.Chunk), e.Log.Truncated}, nil
	case e.Progress != nil:
		return "Progress", e.Progress, nil
	}
	return "", nil, errors.New("Event message carries no kind")
}

func unmarshalEvent(body []byte) (*Event, error) {
	var fields struct {
		Session  SessionID        `json:"session"`
		Epoch    uint64           `json:"epoch"`
		Log      *json.RawMessage `json:"Log,omitempty"`
		Progress *json.RawMessage `json:"Progress,omitempty"`
	}
	dec := json.NewDecoder(bytes.NewReader(body))
	dec.DisallowUnknownFields()
	if err := dec.Decode(&fields); err != nil {
		return nil, err
	}
	event := &Event{Session: fields.Session, Epoch: fields.Epoch}
	decodeStrict := func(raw *json.RawMessage, v any) error {
		if raw == nil {
			return nil
		}
		inner := json.NewDecoder(bytes.NewReader(*raw))
		inner.DisallowUnknownFields()
		return inner.Decode(v)
	}
	if fields.Log != nil {
		var wire struct {
			Vertex    string `json:"vertex"`
			Chunk     string `json:"chunk"`
			Truncated bool   `json:"truncated"`
		}
		if err := decodeStrict(fields.Log, &wire); err != nil {
			return nil, err
		}
		chunk, err := base64.StdEncoding.DecodeString(wire.Chunk)
		if err != nil {
			return nil, err
		}
		if len(chunk) > MaxLogChunkBytes {
			return nil, fmt.Errorf("log chunk exceeds %d bytes", MaxLogChunkBytes)
		}
		event.Log = &LogEvent{Vertex: wire.Vertex, Chunk: chunk, Truncated: wire.Truncated}
	}
	if fields.Progress != nil {
		progress := &ProgressEvent{}
		if err := decodeStrict(fields.Progress, progress); err != nil {
			return nil, err
		}
		event.Progress = progress
	}
	if event.Log == nil && event.Progress == nil {
		return nil, errors.New("Event carries neither Log nor Progress")
	}
	if event.Log != nil && event.Progress != nil {
		return nil, errors.New("Event carries both Log and Progress")
	}
	if err := event.Session.Validate(); err != nil {
		return nil, err
	}
	return event, nil
}

func tagged(tag string, payload any) ([]byte, error) {
	body, err := json.Marshal(payload)
	if err != nil {
		return nil, err
	}
	out := append([]byte(`{"`+tag+`":`), body...)
	return append(out, '}'), nil
}

func taggedFlat(tag string, session SessionID, epoch uint64, kind string, inner any) ([]byte, error) {
	body, err := json.Marshal(inner)
	if err != nil {
		return nil, err
	}
	head := fmt.Sprintf(`{"%s":{"session":%q,"epoch":%d,"%s":`, tag, session, epoch, kind)
	return append(append([]byte(head), body...), '}', '}'), nil
}

func splitTagged(data []byte) (string, []byte, error) {
	var envelope map[string]json.RawMessage
	dec := json.NewDecoder(bytes.NewReader(data))
	dec.DisallowUnknownFields()
	if err := dec.Decode(&envelope); err != nil {
		return "", nil, err
	}
	if len(envelope) != 1 {
		return "", nil, fmt.Errorf("expected exactly one tagged variant, got %d", len(envelope))
	}
	for tag, body := range envelope {
		return tag, body, nil
	}
	return "", nil, errors.New("unreachable")
}

// EncodeFrame writes the LE u64 length header then the JSON body.
func EncodeFrame[M any](message M) ([]byte, error) {
	body, err := json.Marshal(message)
	if err != nil {
		return nil, err
	}
	if len(body) > MaxFrameBytes {
		return nil, fmt.Errorf("encoder produced a %d byte frame over the %d cap", len(body), MaxFrameBytes)
	}
	frame := make([]byte, 8+len(body))
	binary.LittleEndian.PutUint64(frame[:8], uint64(len(body)))
	copy(frame[8:], body)
	return frame, nil
}

// DecodeFrame checks the declared length against the cap BEFORE any
// body allocation, then strictly decodes the JSON body.
func DecodeFrame[M any](frame []byte) (*M, error) {
	if len(frame) < 8 {
		return nil, fmt.Errorf("frame body is %d bytes but its header declared more", len(frame))
	}
	declared := binary.LittleEndian.Uint64(frame[:8])
	if declared > MaxFrameBytes {
		return nil, fmt.Errorf("frame header declares %d bytes; the protocol caps frames at %d", declared, MaxFrameBytes)
	}
	if uint64(len(frame)-8) != declared {
		return nil, fmt.Errorf("frame body is %d bytes but its header declared %d", len(frame)-8, declared)
	}
	var message M
	dec := json.NewDecoder(bytes.NewReader(frame[8:]))
	dec.DisallowUnknownFields()
	if err := dec.Decode(&message); err != nil {
		return nil, err
	}
	return &message, nil
}

// ReadFrame reads one frame from r: the header first, checked against
// the cap, then exactly the declared body — never more.
func ReadFrame[M any](r io.Reader) (*M, error) {
	var header [8]byte
	if _, err := io.ReadFull(r, header[:]); err != nil {
		return nil, err
	}
	declared := binary.LittleEndian.Uint64(header[:])
	if declared > MaxFrameBytes {
		return nil, fmt.Errorf("frame header declares %d bytes; the protocol caps frames at %d", declared, MaxFrameBytes)
	}
	body := make([]byte, declared)
	if _, err := io.ReadFull(r, body); err != nil {
		return nil, err
	}
	var message M
	dec := json.NewDecoder(bytes.NewReader(body))
	dec.DisallowUnknownFields()
	if err := dec.Decode(&message); err != nil {
		return nil, err
	}
	return &message, nil
}

// Negotiate enforces the one exact version pair.
func CheckNegotiation(reply *FromBridge) ([]string, error) {
	if reply.NegotiateOK == nil {
		return nil, fmt.Errorf("negotiation expected NegotiateOk, got something else")
	}
	if reply.NegotiateOK.ProtocolVersion != ProtocolVersion {
		return nil, fmt.Errorf("bridge negotiated protocol %d; this pair speaks %d exactly", reply.NegotiateOK.ProtocolVersion, ProtocolVersion)
	}
	return reply.NegotiateOK.Capabilities, nil
}

// TerminalKind mirrors the Rust Terminal enum.
type TerminalKind int

const (
	TerminalDone TerminalKind = iota
	TerminalFailed
	TerminalCancelled
)

// GateError mirrors the Rust GateError cases.
type GateError struct {
	Kind string
	Msg  string
}

func (e GateError) Error() string { return e.Msg }

// EventGate is the pure fence state machine for one session: events
// are accepted only for the live epoch, a session accepts exactly one
// terminal, and the log budget is bounded.
type EventGate struct {
	epoch       uint64
	terminal    *TerminalKind
	logsEmitted uint64
}

// MaxLogEvents is the per-session log chunk budget.
const MaxLogEvents uint64 = 4096

func NewEventGate(epoch uint64) *EventGate {
	return &EventGate{epoch: epoch}
}

func (g *EventGate) check(epoch uint64) error {
	if g.terminal != nil {
		return GateError{"already_terminal", fmt.Sprintf("session is already terminal (%v); a late event cannot flip it", *g.terminal)}
	}
	if epoch < g.epoch {
		return GateError{"stale_epoch", fmt.Sprintf("event epoch %d is stale; the session fence is at %d", epoch, g.epoch)}
	}
	if epoch > g.epoch {
		return GateError{"future_epoch", fmt.Sprintf("event epoch %d jumps ahead of the session fence %d", epoch, g.epoch)}
	}
	return nil
}

func (g *EventGate) Log(epoch uint64) error {
	if err := g.check(epoch); err != nil {
		return err
	}
	g.logsEmitted++
	if g.logsEmitted > MaxLogEvents {
		return GateError{"log_budget", fmt.Sprintf("log flood: %d event chunks exceed the %d chunk budget", g.logsEmitted, MaxLogEvents)}
	}
	return nil
}

func (g *EventGate) Observe(epoch uint64) error {
	return g.check(epoch)
}

func (g *EventGate) Terminal(epoch uint64, kind TerminalKind) error {
	if err := g.check(epoch); err != nil {
		return err
	}
	g.terminal = &kind
	return nil
}

func (g *EventGate) TerminalState() (TerminalKind, bool) {
	if g.terminal == nil {
		return 0, false
	}
	return *g.terminal, true
}
