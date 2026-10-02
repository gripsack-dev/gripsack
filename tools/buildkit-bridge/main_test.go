package main

import (
	"bytes"
	"encoding/binary"
	"testing"

	"github.com/moby/buildkit/solver/pb"
	"gripsack.dev/buildkit-bridge/protocol"
)

func requestFrame(body string) []byte {
	frame := make([]byte, 8+len(body))
	binary.LittleEndian.PutUint64(frame, uint64(len(body)))
	copy(frame[8:], body)
	return frame
}

func TestLoweringKeepsCopyEdgesAndSelectedOutput(t *testing.T) {
	request := `{"protocol_version":3,"session":"copy-graph","attempt":1,"epoch":1,"plan":{"platform":{"os":"linux","architecture":"amd64"},"nodes":[{"kind":"directory","input":null,"path":"/source","mode":493},{"kind":"file","input":0,"path":"/source/value","data":"a2VwdC1ieXRlcw==","mode":292},{"kind":"copy","input":null,"source":1,"source_path":"/source/value","destination":"/result","contents":false}],"root":2,"exporter":{"kind":"local"}}}`
	var output bytes.Buffer
	if err := serve([]string{"lower"}, bytes.NewReader(requestFrame(request)), &output); err != nil {
		t.Fatal(err)
	}
	response, err := protocol.ReadFrame[protocol.Response](&output)
	if err != nil {
		t.Fatal(err)
	}
	if response.Prepared == nil {
		t.Fatalf("valid graph was rejected: %+v", response.Failed)
	}
	lowered := response.Prepared.Lowered
	var definition pb.Definition
	if err := definition.UnmarshalVT(lowered.Definition); err != nil {
		t.Fatal(err)
	}
	if len(lowered.Witness) != 3 {
		t.Fatalf("declared producer coverage lost: %+v", lowered.Witness)
	}
	actual := make(map[string]*pb.Op, len(definition.Def))
	for _, bytes := range definition.Def {
		var op pb.Op
		if err := op.UnmarshalVT(bytes); err != nil {
			t.Fatal(err)
		}
		actual["sha256:"+protocol.Digest(bytes)] = &op
	}
	file := actual[lowered.Witness[1].Vertex]
	if file == nil || file.GetFile() == nil || len(file.Inputs) != 1 || file.Inputs[0].Digest != lowered.Witness[0].Vertex {
		t.Fatal("file lost its directory producer edge")
	}
	copyOp := actual[lowered.Witness[2].Vertex]
	if copyOp == nil || copyOp.GetFile() == nil || len(copyOp.GetFile().Actions) != 1 {
		t.Fatal("copy operation disappeared")
	}
	action := copyOp.GetFile().Actions[0]
	copy := action.GetCopy()
	if copy == nil || copy.Src != "/source/value" || copy.Dest != "/result" || copy.FollowSymlink || copy.AllowWildcard || copy.AttemptUnpackDockerCompatibility {
		t.Fatalf("copy selectors/policy changed: %+v", copy)
	}
	if action.Input != -1 || action.SecondaryInput != 0 || len(copyOp.Inputs) != 1 || copyOp.Inputs[0].Digest != lowered.Witness[1].Vertex {
		t.Fatal("copy selected the wrong source/input/output")
	}
	var terminal pb.Op
	if err := terminal.UnmarshalVT(definition.Def[len(definition.Def)-1]); err != nil {
		t.Fatal(err)
	}
	if len(terminal.Inputs) != 1 || terminal.Inputs[0].Digest != lowered.Witness[2].Vertex || terminal.Inputs[0].Index != 0 {
		t.Fatal("terminal no longer selects the admitted copy output")
	}
}

func TestExecuteCannotSubstituteDefinitionBeforeWorkerContact(t *testing.T) {
	request := `{"protocol_version":3,"session":"substitution","attempt":1,"epoch":1,"worker":{"instance":"1111111111111111111111111111111111111111111111111111111111111111","epoch":1},"definition":"eA==","definition_digest":"0000000000000000000000000000000000000000000000000000000000000000","exporter":{"kind":"local"},"exporter_digest":"0000000000000000000000000000000000000000000000000000000000000000","sources":[],"platform":{"os":"linux","architecture":"amd64"}}`
	var output bytes.Buffer
	args := []string{"execute", "--address", "unix:///does-not-exist", "--inputs", "/missing", "--output", "/missing", "--timeout-ms", "1000"}
	if err := serve(args, bytes.NewReader(requestFrame(request)), &output); err != nil {
		t.Fatal(err)
	}
	response, err := protocol.ReadFrame[protocol.Response](&output)
	if err != nil {
		t.Fatal(err)
	}
	if response.Failed == nil || response.Failed.Code != "rejected" || response.Accepted != nil || response.Done != nil {
		t.Fatalf("substituted bytes crossed admission: %+v", response)
	}
}
