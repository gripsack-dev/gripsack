package protocol

import (
	"bytes"
	"crypto/sha256"
	"encoding/binary"
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

func sha256SumForTest(text string) []byte {
	sum := sha256.Sum256([]byte(text))
	return sum[:]
}

func roundTripFromBridge(t *testing.T, message FromBridge) {
	t.Helper()
	frame, err := EncodeFrame(message)
	if err != nil {
		t.Fatalf("encode: %v", err)
	}
	decoded, err := DecodeFrame[FromBridge](frame)
	if err != nil {
		t.Fatalf("decode: %v", err)
	}
	first, _ := json.Marshal(message)
	second, _ := json.Marshal(decoded)
	if !bytes.Equal(first, second) {
		t.Fatalf("re-encode changed the message:\n%s\n%s", first, second)
	}
}

func TestEveryFromBridgeMessageRoundTrips(t *testing.T) {
	session := SessionID("build-1a")
	toolDigest := Digest(DigestOfForTest(t, "tool"))
	roundTripFromBridge(t, FromBridge{NegotiateOK: &NegotiateOK{ProtocolVersion: ProtocolVersion, DaemonVersion: "v0.33.0", Capabilities: []string{"llb.v1"}}})
	roundTripFromBridge(t, FromBridge{Accepted: &Accepted{Session: session, Epoch: 7}})
	roundTripFromBridge(t, FromBridge{Event: &Event{Session: session, Epoch: 7, Log: &LogEvent{Vertex: "gcc", Chunk: []byte("compiling\n"), Truncated: false}}})
	roundTripFromBridge(t, FromBridge{Event: &Event{Session: session, Epoch: 7, Progress: &ProgressEvent{Vertex: "export", Message: "writing layer"}}})
	roundTripFromBridge(t, FromBridge{Exported: &Exported{Session: session, Epoch: 7, Outputs: []ExportedOutput{{Name: "tool", Digest: toolDigest, Size: 933288}}}})
	roundTripFromBridge(t, FromBridge{Done: &Terminal{Session: session, Epoch: 7}})
	roundTripFromBridge(t, FromBridge{Failed: &Failed{Session: session, Epoch: 7, Code: FailureWorkerDied, Message: "buildkitd exited"}})
	roundTripFromBridge(t, FromBridge{Cancelled: &Terminal{Session: session, Epoch: 7}})
}

func TestToBridgeRoundTripsAndValidates(t *testing.T) {
	session := SessionID("build-1a")
	definition := Digest(DigestOfForTest(t, "definition"))
	messages := []ToBridge{
		{Negotiate: &Negotiate{ProtocolVersion: ProtocolVersion, Capabilities: []string{"llb.v1"}}},
		{Submit: &Submit{Session: session, Definition: definition, DefinitionLen: 10, Exporter: Digest(DigestOfForTest(t, "exporter"))}},
		{Cancel: &Cancel{Session: session}},
	}
	for _, message := range messages {
		frame, err := EncodeFrame(message)
		if err != nil {
			t.Fatalf("encode: %v", err)
		}
		decoded, err := DecodeFrame[ToBridge](frame)
		if err != nil {
			t.Fatalf("decode: %v", err)
		}
		first, _ := json.Marshal(message)
		second, _ := json.Marshal(decoded)
		if !bytes.Equal(first, second) {
			t.Fatalf("re-encode changed the message:\n%s\n%s", first, second)
		}
	}
	if err := (SessionID("../escape")).Validate(); err == nil {
		t.Fatal("escaping session id accepted")
	}
	if err := (Digest("XYZ")).Validate(); err == nil {
		t.Fatal("malformed digest accepted")
	}
}

func DigestOfForTest(t *testing.T, text string) string {
	t.Helper()
	sum := sha256SumForTest(text)
	var hex string
	for _, b := range sum {
		const digits = "0123456789abcdef"
		hex += string(digits[b>>4]) + string(digits[b&0xf])
	}
	return hex
}

func TestHostileFramesReject(t *testing.T) {
	oversized := make([]byte, 8+16)
	binary.LittleEndian.PutUint64(oversized[:8], uint64(MaxFrameBytes+1))
	if _, err := DecodeFrame[FromBridge](oversized); err == nil {
		t.Fatal("oversized header accepted")
	}
	valid := []byte(`{"Accepted":{"session":"s","epoch":1}}`)
	frame := append(make([]byte, 8), valid...)
	binary.LittleEndian.PutUint64(frame[:8], uint64(len(valid)))
	if _, err := DecodeFrame[FromBridge](frame[:10]); err == nil {
		t.Fatal("truncated frame accepted")
	}
	trailing := append(append([]byte{}, frame...), '{')
	if _, err := DecodeFrame[FromBridge](trailing); err == nil {
		t.Fatal("trailing bytes accepted")
	}
	unknown := []byte(`{"Accepted":{"session":"s","epoch":1,"extra":true}}`)
	unknownFrame := append(make([]byte, 8), unknown...)
	binary.LittleEndian.PutUint64(unknownFrame[:8], uint64(len(unknown)))
	if _, err := DecodeFrame[FromBridge](unknownFrame); err == nil {
		t.Fatal("unknown field accepted")
	}
}

func TestNegotiationIsOneExactPair(t *testing.T) {
	if _, err := CheckNegotiation(&FromBridge{NegotiateOK: &NegotiateOK{ProtocolVersion: ProtocolVersion, DaemonVersion: "v", Capabilities: nil}}); err != nil {
		t.Fatalf("matching negotiation refused: %v", err)
	}
	if _, err := CheckNegotiation(&FromBridge{NegotiateOK: &NegotiateOK{ProtocolVersion: ProtocolVersion - 1, DaemonVersion: "v"}}); err == nil {
		t.Fatal("mismatched version accepted")
	}
	if _, err := CheckNegotiation(&FromBridge{Done: &Terminal{Session: "s", Epoch: 1}}); err == nil {
		t.Fatal("non-negotiate message accepted during negotiation")
	}
}

func TestGateFencesStaleDuplicateAndFloodingEvents(t *testing.T) {
	gate := NewEventGate(7)
	if err := gate.Log(7); err != nil {
		t.Fatalf("live log rejected: %v", err)
	}
	if err := gate.Observe(6); err == nil {
		t.Fatal("stale epoch accepted")
	}
	if err := gate.Observe(8); err == nil {
		t.Fatal("future epoch accepted")
	}
	if err := gate.Terminal(7, TerminalCancelled); err != nil {
		t.Fatalf("first terminal rejected: %v", err)
	}
	if _, ok := gate.TerminalState(); !ok {
		t.Fatal("terminal state lost")
	}
	if err := gate.Terminal(7, TerminalCancelled); err == nil {
		t.Fatal("duplicate terminal accepted")
	}
	if err := gate.Log(7); err == nil {
		t.Fatal("post-terminal log accepted")
	}

	flooding := NewEventGate(1)
	for i := uint64(0); i < MaxLogEvents; i++ {
		if err := flooding.Log(1); err != nil {
			t.Fatalf("budget rejected early at %d: %v", i, err)
		}
	}
	if err := flooding.Log(1); err == nil {
		t.Fatal("log flood accepted over the budget")
	}
}

// TestSharedConformanceCorpus decodes the exact wire bytes the Rust
// side's fuzz corpus ships: the two implementations must agree on
// every seed — the valid ones decode, the hostile ones reject.
func TestSharedConformanceCorpus(t *testing.T) {
	root := os.Getenv("GRIPSACK_PROTOCOL_CORPUS")
	if root == "" {
		root = "../../../fuzz/corpus/buildkit_protocol"
	}
	entries, err := os.ReadDir(root)
	if err != nil {
		t.Fatalf("shared corpus missing at %s: %v", root, err)
	}
	mustDecode := map[string]bool{
		"negotiate.txt": true, "accepted.txt": true, "log-event.txt": true, "done.txt": true,
		"oversized-header.bin": false, "truncated.txt": false, "unknown-field.txt": false,
	}
	seen := 0
	for _, entry := range entries {
		want, known := mustDecode[entry.Name()]
		if !known {
			continue
		}
		seen++
		frame, err := os.ReadFile(filepath.Join(root, entry.Name()))
		if err != nil {
			t.Fatalf("read seed: %v", err)
		}
		// The corpus mixes both message families; try FromBridge first
		// (the valid seeds are bridge→core shapes), then ToBridge.
		_, fromErr := DecodeFrame[FromBridge](frame)
		_, toErr := DecodeFrame[ToBridge](frame)
		if want && fromErr != nil && toErr != nil {
			t.Fatalf("%s must decode on the Go side too (from=%v to=%v)", entry.Name(), fromErr, toErr)
		}
		if !want && (fromErr == nil || toErr == nil) {
			t.Fatalf("%s must reject on the Go side too", entry.Name())
		}
	}
	if seen != len(mustDecode) {
		t.Fatalf("expected %d named seeds, saw %d — corpus drifted", len(mustDecode), seen)
	}
}
