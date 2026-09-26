// The production bridge entrypoint. Today it speaks the full protocol
// over stdio and is deliberately FAIL-CLOSED on Submit: no BuildKit
// client is linked into the binary yet, so a submitted session is
// accepted, fenced and terminally rejected — never a fake success.
// The B0-qualified client/lowering packages attach here in later B1/B2
// slices; nothing else about the loop changes when they do.
package main

import (
	"errors"
	"fmt"
	"io"
	"log"
	"os"

	"gripsack.dev/buildkit-bridge/protocol"
)

// serve drives one framed conversation: strict decode of every
// core→bridge frame, one fence gate per session, exactly one terminal
// per session, bounded logs.
func serve(in io.Reader, out io.Writer) error {
	gates := map[protocol.SessionID]*protocol.EventGate{}
	negotiated := false
	for {
		message, err := protocol.ReadFrame[protocol.ToBridge](in)
		if err != nil {
			if errors.Is(err, io.EOF) {
				return nil
			}
			return fmt.Errorf("reading a core frame: %w", err)
		}
		switch {
		case message.Negotiate != nil:
			if negotiated {
				return errors.New("double negotiation: the protocol negotiates exactly once")
			}
			if message.Negotiate.ProtocolVersion != protocol.ProtocolVersion {
				return fmt.Errorf(
					"core speaks protocol %d; this bridge speaks %d exactly",
					message.Negotiate.ProtocolVersion, protocol.ProtocolVersion)
			}
			negotiated = true
			reply := protocol.FromBridge{NegotiateOK: &protocol.NegotiateOK{
				ProtocolVersion: protocol.ProtocolVersion,
				DaemonVersion:   "standalone (no buildkit client linked)",
				Capabilities:    []string{"protocol.v1"},
			}}
			if err := writeFrame(out, reply); err != nil {
				return err
			}
		case message.Submit != nil:
			if !negotiated {
				return errors.New("submit before negotiation")
			}
			session := message.Submit.Session
			if err := session.Validate(); err != nil {
				return err
			}
			if err := message.Submit.Definition.Validate(); err != nil {
				return err
			}
			if err := message.Submit.Exporter.Validate(); err != nil {
				return err
			}
			if _, exists := gates[session]; exists {
				return fmt.Errorf("session %q submitted twice", session)
			}
			const epoch = 1
			gate := protocol.NewEventGate(epoch)
			gates[session] = gate
			if err := writeFrame(out, protocol.FromBridge{Accepted: &protocol.Accepted{Session: session, Epoch: epoch}}); err != nil {
				return err
			}
			// FAIL-CLOSED: no client is linked, so the session ends in a
			// terminal failure — with bounded diagnostics, never a lie.
			logLine := protocol.FromBridge{Event: &protocol.Event{
				Session: session, Epoch: epoch,
				Log: &protocol.LogEvent{
					Vertex:    "bridge",
					Chunk:     []byte("no buildkit client is linked into this bridge yet\n"),
					Truncated: false,
				},
			}}
			if err := writeFrame(out, logLine); err != nil {
				return err
			}
			if err := gate.Log(epoch); err != nil {
				return err
			}
			if err := gate.Terminal(epoch, protocol.TerminalFailed); err != nil {
				return err
			}
			failure := protocol.FromBridge{Failed: &protocol.Failed{
				Session: session, Epoch: epoch,
				Code:    protocol.FailureInternal,
				Message: "no buildkit client is linked into this bridge yet",
			}}
			if err := writeFrame(out, failure); err != nil {
				return err
			}
		case message.Cancel != nil:
			if !negotiated {
				return errors.New("cancel before negotiation")
			}
			session := message.Cancel.Session
			gate, exists := gates[session]
			if !exists {
				// Idempotent: cancelling an unknown session is not an
				// error and invents no state.
				if err := writeFrame(out, protocol.FromBridge{Cancelled: &protocol.Terminal{Session: session, Epoch: 0}}); err != nil {
					return err
				}
				continue
			}
			if _, terminal := gate.TerminalState(); !terminal {
				if err := gate.Terminal(1, protocol.TerminalCancelled); err != nil {
					return err
				}
			}
			if err := writeFrame(out, protocol.FromBridge{Cancelled: &protocol.Terminal{Session: session, Epoch: 1}}); err != nil {
				return err
			}
		default:
			return errors.New("empty ToBridge frame")
		}
	}
}

func writeFrame(out io.Writer, message protocol.FromBridge) error {
	frame, err := protocol.EncodeFrame(message)
	if err != nil {
		return err
	}
	_, err = out.Write(frame)
	return err
}

func main() {
	log.SetFlags(0)
	log.SetPrefix("bridge: ")
	if err := serve(os.Stdin, os.Stdout); err != nil {
		log.Fatal(err)
	}
}
