// The internal bridge has two bounded, single-attempt commands: pure lowering
// and exact-byte execution. It is not a TypeScript frontend or package manager.
package main

import (
	"context"
	"errors"
	"flag"
	"fmt"
	"io"
	"math"
	"os"
	"os/signal"
	"syscall"
	"time"

	"gripsack.dev/buildkit-bridge/client"
	"gripsack.dev/buildkit-bridge/lower"
	"gripsack.dev/buildkit-bridge/protocol"
)

const maxFailureMessageBytes = 8192

func failure(output io.Writer, identity protocol.Identity, worker *protocol.WorkerBinding, code string, err error) error {
	message := err.Error()
	if len(message) > maxFailureMessageBytes {
		message = message[:maxFailureMessageBytes]
	}
	vertices := []string{}
	var solve *client.SolveFailure
	if errors.As(err, &solve) {
		vertices = solve.Vertices
	}
	return protocol.WriteFrame(output, protocol.Response{Failed: &protocol.Failed{Identity: identity, Worker: worker, Code: code, Message: message, Vertices: vertices}})
}

func serve(arguments []string, input io.Reader, output io.Writer) error {
	if len(arguments) == 0 {
		return fmt.Errorf("expected lower or execute")
	}
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()
	switch arguments[0] {
	case "lower":
		if len(arguments) != 1 {
			return fmt.Errorf("lower accepts no host configuration")
		}
		request, err := protocol.ReadFrame[protocol.LowerRequest](input)
		if err != nil {
			return err
		}
		if err := request.Identity.Validate(); err != nil {
			return err
		}
		if err := request.Validate(); err != nil {
			return failure(output, request.Identity, nil, "rejected", err)
		}
		lowered, err := lower.Prepare(ctx, request.Plan)
		if err != nil {
			return failure(output, request.Identity, nil, "rejected", err)
		}
		return protocol.WriteFrame(output, protocol.Response{Prepared: &protocol.Prepared{Identity: request.Identity, Lowered: *lowered}})
	case "execute":
		flags := flag.NewFlagSet("execute", flag.ContinueOnError)
		flags.SetOutput(io.Discard)
		var paths client.Paths
		flags.StringVar(&paths.Address, "address", "", "owned Unix socket")
		flags.StringVar(&paths.Inputs, "inputs", "", "private immutable input root")
		flags.StringVar(&paths.Output, "output", "", "private empty export staging")
		timeoutMillis := flags.Uint64("timeout-ms", 0, "remaining operation budget")
		if err := flags.Parse(arguments[1:]); err != nil {
			return err
		}
		if flags.NArg() != 0 || *timeoutMillis == 0 || *timeoutMillis > uint64(math.MaxInt64/int64(time.Millisecond)) {
			return fmt.Errorf("execute requires a representable positive remaining deadline")
		}
		ctx, cancel := context.WithTimeout(ctx, time.Duration(*timeoutMillis)*time.Millisecond)
		defer cancel()
		request, err := protocol.ReadFrame[protocol.ExecuteRequest](input)
		if err != nil {
			return err
		}
		if err := request.Identity.Validate(); err != nil {
			return err
		}
		if err := request.Validate(); err != nil {
			return failure(output, request.Identity, &request.Worker, "rejected", err)
		}
		if err := client.Execute(ctx, request, paths, output); err != nil {
			return failure(output, request.Identity, &request.Worker, "export_failed", err)
		}
		return nil
	default:
		return fmt.Errorf("unknown bridge operation %q", arguments[0])
	}
}

func main() {
	if err := serve(os.Args[1:], os.Stdin, os.Stdout); err != nil {
		// Quote controls; arbitrary request/worker bytes never become terminal
		// escape sequences. Structured failures travel on stdout instead.
		fmt.Fprintf(os.Stderr, "bridge: %q\n", err.Error())
		os.Exit(1)
	}
}
