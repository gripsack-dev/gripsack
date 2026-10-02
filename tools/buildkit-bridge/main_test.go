package main

import (
	"bytes"
	"crypto/sha256"
	"encoding/binary"
	"testing"

	"gripsack.dev/buildkit-bridge/protocol"
)

func sha256HexForTest(t *testing.T, text string) string {
	t.Helper()
	sum := sha256.Sum256([]byte(text))
	const digits = "0123456789abcdef"
	out := make([]byte, 0, 64)
	for _, b := range sum {
		out = append(out, digits[b>>4], digits[b&0xf])
	}
	return string(out)
}

type conversation struct {
	t      *testing.T
	input  bytes.Buffer
	output bytes.Buffer
}

func (c *conversation) send(message protocol.ToBridge) {
	c.t.Helper()
	frame, err := protocol.EncodeFrame(message)
	if err != nil {
		c.t.Fatalf("encode core frame: %v", err)
	}
	if _, err := c.input.Write(frame); err != nil {
		c.t.Fatalf("queue core frame: %v", err)
	}
}

func (c *conversation) run() error {
	c.t.Helper()
	return serve(bytes.NewReader(c.input.Bytes()), &c.output)
}

func (c *conversation) replies() []protocol.FromBridge {
	c.t.Helper()
	var replies []protocol.FromBridge
	for c.output.Len() >= 8 {
		declared := binary.LittleEndian.Uint64(c.output.Bytes()[:8])
		frame := c.output.Bytes()[:8+declared]
		message, err := protocol.DecodeFrame[protocol.FromBridge](frame)
		if err != nil {
			c.t.Fatalf("decode bridge reply: %v", err)
		}
		replies = append(replies, *message)
		c.output.Next(8 + int(declared))
	}
	return replies
}

func negotiateMessage() protocol.ToBridge {
	return protocol.ToBridge{Negotiate: &protocol.Negotiate{
		ProtocolVersion: protocol.ProtocolVersion, Capabilities: []string{"protocol.v1"},
	}}
}

func TestServeNegotiateSubmitFailsClosed(t *testing.T) {
	definition := protocol.Digest(sha256HexForTest(t, "definition"))
	exporter := protocol.Digest(sha256HexForTest(t, "exporter"))
	c := &conversation{t: t}
	c.send(negotiateMessage())
	c.send(protocol.ToBridge{Submit: &protocol.Submit{
		Session: "build-1", Definition: definition, DefinitionLen: 10, Exporter: exporter,
	}})
	if err := c.run(); err != nil {
		t.Fatalf("serve: %v", err)
	}
	replies := c.replies()
	if len(replies) != 4 {
		t.Fatalf("expected negotiate-ok, accepted, log, failed — got %d replies", len(replies))
	}
	if replies[1].Accepted == nil || replies[1].Accepted.Session != "build-1" || replies[1].Accepted.Epoch != 1 {
		t.Fatalf("second reply must accept the session at epoch 1: %+v", replies[1])
	}
	if replies[0].NegotiateOK == nil || replies[0].NegotiateOK.ProtocolVersion != protocol.ProtocolVersion {
		t.Fatalf("first reply must be a matching NegotiateOk: %+v", replies[0])
	}
	if replies[2].Event == nil || replies[2].Event.Log == nil {
		t.Fatalf("third reply must be the bounded diagnostic log: %+v", replies[2])
	}
	if len(replies[2].Event.Log.Chunk) == 0 || replies[2].Event.Log.Truncated {
		t.Fatal("diagnostic log must carry a bounded chunk")
	}
	failure := replies[3].Failed
	if failure == nil || failure.Code != protocol.FailureInternal {
		t.Fatalf("submit must terminally fail closed: %+v", replies[3])
	}
	if failure.Session != "build-1" || failure.Epoch != 1 {
		t.Fatalf("failure must fence the submitted session: %+v", failure)
	}
}

func TestServeCancelIsIdempotentAndFences(t *testing.T) {
	c := &conversation{t: t}
	c.send(negotiateMessage())
	c.send(protocol.ToBridge{Cancel: &protocol.Cancel{Session: "unknown"}})
	c.send(protocol.ToBridge{Cancel: &protocol.Cancel{Session: "unknown"}})
	if err := c.run(); err != nil {
		t.Fatalf("serve: %v", err)
	}
	replies := c.replies()
	if len(replies) != 3 {
		t.Fatalf("expected negotiate-ok + two idempotent cancels, got %d", len(replies))
	}
	for _, reply := range replies[1:] {
		if reply.Cancelled == nil || reply.Cancelled.Session != "unknown" {
			t.Fatalf("cancel must reply cancelled idempotently: %+v", reply)
		}
	}
}

func TestServeRejectsProtocolViolations(t *testing.T) {
	cases := []struct {
		name string
		feed func(c *conversation)
	}{
		{"submit before negotiation", func(c *conversation) {
			c.send(protocol.ToBridge{Cancel: &protocol.Cancel{Session: "s"}})
		}},
		{"wrong protocol version", func(c *conversation) {
			c.send(protocol.ToBridge{Negotiate: &protocol.Negotiate{ProtocolVersion: protocol.ProtocolVersion + 1}})
		}},
		{"double negotiation", func(c *conversation) {
			c.send(negotiateMessage())
			c.send(negotiateMessage())
		}},
		{"invalid session on submit", func(c *conversation) {
			definition := protocol.Digest(sha256HexForTest(t, "d"))
			c.send(negotiateMessage())
			c.send(protocol.ToBridge{Submit: &protocol.Submit{
				Session: "../escape", Definition: definition, Exporter: definition,
			}})
		}},
	}
	for _, testCase := range cases {
		t.Run(testCase.name, func(t *testing.T) {
			c := &conversation{t: t}
			testCase.feed(c)
			if err := c.run(); err == nil {
				t.Fatal("protocol violation accepted")
			}
		})
	}
}
